import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type RefObject,
} from 'react';
import { createEditorClient, describeEditorError } from '@meerrec/eurooffice';
import type { DocumentErrorEvent, EditorClient, EditorEvent } from '@meerrec/eurooffice';
import { euroofficeBaseUrl, euroofficeEditorOrigin, euroofficeEditorUrl } from '../../lib/viewer_urls.js';
import { supportsCredentiallessFrames } from '../../lib/isolation.js';

/**
 * Логика страницы «Euro-Office»: оболочка вокруг iframe редактора — выбор файла
 * (кнопка и drag&drop), статус и отправка байтов в режиме только просмотра.
 *
 * Файл читает страница и отдаёт его редактору командой `document:open-buffer`:
 * так не нужен ни CORS, ни URL документа на сервере, а сохранение в редакторе
 * выключено (`readonly`). Протокол Embed API целиком живёт в пакете
 * `@meerrec/eurooffice` — здесь только состояние и хук приёмки.
 */

/** Состояния статуса страницы: те же значения, что читает приёмка. */
export type ViewerState = 'loading' | 'ready' | 'opening' | 'opened' | 'error';

/** Тестовый хук приёмки; странице больше ни для чего не нужен. */
export interface ViewerHarness {
  state: ViewerState;
  /** Растёт на каждый `document:opened` — признак «файл реально открылся». */
  openedCount: number;
  lastFileName: string;
  lastError: string | null;
}

declare global {
  interface Window {
    /**
     * Хук приёмки: счётчик открытий снимает гонку `data-state` (к этому моменту
     * состояние от прошлого файла ещё может висеть в `opened`).
     */
    euroofficeViewer?: ViewerHarness;
  }
}

/**
 * Сколько ждём `document:ready`, прежде чем сказать, что редактор не отвечает.
 * Хост сайта редактора в дев-режиме — отдельный процесс; не поднятый, он не
 * пришлёт `ready` никогда, и без подсказки страница молча висела бы в
 * «запускаю редактор…».
 */
const HOST_HINT_DELAY_MS = 15_000;

/** Подсказка про неподнятый хост: что именно запустить и на каком порту. */
const HOST_HINT =
  'Редактор не отвечает. Поднимите хост сайта: node scripts/eurooffice/serve.mjs (порт 5271)';

/** Расширения, которые редактор умеет открывать (accept-список и drag&drop). */
const ACCEPTED_EXTENSIONS: readonly string[] = [
  '.docx',
  '.doc',
  '.odt',
  '.rtf',
  '.txt',
  '.xlsx',
  '.xls',
  '.ods',
  '.csv',
  '.pptx',
  '.ppt',
  '.odp',
  '.pdf',
];

/** Список `accept` для поля выбора файла — тот же набор и в том же порядке. */
export const ACCEPT = ACCEPTED_EXTENSIONS.join(',');

/** Размер файла человекочитаемо — он идёт в статус «Открываю…». */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} Б`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} КиБ`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} МиБ`;
}

/** Расширение имени с точкой и в нижнем регистре; без расширения — пустая строка. */
function extensionOf(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  return dot < 0 ? '' : fileName.slice(dot).toLowerCase();
}

function isSupportedFile(file: File): boolean {
  return ACCEPTED_EXTENSIONS.includes(extensionOf(file.name));
}

/** Текст ошибки: у исключений он есть, у остальных значений — их запись строкой. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Сколько ждём активации корневого воркера. Дольше — значит, что-то не так на
 * стороне браузера, и рамке полезнее грузиться без изоляции, чем ждать её вечно.
 */
const SW_ACTIVATION_TIMEOUT_MS = 5_000;

/**
 * Поднять корневой воркер сайта (`<корень>/sw.js`): он один ставит документам
 * движка заголовки изоляции, которых на Pages взять негде (что именно и почему —
 * в `scripts/pages/sw.js`). Активации ждём потому, что воркер начинает
 * перехватывать навигации только с неё: ушедшая раньше рамка упрётся в COEP
 * родителя и не загрузится — воркер её уже не увидит.
 *
 * Отказ не фатален: в дев-режиме и в `vite preview` корневого `sw.js` не
 * существует, в браузере может не быть `serviceWorker` вовсе — страница обязана
 * остаться рабочей, просто без изоляции.
 */
async function ensureEngineIsolation(): Promise<void> {
  if (!('serviceWorker' in navigator)) return;
  // Тот же предикат, что в `ensureCrossOriginIsolation` (см.
  // `supportsCredentiallessFrames`): Firefox и Safari не впустят кадр редактора в
  // изолированную страницу. Здесь она важнее — воркер, поднятый на этой странице,
  // изолирует и следующий заход на неё (заголовки приезжают с ответом), и маршрут
  // умер бы на перезагрузке, хотя сейчас открылся бы. Без воркера рамка
  // выпускается сразу: неизолированный родитель кадр не проверяет, а вьюер уходит
  // на транспорт `bytes` (ADR-0021).
  if (!supportsCredentiallessFrames()) return;
  try {
    // `euroofficeBaseUrl` — тот же источник правды о корне сайта, что и у адреса
    // редактора: разъехаться адресу воркера и адресу документа нечем.
    const registration = await navigator.serviceWorker.register(`${euroofficeBaseUrl}sw.js`);
    // Повторный заход: воркер уже активен, ждать нечего.
    if (registration.active?.state === 'activated') return;
    const pending = registration.installing ?? registration.waiting;
    if (pending === null || pending.state === 'activated') return;
    // Сужение гвардией выше замыкания не наследуют — без этой строки тип снова
    // `ServiceWorker | null`.
    const worker: ServiceWorker = pending;
    await new Promise<void>((resolve) => {
      let timer = 0;
      const finish = (): void => {
        window.clearTimeout(timer);
        worker.removeEventListener('statechange', onStateChange);
        resolve();
      };
      function onStateChange(): void {
        // `redundant` — установка провалилась: состояния `activated` уже не будет.
        if (worker.state === 'activated' || worker.state === 'redundant') finish();
      }
      timer = window.setTimeout(finish, SW_ACTIVATION_TIMEOUT_MS);
      worker.addEventListener('statechange', onStateChange);
    });
  } catch {
    // 404 на sw.js, запрет регистрации, недоступное хранилище — рамка грузится
    // без изоляции, как грузилась бы без воркера вовсе.
  }
}

/** Текст ошибки редактора: сообщение из события плюс расшифровка кода. */
function formatEditorError(event: DocumentErrorEvent): string {
  const message = event.payload.message ?? 'неизвестная';
  const code = event.payload.code;
  if (code === undefined) return `Ошибка: ${message}`;
  return `Ошибка: ${message} (код ${code}: ${describeEditorError(code)})`;
}

/** Что страница берёт у хука: состояние оболочки и обработчики. */
export interface EuroofficePageApi {
  /** Iframe редактора: клиент Embed API шлёт команды его `contentWindow`. */
  frameRef: RefObject<HTMLIFrameElement | null>;
  /**
   * Адрес рамки; `undefined` — рамку ещё не выпускают за редактором. Проставляется
   * из хука: в проде — после активации корневого воркера (см. `frameSrc`).
   */
  frameSrc: string | undefined;
  /** Скрытое поле выбора файла: кнопка тулбара открывает системный диалог. */
  fileInputRef: RefObject<HTMLInputElement | null>;
  state: ViewerState;
  /** Строка статуса; состояние несёт `data-state`, текст — эта строка. */
  statusText: string;
  /** Имя документа по центру тулбара; `—` — документ ещё не выбирали. */
  docName: string;
  /** Файл тащат над окном — оверлей поверх области просмотра видим. */
  dropActive: boolean;
  onOpenClick: () => void;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
}

/** Хук страницы «Euro-Office»: клиент Embed API, выбор файла и drag&drop. */
export function useEuroofficePage(): EuroofficePageApi {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const clientRef = useRef<EditorClient | null>(null);

  /** Хук приёмки живёт весь срок страницы: React перерисовывает, он — нет. */
  const harnessRef = useRef<ViewerHarness>({
    state: 'loading',
    openedCount: 0,
    lastFileName: '',
    lastError: null,
  });
  /** Выбор файла до `document:ready`: он ждёт здесь и открывается по готовности. */
  const pendingFileRef = useRef<File | null>(null);
  const readyRef = useRef(false);
  /** Имя последнего выбранного файла: редактор может не вернуть своё в событии. */
  const lastFileNameRef = useRef('');

  /**
   * Адрес рамки, пока воркер изоляции не встал, — `undefined`: атрибут `src`
   * тогда не выставляется вовсе, и рамка никуда не уходит. Так снимается гонка
   * первого захода — навигация кадра обязана уйти после активации воркера.
   */
  const [frameSrc, setFrameSrc] = useState<string | undefined>(undefined);
  const [state, setState] = useState<ViewerState>('loading');
  const [statusText, setStatusText] = useState('запускаю редактор…');
  const [docName, setDocName] = useState('—');
  const [dropActive, setDropActive] = useState(false);

  /**
   * Статус меняется только здесь: `data-state`, строка и хук приёмки идут
   * одной записью — разъехаться им нечем. Запись в хук синхронная: приёмка
   * читает его сразу после события, не дожидаясь перерисовки.
   */
  const setStatus = useCallback((next: ViewerState, text: string): void => {
    harnessRef.current.state = next;
    setState(next);
    setStatusText(text);
  }, []);

  const openFile = useCallback(
    async (file: File): Promise<void> => {
      lastFileNameRef.current = file.name;
      setDocName(file.name);
      try {
        const buffer = await file.arrayBuffer();
        // Размер известен только после чтения — он и есть содержимое статуса.
        setStatus('opening', `Открываю «${file.name}» (${formatSize(buffer.byteLength)})…`);
        clientRef.current?.openBuffer(file.name, buffer, true);
      } catch (error: unknown) {
        setStatus('error', `Ошибка: не удалось прочитать файл — ${messageOf(error)}`);
      }
    },
    [setStatus],
  );

  /** Выбор файла: неподдержанный формат — понятная ошибка, до готовности — очередь. */
  const handlePicked = useCallback(
    (file: File): void => {
      if (!isSupportedFile(file)) {
        const extension = extensionOf(file.name);
        setStatus(
          'error',
          `Ошибка: формат «${extension === '' ? 'без расширения' : extension}» не поддержан`,
        );
        return;
      }
      if (readyRef.current) {
        void openFile(file);
        return;
      }
      pendingFileRef.current = file;
      setStatus('loading', `редактор ещё поднимается — «${file.name}» откроется сразу после готовности`);
    },
    [openFile, setStatus],
  );

  /** События редактора: готовность, открытие и ошибка — остального странице не нужно. */
  const handleEditorEvent = useCallback(
    (event: EditorEvent): void => {
      switch (event.type) {
        case 'document:ready': {
          readyRef.current = true;
          setStatus('ready', 'редактор готов — выберите документ');
          const queued = pendingFileRef.current;
          if (queued !== null) {
            pendingFileRef.current = null;
            void openFile(queued);
          }
          break;
        }
        case 'document:opened': {
          const harness = harnessRef.current;
          harness.openedCount += 1;
          harness.lastFileName = event.payload.fileName ?? lastFileNameRef.current;
          setDocName(harness.lastFileName);
          setStatus('opened', `«${harness.lastFileName}» — только просмотр`);
          break;
        }
        case 'document:error': {
          const text = formatEditorError(event);
          harnessRef.current.lastError = text;
          setStatus('error', text);
          break;
        }
        // saved/state/dirty-changed/readonly-changed в режиме просмотра не нужны —
        // их (и будущие `document:*` события) логирует клиент, чтобы не терять молча.
      }
    },
    [openFile, setStatus],
  );

  /**
   * Рамка идёт за редактором последней: сначала встаёт корневой воркер сайта,
   * который прикроет движок заголовками изоляции. В дев-режиме воркера нет —
   * редактор живёт на чужом хосте и без изоляции, — и рамка выпускается сразу.
   */
  useEffect(() => {
    let cancelled = false;
    const isolation = import.meta.env.PROD ? ensureEngineIsolation() : Promise.resolve();
    void isolation.finally(() => {
      if (!cancelled) setFrameSrc(euroofficeEditorUrl);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Клиент создаётся один раз на страницу: у него своя очередь команд до
  // `document:ready`, и пересоздание на перерисовке теряло бы её.
  useEffect(() => {
    const frame = frameRef.current;
    if (frame === null) return undefined;
    const client = createEditorClient(frame, { origin: euroofficeEditorOrigin });
    clientRef.current = client;
    const unsubscribe = client.onEvent(handleEditorEvent);
    window.euroofficeViewer = harnessRef.current;
    return () => {
      unsubscribe();
      clientRef.current = null;
      // Страница ушла — хук снимается: приёмка не должна видеть мёртвую оболочку.
      delete window.euroofficeViewer;
    };
  }, [handleEditorEvent]);

  // Пока тащим файл, поверх области просмотра лежит оверлей (см. eurooffice.css):
  // только так drop над iframe достаётся нам, а не документу редактора.
  useEffect(() => {
    /** В перетаскивании есть файл (а не, скажем, выделенный текст). */
    const dragHasFiles = (event: DragEvent): boolean =>
      event.dataTransfer?.types.includes('Files') ?? false;

    const onDragEnter = (event: DragEvent): void => {
      if (dragHasFiles(event)) setDropActive(true);
    };
    const onDragOver = (event: DragEvent): void => {
      if (!dragHasFiles(event)) return;
      // Без `preventDefault` браузер не считает элемент готовым принять drop.
      event.preventDefault();
      setDropActive(true);
    };
    const onDragLeave = (event: DragEvent): void => {
      // Ушли из окна совсем (а не в соседний элемент) — прячем оверлей.
      if (event.relatedTarget === null) setDropActive(false);
    };
    const onDrop = (event: DragEvent): void => {
      // Брошенный файл не должен открываться браузером как страница.
      event.preventDefault();
      setDropActive(false);
      const file = event.dataTransfer?.files[0];
      if (file !== undefined) handlePicked(file);
    };

    window.addEventListener('dragenter', onDragEnter);
    window.addEventListener('dragover', onDragOver);
    window.addEventListener('dragleave', onDragLeave);
    window.addEventListener('drop', onDrop);
    return () => {
      window.removeEventListener('dragenter', onDragEnter);
      window.removeEventListener('dragover', onDragOver);
      window.removeEventListener('dragleave', onDragLeave);
      window.removeEventListener('drop', onDrop);
    };
  }, [handlePicked]);

  // Подсказка про неподнятый хост — только в дев-режиме: в проде сайт редактора
  // лежит рядом со страницей, и «редактор не отвечает» там значит другую поломку.
  // Состояние остаётся `loading`: ждём всё то же `ready`, а не новое значение.
  useEffect(() => {
    if (!import.meta.env.DEV || state !== 'loading') return undefined;
    const timer = window.setTimeout(() => {
      setStatusText(HOST_HINT);
    }, HOST_HINT_DELAY_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [state]);

  const onOpenClick = useCallback((): void => {
    fileInputRef.current?.click();
  }, []);

  const onFileChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>): void => {
      const file = event.target.files?.[0];
      // Сброс значения: повторный выбор того же файла снова шлёт `change`.
      event.target.value = '';
      if (file !== undefined) handlePicked(file);
    },
    [handlePicked],
  );

  return {
    frameRef,
    frameSrc,
    fileInputRef,
    state,
    statusText,
    docName,
    dropActive,
    onOpenClick,
    onFileChange,
  };
}
