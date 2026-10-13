/**
 * Страница харнесса: выбирает файл, шлёт его воркеру, показывает метрики и предпросмотр.
 * Вся работа с конвертером — в воркере, здесь только UI и обмен сообщениями.
 */
import { decodeTextPreview, formatBytes, hexPreview, listZipEntries } from './preview.js';
import { PROTOCOL, type MainToWorker, type WorkerToMain } from './protocol.js';

type Phase = 'booting' | 'ready' | 'converting' | 'done' | 'error';

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
  error: string | null;
  bootMs: number | null;
  log: string[];
  last: LastResult | null;
  pending: { id: number; inputBytes: number; target: string } | null;
} = { phase: 'booting', error: null, bootMs: null, log: [], last: null, pending: null };

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
const metricsBox = element<HTMLElement>('metrics');
const previewBox = element<HTMLElement>('preview');
const logBox = element<HTMLElement>('log');
const downloadLink = element<HTMLAnchorElement>('download');

// Классический воркер — вынужденно: x2t.js грузится через importScripts (см. src/worker.ts).
// Vite отдаёт такой воркер без сборки в dev и как iife-бандл в сборке, поэтому в самом
// воркере нет статических импортов, а обвязка подтягивается динамическим import().
const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'classic' });

let nextRequestId = 1;
let inputBytes: Uint8Array | null = null;
let inputName = '';
let downloadUrl: string | null = null;

function setStatus(phase: Phase, text: string): void {
  state.phase = phase;
  statusBox.textContent = text;
  statusBox.className = `status status-${phase}`;
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
      setStatus('ready', 'готово');
      convertButton.disabled = false;
      pdfButton.disabled = false;
      renderMetrics();
      break;
    case 'log':
      appendLog(message.line);
      break;
    case 'error':
      state.error = message.message;
      appendLog(`ошибка: ${message.message}`);
      setStatus('error', 'ошибка');
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
      renderBytes(bytes, message.outputName.slice(message.outputName.lastIndexOf('.') + 1));
      updateDownload(bytes, message.outputName);
      renderMetrics();
      setStatus('done', message.code === 0 ? 'готово' : `отказ (код ${message.code})`);
      convertButton.disabled = false;
      pdfButton.disabled = false;
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
  inputName = file.name;
  void file.arrayBuffer().then((buffer) => {
    inputBytes = new Uint8Array(buffer);
    appendLog(`выбран файл: ${file.name} — ${formatBytes(file.size)}`);
    renderMetrics();
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
    error: state.error,
    bootMs: state.bootMs,
    lastCode: state.last?.code ?? null,
    lastOutput: state.last?.outputName ?? null,
    lastConvertMs: state.last?.convertMs ?? null,
    outputBytes: state.last?.bytes.byteLength ?? null,
    stderr: state.last?.stderr ?? null,
    log: [...state.log],
  }),
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

setStatus('booting', 'подъём модуля');
renderMetrics();
