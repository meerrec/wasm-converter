import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type DragEvent as ReactDragEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react';
import {
  createXlsxViewer,
  type PdfOptions,
  type SheetInfo,
  type XlsxViewerHandle,
} from '@doc-converter/core';
import { clearViewer, publishViewer } from '../../lib/bridge.js';
import { downloadBlob } from '../../lib/download.js';
import { fetchFixture, listFixtures } from '../../lib/fixtures.js';
import { useViewer } from '../../lib/use_viewer.js';
import { pdfWasmBinaryUrl, pdfWasmModuleUrl, xlsxWorkerUrl } from '../../lib/viewer_urls.js';

/**
 * Логика страницы XLSX: вьюер книги и всё, что страница о нём знает.
 *
 * Вьюер императивный — он живёт в воркере и рисует холст сам, — поэтому поток
 * данных здесь односторонний. Команды уходят ему из обработчиков (открыть,
 * показать лист, зум, экспорт), настройки вида — эффектом, а обратно приходит
 * только то, что страница показала сама: результат `open` и подписи.
 * Состояния из кадров не заводится: метрики пишет `FrameMeters` мимо React, а
 * `setState` на каждом кадре перерисовывал бы страницу десятки раз в секунду.
 *
 * Тексты строки состояния — часть контракта сквозных тестов, поэтому собраны
 * здесь целиком, а не по месту вызова: так их видно рядом друг с другом.
 */

/** Имена колонок как в Excel: 0 → A, 26 → AA. */
function columnName(col: number): string {
  let name = '';
  let n = col + 1;
  while (n > 0) {
    const rem = (n - 1) % 26;
    name = String.fromCharCode(65 + rem) + name;
    n = Math.floor((n - 1) / 26);
  }
  return name;
}

/** Текст ошибки: у исключений он есть, у остальных значений — их запись строкой. */
function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Имя файла для скачивания: подпись книги без последнего расширения плюс
 * `suffix`. Без подписи — запасное `sheet<suffix>.pdf`.
 *
 * От `exportFileName` из `lib/download.ts` отличается тем, что не срезает
 * каталог: имена XLSX-фикстур плоские (`content-table.xlsx` без категории),
 * а имя перетащенного файла каталога не несёт вовсе.
 */
function pdfFileName(label: string, suffix: string): string {
  const trimmed = label.trim();
  if (!trimmed) return `sheet${suffix}.pdf`;
  // Расширение у выбранного файла может быть любым, поэтому срезается последнее.
  const base = trimmed.replace(/\.[^.]*$/, '');
  return `${base || trimmed}${suffix}.pdf`;
}

/** Что страница берёт у хука: состояние окна и обработчики его элементов. */
export interface XlsxPageApi {
  /** Ручка вьюера — её получает `FrameMeters`; подписок страница не держит. */
  viewer: XlsxViewerHandle | null;
  /** Хост вьюера: на `<main id="viewer">`, всегда пустой. */
  hostRef: RefObject<HTMLElement | null>;
  /** Поле имени ячейки в строке формул: на `<span id="cell">`. */
  cellRef: RefObject<HTMLSpanElement | null>;
  /** Наведение: адрес ячейки под курсором. */
  onPointerMove: (event: ReactPointerEvent<HTMLElement>) => void;
  /** Перетаскивание книги в окно — рамка на хосте. */
  onDragOver: (event: ReactDragEvent<HTMLElement>) => void;
  onDragLeave: (event: ReactDragEvent<HTMLElement>) => void;
  onDrop: (event: ReactDragEvent<HTMLElement>) => void;
  /** Активная вкладка ленты. */
  tab: string;
  onTabSelect: (id: string) => void;
  /** Строка состояния: отказ вьюера перекрывает всё, что страница писала до него. */
  status: string;
  /** Вьюер не поднялся — нажимать нечего. */
  controlsDisabled: boolean;
  /** Подпись открытого документа (для титульной строки); `null` — ничего не открыто. */
  documentName: string;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
  fixtures: string[];
  fixture: string;
  onFixtureChange: (name: string) => void;
  sheets: SheetInfo[];
  sheetIndex: number;
  onSheetChange: (value: string) => void;
  /** Листов нет — выбирать нечего. */
  sheetDisabled: boolean;
  /** Множитель зума строкой: разметка ленты показывает по нему проценты. */
  zoom: string;
  onZoomChange: (value: string) => void;
  showGrid: boolean;
  showHeaders: boolean;
  onShowGrid: (checked: boolean) => void;
  onShowHeaders: (checked: boolean) => void;
  /** PDF собирается: кнопки гасятся, пока файл не готов. */
  exporting: boolean;
  exportDisabled: boolean;
  onExportSheet: () => void;
  onExportBook: () => void;
}

/** Хук страницы «ДваИксель»: вьюер книги и управление им. */
export function useXlsxPage(): XlsxPageApi {
  const [tab, setTab] = useState('home');
  const [status, setStatus] = useState('файл не открыт');
  const [fixtures, setFixtures] = useState<string[]>([]);
  const [fixture, setFixture] = useState('');
  /** Подпись открытой книги; `null` — открывать было нечего, экспорт недоступен. */
  const [label, setLabel] = useState<string | null>(null);
  const [sheets, setSheets] = useState<SheetInfo[]>([]);
  const [sheetIndex, setSheetIndex] = useState(0);
  const [zoom, setZoom] = useState('1');
  const [showGrid, setShowGrid] = useState(true);
  const [showHeaders, setShowHeaders] = useState(true);
  const [exporting, setExporting] = useState(false);

  const cellRef = useRef<HTMLSpanElement | null>(null);
  /** Номер последнего запроса `hitTest`: ответы по старым точкам не должны обгонять свежие. */
  const hitSeqRef = useRef(0);

  const { hostRef, viewer, error } = useViewer(
    (host) =>
      createXlsxViewer(host, {
        workerUrl: xlsxWorkerUrl,
        pdfWasm: { module: pdfWasmModuleUrl, binary: pdfWasmBinaryUrl },
      }),
    (handle) => {
      publishViewer({ kind: 'xlsx', viewer: handle });
    },
  );

  // Список фикстур — удобство разработки: без него книгу выбирают и перетаскивают.
  useEffect(() => {
    let alive = true;
    void listFixtures('xlsx').then((names) => {
      if (alive) setFixtures(names);
    });
    return () => {
      alive = false;
    };
  }, []);

  // Сетка и заголовки: состояние держит страница, а до вьюера оно доезжает
  // эффектом. В обработчике его применять нечем — ручки вьюера там ещё нет.
  useEffect(() => {
    viewer?.setView({ showGrid, showHeaders });
  }, [viewer, showGrid, showHeaders]);

  // Мост сквозных тестов живёт ровно столько, сколько страница: иначе после
  // перехода на другую страницу тест увидел бы мёртвый вьюер.
  useEffect(() => clearViewer, []);

  /** Открыть книгу байтами и подписью — общий путь файла, фикстуры и перетаскивания. */
  const show = async (bytes: ArrayBuffer, name: string): Promise<void> => {
    const active = viewer;
    if (!active) return;
    setStatus(`разбираю ${name}…`);
    const started = performance.now();
    let list: SheetInfo[];
    try {
      list = await active.open(bytes);
    } catch (e) {
      // Подпись снимается — экспортировать нечего, — а список листов остаётся:
      // книга, показанная раньше, из вьюера никуда не делась.
      setLabel(null);
      setStatus(`не открылось: ${messageOf(e)}`);
      return;
    }
    const elapsed = Math.round(performance.now() - started);
    setLabel(name);
    setSheets(list);
    // Показывается первый нескрытый лист: скрытый открывать нечего.
    const first = list.findIndex((sheet) => !sheet.hidden);
    const index = first < 0 ? 0 : first;
    setSheetIndex(index);
    active.showSheet(index);
    setStatus(`${name}: ${list.length} лист(ов) за ${elapsed} мс`);
  };

  /**
   * Экспорт в PDF: файл собирает Rust в воркере, странице остаётся скачивание.
   * Разделение листа и книги — только в настройках и суффиксе имени.
   */
  const exportPdf = async (options: PdfOptions, suffix: string): Promise<void> => {
    const active = viewer;
    if (!active || label === null || exporting) return;
    setExporting(true);
    setStatus('экспортирую PDF…');
    try {
      const bytes = await active.exportPdf(options);
      const name = pdfFileName(label, suffix);
      // Копия в `Uint8Array` над обычным `ArrayBuffer`: тип из воркера —
      // `Uint8Array<ArrayBufferLike>` (туда входит и `SharedArrayBuffer`), а
      // `BlobPart` принимает только буферы во владении страницы.
      downloadBlob(new Uint8Array(bytes), name, 'application/pdf');
      setStatus(`PDF готов: ${name}, ${Math.round(bytes.byteLength / 1024)} КБ`);
    } catch (e) {
      // Ошибка уходит в строку состояния: `alert` перекрыл бы страницу, а
      // исключение из обработчика отменило бы только его — вьюер жив.
      setStatus(`экспорт не удался: ${messageOf(e)}`);
    } finally {
      setExporting(false);
    }
  };

  /** Открыть файл книги: выбранный в поле или перетащенный в окно. */
  const openFile = (file: File): void => {
    void file
      .arrayBuffer()
      .then((bytes) => show(bytes, file.name))
      // Чтение с диска может отказать (файл увели, доступ пропал): страница
      // говорит об этом строкой состояния, а не исключением в консоли.
      .catch((e: unknown) => {
        setStatus(`не открылось: ${messageOf(e)}`);
      });
  };

  const loadFixture = async (name: string): Promise<void> => {
    setStatus(`загружаю ${name}…`);
    let bytes: ArrayBuffer;
    try {
      bytes = await fetchFixture('xlsx', name);
    } catch {
      // Отсутствующий файл и оборванный запрос для примера одно и то же:
      // фикстуры, которую просили, нет.
      setStatus(`фикстура не найдена: ${name}`);
      return;
    }
    await show(bytes, name);
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLElement>): void => {
    const active = viewer;
    if (!active) return;
    const seq = ++hitSeqRef.current;
    void active.hitTest(event.clientX, event.clientY).then(
      (cell) => {
        // Ответ устарел, если мышь успела перейти на другую ячейку: пишет
        // только последний запрос, иначе строка формул мигала бы задним числом.
        if (seq !== hitSeqRef.current) return;
        const output = cellRef.current;
        if (!output) return;
        output.textContent = cell ? `${columnName(cell[1])}${cell[0] + 1}` : '—';
      },
      () => {
        // Вьюер убрали посреди запроса: подсказка устарела вместе с ним, а
        // необработанный отказ засорил бы консоль.
      },
    );
  };

  const onDragOver = (event: ReactDragEvent<HTMLElement>): void => {
    event.preventDefault();
    // Класс ставится на элемент напрямую: `dragover` летит десятками раз в
    // секунду, и состояние React гоняло бы перерисовку страницы вхолостую.
    hostRef.current?.classList.add('dragover');
  };

  const onDragLeave = (): void => {
    hostRef.current?.classList.remove('dragover');
  };

  const onDrop = (event: ReactDragEvent<HTMLElement>): void => {
    event.preventDefault();
    hostRef.current?.classList.remove('dragover');
    const file = event.dataTransfer.files[0];
    if (file) openFile(file);
  };

  const onFileChange = (event: ChangeEvent<HTMLInputElement>): void => {
    const file = event.target.files?.[0];
    if (file) openFile(file);
  };

  const onFixtureChange = (name: string): void => {
    setFixture(name);
    if (name) void loadFixture(name);
  };

  const onSheetChange = (value: string): void => {
    const index = Number(value);
    setSheetIndex(index);
    viewer?.showSheet(index);
  };

  const onZoomChange = (value: string): void => {
    setZoom(value);
    // Вьюер ограничивает зум сам; состояние хранит выбранное пользователем.
    viewer?.setZoom(Number(value));
  };

  return {
    viewer,
    hostRef,
    cellRef,
    onPointerMove,
    onDragOver,
    onDragLeave,
    onDrop,
    tab,
    onTabSelect: (id) => {
      setTab(id);
    },
    status: error ?? status,
    controlsDisabled: error !== null,
    documentName: label ?? 'Книга1',
    onFileChange,
    fixtures,
    fixture,
    onFixtureChange,
    sheets,
    sheetIndex,
    onSheetChange,
    sheetDisabled: sheets.length === 0,
    zoom,
    onZoomChange,
    showGrid,
    showHeaders,
    onShowGrid: setShowGrid,
    onShowHeaders: setShowHeaders,
    exporting,
    exportDisabled: label === null || exporting,
    onExportSheet: () => {
      void exportPdf({}, '');
    },
    onExportBook: () => {
      void exportPdf({ allSheets: true }, ' (все листы)');
    },
  };
}
