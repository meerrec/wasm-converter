/**
 * UI-оболочка просмотрщика: тулбар в стиле сайта Euro-Office, выбор файла
 * (кнопка и drag&drop), статус и iframe с `/editor.html?embed=1`. Файл читает
 * родитель и отдаёт байтами командой `document:open-buffer` в режиме только
 * просмотра — CORS не нужен, сохранение в редакторе выключено.
 *
 * Контракт приёмки (tools/viewer-check.mjs) держим байт-в-байт: id элементов
 * #viewer-file/#viewer-status/#viewer-frame и хук window.euroofficeViewer.
 */
import { createEditorClient, describeEditorError } from './protocol.js';
import type { DocumentErrorEvent, EditorEvent } from './protocol.js';

// ── Контракт приёмки ───────────────────────────────────────────────────────

/** Состояния статуса страницы: те же значения, что читает viewer-check.mjs. */
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
    /** Хук приёмки (tools/viewer-check.mjs): счётчик открытий снимает гонку data-state. */
    euroofficeViewer?: ViewerHarness;
  }
}

// ── Элементы ───────────────────────────────────────────────────────────────

function mustGet<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (element === null) {
    throw new Error(`нет элемента #${id}`);
  }
  return element as T;
}

const frame = mustGet<HTMLIFrameElement>('viewer-frame');
const fileInput = mustGet<HTMLInputElement>('viewer-file');
const statusEl = mustGet<HTMLElement>('viewer-status');
const openButton = mustGet<HTMLButtonElement>('viewer-open');
const docNameEl = mustGet<HTMLElement>('viewer-doc-name');
const dropOverlay = mustGet<HTMLElement>('viewer-drop-overlay');

// ── Состояние ──────────────────────────────────────────────────────────────

const harness: ViewerHarness = { state: 'loading', openedCount: 0, lastFileName: '', lastError: null };
window.euroofficeViewer = harness;

const client = createEditorClient(frame);

let ready = false;
let pendingFile: File | null = null;
let lastFileName = '';

/** Размер файла человекочитаемо — в статус «Открываю…». */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} Б`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} КиБ`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} МиБ`;
}

function setStatus(state: ViewerState, text: string): void {
  statusEl.dataset.state = state;
  statusEl.textContent = text;
  harness.state = state;
}

/** Имя документа по центру тулбара. */
function setDocName(name: string): void {
  docNameEl.textContent = name;
  docNameEl.title = name;
}

/** Текст ошибки для статуса: сообщение редактора + расшифровка кода. */
function formatEditorError(event: DocumentErrorEvent): string {
  const message = event.payload.message ?? 'неизвестная';
  const code = event.payload.code;
  if (code === undefined) return `Ошибка: ${message}`;
  return `Ошибка: ${message} (код ${code}: ${describeEditorError(code)})`;
}

async function openFile(file: File): Promise<void> {
  lastFileName = file.name;
  setDocName(file.name);
  try {
    const buffer = await file.arrayBuffer();
    setStatus('opening', `Открываю «${file.name}» (${formatSize(buffer.byteLength)})…`);
    client.openBuffer(file.name, buffer, true);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus('error', `Ошибка: не удалось прочитать файл — ${message}`);
  }
}

// ── Принимаемые форматы ────────────────────────────────────────────────────

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

function isSupportedFile(file: File): boolean {
  const dot = file.name.lastIndexOf('.');
  if (dot === -1) return false;
  return ACCEPTED_EXTENSIONS.includes(file.name.slice(dot).toLowerCase());
}

function handlePicked(file: File): void {
  if (!isSupportedFile(file)) {
    const dot = file.name.lastIndexOf('.');
    const extension = dot === -1 ? 'без расширения' : file.name.slice(dot).toLowerCase();
    setStatus('error', `Ошибка: формат «${extension}» не поддержан`);
    return;
  }
  if (ready) {
    void openFile(file);
  } else {
    // Выбор до document:ready: файл ждёт в очереди, статус — loading.
    pendingFile = file;
    setStatus('loading', `редактор ещё поднимается — «${file.name}» откроется сразу после готовности`);
  }
}

// ── События редактора ──────────────────────────────────────────────────────

function handleEditorEvent(event: EditorEvent): void {
  switch (event.type) {
    case 'document:ready':
      ready = true;
      setStatus('ready', 'редактор готов — выберите документ');
      if (pendingFile !== null) {
        const file = pendingFile;
        pendingFile = null;
        void openFile(file);
      }
      break;
    case 'document:opened':
      harness.openedCount += 1;
      harness.lastFileName = event.payload.fileName ?? lastFileName;
      setDocName(harness.lastFileName);
      setStatus('opened', `«${harness.lastFileName}» — только просмотр`);
      break;
    case 'document:error':
      harness.lastError = formatEditorError(event);
      setStatus('error', harness.lastError);
      break;
    // saved/state/dirty-changed/readonly-changed в режиме просмотра не нужны —
    // их (и будущие document:* события) логирует клиент, чтобы не терять молча.
  }
}

client.onEvent((event) => {
  handleEditorEvent(event);
});

// ── Выбор файла ────────────────────────────────────────────────────────────

fileInput.addEventListener('change', () => {
  const file = fileInput.files?.[0];
  fileInput.value = ''; // повторный выбор того же файла снова шлёт change
  if (file !== undefined) handlePicked(file);
});

openButton.addEventListener('click', () => {
  fileInput.click();
});

// ── Drag&drop ──────────────────────────────────────────────────────────────

/** В перетаскивании есть файл (а не, скажем, выделенный текст). */
function dragHasFiles(event: DragEvent): boolean {
  return event.dataTransfer?.types.includes('Files') ?? false;
}

// Пока тащим файл, поверх области просмотра лежит оверлей (см. style.css):
// только так drop над iframe достаётся нам, а не документу редактора.
window.addEventListener('dragenter', (event) => {
  if (dragHasFiles(event)) dropOverlay.classList.add('visible');
});
window.addEventListener('dragover', (event) => {
  if (!dragHasFiles(event)) return;
  event.preventDefault();
  dropOverlay.classList.add('visible');
});
window.addEventListener('dragleave', (event) => {
  // Ушли из окна совсем (а не в соседний элемент) — прячем оверлей.
  if (event.relatedTarget === null) dropOverlay.classList.remove('visible');
});
window.addEventListener('drop', (event) => {
  // Брошенный файл не должен открываться браузером как страница.
  event.preventDefault();
  dropOverlay.classList.remove('visible');
  const file = event.dataTransfer?.files[0];
  if (file !== undefined) handlePicked(file);
});
