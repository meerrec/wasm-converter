// Приёмка страницы харнесса x2t: поднимает её, проверяет просмотр документа нашим вьюером и
// прогоняет три конвертации мелкой фикстуры — docx → odt, docx → txt и попытку docx → pdf.
//
// Запуск:
//   node experiments/x2t-spike/tools/preview-check.mjs
//
// Сервер: если на порту из vite.config.ts уже отвечает харнесс (например, поднятый
// `pnpm -C experiments/x2t-spike dev`), скрипт пользуется им и не закрывает его; иначе
// поднимает собственный dev-сервер и гасит его за собой.
//
// Переопределения: X2T_URL (готовый адрес), X2T_PORT, X2T_OUT (каталог результатов),
// X2T_TIMEOUT (мс на конвертацию), X2T_SKIP_CONVERT=1 — остановиться после проверки просмотра.
// Последнее — прогон без x2t: страница и вьюер работают без него, и в окружении, где сторонний
// wasm запускать нельзя, проверяется ровно то, что можно.
//
// РЕЗУЛЬТАТ: выходы, скриншоты и preview-check.json в /tmp/x2t-preview-check + сводка в stdout.
// Код возврата 1 — если не сошлась проверка просмотра, модуль x2t не поднялся или не сошлась
// проверка odt/txt; попытка pdf неудачей не считается: у сборки нет движка раскладки, и оба
// известных исхода — «нет pdf».

import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const TOOLS_DIR = dirname(fileURLToPath(import.meta.url));
const SPIKE_DIR = resolve(TOOLS_DIR, '..');
const VIEWER_DIR = resolve(SPIKE_DIR, '..', '..', 'examples', 'viewer');
const FIXTURE = resolve(SPIKE_DIR, '..', '..', 'test-fixtures', 'docx', 'styles', 'doc_defaults.docx');
/** Чужой формат: им проверяется, что выбор файла не поднимает x2t и целится в docx. */
const FOREIGN_FIXTURE = resolve(
  SPIKE_DIR,
  '..',
  '..',
  'test-fixtures',
  'xlsx',
  'content-single-cell.xlsx',
);
const OUT_DIR = process.env.X2T_OUT ?? '/tmp/x2t-preview-check';
const CONVERT_TIMEOUT_MS = Number(process.env.X2T_TIMEOUT ?? 120_000);
// Подъём — это 38 МБ wasm с локального сервера; запас нужен на медленных машинах.
const BOOT_TIMEOUT_MS = 300_000;
// Просмотр: ленивый DOCX-модуль страница забирает при первом открытии документа — это мегабайт
// с небольшим, но с холодного сервера и на медленной машине он идёт не мгновенно.
const VIEWER_TIMEOUT_MS = Number(process.env.X2T_VIEWER_TIMEOUT ?? 120_000);
/** Прогон без x2t: проверяется только просмотр (страница его не требует). */
const SKIP_CONVERT = process.env.X2T_SKIP_CONVERT === '1';

/**
 * Зависимости харнесса берутся из его собственного node_modules (он вне pnpm-workspace):
 * динамический import() в tools/ резолвится вверх по каталогам до `experiments/x2t-spike`.
 */
async function loadDependency(name) {
  try {
    return await import(name);
  } catch (error) {
    throw new Error(
      `${name} не загрузился — поставьте зависимости харнесса ` +
        `(pnpm -C experiments/x2t-spike install): ${error.message}`,
    );
  }
}

/**
 * Playwright берётся из `examples/viewer`: браузеры живут в общем кэше под сборки 1.63, а копия
 * харнесса (1.64) просит сборку, которой в кэше нет. Два экземпляра Playwright в одном процессе
 * не уживаются, поэтому копию выбираем до первого require.
 */
async function loadChromium() {
  const viewerEntry = join(VIEWER_DIR, 'package.json');
  if (existsSync(viewerEntry)) {
    return createRequire(viewerEntry)('@playwright/test').chromium;
  }
  return (await loadDependency('@playwright/test')).chromium;
}

/** Порт из vite.config.ts — тот же, что у `pnpm dev`; иначе скрипт не нашёл бы готовый сервер. */
function portFromConfig() {
  const match = /port:\s*(\d+)/.exec(readFileSync(join(SPIKE_DIR, 'vite.config.ts'), 'utf8'));
  return match ? Number(match[1]) : 5191;
}

function formatBytes(size) {
  if (size < 1024) return `${size} Б`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} КиБ`;
  return `${(size / (1024 * 1024)).toFixed(1)} МиБ`;
}

/** Наша это страница или чужой сервер на том же порту: у чужого нет разметки харнесса. */
async function looksLikeHarness(url) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(2000) });
    return response.ok && (await response.text()).includes('x2t');
  } catch {
    return false;
  }
}

/** Готовый сервер или свой: свой поднимается на свободном порту, даже если порт из конфига занят. */
async function ensureServer() {
  if (process.env.X2T_URL !== undefined) {
    return { url: process.env.X2T_URL, server: null, reused: true };
  }
  const port = Number(process.env.X2T_PORT ?? portFromConfig());
  const candidate = `http://localhost:${port}/`;
  if (await looksLikeHarness(candidate)) {
    return { url: candidate, server: null, reused: true };
  }
  const { createServer } = await loadDependency('vite');
  const server = await createServer({
    root: SPIKE_DIR,
    configFile: join(SPIKE_DIR, 'vite.config.ts'),
    server: { port, strictPort: false },
  });
  await server.listen();
  // На холодном dev-сервере первый запрос `?worker_file` изредка уходит мимо трансформации,
  // и воркер приезжает сырым TypeScript. Явный прогрев кладёт результат в граф модулей —
  // дальше браузер получает уже собранный воркер.
  await server.transformRequest('/src/worker.ts?worker_file&type=module');
  return { url: server.resolvedUrls?.local?.[0] ?? candidate, server, reused: false };
}

const EOCD_SIGNATURE = 0x06054b50;
const CENTRAL_HEADER_SIGNATURE = 0x02014b50;

/**
 * Имена записей zip по центральному каталогу — та же проверка, что на странице в
 * src/preview.ts: распаковывать нечего, важен сам факт пакета и запись `mimetype`.
 * `null` — сигнатуры не сошлись, то есть выход не zip.
 */
function zipEntryNames(buffer) {
  if (buffer.byteLength < 22) return null;
  let eocd = -1;
  for (let i = buffer.byteLength - 22; i >= Math.max(0, buffer.byteLength - 22 - 0xffff); i--) {
    if (buffer.readUInt32LE(i) === EOCD_SIGNATURE) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) return null;
  const count = buffer.readUInt16LE(eocd + 10);
  let offset = buffer.readUInt32LE(eocd + 16);
  const names = [];
  for (let i = 0; i < count; i++) {
    if (offset + 46 > buffer.byteLength || buffer.readUInt32LE(offset) !== CENTRAL_HEADER_SIGNATURE) {
      return null;
    }
    const nameLength = buffer.readUInt16LE(offset + 28);
    const extraLength = buffer.readUInt16LE(offset + 30);
    const commentLength = buffer.readUInt16LE(offset + 32);
    names.push(buffer.subarray(offset + 46, offset + 46 + nameLength).toString('utf8'));
    offset += 46 + nameLength + extraLength + commentLength;
  }
  return names;
}

/** Сколько символов decode() не понял как UTF-8 — для txt-выхода это отдельная строка отчёта. */
function utf8Replacements(buffer) {
  const text = new TextDecoder('utf-8', { fatal: false }).decode(buffer);
  return (text.match(/�/g) ?? []).length;
}

function statusOf(page) {
  return page.evaluate(() => window.x2tHarness?.status() ?? null);
}

/**
 * Положить фикстуру в <input type=file> и дождаться, пока страница её прочитает.
 *
 * Ждём по счётчику прочитанных файлов, а не по последней строке лога: следом за «выбран файл»
 * страница пишет строки просмотра, и строка-признак перестала бы быть последней. Счётчик же
 * растёт ровно один раз на выбор файла — по нему видно и второй прогон, а не только первый.
 */
async function selectFixture(page, fixture) {
  const before = (await statusOf(page))?.inputSerial ?? 0;
  await page.setInputFiles('#file', fixture);
  await page.waitForFunction(
    (prev) => (window.x2tHarness?.status()?.inputSerial ?? 0) > prev,
    before,
    { timeout: 30_000, polling: 50 },
  );
}

/**
 * Просмотр документа: выбранный `.docx` страница открывает нашим вьюером без x2t. Проверяем,
 * что холст вьюера появился ненулевым, документ разобран на страницы и кадр не пустой — PNG из
 * воркера раскладывается по пикселям, и в отчёт идёт доля непустых и число разных цветов.
 *
 * Разбирать кадр в самой странице дешевле, чем тащить в Node декодер PNG: `OffscreenCanvas`
 * с `getImageData` там уже есть — вьюер на нём и рисует.
 */
async function checkViewer(page, fixture) {
  const startedAt = Date.now();
  await selectFixture(page, fixture);
  await page.waitForFunction(
    () => ['ready', 'error'].includes(window.x2tHarness?.viewer()?.status),
    undefined,
    { timeout: VIEWER_TIMEOUT_MS, polling: 100 },
  );
  // Кадр рисует воркер: к моменту `ready` первый кадр уже попросили, но он мог ещё не дойти.
  await page.waitForFunction(
    () => (window.x2tHarness?.viewer()?.ticks ?? 0) > 0,
    undefined,
    { timeout: VIEWER_TIMEOUT_MS, polling: 100 },
  );
  const viewer = await page.evaluate(() => window.x2tHarness?.viewer() ?? null);
  const canvas = await page.evaluate(() => {
    const host = document.querySelector('#viewerHost');
    const canvas = host?.querySelector('canvas') ?? null;
    const box = canvas?.getBoundingClientRect() ?? null;
    return {
      present: canvas !== null,
      cssWidth: Math.round(box?.width ?? 0),
      cssHeight: Math.round(box?.height ?? 0),
      hostWidth: host?.clientWidth ?? 0,
      hostHeight: host?.clientHeight ?? 0,
    };
  });
  const frame = await page.evaluate(async () => {
    const base64 = (await window.x2tHarness?.exportViewerPng()) ?? null;
    if (base64 === null) return null;
    const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    const surface = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = surface.getContext('2d');
    context.drawImage(bitmap, 0, 0);
    const pixels = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
    const colors = new Set();
    let ink = 0;
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i] < 200 || pixels[i + 1] < 200 || pixels[i + 2] < 200) ink += 1;
      if (colors.size < 64) colors.add((pixels[i] << 16) | (pixels[i + 1] << 8) | pixels[i + 2]);
    }
    return {
      base64,
      bytes: bytes.byteLength,
      width: bitmap.width,
      height: bitmap.height,
      pixels: pixels.length / 4,
      ink,
      colors: [...colors],
    };
  });

  const framePath = join(OUT_DIR, 'viewer-frame.png');
  if (frame !== null) {
    writeFileSync(framePath, Buffer.from(frame.base64, 'base64'));
  }
  const shotPath = join(OUT_DIR, 'viewer-shot.png');
  await page.screenshot({ path: shotPath });
  const hostShotPath = join(OUT_DIR, 'viewer-host.png');
  await page.locator('#viewerHost').screenshot({ path: hostShotPath });

  const ok =
    viewer?.status === 'ready' &&
    viewer.pageCount > 0 &&
    viewer.pageSize !== null &&
    canvas.present &&
    canvas.cssWidth > 0 &&
    canvas.cssHeight > 0 &&
    frame !== null &&
    frame.ink > 0 &&
    frame.colors.length > 1;
  return {
    ok,
    wallMs: Date.now() - startedAt,
    status: viewer?.status ?? null,
    error: viewer?.error ?? null,
    source: viewer?.source ?? null,
    pageCount: viewer?.pageCount ?? null,
    pageSize: viewer?.pageSize ?? null,
    transport: viewer?.transport ?? null,
    ticks: viewer?.ticks ?? null,
    paintMs: viewer?.paintMs ?? null,
    canvas,
    frame:
      frame === null
        ? null
        : {
            bytes: frame.bytes,
            width: frame.width,
            height: frame.height,
            pixels: frame.pixels,
            ink: frame.ink,
            colors: frame.colors.length,
          },
    framePath: frame === null ? null : framePath,
    shotPath,
    hostShotPath,
  };
}

/**
 * Чужой формат в выборе файла. Вьюер разбирает только `.docx`, поэтому такой файл показывается
 * после конвертации — и проверка тут в том, чего страница **не** делает: выбора файла мало для
 * подъёма x2t (38 МБ), он остаётся холодным до нажатия «Конвертировать», а целевой формат
 * подставляется сам. Сама конвертация — уже прогон с x2t, он ниже.
 */
async function checkForeignFile(page, fixture) {
  await selectFixture(page, fixture);
  const status = await statusOf(page);
  const target = await page.evaluate(() => document.querySelector('#format')?.value ?? null);
  const hint = (status?.log ?? []).some((line) => line.includes('покажется в просмотре после конвертации'));
  const ok = status?.module === 'cold' && target === 'docx' && hint;
  return { ok, module: status?.module ?? null, target, hint };
}

/**
 * Один прогон через страницу: файл в <input type=file>, формат — в <select> или кнопкой
 * проверки pdf, ожидание конца, выгрузка результата. Состояние читается только после того,
 * как фаза стала терминальной.
 */
async function convert(page, target) {
  await selectFixture(page, FIXTURE);

  const startedAt = Date.now();
  if (target === 'pdf') {
    await page.click('#pdfCheck');
  } else {
    await page.selectOption('#format', target);
    await page.click('#convert');
  }
  // Клик переводит страницу в 'converting' синхронно, до ухода сообщения в воркер, поэтому
  // ожидание 'done' не может зацепить результат прошлой конвертации.
  await page.waitForFunction(
    () => ['done', 'error'].includes(window.x2tHarness?.status()?.phase),
    undefined,
    { timeout: CONVERT_TIMEOUT_MS, polling: 200 },
  );
  const status = await statusOf(page);
  const base64 = await page.evaluate(() => window.x2tHarness?.readResultBase64() ?? null);
  const bytes = base64 === null ? Buffer.alloc(0) : Buffer.from(base64, 'base64');
  const outPath = join(OUT_DIR, `doc_defaults.${target}`);
  if (bytes.byteLength > 0) {
    writeFileSync(outPath, bytes);
  }
  return {
    target,
    phase: status?.phase ?? null,
    error: status?.error ?? null,
    code: status?.lastCode ?? null,
    stderr: status?.stderr ?? '',
    convertMs: status?.lastConvertMs ?? null,
    wallMs: Date.now() - startedAt,
    bytes,
    outPath: bytes.byteLength > 0 ? outPath : null,
  };
}

/**
 * Конвертации фикстуры через x2t: docx → odt, docx → txt и попытка docx → pdf. Модуль
 * поднимается внутри первой конвертации — время подъёма и уходит в отчёт (`result.x2t`).
 */
async function runConversions(page, result) {
  // docx → odt: выход обязан быть zip с записью mimetype (в ODT она ещё и первая, несжатая).
  const odt = await convert(page, 'odt');
  // Модуль поднялся внутри этой конвертации: её `wallMs` включает подъём, отдельно его время
  // берём со страницы — она получила `ready` от воркера.
  const boot = await statusOf(page);
  result.x2t = { module: boot?.module ?? null, bootMs: boot?.bootMs ?? null };
  console.log(
    `модуль x2t: ${result.x2t.module ?? '—'}, подъём по странице ` +
      `${((result.x2t.bootMs ?? 0) / 1000).toFixed(1)} с (внутри первой конвертации)`,
  );
  const names = odt.bytes.byteLength > 0 ? zipEntryNames(odt.bytes) : null;
  const mimetypeIndex = names === null ? -1 : names.indexOf('mimetype');
  const odtOk = odt.phase === 'done' && odt.code === 0 && mimetypeIndex >= 0;
  result.odt = {
    code: odt.code,
    error: odt.error,
    stderr: odt.stderr,
    convertMs: odt.convertMs,
    wallMs: odt.wallMs,
    bytes: odt.bytes.byteLength,
    entries: names,
    mimetypeFirst: mimetypeIndex === 0,
    ok: odtOk,
    outPath: odt.outPath,
  };
  const odtShape =
    names === null
      ? 'не zip'
      : `zip, записей ${names.length}, mimetype ${mimetypeIndex < 0 ? 'нет' : mimetypeIndex === 0 ? 'первой записью' : `записью №${mimetypeIndex + 1}`}`;
  console.log(
    `docx → odt: код ${odt.code}, ${formatBytes(odt.bytes.byteLength)} за ${odt.wallMs} мс — ` +
      `${odtShape}${odtOk ? '' : ' — ПРОВАЛ'}`,
  );

  // docx → txt: выход должен быть непустым. Битая UTF-8 — известная беда на больших
  // документах (в params.xml не выставлен m_nCsvTxtEncoding), здесь только считаем замены.
  const txt = await convert(page, 'txt');
  const replacements = utf8Replacements(txt.bytes);
  const txtOk = txt.phase === 'done' && txt.code === 0 && txt.bytes.byteLength > 0;
  const firstLine = txt.bytes.subarray(0, 200).toString('utf8').split('\n')[0];
  result.txt = {
    code: txt.code,
    error: txt.error,
    stderr: txt.stderr,
    convertMs: txt.convertMs,
    wallMs: txt.wallMs,
    bytes: txt.bytes.byteLength,
    utf8Replacements: replacements,
    firstLine,
    ok: txtOk,
    outPath: txt.outPath,
  };
  console.log(
    `docx → txt: код ${txt.code}, ${formatBytes(txt.bytes.byteLength)} за ${txt.wallMs} мс — ` +
      `замен UTF-8 ${replacements}, первая строка: ${JSON.stringify(firstLine)}` +
      `${txtOk ? '' : ' — ПРОВАЛ'}`,
  );

  // docx → pdf: оба известных исхода — «pdf нет», их и печатаем. Созданный файл — сюрприз,
  // он попадает в отчёт, но приёмку odt/txt не валит: у сборки нет движка раскладки.
  const pdf = await convert(page, 'pdf');
  const pdfKind =
    pdf.bytes.byteLength > 0
      ? 'created'
      : pdf.code < 0 || /crash|abort|out of bounds/i.test(pdf.stderr)
        ? 'crash'
        : 'code';
  result.pdf = {
    kind: pdfKind,
    code: pdf.code,
    error: pdf.error,
    stderr: pdf.stderr.split('\n').slice(0, 3).join(' | '),
    convertMs: pdf.convertMs,
    wallMs: pdf.wallMs,
    bytes: pdf.bytes.byteLength,
    outPath: pdf.outPath,
  };
  const pdfLines = {
    code: `PDF нет: конвертер вернул код ${pdf.code} и не создал файл`,
    crash: `PDF нет: рантайм упал, обвязка бросила X2tError (${result.pdf.stderr.trim() || 'без stderr'})`,
    created: `PDF создан: ${formatBytes(pdf.bytes.byteLength)} — неожиданно, смотрите отчёт`,
  };
  console.log(`docx → pdf: ${pdfLines[pdfKind]} за ${pdf.wallMs} мс`);

  const memoryAfter = await readMemory(page);
  if (memory !== null && memoryAfter !== null) {
    result.memoryAfter = memoryAfter;
    console.log(`память после прогона: heap ${formatBytes(memoryAfter.used)}`);
  }

  return { odtOk, txtOk };
}

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  if (!existsSync(FIXTURE)) {
    throw new Error(`фикстура не найдена: ${FIXTURE}`);
  }
  const fixtureBytes = statSync(FIXTURE).size;
  console.log(`фикстура: ${FIXTURE} — ${formatBytes(fixtureBytes)}`);

  const result = { startedAt: new Date().toISOString(), fixtureBytes, outDir: OUT_DIR };
  const consoleLines = [];
  let server = null;
  let browser = null;

  try {
    const chromium = await loadChromium();
    const ensured = await ensureServer();
    server = ensured.server;
    result.url = ensured.url;
    console.log(`страница: ${result.url} — сервер ${ensured.reused ? 'поднят заранее' : 'поднял скрипт'}`);
    browser = await chromium.launch();
    const page = await browser.newPage();
    page.on('console', (message) => consoleLines.push(`${message.type()}: ${message.text()}`));
    page.on('pageerror', (error) => consoleLines.push(`pageerror: ${error.message}`));
    page.on('requestfailed', (request) =>
      consoleLines.push(`requestfailed: ${request.url()} — ${request.failure()?.errorText ?? ''}`),
    );

    await page.goto(result.url, { waitUntil: 'domcontentloaded' });
    // Страница готова сразу: модуль x2t поднимается по первой конвертации, а просмотр `.docx`
    // не ждёт его вовсе — ждать здесь нечего, кроме самой страницы.
    await page.waitForFunction(() => window.x2tHarness !== undefined, undefined, {
      timeout: 30_000,
      polling: 50,
    });

    // Просмотр идёт первым: это наш вьюер, и x2t ему не нужен.
    const viewer = await checkViewer(page, FIXTURE);
    result.viewer = viewer;
    const frameLine =
      viewer.frame === null
        ? 'кадр не выгрузился'
        : `кадр ${formatBytes(viewer.frame.bytes)} ${viewer.frame.width}×${viewer.frame.height}, ` +
          `непустых пикселей ${viewer.frame.ink} из ${viewer.frame.pixels}, цветов ${viewer.frame.colors}`;
    console.log(
      `просмотр: ${viewer.status}, страниц ${viewer.pageCount}, ` +
        `холст ${viewer.canvas.cssWidth}×${viewer.canvas.cssHeight}, транспорт ${viewer.transport}, ` +
        `${frameLine}${viewer.ok ? '' : ' — ПРОВАЛ'}`,
    );
    if (viewer.error !== null) {
      console.log(`  отказ просмотра: ${viewer.error}`);
    }
    console.log(`  скриншоты: ${viewer.shotPath}, ${viewer.hostShotPath}, кадр ${viewer.framePath}`);

    // Чужой формат: просмотр его ещё не показывает, но и x2t ради него не поднимается.
    const foreign = await checkForeignFile(page, FOREIGN_FIXTURE);
    result.foreign = foreign;
    console.log(
      `чужой формат: модуль ${foreign.module}, целевой формат ${foreign.target}` +
        `${foreign.ok ? '' : ' — ПРОВАЛ'}`,
    );

    const memory = await readMemory(page);
    result.memory = memory;
    console.log(
      memory === null
        ? 'память: performance.memory недоступна'
        : `память: heap ${formatBytes(memory.used)} из ${formatBytes(memory.limit)}`,
    );

    if (SKIP_CONVERT) {
      // Прогон без x2t: страница и просмотр проверены, конвертации пропускаем.
      result.skipped = 'X2T_SKIP_CONVERT=1 — конвертации не прогонялись';
      console.log('конвертации пропущены: X2T_SKIP_CONVERT=1');
      result.ok = viewer.ok && foreign.ok;
    } else {
      const { odtOk, txtOk } = await runConversions(page, result);
      result.ok = viewer.ok && foreign.ok && odtOk && txtOk;
    }
  } catch (error) {
    result.failure = String(error?.message ?? error);
    console.log(`ПРОВАЛ: ${result.failure}`);
  } finally {
    result.console = consoleLines.slice(-40);
    for (const line of result.console) {
      console.log(`  ${line}`);
    }
    await browser?.close();
    await server?.close();
  }

  writeFileSync(join(OUT_DIR, 'preview-check.json'), JSON.stringify(result, null, 2));
  console.log(`сводка: ${join(OUT_DIR, 'preview-check.json')}`);
  console.log(
    result.ok === true
      ? `OK: просмотр${SKIP_CONVERT ? '' : ', odt и txt'} сошлись`
      : 'ПРОВАЛ: подробности выше и в сводке',
  );
  process.exit(result.ok === true ? 0 : 1);
}

function readMemory(page) {
  return page.evaluate(() => {
    const memory = performance.memory;
    return memory
      ? { used: memory.usedJSHeapSize, total: memory.totalJSHeapSize, limit: memory.jsHeapSizeLimit }
      : null;
  });
}

await main();
