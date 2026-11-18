import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type DragEvent,
  type RefObject,
} from 'react';
import { createDocxViewer, type DocxViewerHandle, type PageInfo } from '@meerrec/core';
import { publishViewer } from '../../lib/bridge.js';
import { csvRecords } from '../../lib/csv.js';
import { cacheStats, fileDocumentKey, getDocument, putDocument } from '../../lib/doc_cache.js';
import { downloadBlob, exportFileName } from '../../lib/download.js';
import { copyText, OOXML_MIME, pickFile, printPdf, readText, sharePdf } from '../../lib/file_actions.js';
import { fetchFixture, listFixtures } from '../../lib/fixtures.js';
import { useEditor } from '../../lib/use_editor.js';
import { useViewer } from '../../lib/use_viewer.js';
import type { EditorArgs, EditorResult } from '../../editor/types.js';
import {
  docxWorkerUrl,
  releaseAllWasmUrls,
  releaseDocxWasmUrls,
  releasePdfWasmUrls,
  resolveDocxWasm,
  resolvePdfWasm,
  warmDocxWasmCache,
  warmPdfWasmCache,
} from '../../lib/viewer_urls.js';
import { textFormats, type TextFormat } from './text_formats.js';

/** Виды экспорта: PDF плюс текстовые форматы — на всех одно состояние занятости. */
export type ExportKind = 'pdf' | TextFormat;

/** Положение в документе: номер видимой страницы и сколько их всего. */
interface Pager {
  index: number;
  count: number;
}

/** Состояние, которое страница получает от хука: данные плюс готовые обработчики. */
export interface DocxPageController {
  /** Хост вьюера — его вешают на `<main id="viewer" />`. */
  hostRef: RefObject<HTMLElement | null>;
  /** Ручка вьюера: нужна счётчикам кадров; `null`, пока вьюер не создан. */
  viewer: DocxViewerHandle | null;
  /** Текст строки состояния; сюда же попадает причина отказа вьюера. */
  status: string;
  /** Подпись открытого документа: имя выбранного файла или путь фикстуры. */
  label: string | null;
  /** Номер страницы для подвала: «1 / 3» или «—», пока документа нет. */
  pageInfo: string;
  /** Текст индикатора кэша: «кэш: N» с числом документов или «кэш: —». */
  cacheLabel: string;
  /** Сколько страниц в документе; `0` — документа нет. */
  pageCount: number;
  canPrev: boolean;
  canNext: boolean;
  /** Вьюер поднят: без него нажимать нечего. */
  ready: boolean;
  /** Документ открыт: есть что экспортировать и по чему ходить. */
  opened: boolean;
  /** Текст поля `#edit-run`: правка run'а, выбранного кликом по тексту. */
  editText: string;
  /** Подпись выбранного run'а («абзац N, run M») либо `null` — ничего не выбрано. */
  editTarget: string | null;
  /** Запомнить ввод в поле `#edit-run` (controlled input). */
  onEditText: (text: string) => void;
  /**
   * Применить правку из поля `#edit-run` к выбранному кликом run'у: воркер
   * перераскладывает документ, кадр и доступный слой обновляются по ответу.
   */
  onEditRun: () => void;
  /** Документ не открыт — править нечего. */
  editDisabled: boolean;
  zoom: number;
  setZoom: (zoom: number) => void;
  fixtures: string[];
  fixture: string;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onFixtureChange: (event: ChangeEvent<HTMLSelectElement>) => void;
  onDragOver: (event: DragEvent<HTMLElement>) => void;
  onDragLeave: (event: DragEvent<HTMLElement>) => void;
  onDrop: (event: DragEvent<HTMLElement>) => void;
  /** Перейти на страницу со сдвигом от текущей. */
  stepPage: (delta: number) => void;
  /** Запустить экспорт: страницу вьюера отпускает, дальше он сам. */
  exportDocument: (kind: ExportKind) => void;
  /** Недоступна ли кнопка экспорта: без документа экспортировать нечего,
      а нажатая кнопка ждёт вьюера. */
  exportDisabled: (kind: ExportKind) => boolean;
  /** Любая команда ленты, включая операции editor-worker и режимы вида. */
  onCommand: (command: string, args?: EditorArgs) => void;
  onUndo: () => void;
  onRedo: () => void;
  onSave: () => void;
  canUndo: boolean;
  canRedo: boolean;
  busyLabel: string | null;
  reading: boolean;
  showRuler: boolean;
  showGrid: boolean;
  showParagraphMarks: boolean;
  dialog: 'help' | 'pen' | 'marker' | 'eraser' | 'spellcheck' | null;
  closeDialog: () => void;
  insertDrawing: (bytes: ArrayBuffer, width: number, height: number) => Promise<void>;
  applySpellcheck: (text: string) => Promise<void>;
}

/** Сообщение об ошибке: у исключения оно своё, у остального — как есть. */
function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Вся работа DOCX-страницы: вьюер, открытие документа, пагинация, зум, фикстуры
 * и экспорт. Страница остаётся каркасом Office и контрактом id, а состояние и
 * императивные вызовы вьюера живут здесь.
 */
export function useDocxPage(): DocxPageController {
  const [status, setStatus] = useState('файл не открыт');
  const [pager, setPager] = useState<Pager>({ index: 0, count: 0 });
  const [label, setLabel] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [fixtures, setFixtures] = useState<string[]>([]);
  const [fixture, setFixture] = useState('');
  const [busy, setBusy] = useState<ExportKind | 'editor' | 'print' | 'share' | null>(null);
  const editor = useEditor();
  const [reading, setReading] = useState(false);
  const [showRuler, setShowRuler] = useState(true);
  const [showGrid, setShowGrid] = useState(false);
  const [showParagraphMarks, setShowParagraphMarks] = useState(false);
  const [dialog, setDialog] = useState<'help' | 'pen' | 'marker' | 'eraser' | 'spellcheck' | null>(null);
  const formatPainterRef = useRef(false);
  const spellTextRef = useRef('');
  /** Ввод поля `#edit-run`: правка run'а, выбранного кликом по тексту. */
  const [editText, setEditText] = useState('');
  /** Подпись выбранного run'а; `null` — кликом ничего не выбрано. */
  const [editTarget, setEditTarget] = useState<string | null>(null);
  /** Run под курсором: его адрес правка несёт воркеру; живёт ref'ом, а не
      состоянием — клик не обязан перерисовывать страницу. */
  const editAnchorRef = useRef<{ paragraph: number; run: number } | null>(null);

  // Вьюер создаётся один раз за жизнь страницы и публикуется для сквозных
  // тестов: их мост к странице — `window.docConverter`.
  const { hostRef, viewer, error } = useViewer(
    async (host) =>
      createDocxViewer(host, {
        workerUrl: docxWorkerUrl,
        // Резолверы только читают кэш: за прогретые байты отдаётся blob-URL, а
        // при промахе воркер пойдёт по сети сам — в момент открытия документа
        // или первого экспорта, как и раньше.
        docxWasm: await resolveDocxWasm(),
        pdfWasm: await resolvePdfWasm(),
        // Сквозные тесты и ручная проверка NVDA/VoiceOver включают слой
        // параметром `?a11y=1`; по умолчанию он выключен.
        ...(new URLSearchParams(window.location.search).has('a11y') ? { a11y: true } : {}),
        // Клик по тексту доступного слоя заполняет поле `#edit-run` адресом
        // run'а: правка текста живёт на этом подписчике, а не на опросе DOM.
        onTextRunClick: (run) => {
          if (run.paragraphIndex === null || run.runIndex === null) {
            setStatus('этот текст не правится: таблица или колонтитул');
            return;
          }
          editAnchorRef.current = { paragraph: run.paragraphIndex, run: run.runIndex };
          const target = `абзац ${run.paragraphIndex + 1}, run ${run.runIndex + 1}`;
          setEditTarget(target);
          setEditText(run.text);
          setStatus(`выбран ${target}: Enter в поле «править текст» применяет правку`);
        },
      }),
    (handle) => {
      publishViewer({ kind: 'docx', viewer: handle });
    },
  );

  /** Индикатор кэша: число документов в хранилище; `—` — хранилища нет. */
  const [cacheLabel, setCacheLabel] = useState('кэш: —');
  const refreshCache = useCallback(async () => {
    const stats = await cacheStats();
    setCacheLabel(stats === null ? 'кэш: —' : `кэш: ${stats.docs}`);
  }, []);

  // Мерки читаются и на входе на страницу — хранилище живёт дольше вкладки, и
  // в нём уже может что-то лежать от прошлого раза.
  useEffect(() => {
    void refreshCache();
  }, [refreshCache]);

  // Уходя со страницы, отдаём blob-URL-ы: воркер к этому времени уничтожен, и
  // никто, кроме набора в модуле URL-ов, на них не ссылается.
  useEffect(() => releaseAllWasmUrls, []);

  /** Открыть документ и отчитаться в строке состояния; отказ — тоже отчёт. */
  const openDocument = useCallback(
    async (bytes: ArrayBuffer, name: string, fromEditor = false) => {
      if (viewer === null) return;
      setStatus(`разбираю ${name}…`);
      const started = performance.now();
      let pages: PageInfo[];
      try {
        if (!fromEditor) await editor.open('docx', bytes.slice(0), name);
        pages = await viewer.open(bytes);
      } catch (e) {
        // Документ не открылся — экспортировать нечего, а подпись потеряла бы
        // связь с содержимым.
        setLabel(null);
        setStatus(`не открылось: ${errorText(e)}`);
        return;
      }
      setLabel(name);
      setPager({ index: viewer.pageIndex, count: viewer.pageCount });
      // Новый документ — новая нумерация абзацев: адрес старой правки гасится.
      editAnchorRef.current = null;
      setEditTarget(null);
      setEditText('');
      const elapsed = Math.round(performance.now() - started);
      setStatus(`${name}: ${pages.length} стр. за ${elapsed} мс`);
      // Документ открылся — значит, воркер уже загрузил DOCX-модуль: его байты
      // можно положить в кэш (URL-ы к этому моменту в HTTP-кэше браузера), а
      // выданные воркеру blob-URL-ы отозвать: `import` больше не случится.
      releaseDocxWasmUrls();
      void warmDocxWasmCache();
      // К этому моменту байты документа уже в кэше (фикстуру кладёт
      // `fetchFixture`), а файл пользователя — вызывающий: индикатору остаётся
      // их посчитать.
      void refreshCache();
    },
    [viewer, refreshCache, editor.open],
  );

  /** Открыть выбранный файл: байты читает main, разбирает воркер. */
  const openFile = useCallback(
    async (file: File) => {
      try {
        // Ключ строится по метаданным файла, до чтения: повторное открытие того
        // же файла не читает диск, а правка файла меняет ключ — старое из кэша
        // не подменит новое.
        const key = fileDocumentKey(file);
        const cached = await getDocument(key);
        const bytes = cached ?? (await file.arrayBuffer());
        // Копия в базе должна лечь до `openDocument`: тот вправе передать буфер
        // воркеру переносом, и тогда записывать было бы уже нечего.
        if (cached === null) await putDocument(key, bytes);
        await openDocument(bytes, file.name);
      } catch (e) {
        // Чтение файла тоже может сорваться (файл исчез с диска) — исключение
        // из обработчика не должно ронять страницу.
        setStatus(`не открылось: ${errorText(e)}`);
      }
    },
    [openDocument],
  );

  /** Загрузить фикстуру и открыть её; 404 — это сообщение, а не падение. */
  const loadFixture = useCallback(
    async (name: string) => {
      setStatus(`загружаю ${name}…`);
      let bytes: ArrayBuffer;
      try {
        bytes = await fetchFixture('docx', name);
      } catch (e) {
        // У ненайденной фикстуры своё сообщение, и оно уже сложено в ошибке.
        setStatus(errorText(e));
        return;
      }
      await openDocument(bytes, name);
    },
    [openDocument],
  );

  // Список фикстур — удобство разработки: без него документ выбирают файлом
  // или перетаскивают, поэтому недоступный список — не ошибка страницы.
  useEffect(() => {
    let alive = true;
    void listFixtures('docx').then((names) => {
      if (alive) setFixtures(names);
    });
    return () => {
      alive = false;
    };
  }, []);

  // Номер страницы вьюер ведёт сам — он меняется и от прокрутки, поэтому
  // счётчик идёт за кадрами, а не только за кнопками. Состояние обновляется,
  // лишь когда значение изменилось: иначе страница перерисовывалась бы на
  // каждом кадре вхолостую.
  useEffect(() => {
    if (viewer === null) return undefined;
    return viewer.onTick(() => {
      // Ручку читаем до апдейтера: внутри `setPager` императивных вызовов нет.
      const index = viewer.pageIndex;
      const count = viewer.pageCount;
      setPager((prev) => (prev.index === index && prev.count === count ? prev : { index, count }));
    });
  }, [viewer]);

  // Зум — состояние страницы, а вьюер узнаёт о нём эффектом: так выбранное
  // значение не теряется, пока вьюер создаётся, и переживает его отказ.
  useEffect(() => {
    if (viewer !== null && viewer.zoom !== zoom) viewer.setZoom(zoom);
  }, [viewer, zoom]);

  const onFileChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      if (file) void openFile(file);
    },
    [openFile],
  );

  const onFixtureChange = useCallback(
    (event: ChangeEvent<HTMLSelectElement>) => {
      const name = event.target.value;
      setFixture(name);
      if (name) void loadFixture(name);
    },
    [loadFixture],
  );

  const onDragOver = useCallback((event: DragEvent<HTMLElement>) => {
    // Без preventDefault браузер не даст отпустить файл над элементом.
    event.preventDefault();
    event.currentTarget.classList.add('dragover');
  }, []);

  const onDragLeave = useCallback((event: DragEvent<HTMLElement>) => {
    event.currentTarget.classList.remove('dragover');
  }, []);

  const onDrop = useCallback(
    (event: DragEvent<HTMLElement>) => {
      event.preventDefault();
      event.currentTarget.classList.remove('dragover');
      const file = event.dataTransfer.files[0];
      if (file) void openFile(file);
    },
    [openFile],
  );

  const stepPage = useCallback(
    (delta: number) => {
      const active = viewer;
      if (active === null) return;
      active.showPage(active.pageIndex + delta);
      // Прокрутка встала на новую страницу — счётчику ждать кадра незачем.
      setPager({ index: active.pageIndex, count: active.pageCount });
    },
    [viewer],
  );

  const runExport = useCallback(
    async (kind: ExportKind) => {
      const active = viewer;
      if (active === null || label === null) return;
      setBusy(kind);
      setStatus(kind === 'pdf' ? 'экспортирую PDF…' : textFormats[kind].progress);
      try {
        if (kind === 'pdf') {
          const bytes = await active.exportPdf();
          const name = exportFileName(label, 'pdf');
          // Копия в Uint8Array над обычным ArrayBuffer: тип байтов из воркера
          // допускает SharedArrayBuffer, а `BlobPart` — нет; заодно гарантирован
          // сдвиг 0.
          downloadBlob(new Uint8Array(bytes), name, 'application/pdf');
          setStatus(`PDF готов: ${name}, ${Math.round(bytes.byteLength / 1024)} КБ`);
          // Экспорт прошёл — PDF-модуль в воркере загружен: кладём его байты в
          // кэш и отпускаем blob-URL-ы (см. открытие документа выше).
          releasePdfWasmUrls();
          void warmPdfWasmCache();
        } else {
          const spec = textFormats[kind];
          const text = await spec.run(active);
          const name = exportFileName(label, spec.extension);
          downloadBlob(text, name, spec.mime);
          setStatus(`${spec.title} готов: ${name}`);
        }
      } catch (e) {
        // Ошибку показываем в строке состояния: исключение из обработчика не
        // должно ронять страницу.
        setStatus(`экспорт не удался: ${errorText(e)}`);
      } finally {
        setBusy(null);
      }
    },
    [viewer, label],
  );

  const exportDocument = useCallback(
    (kind: ExportKind) => {
      void runExport(kind);
    },
    [runExport],
  );

  /** Выполнить команду и отдать новые байты движку: canvas/PDF/файл не расходятся. */
  const runEditor = useCallback(async (command: string, args: EditorArgs = {}): Promise<EditorResult | null> => {
    if (viewer === null || label === null) {
      setStatus('сначала откройте документ');
      return null;
    }
    setBusy('editor');
    try {
      const result = await editor.execute(command, args);
      if (result.bytes) await openDocument(result.bytes, label, true);
      setStatus(result.message ?? `команда выполнена: ${command}`);
      return result;
    } catch (e) {
      setStatus(`команда не выполнена: ${errorText(e)}`);
      return null;
    } finally {
      setBusy(null);
    }
  }, [viewer, label, editor.execute, openDocument]);

  const restore = useCallback((direction: 'undo' | 'redo'): void => {
    if (viewer === null || label === null) return;
    setBusy('editor');
    void (direction === 'undo' ? editor.undo() : editor.redo())
      .then(async (result) => {
        if (result.bytes) await openDocument(result.bytes, label, true);
        setStatus(direction === 'undo' ? 'изменение отменено' : 'изменение повторено');
      })
      .catch((e: unknown) => setStatus(`история недоступна: ${errorText(e)}`))
      .finally(() => setBusy(null));
  }, [viewer, label, editor.undo, editor.redo, openDocument]);

  const saveDocument = useCallback((): void => {
    if (label === null) {
      setStatus('сначала откройте документ');
      return;
    }
    setBusy('editor');
    void editor.save().then((result) => {
      if (!result.bytes) throw new Error('редактор не вернул байты');
      const name = exportFileName(label, 'docx');
      downloadBlob(result.bytes, name, OOXML_MIME.docx);
      setStatus(`DOCX сохранён: ${name}`);
    }).catch((e: unknown) => setStatus(`сохранение не удалось: ${errorText(e)}`))
      .finally(() => setBusy(null));
  }, [label, editor.save]);

  const selection = useCallback((): EditorArgs | null => {
    const anchor = editAnchorRef.current;
    if (anchor) return anchor;
    setStatus('кликните по тексту документа — цели форматирования');
    return null;
  }, []);

  const runExportPdf = useCallback(async (kind: 'print' | 'share'): Promise<void> => {
    if (viewer === null || label === null) return;
    setBusy(kind);
    try {
      const bytes = await viewer.exportPdf();
      if (kind === 'print') {
        printPdf(bytes);
        setStatus('PDF передан в системный диалог печати');
      } else {
        const result = await sharePdf(bytes, label);
        setStatus(result === 'shared' ? 'PDF передан в меню «Поделиться»' : 'Web Share недоступен — PDF скачан');
      }
    } catch (e) {
      setStatus(`${kind === 'print' ? 'печать' : 'sharing'} не удалась: ${errorText(e)}`);
    } finally {
      setBusy(null);
    }
  }, [viewer, label]);

  const insertDrawing = useCallback(async (bytes: ArrayBuffer, width: number, height: number): Promise<void> => {
    const result = await runEditor('insert-drawing', { bytes, mime: 'image/png', width, height, name: 'drawing.png' });
    if (result) setDialog(null);
  }, [runEditor]);

  const applySpellcheck = useCallback(async (text: string): Promise<void> => {
    const target = selection();
    if (!target) return;
    const result = await runEditor('set-run', { ...target, text });
    if (result) {
      setEditText(text);
      setDialog(null);
    }
  }, [runEditor, selection]);

  /** Все команды ленты; пользовательский ввод запрашивается только там, где он нужен формату. */
  const onCommand = useCallback((command: string, args: EditorArgs = {}): void => {
    void (async () => {
      if (command === 'create') {
        const bytes = await fetchFixture('docx', 'simple/one_paragraph.docx');
        await openDocument(bytes, 'Документ1.docx');
        await runEditor('new-document');
        return;
      }
      if (command === 'save') return saveDocument();
      if (command === 'print' || command === 'share') return runExportPdf(command);
      if (command === 'undo' || command === 'redo') return restore(command);
      if (command === 'help') return setDialog('help');
      if (command === 'feedback') {
        window.open('https://github.com/meerrec/wasm-converter/issues/new', '_blank', 'noopener,noreferrer');
        return;
      }
      if (command === 'reading') return setReading(true);
      if (command === 'page-layout') return setReading(false);
      if (command === 'ruler') return setShowRuler((value) => !value);
      if (command === 'grid') return setShowGrid((value) => !value);
      if (command === 'paragraph-marks') return setShowParagraphMarks((value) => !value);
      if (command === 'pen' || command === 'marker' || command === 'eraser') return setDialog(command);
      if (command === 'copy' || command === 'cut') {
        const selectedText = window.getSelection()?.toString() ?? '';
        let text = selectedText;
        if (!text) {
          const target = selection();
          if (!target) return;
          const result = await editor.execute('get-run', target);
          const value = result.value as { text?: string } | string | undefined;
          text = typeof value === 'string' ? value : value?.text ?? '';
        }
        await copyText(text);
        setStatus(`скопировано символов: ${text.length}`);
        if (command === 'cut') {
          const target = selection();
          if (target) await runEditor('set-run', { ...target, text: '' });
        }
        return;
      }
      if (command === 'paste') {
        const target = selection();
        if (!target) return;
        const text = await readText();
        await runEditor('set-run', { ...target, text });
        setEditText(text);
        return;
      }
      if (command === 'format-painter') {
        const target = selection();
        if (!target) return;
        const apply = formatPainterRef.current;
        const result = await runEditor(apply ? 'paste-format' : 'copy-format', target);
        if (result) {
          formatPainterRef.current = !apply;
          setStatus(apply ? 'формат применён' : 'формат скопирован — выберите другой run и нажмите снова');
        }
        return;
      }
      if (command === 'find') {
        const text = window.prompt('Найти текст', editText);
        if (!text) return;
        const result = await runEditor('find', { text });
        const match = result?.value as { paragraph?: number; run?: number; count?: number } | undefined;
        if (match?.paragraph !== undefined && match.run !== undefined) {
          editAnchorRef.current = { paragraph: match.paragraph, run: match.run };
          setEditTarget(`абзац ${match.paragraph + 1}, run ${match.run + 1}`);
          const layer = await viewer?.getFullTextLayer();
          const run = layer?.runs.find((item) => item.paragraphIndex === match.paragraph && item.runIndex === match.run);
          if (run) viewer?.showPage(run.page);
        }
        setStatus(`найдено совпадений: ${match?.count ?? 0}`);
        return;
      }
      if (command === 'replace') {
        const find = window.prompt('Найти текст', editText);
        if (!find) return;
        const replacement = window.prompt('Заменить на', '');
        if (replacement === null) return;
        await runEditor('replace', { find, replacement, all: true });
        return;
      }
      if (command === 'select-all') {
        const root = hostRef.current?.querySelector('.dc-text-semantic');
        const selectionObject = window.getSelection();
        if (root && selectionObject) {
          selectionObject.removeAllRanges();
          const range = document.createRange();
          range.selectNodeContents(root);
          selectionObject.addRange(range);
          setStatus('выделен весь текст документа');
        }
        return;
      }
      if (command === 'spellcheck') {
        const target = selection();
        if (!target) return;
        const result = await editor.execute('get-run', target);
        const value = result.value as { text?: string } | string | undefined;
        spellTextRef.current = typeof value === 'string' ? value : value?.text ?? '';
        setDialog('spellcheck');
        return;
      }
      if (command === 'insert-picture') {
        const file = await pickFile('image/*');
        if (!file) return;
        const bitmap = await createImageBitmap(file);
        try {
          await runEditor('insert-picture', {
            bytes: await file.arrayBuffer(), mime: file.type || 'image/png', width: bitmap.width, height: bitmap.height, name: file.name,
          });
        } finally {
          bitmap.close();
        }
        return;
      }
      if (command === 'insert-table') {
        const rows = Number(window.prompt('Число строк', '3'));
        const cols = Number(window.prompt('Число столбцов', '3'));
        if (rows > 0 && cols > 0) await runEditor(command, { rows, cols });
        return;
      }
      if (command === 'insert-link') {
        const url = window.prompt('Адрес ссылки', 'https://');
        if (!url) return;
        const text = window.prompt('Текст ссылки', url) ?? url;
        await runEditor(command, { url, text });
        return;
      }
      if (command === 'mail-recipients') {
        const file = await pickFile('.csv,text/csv');
        if (!file) return;
        const records = csvRecords(await file.text());
        if (records.length === 0) throw new Error('в CSV нет получателей');
        await runEditor('mail-merge', { records });
        return;
      }
      if (command === 'mail-start') {
        const target = selection();
        if (!target) return;
        await runEditor('set-run', { ...target, text: 'Уважаемый(ая) {{Имя}}!\n\n{{Должность}}, {{Компания}}' });
        setStatus('шаблон слияния вставлен; выберите CSV на вкладке «Рассылки»');
        return;
      }
      if (command === 'page-margins') {
        const value = window.prompt('Поля сверху, справа, снизу, слева (мм)', '20,20,20,20');
        if (!value) return;
        const [top = 20, right = 20, bottom = 20, left = 20] = value.split(',').map(Number);
        await runEditor(command, { top, right, bottom, left });
        return;
      }
      if (command === 'page-orientation') {
        const value = window.prompt('Ориентация: portrait или landscape', 'landscape');
        if (value === 'portrait' || value === 'landscape') await runEditor(command, { value });
        return;
      }
      if (command === 'page-size') {
        const value = window.prompt('Размер: A4, A3, Letter или Legal', 'A4');
        if (value) await runEditor(command, { value });
        return;
      }
      if (command === 'theme' || command === 'style-set') {
        const value = window.prompt(command === 'theme' ? 'Цвет темы RRGGBB' : 'Набор стилей', command === 'theme' ? '0F6CBD' : 'modern');
        if (value) await runEditor(command, { value });
        return;
      }
      if (command === 'insert-cover' || command === 'insert-chart' || command === 'insert-caption' || command === 'insert-cross-ref' || command === 'add-comment') {
        const value = window.prompt('Текст', command === 'insert-cover' ? 'Новый документ' : '');
        if (value === null) return;
        await runEditor(command, command === 'add-comment' ? { ...(selection() ?? {}), text: value } : command === 'insert-cross-ref' ? { target: value } : { text: value, title: value });
        return;
      }
      if (command === 'indent-left' || command === 'indent-right') {
        const target = selection();
        if (target) await runEditor('indent', { ...target, value: command === 'indent-left' ? 'left' : 'right', delta: 360 });
        return;
      }
      if (['font-family', 'font-size', 'text-color', 'highlight', 'toggle-bold', 'toggle-italic', 'toggle-underline', 'toggle-strikethrough', 'toggle-subscript', 'toggle-superscript', 'bullets', 'numbering', 'multilevel-list', 'line-spacing', 'align', 'style', 'copy-format', 'paste-format'].includes(command)) {
        const target = selection();
        if (target) await runEditor(command, { ...target, ...args });
        return;
      }
      await runEditor(command, args);
    })().catch((e: unknown) => setStatus(`команда не выполнена: ${errorText(e)}`));
  }, [openDocument, runEditor, saveDocument, runExportPdf, restore, selection, editor.execute, editText, viewer, hostRef]);

  /** Запомнить ввод поля `#edit-run` — значение живёт состоянием React. */
  const onEditText = useCallback((text: string) => {
    setEditText(text);
  }, []);

  /**
   * Применить правку из поля `#edit-run`: цель — run, выбранный кликом по
   * тексту. Воркер перераскладывает документ, страницы могут измениться, и
   * вьюер по ответу перестраивает стопку, кадр и доступный слой.
   */
  const onEditRun = useCallback(() => {
    const active = viewer;
    if (active === null) {
      setStatus('вьюер не поднялся — править нечего');
      return;
    }
    const anchor = editAnchorRef.current;
    if (anchor === null) {
      setStatus('кликните по тексту документа, который правите');
      return;
    }
    void runEditor('set-run', { ...anchor, text: editText }).then((result) => {
      if (result) setStatus(`правка применена: ${editTarget ?? 'выбранный run'}`);
    });
  }, [viewer, editText, editTarget, runEditor]);

  const ready = viewer !== null;
  const opened = ready && label !== null;
  const pageInfo = pager.count === 0 ? '—' : `${pager.index + 1} / ${pager.count}`;

  return {
    hostRef,
    viewer,
    // Причина отказа вьюера важнее последнего сообщения: без вьюера не
    // работает ничего, и строка состояния обязана это назвать.
    status: error !== null ? error : status,
    label,
    pageInfo,
    cacheLabel,
    pageCount: pager.count,
    canPrev: pager.count > 0 && pager.index > 0,
    canNext: pager.count > 0 && pager.index < pager.count - 1,
    ready,
    opened,
    editText,
    editTarget,
    onEditText,
    onEditRun,
    editDisabled: !opened,
    zoom,
    setZoom,
    fixtures,
    fixture,
    onFileChange,
    onFixtureChange,
    onDragOver,
    onDragLeave,
    onDrop,
    stepPage,
    exportDocument,
    exportDisabled: (kind: ExportKind) => !opened || busy === kind,
    onCommand,
    onUndo: () => restore('undo'),
    onRedo: () => restore('redo'),
    onSave: saveDocument,
    canUndo: editor.canUndo,
    canRedo: editor.canRedo,
    busyLabel: busy === 'editor' ? 'Применяю изменения…' : busy === 'print' ? 'Готовлю печать…' : busy === 'share' ? 'Готовлю PDF…' : null,
    reading,
    showRuler,
    showGrid,
    showParagraphMarks,
    dialog,
    closeDialog: () => setDialog(null),
    insertDrawing,
    applySpellcheck,
  };
}
