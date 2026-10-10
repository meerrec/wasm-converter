// Пример просмотрщика DOCX: документ разбирается в воркере, раскладка страниц
// считается в Rust, кадр рисует painter на OffscreenCanvas. На долю main
// остаётся разметка, навигация по страницам и строка состояния.

import { createDocxViewer, type DocxViewerHandle, type PageInfo } from '@doc-converter/core';

// Воркер подключается суффиксом `?worker&url`: только так Vite собирает его
// отдельным чанком. Голый `new URL('./worker.ts', import.meta.url)` он считает
// обычным ассетом и инлайнит исходник в data-URL, где импорт
// `@doc-converter/core/worker` уже не разрешается, — воркер молча не стартует.
import workerUrl from './worker.ts?worker&url';

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
const pagesOutput = required<HTMLElement>('#pages');
const cmdsOutput = required<HTMLElement>('#cmds');
const buildOutput = required<HTMLElement>('#build');
const paintOutput = required<HTMLElement>('#paint');
const frameOutput = required<HTMLElement>('#frame');
const status = required<HTMLElement>('#status');

let handle: DocxViewerHandle | null = null;

function setStatus(text: string): void {
  status.textContent = text;
}

/** Погасить всё управление: без вьюера нажимать нечего. */
function disableControls(): void {
  for (const control of [fileInput, fixtureSelect, prevButton, nextButton, zoomSelect]) {
    control.disabled = true;
  }
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
    setStatus(`не открылось: ${e instanceof Error ? e.message : String(e)}`);
    return;
  }
  const elapsed = Math.round(performance.now() - started);
  updatePager();
  setStatus(`${label}: ${pages.length} стр. за ${elapsed} мс`);
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
    handle = await createDocxViewer(viewerHost, { workerUrl });
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
