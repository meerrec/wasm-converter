// Харнесс спайка LOWA: сторона страницы (главный поток).
//
// Страница только возит байты: кладёт выбранный файл в memfs, просит офис сконвертировать
// и показывает готовый PDF. UNO живёт в воркере (public/office-thread.js) — здесь его нет.
// Контракт инициализации — runtime-contract §1: выставить Module.{module.uno_scripts,
// locateFile, mainScriptUrlOrBlob, canvas} до запуска soffice.js, затем дождаться
// Module.uno_main (это MessagePort к офису) и window.FS.

const MIRROR_BASE = '/lowa/';
const BOOT_TIMEOUT_MS = 300_000;
const CONVERT_TIMEOUT_MS = 300_000;

// Фильтры экспорта по расширению. Имена проверены перечислением FilterFactory
// (кнопка «Перечислить фильтры», см. docs/sprint-13/harness.md).
const FILTERS = {
  docx: 'writer_pdf_Export',
  doc: 'writer_pdf_Export',
  odt: 'writer_pdf_Export',
  rtf: 'writer_pdf_Export',
  txt: 'writer_pdf_Export',
  xlsx: 'calc_pdf_Export',
  xls: 'calc_pdf_Export',
  ods: 'calc_pdf_Export',
  csv: 'calc_pdf_Export',
};

const state = {
  phase: 'idle',
  log: [],
  error: null,
  pdfBytes: 0,
  pdfUrl: null,
  probe: null,
};

let port = null;
let nextId = 1;
const pending = new Map();
// Резолвится сообщением {cmd:'ready'} из офиса; назначается в boot().
let officeReady = null;

function log(message) {
  const line = `[${new Date().toISOString().slice(11, 19)}] ${message}`;
  state.log.push(line);
  const box = document.getElementById('log');
  if (box) {
    box.textContent = `${state.log.join('\n')}\n`;
    box.scrollTop = box.scrollHeight;
  }
  console.log(`[harness] ${message}`);
  syncStatus();
}

function setPhase(phase) {
  state.phase = phase;
  syncStatus();
}

function syncStatus() {
  const el = document.getElementById('status');
  if (el) {
    el.textContent = `${state.phase}${state.error ? `: ${state.error}` : ''}`;
  }
  const box = document.getElementById('statusBox');
  if (box) {
    box.className = `status status-${state.phase === 'error' ? 'error' : state.phase === 'ready' || state.phase === 'done' ? 'ok' : 'busy'}`;
  }
}

/** Ждёт промис, но не бесконечно: у офиса есть привычка молчать при внутренней ошибке. */
function withTimeout(promise, ms, what) {
  let timer = null;
  const timeout = new Promise((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`${what}: нет ответа за ${ms / 1000} с`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => {
    if (timer !== null) {
      clearTimeout(timer);
    }
  });
}

/** Оборачивает Module.uno_main: обещание, которое резолвит офис, и таймаут на его молчание. */
function unoMain(module) {
  return withTimeout(Promise.resolve(module.uno_main), BOOT_TIMEOUT_MS, 'uno_main');
}

/** Шлёт команду офису и ждёт ответ с тем же id. */
function send(msg, timeoutMs = CONVERT_TIMEOUT_MS) {
  if (port === null) {
    return Promise.reject(new Error('офис не поднят: нет порта'));
  }
  const id = nextId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`команда ${msg.cmd} не получила ответа за ${timeoutMs / 1000} с`));
    }, timeoutMs);
    pending.set(id, { resolve, reject, timer });
    port.postMessage({ ...msg, id });
  });
}

function onOfficeMessage(event) {
  const data = event.data;
  if (data === null || typeof data !== 'object') {
    return;
  }
  if (data.cmd === 'ready') {
    log(`офис готов (протокол ${data.protocol})`);
    officeReady?.();
    setPhase('ready');
    return;
  }
  const waiter = pending.get(data.id);
  if (waiter === undefined) {
    log(`ответ без запроса: ${JSON.stringify(data).slice(0, 200)}`);
    return;
  }
  pending.delete(data.id);
  clearTimeout(waiter.timer);
  if (data.ok) {
    waiter.resolve(data);
  } else {
    const err = data.error ?? {};
    const error = new Error(`${err.type ?? 'ошибка UNO'}: ${err.message}`);
    error.unoType = err.type;
    error.unoStack = err.stack;
    waiter.reject(error);
  }
}

/** Собирает Module ровно в том виде, в каком его ждёт soffice.js (runtime-contract §1). */
function makeModule(base) {
  const canvas = document.getElementById('qtcanvas');
  const module = {
    canvas,
    uno_scripts: [
      new URL('zeta.js', base).href,
      new URL('/office-thread.js', location.href).href,
    ],
    locateFile: (path, prefix) => (prefix ?? base.href) + path,
  };
  // Emscripten берёт отсюда скрипт для pthread-воркеров; Blob нужен, чтобы воркеры
  // догружали soffice.js с того же URL, что и страница.
  module.mainScriptUrlOrBlob = new Blob([`importScripts('${new URL('soffice.js', base).href}');`], {
    type: 'text/javascript',
  });
  return module;
}

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const el = document.createElement('script');
    el.src = src;
    el.onload = () => resolve(undefined);
    el.onerror = () => reject(new Error(`не загрузился ${src}`));
    document.body.appendChild(el);
  });
}

let booting = null;

/** Поднимает LOWA один раз: загрузка ~50 МБ и холодный старт ~30 с. */
export function boot() {
  if (booting !== null) {
    return booting;
  }
  booting = (async () => {
    if (!crossOriginIsolated) {
      throw new Error('нет cross-origin isolation: страница отдана без COOP/COEP');
    }
    const params = new URLSearchParams(location.search);
    const base = new URL(params.get('base') ?? MIRROR_BASE, location.href);
    log(`загрузка LOWA с ${base.href}`);
    setPhase('booting');
    const module = makeModule(base);
    // soffice.js читает Module из глобальной области — выставляем до вставки скрипта.
    window.Module = module;
    await loadScript(new URL('soffice.js', base).href);
    log('soffice.js загружен, ждём uno_main');
    const officePort = await unoMain(module);
    port = officePort;
    port.onmessage = onOfficeMessage;
    const fs = window.FS ?? module.FS;
    if (fs === undefined) {
      throw new Error('нет FS: soffice.js собран без EXPORTED_RUNTIME_METHODS=FS');
    }
    window.lowaFS = fs;
    log('uno_main получен, FS доступен, ждём готовности офиса');
    setPhase('office-starting');
    const readyPromise = new Promise((resolve) => {
      officeReady = resolve;
    });
    await withTimeout(readyPromise, BOOT_TIMEOUT_MS, 'готовность офиса');
    return state;
  })().catch((error) => {
    state.error = String(error.message ?? error);
    setPhase('error');
    log(`ошибка подъёма: ${state.error}`);
    throw error;
  });
  return booting;
}

/** Перечисление фильтров через FilterFactory (XNameAccess::getElementNames). */
export async function probe() {
  const reply = await send({ cmd: 'probe' }, 120_000);
  state.probe = reply;
  log(`фильтров всего: ${reply.count}; writer_pdf_Export: ${reply.names.includes('writer_pdf_Export')}; calc_pdf_Export: ${reply.names.includes('calc_pdf_Export')}`);
  return reply;
}

function filterFor(fileName) {
  const dot = fileName.lastIndexOf('.');
  const ext = dot > 0 ? fileName.slice(dot + 1).toLowerCase() : '';
  const filter = FILTERS[ext];
  if (filter === undefined) {
    log(`неизвестное расширение «${ext}», беру writer_pdf_Export`);
    return 'writer_pdf_Export';
  }
  return filter;
}

/** Конвертирует байты файла в PDF: memfs -> UNO -> memfs -> Blob. */
export async function convertFile(fileName, bytes) {
  const fs = window.lowaFS;
  const dot = fileName.lastIndexOf('.');
  const from = `/tmp/input${dot > 0 ? fileName.slice(dot) : ''}`;
  const to = '/tmp/output.pdf';
  const filter = filterFor(fileName);
  fs.writeFile(from, new Uint8Array(bytes));
  try {
    fs.unlink(to);
  } catch {
    /* выходного файла ещё нет — обычное дело */
  }
  log(`конвертация ${fileName} (${(bytes.byteLength ?? bytes.length) / 1048576} МБ), фильтр ${filter}`);
  setPhase('converting');
  const started = Date.now();
  try {
    const reply = await send({ cmd: 'convert', from, to, filter });
    const data = fs.readFile(to);
    const blob = new Blob([data], { type: 'application/pdf' });
    state.pdfBytes = data.length;
    state.pdfUrl = URL.createObjectURL(blob);
    const link = document.getElementById('download');
    link.href = state.pdfUrl;
    link.download = 'output.pdf';
    link.hidden = false;
    const frame = document.getElementById('preview');
    frame.src = state.pdfUrl;
    frame.hidden = false;
    log(`готово: ${data.length} Б за ${((Date.now() - started) / 1000).toFixed(1)} с (UNO ${reply.ms} мс)`);
    setPhase('done');
    return { bytes: data.length, ms: Date.now() - started, unoMs: reply.ms };
  } catch (error) {
    state.error = String(error.message ?? error);
    setPhase('error');
    log(`ошибка конвертации: ${state.error}`);
    throw error;
  } finally {
    try {
      fs.unlink(from);
    } catch {
      /* входной файл мог не создаться */
    }
  }
}

/** Отдаёт PDF из memfs в base64 — так результат забирает tools/harness-check.mjs без download-события. */
export function readPdfBase64() {
  const data = window.lowaFS.readFile('/tmp/output.pdf');
  let binary = '';
  for (let i = 0; i < data.length; i += 0x8000) {
    binary += String.fromCharCode.apply(null, data.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

export function status() {
  return { ...state, log: [...state.log] };
}

window.lowaHarness = { boot, probe, convertFile, readPdfBase64, status };

// Страница поднимает офис сразу: пользователю остаётся выбрать файл.
document.getElementById('file').addEventListener('change', (event) => {
  const file = event.target.files[0];
  if (file === undefined) {
    return;
  }
  event.target.disabled = true;
  file
    .arrayBuffer()
    .then((buffer) => convertFile(file.name, new Uint8Array(buffer)))
    .catch((error) => log(`выбор файла: ${error.message}`))
    .finally(() => {
      event.target.disabled = false;
    });
});

document.getElementById('probeButton').addEventListener('click', () => {
  probe().catch((error) => log(`перечисление фильтров: ${error.message}`));
});

boot()
  .then(() => document.getElementById('file').removeAttribute('disabled'))
  .catch(() => {
    /* ошибка уже в статусе и в логе */
  });
