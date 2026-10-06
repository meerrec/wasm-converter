// Пример просмотрщика XLSX: книга разбирается в воркере, кадр собирается в
// Rust, рисует painter на OffscreenCanvas. На долю main остаётся разметка,
// прокрутка и строка состояния.

import { createXlsxViewer, type SheetInfo, type XlsxViewerHandle } from '@doc-converter/core';

/** Элемент разметки: его отсутствие — ошибка сборки примера, а не рантайма. */
function required<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`в разметке нет ${selector}`);
  return element;
}

const viewer = required<HTMLElement>('#viewer');
const fileInput = required<HTMLInputElement>('#file');
const fixtureSelect = required<HTMLSelectElement>('#fixture');
const sheetSelect = required<HTMLSelectElement>('#sheet');
const zoomSelect = required<HTMLSelectElement>('#zoom');
const gridBox = required<HTMLInputElement>('#grid');
const headersBox = required<HTMLInputElement>('#headers');
const exportButton = required<HTMLButtonElement>('#export-pdf');
const cellOutput = required<HTMLElement>('#cell');
const cmdsOutput = required<HTMLElement>('#cmds');
const buildOutput = required<HTMLElement>('#build');
const paintOutput = required<HTMLElement>('#paint');
const frameOutput = required<HTMLElement>('#frame');
const status = required<HTMLElement>('#status');

let handle: XlsxViewerHandle | null = null;
let sheets: SheetInfo[] = [];
/** Подпись открытого документа: из неё получается имя PDF при экспорте. */
let currentLabel: string | null = null;

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

function setStatus(text: string): void {
  status.textContent = text;
}

/** Погасить всё управление: без вьюера нажимать нечего. */
function disableControls(): void {
  const controls = [
    fileInput,
    fixtureSelect,
    sheetSelect,
    zoomSelect,
    gridBox,
    headersBox,
    exportButton,
  ];
  for (const control of controls) control.disabled = true;
}

function fillSheetList(list: SheetInfo[]): void {
  sheetSelect.replaceChildren();
  for (const sheet of list) {
    const option = document.createElement('option');
    option.value = String(sheet.index);
    option.textContent = sheet.hidden ? `${sheet.name} (скрыт)` : sheet.name;
    sheetSelect.append(option);
  }
  sheetSelect.disabled = list.length === 0;
}

async function show(bytes: ArrayBuffer, label: string): Promise<void> {
  if (!handle) return;
  setStatus(`разбираю ${label}…`);
  const started = performance.now();
  try {
    sheets = await handle.open(bytes);
  } catch (e) {
    currentLabel = null;
    exportButton.disabled = true;
    setStatus(`не открылось: ${e instanceof Error ? e.message : String(e)}`);
    return;
  }
  currentLabel = label;
  exportButton.disabled = false;
  const elapsed = Math.round(performance.now() - started);
  fillSheetList(sheets);
  const first = sheets.findIndex((sheet) => !sheet.hidden);
  handle.showSheet(first < 0 ? 0 : first);
  setStatus(`${label}: ${sheets.length} лист(ов) за ${elapsed} мс`);
}

/** Имя файла для скачивания: подпись документа без расширения, иначе запасное. */
function pdfFileName(): string {
  const label = currentLabel?.trim();
  if (!label) return 'sheet.pdf';
  // Расширение у выбранного файла может быть любым, поэтому срезаем последнее.
  const base = label.replace(/\.[^.]*$/, '');
  return `${base || label}.pdf`;
}

/** Отдать байты файлом: временная ссылка на blob, клик и отзыв ссылки. */
function downloadPdf(bytes: Uint8Array, name: string): void {
  // Копия в Uint8Array над обычным ArrayBuffer: тип из воркера —
  // Uint8Array<ArrayBufferLike> (в ArrayBufferLike входит SharedArrayBuffer),
  // а BlobPart принимает только ArrayBuffer. Заодно гарантирован сдвиг 0.
  const copy = new Uint8Array(bytes);
  const url = URL.createObjectURL(new Blob([copy], { type: 'application/pdf' }));
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

/** Экспорт текущего листа в PDF: файл собирает Rust в воркере, здесь — скачивание. */
async function exportCurrentPdf(): Promise<void> {
  const active = handle;
  if (!active) return;
  exportButton.disabled = true;
  setStatus('экспортирую PDF…');
  try {
    const bytes = await active.exportPdf();
    const name = pdfFileName();
    downloadPdf(bytes, name);
    setStatus(`PDF готов: ${name}, ${Math.round(bytes.byteLength / 1024)} КБ`);
  } catch (e) {
    // Ошибку показываем в строке состояния: alert перекрыл бы пример,
    // а исключение из обработчика не должно ронять страницу.
    setStatus(`экспорт не удался: ${e instanceof Error ? e.message : String(e)}`);
  } finally {
    exportButton.disabled = false;
  }
}

async function loadFixture(name: string): Promise<void> {
  setStatus(`загружаю ${name}…`);
  const response = await fetch(`/fixtures/${name}`);
  if (!response.ok) {
    setStatus(`фикстура не найдена: ${name}`);
    return;
  }
  await show(await response.arrayBuffer(), name);
}

async function listFixtures(): Promise<void> {
  try {
    const response = await fetch('/fixtures/index.json');
    if (!response.ok) return;
    const names = (await response.json()) as string[];
    for (const name of names) {
      const option = document.createElement('option');
      option.value = name;
      option.textContent = name.replace(/\.xlsx$/, '');
      fixtureSelect.append(option);
    }
  } catch {
    // Список фикстур — удобство разработки, без него пример работает.
  }
}

async function main(): Promise<void> {
  try {
    handle = await createXlsxViewer(viewer, {
      workerUrl: new URL('./worker.ts', import.meta.url),
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
  });

  viewer.addEventListener('pointermove', (event) => {
    if (!handle) return;
    void handle.hitTest(event.clientX, event.clientY).then((cell) => {
      cellOutput.textContent = cell
        ? `${columnName(cell[1])}${cell[0] + 1}`
        : '—';
    });
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

  sheetSelect.addEventListener('change', () => {
    handle?.showSheet(Number(sheetSelect.value));
  });

  zoomSelect.addEventListener('change', () => {
    handle?.setZoom(Number(zoomSelect.value));
  });

  const applyView = () => {
    handle?.setView({ showGrid: gridBox.checked, showHeaders: headersBox.checked });
  };
  gridBox.addEventListener('change', applyView);
  headersBox.addEventListener('change', applyView);

  exportButton.addEventListener('click', () => {
    void exportCurrentPdf();
  });

  // Перетаскивание файла в окно.
  viewer.addEventListener('dragover', (event) => {
    event.preventDefault();
    viewer.classList.add('dragover');
  });
  viewer.addEventListener('dragleave', () => viewer.classList.remove('dragover'));
  viewer.addEventListener('drop', (event) => {
    event.preventDefault();
    viewer.classList.remove('dragover');
    const file = event.dataTransfer?.files?.[0];
    if (file) void file.arrayBuffer().then((bytes) => show(bytes, file.name));
  });

  void listFixtures();
  setStatus('файл не открыт');

  // Для тестов: снаружи видно, что вьюер поднялся. Имя не `viewer` — под ним
  // уже живёт элемент с этим `id`: браузер делает id глобальной переменной.
  (window as unknown as { docConverter?: { viewer: XlsxViewerHandle } }).docConverter = {
    viewer: handle,
  };
}

void main();
