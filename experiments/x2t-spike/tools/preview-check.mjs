// Приёмка страницы харнесса x2t: поднимает её, проверяет просмотр документа нашим вьюером,
// прогоняет три конвертации мелкой фикстуры — docx → odt, docx → txt и попытку docx → pdf, —
// и следом смоук пакета @doc-converter/browser-convert: docx → odt, docx → pdf и
// extractDocxMediaUrls через X2TConverter из собранного dist (src/bc-smoke.ts, тянется
// страницей из evaluate мимо воркера харнесса).
//
// Запуск:
//   pnpm -F @doc-converter/browser-convert build   # dist смоука — собирает вызывающий
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
// РЕЗУЛЬТАТ: выходы, скриншоты и preview-check.json (поле browserConvert) в
// /tmp/x2t-preview-check + сводка в stdout. Код возврата 1 — если не сошлась проверка
// просмотра, модуль x2t не поднялся, не сошлась проверка odt/txt или разошёлся смоук
// browser-convert; попытка pdf неудачей не считается ни у воркера, ни у смоука: у сборки нет
// движка раскладки, и ожидаемый исход — код 80 без выходного файла.

import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const TOOLS_DIR = dirname(fileURLToPath(import.meta.url));
const SPIKE_DIR = resolve(TOOLS_DIR, '..');
const VIEWER_DIR = resolve(SPIKE_DIR, '..', '..', 'examples', 'viewer');
const FIXTURE = resolve(SPIKE_DIR, '..', '..', 'test-fixtures', 'docx', 'styles', 'doc_defaults.docx');
/** Чужой формат: им проверяется, что выбор файла не поднимает x2t и целится в формат своего семейства. */
const FOREIGN_FIXTURE = resolve(
  SPIKE_DIR,
  '..',
  '..',
  'test-fixtures',
  'xlsx',
  'content-single-cell.xlsx',
);
/** Собранный dist пакета browser-convert: скрипт его не собирает, только проверяет наличие. */
const BROWSER_CONVERT_DIST = resolve(
  SPIKE_DIR,
  '..',
  '..',
  'packages',
  'browser-convert',
  'dist',
  'index.js',
);
/** Фикстура с медиа (word/media/tight.png) — для extractDocxMediaUrls. */
const MEDIA_FIXTURE = resolve(
  SPIKE_DIR,
  '..',
  '..',
  'test-fixtures',
  'docx',
  'images',
  'anchor_wrap_tight.docx',
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
 * Чужой формат в выборе файла. Вьюер разбирает только `.docx`, а цель из чужого семейства
 * (таблица в `docx`) в x2t заведомо отказывает (код 88), поэтому страница целится в формат
 * своего семейства и просмотр не обещает. Проверка тут в том, чего страница **не** делает:
 * выбора файла мало для подъёма x2t (38 МБ), он остаётся холодным до нажатия «Конвертировать»,
 * а целевой формат подставляется сам. Сама конвертация — уже прогон с x2t, он ниже.
 */
async function checkForeignFile(page, fixture) {
  await selectFixture(page, fixture);
  const status = await statusOf(page);
  const target = await page.evaluate(() => document.querySelector('#format')?.value ?? null);
  const hint = (status?.log ?? []).some((line) =>
    line.includes('просмотр в этом вьюере доступен только документному семейству'),
  );
  const ok = status?.module === 'cold' && target === 'ods' && hint;
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
  // Модуль поднимается внутри первой конвертации, и её ожидание обязано вместить подъём: у него
  // свой, куда более щедрый бюджет — это 38 МБ с локального сервера.
  const module = (await statusOf(page))?.module ?? null;
  const timeout = module === 'ready' ? CONVERT_TIMEOUT_MS : CONVERT_TIMEOUT_MS + BOOT_TIMEOUT_MS;
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
    { timeout, polling: 200 },
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
  if (memoryAfter !== null) {
    result.memoryAfter = memoryAfter;
    console.log(`память после прогона: heap ${formatBytes(memoryAfter.used)}`);
  }

  return { odtOk, txtOk };
}

/**
 * Байты из отчёта смоука в Buffer. Playwright сериализует typed arrays между браузером и
 * Node, и на выходе они приезжают Buffer'ом или Uint8Array — принимаем оба вида.
 */
function toBuffer(value) {
  if (value === null || value === undefined) return Buffer.alloc(0);
  if (Buffer.isBuffer(value)) return value;
  if (value instanceof Uint8Array) {
    return Buffer.from(value.buffer, value.byteOffset, value.byteLength);
  }
  return Buffer.alloc(0);
}

/**
 * Смоук пакета @doc-converter/browser-convert: страница гоняет конвертации через
 * `X2TConverter` из собранного dist — мимо воркера харнесса, на главном потоке (модуль
 * src/bc-smoke.ts тянется из evaluate динамическим import() через Vite, в разметке его нет).
 * Байты фикстур читает Node и передаёт в evaluate; отчёты — сериализуемые структуры.
 *
 * Сборку dist скрипт не делает — только проверяет наличие и просит собрать
 * (`pnpm -F @doc-converter/browser-convert build`): так надёжнее для CI-стиля запуска, где
 * сборку делает вызывающий. Сбои смоука валят приёмку, кроме pdf-кода 80 (нет движка
 * раскладки, ADR-0024) — это ожидаемый исход, и он печатается как таковой.
 */
async function runBrowserConvertSmoke(page, result, server) {
  if (!existsSync(BROWSER_CONVERT_DIST)) {
    throw new Error(
      `browser-convert не собран: нет ${BROWSER_CONVERT_DIST} — соберите пакет ` +
        '(pnpm -F @doc-converter/browser-convert build) и повторите',
    );
  }
  if (!existsSync(MEDIA_FIXTURE)) {
    throw new Error(`фикстура не найдена: ${MEDIA_FIXTURE}`);
  }
  // Прогрев как для воркера в ensureServer: первый запрос модуля на холодном dev-сервере
  // изредка уходит мимо трансформации. Готовый чужой сервер не трогаем — он уже прогрет.
  if (server !== null) {
    await server.transformRequest('/src/bc-smoke.ts');
  }
  console.log(
    `browser-convert: dist готов (${formatBytes(statSync(BROWSER_CONVERT_DIST).size)})`,
  );
  result.browserConvert = { dist: BROWSER_CONVERT_DIST };

  // docx → odt через X2TConverter: выход обязан быть zip с записью mimetype. Это первый вызов
  // смоука — он поднимает свой модуль x2t на странице (38 МБ wasm), поэтому бюджет как у
  // подъёма воркера: таймаут конвертации плюс таймаут подъёма.
  const odtStartedAt = Date.now();
  const odt = await page.evaluate(
    async ({ bytes, name, targetExt }) => {
      const { bcSmoke } = await import('/src/bc-smoke.ts');
      return bcSmoke({ convert: { bytes, name, targetExt } });
    },
    { bytes: new Uint8Array(readFileSync(FIXTURE)), name: 'doc_defaults.docx', targetExt: 'odt' },
    { timeout: CONVERT_TIMEOUT_MS + BOOT_TIMEOUT_MS },
  );
  const odtOutBytes = toBuffer(odt.convert?.outBytes ?? null);
  const odtNames = odtOutBytes.byteLength > 0 ? zipEntryNames(odtOutBytes) : null;
  const odtMimetype = odtNames === null ? -1 : odtNames.indexOf('mimetype');
  const odtOk = odt.convert !== null && odt.convert.code === 0 && odtMimetype >= 0;
  result.browserConvert.odt = {
    code: odt.convert?.code ?? null,
    boot: odt.boot,
    stderr: odt.convert?.stderr ?? '',
    bytes: odtOutBytes.byteLength,
    entries: odtNames,
    mimetypeFirst: odtMimetype === 0,
    wallMs: Date.now() - odtStartedAt,
    ok: odtOk,
  };
  const odtShape =
    odtNames === null
      ? 'не zip'
      : `zip, записей ${odtNames.length}, mimetype ${odtMimetype < 0 ? 'нет' : odtMimetype === 0 ? 'первой записью' : `записью №${odtMimetype + 1}`}`;
  const odtBoot =
    odt.boot === 'booted'
      ? 'модуль поднят этим вызовом'
      : odt.boot === 'reused'
        ? 'модуль переиспользован'
        : 'модуль не поднимался';
  console.log(
    `browser-convert → odt: код ${result.browserConvert.odt.code}, ` +
      `${formatBytes(odtOutBytes.byteLength)} за ${result.browserConvert.odt.wallMs} мс — ` +
      `${odtShape} (${odtBoot})${odtOk ? '' : ' — ПРОВАЛ'}`,
  );

  // docx → pdf через X2TConverter: рендер-цель, движка раскладки в wasm-сборке нет (ADR-0024) —
  // ожидаем код 80 и пустой выход. Любой другой исход — расхождение, он валит приёмку.
  const pdfStartedAt = Date.now();
  const pdf = await page.evaluate(
    async ({ bytes, name, targetExt }) => {
      const { bcSmoke } = await import('/src/bc-smoke.ts');
      return bcSmoke({ convert: { bytes, name, targetExt } });
    },
    { bytes: new Uint8Array(readFileSync(FIXTURE)), name: 'doc_defaults.docx', targetExt: 'pdf' },
    { timeout: CONVERT_TIMEOUT_MS },
  );
  const pdfOutBytes = toBuffer(pdf.convert?.outBytes ?? null);
  const pdfOk = pdf.convert !== null && pdf.convert.code === 80 && pdfOutBytes.byteLength === 0;
  result.browserConvert.pdf = {
    code: pdf.convert?.code ?? null,
    boot: pdf.boot,
    stderr: pdf.convert?.stderr ?? '',
    bytes: pdfOutBytes.byteLength,
    wallMs: Date.now() - pdfStartedAt,
    ok: pdfOk,
  };
  console.log(
    pdfOk
      ? `browser-convert → pdf: код 80, выход пуст за ${result.browserConvert.pdf.wallMs} мс — ` +
          'ожидаемо (движка раскладки нет)'
      : `browser-convert → pdf: код ${result.browserConvert.pdf.code}, ` +
          `${formatBytes(pdfOutBytes.byteLength)} за ${result.browserConvert.pdf.wallMs} мс — ` +
          'ПРОВАЛ (ждали код 80 и пустой выход)',
  );

  // extractDocxMediaUrls на фикстуре с медиа: word/media/tight.png → ключ 'media/tight.png'.
  const mediaStartedAt = Date.now();
  const media = await page.evaluate(
    async (bytes) => {
      const { bcSmoke } = await import('/src/bc-smoke.ts');
      return bcSmoke({ media: bytes });
    },
    new Uint8Array(readFileSync(MEDIA_FIXTURE)),
    { timeout: CONVERT_TIMEOUT_MS },
  );
  const mediaKeys = media.media?.mediaKeys ?? [];
  const mediaOk =
    media.media !== null && mediaKeys.length >= 1 && mediaKeys.every((key) => key.startsWith('media/'));
  result.browserConvert.media = {
    keys: mediaKeys,
    wallMs: Date.now() - mediaStartedAt,
    ok: mediaOk,
  };
  console.log(
    `browser-convert: media — ключей ${mediaKeys.length}: ${mediaKeys.join(', ') || 'нет'}` +
      `${mediaOk ? '' : ' — ПРОВАЛ (ждали ≥1 ключа вида media/…)'}`,
  );

  const ok = odtOk && pdfOk && mediaOk;
  result.browserConvert.ok = ok;
  return { ok };
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
      const browserConvert = await runBrowserConvertSmoke(page, result, server);
      result.ok = viewer.ok && foreign.ok && odtOk && txtOk && browserConvert.ok;
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
      ? `OK: просмотр${SKIP_CONVERT ? '' : ', odt, txt и browser-convert'} сошлись`
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
