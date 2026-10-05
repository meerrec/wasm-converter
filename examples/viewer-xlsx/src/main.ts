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
const cellOutput = required<HTMLElement>('#cell');
const cmdsOutput = required<HTMLElement>('#cmds');
const buildOutput = required<HTMLElement>('#build');
const paintOutput = required<HTMLElement>('#paint');
const frameOutput = required<HTMLElement>('#frame');
const status = required<HTMLElement>('#status');

let handle: XlsxViewerHandle | null = null;
let sheets: SheetInfo[] = [];

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
    setStatus(`не открылось: ${e instanceof Error ? e.message : String(e)}`);
    return;
  }
  const elapsed = Math.round(performance.now() - started);
  fillSheetList(sheets);
  const first = sheets.findIndex((sheet) => !sheet.hidden);
  handle.showSheet(first < 0 ? 0 : first);
  setStatus(`${label}: ${sheets.length} лист(ов) за ${elapsed} мс`);
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
  handle = await createXlsxViewer(viewer, {
    workerUrl: new URL('./worker.ts', import.meta.url),
  });

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
