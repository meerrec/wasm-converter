// Приёмка страницы харнесса x2t: поднимает её, дожидается готовности модуля и прогоняет
// три конвертации мелкой фикстуры — docx → odt, docx → txt и попытку docx → pdf.
//
// Запуск:
//   node experiments/x2t-spike/tools/preview-check.mjs
//
// Сервер: если на порту из vite.config.ts уже отвечает харнесс (например, поднятый
// `pnpm -C experiments/x2t-spike dev`), скрипт пользуется им и не закрывает его; иначе
// поднимает собственный dev-сервер и гасит его за собой.
//
// Переопределения: X2T_URL (готовый адрес), X2T_PORT, X2T_OUT (каталог результатов),
// X2T_TIMEOUT (мс на конвертацию).
//
// РЕЗУЛЬТАТ: выходы и preview-check.json в /tmp/x2t-preview-check + сводка в stdout.
// Код возврата 1 — если модуль не поднялся или не сошлась проверка odt/txt; попытка pdf
// неудачей не считается: у сборки нет движка раскладки, и оба известных исхода — «нет pdf».

import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const TOOLS_DIR = dirname(fileURLToPath(import.meta.url));
const SPIKE_DIR = resolve(TOOLS_DIR, '..');
const FIXTURE = resolve(SPIKE_DIR, '..', '..', 'test-fixtures', 'docx', 'styles', 'doc_defaults.docx');
const OUT_DIR = process.env.X2T_OUT ?? '/tmp/x2t-preview-check';
const CONVERT_TIMEOUT_MS = Number(process.env.X2T_TIMEOUT ?? 120_000);
// Подъём — это 38 МБ wasm с локального сервера; запас нужен на медленных машинах.
const BOOT_TIMEOUT_MS = 300_000;

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
 * Один прогон через страницу: файл в <input type=file>, формат — в <select> или кнопкой
 * проверки pdf, ожидание конца, выгрузка результата. Состояние читается только после того,
 * как фаза стала терминальной.
 */
async function convert(page, target) {
  await page.setInputFiles('#file', FIXTURE);
  // Файл читается асинхронно: страница узнаёт о нём, когда дочитает File.arrayBuffer(),
  // а до этого клик попадёт в «сначала выберите файл» и ничего не сделает.
  await page.waitForFunction(
    () => window.x2tHarness?.status()?.log?.at(-1)?.startsWith('выбран файл') === true,
    undefined,
    { timeout: 30_000, polling: 50 },
  );

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

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  if (!existsSync(FIXTURE)) {
    throw new Error(`фикстура не найдена: ${FIXTURE}`);
  }
  const fixtureBytes = statSync(FIXTURE).size;
  console.log(`фикстура: ${FIXTURE} — ${formatBytes(fixtureBytes)}`);

  const { chromium } = await loadDependency('@playwright/test');
  const { url, server, reused } = await ensureServer();
  console.log(`страница: ${url} — сервер ${reused ? 'поднят заранее' : 'поднял скрипт'}`);

  const result = { startedAt: new Date().toISOString(), url, fixtureBytes, outDir: OUT_DIR };
  const consoleLines = [];
  const browser = await chromium.launch();
  const page = await browser.newPage();
  page.on('console', (message) => consoleLines.push(`${message.type()}: ${message.text()}`));
  page.on('pageerror', (error) => consoleLines.push(`pageerror: ${error.message}`));
  page.on('requestfailed', (request) =>
    consoleLines.push(`requestfailed: ${request.url()} — ${request.failure()?.errorText ?? ''}`),
  );

  try {
    const bootStarted = Date.now();
    await page.goto(url, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(
      // Терминальная ошибка — тоже повод выйти из ожидания: иначе таймаут молчит о причине.
      () => ['ready', 'error'].includes(window.x2tHarness?.status()?.phase),
      undefined,
      { timeout: BOOT_TIMEOUT_MS, polling: 250 },
    );
    const boot = await statusOf(page);
    result.bootMs = boot?.bootMs ?? null;
    result.bootWallMs = Date.now() - bootStarted;
    result.bootPhase = boot?.phase ?? null;
    result.bootError = boot?.error ?? null;
    if (boot?.phase !== 'ready') {
      throw new Error(`модуль не поднялся: ${boot?.error ?? 'неизвестная причина'}`);
    }
    const memory = await readMemory(page);
    result.memory = memory;
    console.log(
      `модуль поднялся за ${(result.bootWallMs / 1000).toFixed(1)} с ` +
        `(по странице ${(result.bootMs / 1000).toFixed(1)} с)`,
    );
    console.log(
      memory === null
        ? 'память: performance.memory недоступна'
        : `память: heap ${formatBytes(memory.used)} из ${formatBytes(memory.limit)}`,
    );

    // docx → odt: выход обязан быть zip с записью mimetype (в ODT она ещё и первая, несжатая).
    const odt = await convert(page, 'odt');
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
    result.ok = odtOk && txtOk;
  } catch (error) {
    result.failure = String(error?.message ?? error);
    console.log(`ПРОВАЛ: ${result.failure}`);
  } finally {
    result.console = consoleLines.slice(-40);
    for (const line of result.console) {
      console.log(`  ${line}`);
    }
    await browser.close();
    await server?.close();
  }

  writeFileSync(join(OUT_DIR, 'preview-check.json'), JSON.stringify(result, null, 2));
  console.log(`сводка: ${join(OUT_DIR, 'preview-check.json')}`);
  console.log(result.ok === true ? 'OK: odt и txt сошлись' : 'ПРОВАЛ: подробности выше и в сводке');
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
