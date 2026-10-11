// Пример просмотрщика DOCX: документ разбирается в воркере, раскладка страниц
// считается в Rust, кадр рисует painter на OffscreenCanvas. На долю main
// остаётся разметка, навигация по страницам и строка состояния.

import { createDocxViewer, type DocxViewerHandle, type PageInfo } from '@doc-converter/core';

// Воркер подключается суффиксом `?worker&url`: только так Vite собирает его
// отдельным чанком. Голый `new URL('./worker.ts', import.meta.url)` он считает
// обычным ассетом и инлайнит исходник в data-URL, где импорт
// `@doc-converter/core/worker` уже не разрешается, — воркер молча не стартует.
import workerUrl from './worker.ts?worker&url';

// DOCX-модуль грузится лениво и отдельным файлом: Vite отдаёт его URL-ом
// (`new URL(..., import.meta.url)` разворачивается в ассет), а не импортом
// модуля — импорт втянул бы его в основной бандл, и страница платила бы за
// wasm ещё до открытия документа. Копируя JS как есть, сборщик не кладёт
// рядом его wasm-бинарь, поэтому второй URL передаётся явно.
const docxWasmModuleUrl = new URL(
  '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm.js',
  import.meta.url,
).href;
const docxWasmBinaryUrl = new URL(
  '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm_bg.wasm',
  import.meta.url,
).href;

// PDF-модуль тоже ленивый и отдельным файлом: URL-ом, а не импортом модуля —
// импорт втянул бы printpdf в основной бандл, и страница платила бы за него
// до первого экспорта. JS копируется как есть, поэтому сборщик не кладёт рядом
// его wasm-бинарь: второй URL передаётся явно.
const pdfWasmModuleUrl = new URL(
  '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
  import.meta.url,
).href;
const pdfWasmBinaryUrl = new URL(
  '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm',
  import.meta.url,
).href;

/** Элемент разметки: его отсутствие — ошибка сборки примера, а не рантайма. */
function required<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`в разметке нет ${selector}`);
  return element;
}

const viewerHost = required<HTMLElement>('#viewer');
const fileInput = required<HTMLInputElement>('#file');
const fixtureSelect = required<HTMLSelectElement>('#fixture');
const prevButton = required<HTMLButtonElement>('#page-prev');
const nextButton = required<HTMLButtonElement>('#page-next');
const pageOutput = required<HTMLElement>('#page-info');
const zoomSelect = required<HTMLSelectElement>('#zoom');
const exportPdfButton = required<HTMLButtonElement>('#export-pdf');
const exportMdButton = required<HTMLButtonElement>('#export-md');
const exportHtmlButton = required<HTMLButtonElement>('#export-html');
const exportTxtButton = required<HTMLButtonElement>('#export-txt');
const pagesOutput = required<HTMLElement>('#pages');
const cmdsOutput = required<HTMLElement>('#cmds');
const buildOutput = required<HTMLElement>('#build');
const paintOutput = required<HTMLElement>('#paint');
const frameOutput = required<HTMLElement>('#frame');
const status = required<HTMLElement>('#status');

let handle: DocxViewerHandle | null = null;
/** Подпись открытого документа: из неё получается имя файла при экспорте. */
let currentLabel: string | null = null;

function setStatus(text: string): void {
  status.textContent = text;
}

/** Кнопки экспорта: включаются только вместе с документом — иначе экспортировать нечего. */
const exportButtons = [exportPdfButton, exportMdButton, exportHtmlButton, exportTxtButton];

function setExportEnabled(enabled: boolean): void {
  for (const button of exportButtons) button.disabled = !enabled;
}

/** Погасить всё управление: без вьюера нажимать нечего. */
function disableControls(): void {
  for (const control of [fileInput, fixtureSelect, prevButton, nextButton, zoomSelect]) {
    control.disabled = true;
  }
  setExportEnabled(false);
}

/**
 * Счётчик страниц и кнопки перехода. Номер страницы вьюер ведёт сам — он
 * меняется и от прокрутки, поэтому состояние обновляется на каждом кадре,
 * а не только по нажатию кнопки.
 */
function updatePager(): void {
  const index = handle?.pageIndex ?? 0;
  const count = handle?.pageCount ?? 0;
  pageOutput.textContent = count === 0 ? '—' : `${index + 1} / ${count}`;
  pagesOutput.textContent = String(count);
  prevButton.disabled = count === 0 || index <= 0;
  nextButton.disabled = count === 0 || index >= count - 1;
}

async function show(bytes: ArrayBuffer, label: string): Promise<void> {
  if (!handle) return;
  setStatus(`разбираю ${label}…`);
  const started = performance.now();
  let pages: PageInfo[];
  try {
    pages = await handle.open(bytes);
  } catch (e) {
    // Документ не открылся — экспортировать нечего, а имя файла потеряло бы
    // связь с содержимым.
    currentLabel = null;
    setExportEnabled(false);
    setStatus(`не открылось: ${e instanceof Error ? e.message : String(e)}`);
    return;
  }
  currentLabel = label;
  setExportEnabled(true);
  const elapsed = Math.round(performance.now() - started);
  updatePager();
  setStatus(`${label}: ${pages.length} стр. за ${elapsed} мс`);
}

/** Формат текстового экспорта: имя файла, MIME-тип, подписи и метод вьюера. */
type TextFormat = 'markdown' | 'html' | 'text';

interface TextFormatSpec {
  /** Расширение файла: приписывается к подписи документа вместо старого. */
  extension: string;
  /** MIME-тип скачиваемого файла. */
  mime: string;
  /** Имя формата в строке состояния: «Markdown готов: …». */
  title: string;
  /** Строка состояния, пока воркер собирает файл. */
  progress: string;
  /** Метод вьюера, отдающий содержимое. */
  run: (viewer: DocxViewerHandle) => Promise<string>;
}

/** Форматы отличаются только этими значениями, поэтому логика экспорта у них общая. */
const textFormats: Record<TextFormat, TextFormatSpec> = {
  markdown: {
    extension: 'md',
    mime: 'text/markdown',
    title: 'Markdown',
    progress: 'экспортирую Markdown…',
    run: (viewer) => viewer.exportMarkdown(),
  },
  html: {
    extension: 'html',
    mime: 'text/html',
    title: 'HTML',
    progress: 'экспортирую HTML…',
    run: (viewer) => viewer.exportHtml(),
  },
  text: {
    extension: 'txt',
    mime: 'text/plain',
    title: 'Текст',
    progress: 'экспортирую текст…',
    run: (viewer) => viewer.exportText(),
  },
};

/**
 * Имя файла для скачивания: подпись документа без каталога и расширения плюс
 * своё расширение; без подписи — запасное имя `document.<ext>`.
 */
function exportFileName(extension: string): string {
  const label = currentLabel?.trim();
  if (!label) return `document.${extension}`;
  // Имя фикстуры приходит путём с категорией (`simple/one_paragraph.docx`),
  // а скачивается файл с плоским именем: каталог срезаем. Расширение у
  // выбранного файла может быть любым, поэтому срезается последнее.
  const name = label.replace(/^.*[/\\]/, '');
  const base = name.replace(/\.[^.]*$/, '');
  return `${base || name}.${extension}`;
}

/** Отдать данные файлом: временная ссылка на blob, клик и отзыв ссылки. */
function downloadFile(data: BlobPart, name: string, mime: string): void {
  const url = URL.createObjectURL(new Blob([data], { type: mime }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  // Отзываем не сразу: браузер начинает скачивание асинхронно, и немедленный
  // revoke успевает отменить его (в WebKit — стабильно).
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

/** Экспорт в PDF: файл собирает Rust в воркере, здесь остаётся скачивание. */
async function exportPdf(): Promise<void> {
  const active = handle;
  if (!active) return;
  exportPdfButton.disabled = true;
  setStatus('экспортирую PDF…');
  try {
    const bytes = await active.exportPdf();
    // Копия в Uint8Array над обычным ArrayBuffer: тип из воркера —
    // Uint8Array<ArrayBufferLike> (в ArrayBufferLike входит SharedArrayBuffer),
    // а BlobPart принимает только ArrayBuffer. Заодно гарантирован сдвиг 0.
    const copy = new Uint8Array(bytes);
    const name = exportFileName('pdf');
    downloadFile(copy, name, 'application/pdf');
    setStatus(`PDF готов: ${name}, ${Math.round(bytes.byteLength / 1024)} КБ`);
  } catch (e) {
    // Ошибку показываем в строке состояния: alert перекрыл бы пример,
    // а исключение из обработчика не должно ронять страницу.
    setStatus(`экспорт не удался: ${e instanceof Error ? e.message : String(e)}`);
  } finally {
    exportPdfButton.disabled = false;
  }
}

/** Экспорт в текстовый формат: Markdown, HTML или содержимое без разметки. */
async function exportText(format: TextFormat, button: HTMLButtonElement): Promise<void> {
  const active = handle;
  if (!active) return;
  const spec = textFormats[format];
  button.disabled = true;
  setStatus(spec.progress);
  try {
    const text = await spec.run(active);
    const name = exportFileName(spec.extension);
    downloadFile(text, name, spec.mime);
    setStatus(`${spec.title} готов: ${name}`);
  } catch (e) {
    setStatus(`экспорт не удался: ${e instanceof Error ? e.message : String(e)}`);
  } finally {
    button.disabled = false;
  }
}

async function loadFixture(name: string): Promise<void> {
  setStatus(`загружаю ${name}…`);
  const response = await fetch(`${import.meta.env.BASE_URL}fixtures/${name}`);
  if (!response.ok) {
    setStatus(`фикстура не найдена: ${name}`);
    return;
  }
  await show(await response.arrayBuffer(), name);
}

/** Список фикстур: имена с категорией, как их отдаёт dev-сервер примера. */
async function listFixtures(): Promise<void> {
  try {
    const response = await fetch(`${import.meta.env.BASE_URL}fixtures/index.json`);
    if (!response.ok) return;
    const names = (await response.json()) as string[];
    for (const name of names) {
      const option = document.createElement('option');
      option.value = name;
      option.textContent = name.replace(/\.docx$/, '');
      fixtureSelect.append(option);
    }
  } catch {
    // Список фикстур — удобство разработки, без него пример работает.
  }
}

/** Перейти на страницу со сдвигом от текущей; за пределами документа — никуда. */
function stepPage(delta: number): void {
  const active = handle;
  if (!active) return;
  active.showPage(active.pageIndex + delta);
  updatePager();
}

async function main(): Promise<void> {
  try {
    handle = await createDocxViewer(viewerHost, {
      workerUrl,
      docxWasm: { module: docxWasmModuleUrl, binary: docxWasmBinaryUrl },
      pdfWasm: { module: pdfWasmModuleUrl, binary: pdfWasmBinaryUrl },
    });
  } catch (e) {
    // Нет OffscreenCanvas/transferControlToOffscreen — вьюер не поднять:
    // показываем причину и гасим управление, не роняя страницу исключением.
    setStatus(e instanceof Error ? e.message : String(e));
    disableControls();
    return;
  }

  handle.onTick((stats) => {
    cmdsOutput.textContent = String(stats.cmds);
    buildOutput.textContent = `${(stats.buildMs ?? 0).toFixed(1)} мс`;
    paintOutput.textContent = `${stats.paintMs.toFixed(1)} мс`;
    frameOutput.textContent = String(stats.frameId);
    // Прокрутка меняет текущую страницу без участия кнопок — счётчик идёт за ней.
    updatePager();
  });

  fileInput.addEventListener('change', () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    void file.arrayBuffer().then((bytes) => show(bytes, file.name));
  });

  fixtureSelect.addEventListener('change', () => {
    const name = fixtureSelect.value;
    if (name) void loadFixture(name);
  });

  prevButton.addEventListener('click', () => stepPage(-1));
  nextButton.addEventListener('click', () => stepPage(1));

  zoomSelect.addEventListener('change', () => {
    handle?.setZoom(Number(zoomSelect.value));
  });

  exportPdfButton.addEventListener('click', () => {
    void exportPdf();
  });
  exportMdButton.addEventListener('click', () => {
    void exportText('markdown', exportMdButton);
  });
  exportHtmlButton.addEventListener('click', () => {
    void exportText('html', exportHtmlButton);
  });
  exportTxtButton.addEventListener('click', () => {
    void exportText('text', exportTxtButton);
  });

  // Перетаскивание файла в окно.
  viewerHost.addEventListener('dragover', (event) => {
    event.preventDefault();
    viewerHost.classList.add('dragover');
  });
  viewerHost.addEventListener('dragleave', () => viewerHost.classList.remove('dragover'));
  viewerHost.addEventListener('drop', (event) => {
    event.preventDefault();
    viewerHost.classList.remove('dragover');
    const file = event.dataTransfer?.files?.[0];
    if (file) void file.arrayBuffer().then((bytes) => show(bytes, file.name));
  });

  void listFixtures();
  updatePager();
  setStatus('файл не открыт');

  // Для тестов: снаружи видно, что вьюер поднялся. Имя не `viewer` — под ним
  // уже живёт элемент с этим `id`: браузер делает id глобальной переменной.
  (window as unknown as { docConverter?: { viewer: DocxViewerHandle } }).docConverter = {
    viewer: handle,
  };
}

void main();
