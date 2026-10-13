/**
 * Страница харнесса: выбирает файл, шлёт его воркеру, показывает метрики и предпросмотр.
 * Вся работа с конвертером — в воркере, здесь только UI и обмен сообщениями.
 */
import { decodeTextPreview, formatBytes, hexPreview, listZipEntries } from './preview.js';
import { PROTOCOL, type MainToWorker, type WorkerToMain } from './protocol.js';
import { destroyViewer, openDocument, viewerPngBase64, viewerState } from './viewer.js';

type Phase = 'ready' | 'converting' | 'done' | 'error';
/** Состояние модуля x2t: он поднимается по первой конвертации, а не при загрузке страницы. */
type ModuleState = 'cold' | 'booting' | 'ready' | 'error';

/** Форматы-пакеты: их выход показываем списком записей zip. */
const ZIP_TARGETS = new Set(['docx', 'odt', 'xlsx', 'ods', 'pptx', 'odp']);
/** Форматы-тексты: их выход показываем текстом. */
const TEXT_TARGETS = new Set(['txt', 'csv', 'html']);
/** Сколько записей zip показывать в предпросмотре — остальные только считаем. */
const PREVIEW_ENTRIES = 20;
/** Сколько строк лога держать на странице. */
const LOG_LIMIT = 200;

const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  odt: 'application/vnd.oasis.opendocument.text',
  ods: 'application/vnd.oasis.opendocument.spreadsheet',
  odp: 'application/vnd.oasis.opendocument.presentation',
  pdf: 'application/pdf',
  txt: 'text/plain;charset=utf-8',
  csv: 'text/csv;charset=utf-8',
  html: 'text/html;charset=utf-8',
};

interface LastResult {
  readonly outputName: string;
  readonly code: number;
  readonly stderr: string;
  readonly convertMs: number;
  readonly bytes: Uint8Array<ArrayBuffer>;
}

/** Состояние, которое видит страница (и Playwright — через `window.x2tHarness`). */
const state: {
  phase: Phase;
  module: ModuleState;
  error: string | null;
  bootMs: number | null;
  log: string[];
  last: LastResult | null;
  pending: { id: number; inputBytes: number; target: string } | null;
  /** Счётчик прочитанных файлов: по нему приёмка ждёт именно свой выбор файла, а не прошлый. */
  inputSerial: number;
  /** Чем открыт документ в просмотре и что вьюер об этом говорит. */
  viewerSource: string | null;
} = {
  phase: 'ready',
  module: 'cold',
  error: null,
  bootMs: null,
  log: [],
  last: null,
  pending: null,
  inputSerial: 0,
  viewerSource: null,
};

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (found === null) {
    throw new Error(`harness markup is missing #${id}`);
  }
  return found as T;
}

const fileInput = element<HTMLInputElement>('file');
const formatSelect = element<HTMLSelectElement>('format');
const convertButton = element<HTMLButtonElement>('convert');
const pdfButton = element<HTMLButtonElement>('pdfCheck');
const statusBox = element<HTMLSpanElement>('status');
const moduleBox = element<HTMLSpanElement>('moduleStatus');
const metricsBox = element<HTMLElement>('metrics');
const previewBox = element<HTMLElement>('preview');
const logBox = element<HTMLElement>('log');
const downloadLink = element<HTMLAnchorElement>('download');
const viewerHost = element<HTMLElement>('viewerHost');
const viewerBox = element<HTMLSpanElement>('viewerStatus');

// Воркер — модульный, и в dev, и в сборке (`worker.format: 'es'` в vite.config.ts): в dev
// Vite отдаёт воркеры без сборки, поэтому классический упал бы на первом же ESM-импорте.
// Модульному воркеру недоступен `importScripts`, и обвязка x2t грузит glue через fetch
// (см. `loadX2t` в packages/x2t/src/x2t.ts).
const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });

let nextRequestId = 1;
let inputBytes: Uint8Array | null = null;
let inputName = '';
let downloadUrl: string | null = null;

function setStatus(phase: Phase, text: string): void {
  state.phase = phase;
  statusBox.textContent = text;
  statusBox.className = `status status-${phase}`;
}

/** Строка о модуле x2t: он поднимается по первой конвертации, и страница об этом сообщает. */
function renderModule(): void {
  const text = {
    cold: 'модуль x2t: не поднят',
    booting: 'модуль x2t: поднимается…',
    ready: `модуль x2t: поднят за ${((state.bootMs ?? 0) / 1000).toFixed(1)} с`,
    error: 'модуль x2t: не поднялся',
  }[state.module];
  moduleBox.textContent = text;
}

/**
 * Строка состояния просмотра. Вьюер живёт своей жизнью (его воркер), поэтому темп его
 * кадров берём из `viewerState`: `ticks` растёт, когда painter вправду что-то нарисовал.
 */
function renderViewerStatus(): void {
  const viewer = viewerState();
  switch (viewer.status) {
    case 'closed':
      viewerBox.textContent = 'документ не открыт';
      break;
    case 'opening':
      viewerBox.textContent = `открываю ${state.viewerSource ?? 'документ'}…`;
      break;
    case 'ready': {
      const size = viewer.pageSize;
      const geometry = size === null ? '' : `, страница ${Math.round(size.width)}×${Math.round(size.height)} пкс`;
      viewerBox.textContent =
        `${state.viewerSource ?? 'документ'}: страниц ${viewer.pageCount}${geometry}, ` +
        `транспорт ${viewer.transport ?? '—'}, кадров ${viewer.ticks}`;
      break;
    }
    case 'error':
      viewerBox.textContent = `просмотр не открылся: ${viewer.error ?? 'неизвестная причина'}`;
      break;
  }
}

function renderLog(): void {
  logBox.textContent = state.log.join('\n');
}

function appendLog(line: string): void {
  state.log.push(line);
  if (state.log.length > LOG_LIMIT) {
    state.log.splice(0, state.log.length - LOG_LIMIT);
  }
  renderLog();
}

function renderMetrics(): void {
  const rows: string[] = [];
  if (state.bootMs !== null) {
    rows.push(`подъём модуля: ${(state.bootMs / 1000).toFixed(1)} с`);
  }
  if (state.pending !== null) {
    rows.push(`вход: ${inputName} — ${formatBytes(state.pending.inputBytes)}`);
  }
  const last = state.last;
  if (last !== null) {
    rows.push(`выход: ${last.outputName} — ${formatBytes(last.bytes.byteLength)}`);
    rows.push(`конвертация: ${(last.convertMs / 1000).toFixed(2)} с`);
    rows.push(`код возврата main: ${last.code}`);
    if (last.stderr.trim() !== '') {
      rows.push(`stderr: ${last.stderr.trim().split('\n').slice(0, 5).join(' | ')}`);
    }
  }
  metricsBox.textContent = rows.join('\n');
}

function renderBytes(bytes: Uint8Array, target: string): void {
  previewBox.textContent = '';
  if (bytes.byteLength === 0) {
    previewBox.textContent = 'Выходной файл не создан — смотрите код возврата и stderr.';
    return;
  }
  if (TEXT_TARGETS.has(target)) {
    previewBox.textContent = decodeTextPreview(bytes);
    return;
  }
  if (ZIP_TARGETS.has(target)) {
    const entries = listZipEntries(bytes);
    if (entries !== null) {
      const lines = entries
        .slice(0, PREVIEW_ENTRIES)
        .map((entry) => `${entry.name} — ${formatBytes(entry.size)}`);
      if (entries.length > PREVIEW_ENTRIES) {
        lines.push(`… всего записей: ${entries.length}`);
      }
      previewBox.textContent = `zip, записей ${entries.length}:\n${lines.join('\n')}`;
      return;
    }
  }
  previewBox.textContent = `${formatBytes(bytes.byteLength)}, первые байты:\n${hexPreview(bytes)}`;
}

/**
 * Показать документ нашим вьюером. Байты отдаются копией: воркер вьюера забирает буфер
 * переводом владения, а оригинал остаётся странице — он же уходит в конвертацию и в скачивание.
 */
async function renderDocument(bytes: Uint8Array, from: string): Promise<void> {
  state.viewerSource = from;
  if (bytes.byteLength === 0) {
    renderViewerStatus();
    appendLog(`просмотр: нечего показывать — ${from} пуст`);
    return;
  }
  renderViewerStatus();
  appendLog(`просмотр: открываю ${from}`);
  try {
    const pages = await openDocument(viewerHost, bytes.slice().buffer, from);
    appendLog(`просмотр: страниц ${pages.length}`);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    appendLog(`просмотр: отказ — ${message}`);
  }
  renderViewerStatus();
}

/** Расширение имени файла без точки, в нижнем регистре. */
function extensionOf(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  return dot === -1 ? '' : fileName.slice(dot + 1).toLowerCase();
}

function updateDownload(bytes: Uint8Array<ArrayBuffer>, outputName: string): void {
  if (downloadUrl !== null) {
    URL.revokeObjectURL(downloadUrl);
    downloadUrl = null;
  }
  const extension = outputName.slice(outputName.lastIndexOf('.') + 1).toLowerCase();
  const blob = new Blob([bytes], { type: MIME_BY_EXTENSION[extension] ?? 'application/octet-stream' });
  downloadUrl = URL.createObjectURL(blob);
  downloadLink.href = downloadUrl;
  downloadLink.download = outputName;
  downloadLink.hidden = false;
}

worker.onmessage = (event: MessageEvent<WorkerToMain>) => {
  const message = event.data;
  if (message.protocol !== PROTOCOL) {
    appendLog(`незнакомый протокол сообщения: ${message.protocol}`);
    return;
  }
  switch (message.kind) {
    case 'ready':
      state.bootMs = message.bootMs;
      state.module = 'ready';
      appendLog(`модуль x2t поднялся за ${(message.bootMs / 1000).toFixed(1)} с`);
      renderModule();
      renderMetrics();
      break;
    case 'log':
      appendLog(message.line);
      break;
    case 'error':
      state.error = message.message;
      state.module = 'error';
      appendLog(`ошибка: ${message.message}`);
      setStatus('error', 'ошибка');
      renderModule();
      break;
    case 'result': {
      const bytes = new Uint8Array(message.bytes);
      state.last = {
        outputName: message.outputName,
        code: message.code,
        stderr: message.stderr,
        convertMs: message.convertMs,
        bytes,
      };
      const extension = extensionOf(message.outputName);
      renderBytes(bytes, extension);
      updateDownload(bytes, message.outputName);
      renderMetrics();
      setStatus('done', message.code === 0 ? 'готово' : `отказ (код ${message.code})`);
      convertButton.disabled = false;
      pdfButton.disabled = false;
      // Модуль не пережил первую конвертацию: `ready` от него так и не пришёл.
      if (message.code !== 0 && state.module === 'booting') {
        state.module = 'error';
        renderModule();
      }
      // Конвертированный документ показывается тем же вьюером, что и выбранный файл.
      if (extension === 'docx' && bytes.byteLength > 0) {
        void renderDocument(bytes, `${message.outputName} (результат конвертации)`);
      }
      break;
    }
  }
};

worker.onerror = (event) => {
  state.error = event.message;
  appendLog(`ошибка воркера: ${event.message}`);
  setStatus('error', 'ошибка');
};

fileInput.addEventListener('change', () => {
  const file = fileInput.files?.[0];
  if (file === undefined) {
    inputBytes = null;
    inputName = '';
    return;
  }
  // Chromium не шлёт `change`, если выбран тот же самый файл: без сброса повторный выбор
  // (в приёмке — вторая конвертация той же фикстуры) не запускал бы ничего.
  fileInput.value = '';
  inputName = file.name;
  const extension = extensionOf(file.name);
  void file.arrayBuffer().then((buffer) => {
    // Счётчик растёт, когда файл вправду прочитан: по нему ждёт приёмка, а не по строке лога —
    // её вытесняют записи просмотра, которые идут следом.
    state.inputSerial += 1;
    inputBytes = new Uint8Array(buffer);
    appendLog(`выбран файл: ${file.name} — ${formatBytes(file.size)}`);
    renderMetrics();
    if (extension === 'docx') {
      // Свой формат вьюера: показываем сразу, x2t для этого не нужен.
      void renderDocument(inputBytes, file.name);
      return;
    }
    // Остальные форматы сначала конвертируются в docx — вьюер их не разбирает. Целевой формат
    // подставляем сами: иначе выбранный файл пришлось бы показывать после ручной правки select.
    if (formatSelect.querySelector(`option[value="docx"]`) !== null) {
      formatSelect.value = 'docx';
    }
    appendLog(`${extension} покажется в просмотре после конвертации в docx — нажмите «Конвертировать»`);
  });
});

async function startConversion(target: string): Promise<void> {
  if (inputBytes === null) {
    appendLog('сначала выберите файл');
    return;
  }
  const id = nextRequestId++;
  state.pending = { id, inputBytes: inputBytes.byteLength, target };
  const request: MainToWorker = {
    protocol: PROTOCOL,
    kind: 'convert',
    id,
    fileName: inputName,
    to: target,
    bytes: inputBytes.slice().buffer,
  };
  convertButton.disabled = true;
  pdfButton.disabled = true;
  if (state.module === 'cold') {
    state.module = 'booting';
    renderModule();
  }
  setStatus('converting', `конвертация в ${target}`);
  renderMetrics();
  worker.postMessage(request, [request.bytes]);
}

convertButton.addEventListener('click', () => {
  void startConversion(formatSelect.value);
});

// Проверка pdf — такой же вызов конвертера, просто с заведомо неподдерживаемым выходом:
// код возврата и stderr показываются как есть.
pdfButton.addEventListener('click', () => {
  void startConversion('pdf');
});

/** Состояние для приёмки: Playwright читает его через `page.evaluate`. */
const harnessApi = {
  status: () => ({
    phase: state.phase,
    module: state.module,
    error: state.error,
    bootMs: state.bootMs,
    lastCode: state.last?.code ?? null,
    lastOutput: state.last?.outputName ?? null,
    lastConvertMs: state.last?.convertMs ?? null,
    outputBytes: state.last?.bytes.byteLength ?? null,
    stderr: state.last?.stderr ?? null,
    inputSerial: state.inputSerial,
    log: [...state.log],
  }),
  viewer: () => viewerState(),
  /** Текущий кадр холста вьюера картинкой PNG — по ней видно, что нарисовано, а не пусто. */
  exportViewerPng: () => viewerPngBase64(),
  readResultBase64: (): string | null => {
    const bytes = state.last?.bytes;
    if (bytes === undefined || bytes.byteLength === 0) {
      return null;
    }
    // По кускам: `String.fromCharCode(...bytes)` на большом файле упирается в лимит аргументов.
    const chunk = 0x8000;
    let binary = '';
    for (let offset = 0; offset < bytes.byteLength; offset += chunk) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
    }
    return btoa(binary);
  },
};

window.x2tHarness = harnessApi;

declare global {
  interface Window {
    x2tHarness?: typeof harnessApi;
  }
}

// Страница готова сразу: модуль x2t поднимается по первой конвертации, и просмотр `.docx`
// его не ждёт вовсе. Вьюер создаётся при первом показе — пустой холст на входе не нужен.
setStatus('ready', 'готово');
convertButton.disabled = false;
pdfButton.disabled = false;
renderModule();
renderViewerStatus();
renderMetrics();

// Уход со страницы останавливает воркер вьюера: он держит разобранный документ и wasm.
window.addEventListener('pagehide', () => destroyViewer(), { once: true });
