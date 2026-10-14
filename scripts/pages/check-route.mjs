// Приёмка маршрута «Euro-Office» (`#/eurooffice`) на staging-артефакте GitHub Pages:
// поднимает статику артефакта (scripts/pages/stage.mjs --serve), открывает в
// Chromium страницу демо, открывает через неё docx- и xlsx-фикстуры и проверяет,
// что движок дошёл до `data-state="ready"`/`opened`, а документ и вправду нарисован —
// внутри рамки редактора (и вложенных в неё фреймов) есть canvas с ненулевыми
// размерами.
//
// Зачем отдельно от e2e примеров: те гоняют дев-сборку на дев-хосте, а уезжает на
// Pages артефакт — с запечённой базой `/wasm-converter/`, со своей раскладкой
// каталогов и без заголовков кодирования. Проверять надо ровно его.
//
// Это ручная (staging) приёмка, а не часть `pnpm turbo run test`: артефакт — это
// 600 МиБ движка, собранного из сети, и в CI такие прогоны не место.
//
// Запуск (из корня репозитория):
//   node scripts/pages/check-route.mjs
//
// Переопределения:
//   VIEWER_URL           — уже поднятый сервер или сразу адрес страницы
//                          (например http://127.0.0.1:5272/wasm-converter/viewer/#/eurooffice);
//                          задан — свой сервер скрипт не поднимает;
//   VIEWER_PORT          — порт своего сервера (не задан — берётся свободный);
//   VIEWER_STAGE_DIR     — каталог артефакта, по умолчанию .pages;
//   VIEWER_OUT           — каталог артефактов проверки, по умолчанию /tmp/eurooffice-route-check;
//   VIEWER_READY_TIMEOUT — мс на `data-state="ready"`, по умолчанию 120000;
//   VIEWER_OPEN_TIMEOUT  — мс на `document:opened` одного файла, по умолчанию 180000.
//
// Playwright берётся из examples/viewer, а не из зависимостей скрипта: браузеры в
// общем кэше стоят под его сборку, а второй копии пакета со своими ожиданиями по
// версии браузеров в workspace не заводится (тот же приём — в
// examples/eurooffice-viewer/tools/viewer-check.mjs).
//
// Код возврата 1 — если сервер не поднялся, ready не наступил, файл не открылся
// или canvas не отрисовался. Сообщения консоли браузера печатаются, но проверку
// не роняют: у чужого движка в консоли всегда что-то есть, а приёмка смотрит на
// состояние страницы и на холст.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { createServer } from 'node:net';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
/** Корень репозитория — от расположения скрипта, а не от cwd: запускать можно откуда угодно. */
const ROOT_DIR = resolve(SCRIPT_DIR, '..', '..');
const STAGE_SCRIPT = join(SCRIPT_DIR, 'stage.mjs');
const VIEWER_DIR = join(ROOT_DIR, 'examples', 'viewer');

/** Префикс сайта на Pages — тот же, что печёт stage.mjs: проверяем ровно его. */
const SITE_PREFIX = '/wasm-converter/';
/** Маршрут демо внутри артефакта: hash-роутер примера, база запечена на сборке. */
const DEMO_ROUTE = 'viewer/#/eurooffice';

const DOCX_FIXTURE = join(ROOT_DIR, 'test-fixtures', 'docx', 'styles', 'doc_defaults.docx');
const XLSX_FIXTURES_DIR = join(ROOT_DIR, 'test-fixtures', 'xlsx');

const STAGE_DIR = process.env.VIEWER_STAGE_DIR ?? '.pages';
const OUT_DIR = process.env.VIEWER_OUT ?? '/tmp/eurooffice-route-check';
const READY_TIMEOUT_MS = Number(process.env.VIEWER_READY_TIMEOUT ?? 120_000);
const OPEN_TIMEOUT_MS = Number(process.env.VIEWER_OPEN_TIMEOUT ?? 180_000);
/** 600 МиБ артефакта stage.mjs копирует заново на каждый прогон — и до конца копии молчит. */
const STAGE_TIMEOUT_MS = 180_000;

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

function formatBytes(size) {
  if (size < 1024) return `${size} Б`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} КиБ`;
  return `${(size / (1024 * 1024)).toFixed(1)} МиБ`;
}

/**
 * Playwright — из examples/viewer: браузеры в общем кэше стоят под его сборку, у
 * которой могут быть свои ожидания по версии. createRequire от его package.json
 * резолвит ту же копию пакета, что использует e2e примеров.
 */
function loadChromium() {
  const viewerEntry = join(VIEWER_DIR, 'package.json');
  if (!existsSync(viewerEntry)) {
    throw new Error(`нет ${viewerEntry} — Playwright ожидается из examples/viewer`);
  }
  return createRequire(viewerEntry)('@playwright/test').chromium;
}

/**
 * Мелкая xlsx-фикстура семейства `content-*`: самый маленький файл, но не
 * `content-empty-sheet` — проверке нужен факт отрисовки книги с содержимым, а не
 * объём. Остальные семейства (форматы, краевые случаи) для этой приёмки не нужны.
 */
function smallestXlsx() {
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name.startsWith('content-') && entry.name.endsWith('.xlsx') && !entry.name.includes('empty')) {
        files.push({ path, size: statSync(path).size });
      }
    }
  };
  walk(XLSX_FIXTURES_DIR);
  files.sort((a, b) => a.size - b.size);
  if (files.length === 0) {
    throw new Error(`в ${XLSX_FIXTURES_DIR} нет content-*.xlsx — сгенерируйте: pnpm gen:fixtures`);
  }
  return files[0];
}

/** Адрес страницы приёмки: в VIEWER_URL маршрут может быть уже указан, а может — только база. */
function routeUrl(value) {
  const trimmed = value.replace(/\/+$/, '');
  if (trimmed.includes('#')) return trimmed;
  if (trimmed.endsWith('/viewer')) return `${trimmed}/#/eurooffice`;
  return `${trimmed}/${DEMO_ROUTE}`;
}

/** Свободный порт от ядра: слушающий сокет на 0 выбирает порт сам, дальше он закрывается. */
function freePort() {
  return new Promise((resolvePort, reject) => {
    const probe = createServer();
    probe.on('error', reject);
    probe.listen(0, '127.0.0.1', () => {
      const { port } = probe.address();
      probe.close(() => resolvePort(port));
    });
  });
}

/** Есть ли страница по адресу: нужна и как проба готовности, и как проба чужого сервера. */
async function responds(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(5000) });
  // Тело дочитываем: иначе на чужом сервере незакрытое соединение переживёт проверку.
  await response.arrayBuffer();
  return response;
}

/**
 * Сервер приёмки: stage.mjs сам соберёт артефакт из источников, отдаст его под
 * префиксом `/wasm-converter/` и закроет гейты публикуемого. Готовность — ответ
 * на страницу демо, а не открытый порт: до конца проверки артефакта порт молчит
 * дольше, чем копируются 600 МиБ.
 */
async function startStageServer(port) {
  const child = spawn(
    process.execPath,
    [STAGE_SCRIPT, '--out', STAGE_DIR, '--serve', '--port', String(port)],
    { cwd: ROOT_DIR, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  let output = '';
  child.stdout.on('data', (chunk) => (output += chunk));
  child.stderr.on('data', (chunk) => (output += chunk));

  const base = `http://127.0.0.1:${port}${SITE_PREFIX}`;
  const deadline = Date.now() + STAGE_TIMEOUT_MS;
  while (Date.now() < deadline) {
    // Упавший stage.mjs (нет источников, не прошли гейты) не поднимется уже никогда.
    if (child.exitCode !== null) {
      throw new Error(`stage.mjs завершился с кодом ${child.exitCode} — сервер не поднялся:\n${output.trim()}`);
    }
    try {
      const response = await responds(`${base}viewer/`);
      if (response.ok) return { child, base };
    } catch {
      // сервер ещё собирает артефакт — не отвечает никто, это ожидаемо
    }
    await sleep(300);
  }
  child.kill();
  throw new Error(`артефакт не отдался за ${STAGE_TIMEOUT_MS / 1000} с:\n${output.trim()}`);
}

/** Состояние страницы и хук приёмки одним снимком — его же пишем в отчёт. */
async function readPage(page) {
  return page.evaluate(() => {
    const harness = window.euroofficeViewer;
    const status = document.querySelector('#viewer-status');
    return {
      state: status?.dataset.state ?? 'нет элемента',
      statusText: (status?.textContent ?? '').trim(),
      openedCount: harness?.openedCount ?? -1,
      lastFileName: harness?.lastFileName ?? '',
      lastError: harness?.lastError ?? null,
    };
  });
}

/**
 * Готовность движка. Ошибку ждём наравне с готовностью: `document:error` оставил
 * бы проверку висеть весь таймаут, хотя исход уже ясен, — а строка статуса в этом
 * случае и есть диагноз. Требование к состоянию от этого не мягче: проверка
 * проходит только на `ready`.
 */
async function waitForReady(page, timeout) {
  try {
    await page.waitForSelector('#viewer-status[data-state="ready"], #viewer-status[data-state="error"]', { timeout });
  } catch {
    // таймаут: фактическое состояние снимается ниже и попадает в отчёт
  }
  const snapshot = await readPage(page);
  if (snapshot.state !== 'ready') {
    throw new Error(
      `движок не дошёл до ready (состояние «${snapshot.state}»): ${snapshot.statusText}` +
        (snapshot.lastError === null ? '' : ` — ${snapshot.lastError}`),
    );
  }
  return snapshot;
}

/**
 * Ожидание окончания открытия файла. Счётчик открытий в `window.euroofficeViewer`
 * (хук приёмки страницы) снимает гонку: `data-state` от предыдущего файла ещё
 * может висеть в `opened`, пока новый только начинает читаться.
 */
async function waitForOpen(page, before, timeout) {
  try {
    await page.waitForFunction(
      (prev) => {
        const harness = window.euroofficeViewer;
        return harness !== undefined && (harness.openedCount > prev || harness.state === 'error');
      },
      before,
      { timeout, polling: 250 },
    );
  } catch {
    // таймаут: фактическое состояние снимается ниже и попадает в отчёт
  }
  return readPage(page);
}

/** Все рамки редактора: сам iframe и вложенные (движок рисует во внутренней рамке). */
function editorFrames(page) {
  const root = page.frames().find((frame) => frame !== page.mainFrame() && frame.url().includes('editor.html'));
  if (root === undefined) return [];
  const collect = (frame) => [frame, ...frame.childFrames().flatMap(collect)];
  return collect(root);
}

/** Ищет самый крупный видимый canvas в рамках редактора; null — так и не появился. */
async function waitForCanvas(page, timeout) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const frame of editorFrames(page)) {
      try {
        const info = await frame.evaluate(() => {
          let best = null;
          for (const canvas of document.querySelectorAll('canvas')) {
            const rect = canvas.getBoundingClientRect();
            if (canvas.width === 0 || canvas.height === 0 || rect.width === 0 || rect.height === 0) continue;
            const area = rect.width * rect.height;
            if (best === null || area > best.area) {
              best = {
                area: Math.round(area),
                width: canvas.width,
                height: canvas.height,
                cssWidth: Math.round(rect.width),
                cssHeight: Math.round(rect.height),
              };
            }
          }
          return best;
        });
        if (info !== null) return { frameUrl: frame.url(), ...info };
      } catch {
        // рамка недоступна (чужая / ещё не готова) — пропускаем
      }
    }
    await sleep(500);
  }
  return null;
}

/** Один документ: выбор файла → `document:opened` → canvas в рамке → скриншоты. */
async function checkDocument(page, fixture, label) {
  const startedAt = Date.now();
  const before = await page.evaluate(() => window.euroofficeViewer?.openedCount ?? 0);
  await page.setInputFiles('#viewer-file', fixture.path);
  const snapshot = await waitForOpen(page, before, OPEN_TIMEOUT_MS);
  const opened = snapshot.state === 'opened' && snapshot.openedCount > before;
  const canvas = opened ? await waitForCanvas(page, OPEN_TIMEOUT_MS) : null;

  const pageShot = join(OUT_DIR, `${label}-page.png`);
  await page.screenshot({ path: pageShot, fullPage: true });
  const frameShot = join(OUT_DIR, `${label}-frame.png`);
  let frameShotSaved = true;
  await page.locator('#viewer-frame').screenshot({ path: frameShot }).catch(() => (frameShotSaved = false));

  const ok = opened && canvas !== null;
  console.log(
    `${label}: ${fixture.path} (${formatBytes(fixture.size)}) — состояние «${snapshot.state}», ` +
      `openedCount ${before} → ${snapshot.openedCount}, canvas ${
        canvas === null
          ? 'нет'
          : `${canvas.cssWidth}×${canvas.cssHeight} px (${canvas.width}×${canvas.height}) в ${canvas.frameUrl}`
      } за ${Date.now() - startedAt} мс${ok ? '' : ' — ПРОВАЛ'}`,
  );
  if (!ok) console.log(`  статус страницы: ${snapshot.statusText}`);

  return {
    label,
    ok,
    wallMs: Date.now() - startedAt,
    fixture: fixture.path,
    fixtureBytes: fixture.size,
    openedCountBefore: before,
    ...snapshot,
    canvas,
    pageShot,
    frameShot: frameShotSaved ? frameShot : null,
  };
}

/** Почему документ не прошёл — словами, в отчёт и в консоль. */
function describeFailure(document) {
  if (document.state === 'error') return `${document.label}: движок вернул ошибку — ${document.statusText}`;
  if (document.state !== 'opened') return `${document.label}: состояние «${document.state}» вместо opened`;
  if (document.openedCount <= document.openedCountBefore) {
    return `${document.label}: счётчик открытий не вырос (${document.openedCountBefore} → ${document.openedCount})`;
  }
  return `${document.label}: canvas в рамке редактора не отрисовался`;
}

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  const docxFixture = { path: DOCX_FIXTURE, size: statSync(DOCX_FIXTURE).size };
  const xlsxFixture = smallestXlsx();
  /** Оба семейства документа: текстовый редактор и табличный — разные ветки движка. */
  const fixtures = [
    ['docx', docxFixture],
    ['xlsx', xlsxFixture],
  ];
  const result = {
    startedAt: new Date().toISOString(),
    outDir: OUT_DIR,
    fixtures: { docx: docxFixture.path, xlsx: xlsxFixture.path },
    console: [],
    documents: [],
    ok: false,
    failure: null,
  };
  const consoleLines = [];
  let server = null;
  let browser = null;
  let page = null;

  try {
    const chromium = loadChromium();
    if (process.env.VIEWER_URL) {
      result.url = routeUrl(process.env.VIEWER_URL);
      result.server = 'готовый (VIEWER_URL)';
      // Отдельная проба до браузера: недоступный хост иначе всплыл бы таймаутом
      // навигации Playwright'а — тем же сообщением, что и «страница пуста».
      const pageUrl = new URL(result.url);
      pageUrl.hash = '';
      const probe = await responds(pageUrl.href).catch((error) => {
        throw new Error(`VIEWER_URL не отвечает (${pageUrl.href}): ${String(error?.message ?? error)}`);
      });
      if (!probe.ok) throw new Error(`VIEWER_URL отвечает ${probe.status} (${pageUrl.href})`);
    } else {
      const port = process.env.VIEWER_PORT ? Number(process.env.VIEWER_PORT) : await freePort();
      const started = await startStageServer(port);
      server = started.child;
      result.url = `${started.base}${DEMO_ROUTE}`;
      result.server = `stage.mjs --out ${STAGE_DIR} --serve --port ${port}`;
    }
    console.log(`страница: ${result.url}`);
    console.log(`сервер:   ${result.server}`);

    browser = await chromium.launch();
    page = await browser.newPage();
    page.on('console', (message) => {
      if (message.type() === 'error' || message.type() === 'warning') {
        consoleLines.push(`${message.type()}: ${message.text()}`);
      }
    });
    page.on('pageerror', (error) => consoleLines.push(`pageerror: ${error.message}`));
    page.on('requestfailed', (request) =>
      consoleLines.push(`requestfailed: ${request.url()} — ${request.failure()?.errorText ?? ''}`),
    );

    await page.goto(result.url, { waitUntil: 'domcontentloaded' });
    console.log(`жду готовности движка (data-state="ready", до ${READY_TIMEOUT_MS / 1000} с)…`);
    const readyStartedAt = Date.now();
    const ready = await waitForReady(page, READY_TIMEOUT_MS);
    result.readyMs = Date.now() - readyStartedAt;
    result.readyText = ready.statusText;
    console.log(`движок готов за ${result.readyMs} мс: ${ready.statusText}`);

    for (const [label, fixture] of fixtures) {
      result.documents.push(await checkDocument(page, fixture, label));
    }

    const failures = result.documents.filter((document) => !document.ok).map(describeFailure);
    result.ok = result.documents.length === fixtures.length && failures.length === 0;
    result.failure = failures.length === 0 ? null : failures.join('; ');
  } catch (error) {
    result.failure = String(error?.message ?? error);
    // Строка статуса страницы (если она успела появиться) — самое полезное при разборе:
    // в ней текст ошибки движка с расшифровкой кода.
    result.statusOnFailure = null;
    if (page !== null) {
      result.statusOnFailure = await readPage(page)
        .then((snapshot) => snapshot.statusText)
        .catch(() => null);
    }
    console.log(`ПРОВАЛ: ${result.failure}`);
    if (result.statusOnFailure) console.log(`  статус страницы: ${result.statusOnFailure}`);
  } finally {
    await browser?.close();
    server?.kill();
  }

  // Консоль печатается всегда, в том числе после провала: при разборе она и есть
  // главная улика (перезагрузка страницы service worker'ом, заблокированные ответы).
  result.console = consoleLines.slice(-40);
  if (consoleLines.length > 0) {
    console.log(`\nсообщения консоли (${consoleLines.length}, последние 40):`);
    for (const line of result.console) console.log(`  ${line}`);
  }

  // Заблокированный ответ — отдельная подсказка: страница демо становится
  // cross-origin isolated (viewer/coi-serviceworker.js), а рамка редактора лежит
  // вне области его действия и заголовка COEP не несёт, — Chromium её не пускает
  // (coep-frame-resource-needs-coep-header). Без этой подсказки такой провал
  // выглядит как «движок не отвечает», и искать пойдут не там.
  const blockedRequests = consoleLines.filter((line) => line.includes('ERR_BLOCKED_BY_RESPONSE'));
  if (blockedRequests.length > 0) {
    result.blockedRequests = blockedRequests;
    console.log(`\nзаблокированные ответы (${blockedRequests.length}) — проверьте COEP/COOP у рамки редактора:`);
    for (const line of blockedRequests) console.log(`  ${line}`);
  }

  writeFileSync(join(OUT_DIR, 'check-route.json'), JSON.stringify(result, null, 2));
  console.log(`\nсводка:     ${join(OUT_DIR, 'check-route.json')}`);
  console.log(`скриншоты:  ${OUT_DIR}/*.png`);
  if (result.ok) {
    console.log('OK: маршрут #/eurooffice открыл docx и xlsx, документы отрисованы');
  } else {
    console.log(`ПРОВАЛ: ${result.failure ?? 'проверка не дошла до файлов'}`);
  }
  process.exit(result.ok ? 0 : 1);
}

await main();
