import {
  useCallback,
  useEffect,
  useState,
  type ChangeEvent,
  type DragEvent,
  type RefObject,
} from 'react';
import { createDocxViewer, type DocxViewerHandle, type PageInfo } from '@meerrec/core';
import { publishViewer } from '../../lib/bridge.js';
import { cacheStats, fileDocumentKey, getDocument, putDocument } from '../../lib/doc_cache.js';
import { downloadBlob, exportFileName } from '../../lib/download.js';
import { fetchFixture, listFixtures } from '../../lib/fixtures.js';
import { useViewer } from '../../lib/use_viewer.js';
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
  const [busy, setBusy] = useState<ExportKind | null>(null);

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
    async (bytes: ArrayBuffer, name: string) => {
      if (viewer === null) return;
      setStatus(`разбираю ${name}…`);
      const started = performance.now();
      let pages: PageInfo[];
      try {
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
    [viewer, refreshCache],
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
  };
}
