// Приёмка спайка eurooffice-viewer через Playwright: поднимает статический сервер
// собранного site/, открывает нашу страницу viewer.html, открывает через embed-API
// docx- и xlsx-фикстуры и проверяет, что редактор дошёл до document:opened и
// отрисовался (внутри iframe есть canvas с ненулевыми размерами).
//
// Запуск:
//   node experiments/eurooffice-viewer/scripts/assemble.mjs    # сайт должен быть собран
//   node experiments/eurooffice-viewer/tools/viewer-check.mjs
//
// Сервер: если VIEWER_URL не задан, скрипт ищет уже поднятый наш сервер на порту
// (VIEWER_PORT, по умолчанию 5271) и переиспользует его; иначе поднимает свой и
// гасит за собой.
//
// Переопределения: VIEWER_URL (готовый сервер), VIEWER_PORT, VIEWER_OUT (каталог
// артефактов, по умолчанию /tmp/eurooffice-viewer-check), VIEWER_READY_TIMEOUT
// (мс на document:ready, по умолчанию 120000), VIEWER_OPEN_TIMEOUT (мс на
// document:opened, по умолчанию 180000).
//
// Playwright берётся из examples/viewer: браузеры стоят под его сборку (паттерн
// x2t-spike/tools/preview-check.mjs). Своих node_modules у спайка нет.
// Код возврата 1 — если документ не открылся или canvas не отрисовался.

import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const TOOLS_DIR = dirname(fileURLToPath(import.meta.url));
const SPIKE_DIR = resolve(TOOLS_DIR, '..');
const REPO_ROOT = resolve(SPIKE_DIR, '..', '..');
const SERVE_SCRIPT = join(SPIKE_DIR, 'scripts', 'serve.mjs');
const SITE_EDITOR = join(SPIKE_DIR, 'site', 'editor.html');
const VIEWER_DIR = join(REPO_ROOT, 'examples', 'viewer');

const DOCX_FIXTURE = join(REPO_ROOT, 'test-fixtures', 'docx', 'styles', 'doc_defaults.docx');
const XLSX_FIXTURES_DIR = join(REPO_ROOT, 'test-fixtures', 'xlsx');

const OUT_DIR = process.env.VIEWER_OUT ?? '/tmp/eurooffice-viewer-check';
const READY_TIMEOUT_MS = Number(process.env.VIEWER_READY_TIMEOUT ?? 120_000);
const OPEN_TIMEOUT_MS = Number(process.env.VIEWER_OPEN_TIMEOUT ?? 180_000);

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

function formatBytes(size) {
  if (size < 1024) return `${size} Б`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} КиБ`;
  return `${(size / (1024 * 1024)).toFixed(1)} МиБ`;
}

/**
 * Playwright — из examples/viewer: браузеры в общем кэше стоят под его сборку,
 * у которой могут быть свои ожидания по версии. createRequire от его package.json
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
 * Мелкая xlsx-фикстура: берём самый маленький .xlsx, но не «пустой лист» —
 * проверке нужен факт отрисовки книги с содержимым, а не объём.
 */
function smallestXlsx() {
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name.endsWith('.xlsx') && !entry.name.includes('empty')) {
        files.push({ path, size: statSync(path).size });
      }
    }
  };
  walk(XLSX_FIXTURES_DIR);
  files.sort((a, b) => a.size - b.size);
  if (files.length === 0) {
    throw new Error(`в ${XLSX_FIXTURES_DIR} нет .xlsx-фикстур — сгенерируйте: pnpm gen:fixtures`);
  }
  return files[0];
}

/** Наш сервер или чужой на этом порту: у нашего в viewer.html есть заголовок страницы. */
async function ourServerResponds(baseUrl) {
  try {
    const response = await fetch(`${baseUrl}/viewer.html`, { signal: AbortSignal.timeout(2000) });
    if (!response.ok) return false;
    return (await response.text()).includes('Просмотрщик (Euro-Office)');
  } catch {
    return false;
  }
}

/** Готовый сервер (VIEWER_URL / уже поднятый на порту) или свой на свободном порту. */
async function ensureServer() {
  if (process.env.VIEWER_URL) {
    const url = process.env.VIEWER_URL.replace(/\/+$/, '');
    return { url, child: null, reused: true };
  }
  const port = Number(process.env.VIEWER_PORT ?? 5271);
  const candidate = `http://127.0.0.1:${port}`;
  if (await ourServerResponds(candidate)) {
    return { url: candidate, child: null, reused: true };
  }
  const child = spawn(process.execPath, [SERVE_SCRIPT, String(port)], { stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.on('data', (chunk) => (output += chunk));
  child.stderr.on('data', (chunk) => (output += chunk));
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`serve.mjs завершился с кодом ${child.exitCode}: ${output.trim()}`);
    }
    if (await ourServerResponds(candidate)) {
      return { url: candidate, child, reused: false };
    }
    await sleep(200);
  }
  child.kill();
  throw new Error(`сервер на порту ${port} не поднялся за 15 с: ${output.trim()}`);
}

/**
 * Ожидание окончания открытия. Счётчик открытий в window.euroofficeViewer
 * (тестовый хук viewer.html) снимает гонку: data-state от предыдущего файла ещё
 * может висеть в 'opened', пока новый только начинает читаться. Возвращает
 * состояние и строку статуса страницы — их проверка печатает в отчёт.
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
  const state = await page.evaluate(() => document.querySelector('#viewer-status')?.dataset.state ?? 'нет элемента');
  const text = ((await page.locator('#viewer-status').textContent()) ?? '').trim();
  return { state, text };
}

/** Все рамки редактора: сам iframe и вложенные (SDK рисует во внутренней рамке). */
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

/** Один документ: выбор файла → document:opened → canvas в iframe → скриншоты. */
async function checkDocument(page, fixture, label) {
  const startedAt = Date.now();
  const before = await page.evaluate(() => window.euroofficeViewer?.openedCount ?? 0);
  await page.setInputFiles('#viewer-file', fixture.path);
  const { state, text: statusText } = await waitForOpen(page, before, OPEN_TIMEOUT_MS);
  const canvas = state === 'opened' ? await waitForCanvas(page, OPEN_TIMEOUT_MS) : null;

  const pageShot = join(OUT_DIR, `${label}-page.png`);
  await page.screenshot({ path: pageShot });
  const frameShot = join(OUT_DIR, `${label}-iframe.png`);
  let frameShotSaved = true;
  await page.locator('#viewer-frame').screenshot({ path: frameShot }).catch(() => (frameShotSaved = false));

  const ok = state === 'opened' && canvas !== null;
  console.log(
    `${label}: ${fixture.path} (${formatBytes(fixture.size)}) — состояние ${state}, canvas ${
      canvas === null ? 'нет' : `${canvas.cssWidth}×${canvas.cssHeight} px (${canvas.width}×${canvas.height}) в ${canvas.frameUrl}`
    } за ${Date.now() - startedAt} мс${ok ? '' : ' — ПРОВАЛ'}`,
  );
  if (!ok) {
    console.log(`  статус страницы: ${statusText}`);
  }
  return {
    ok,
    state,
    statusText,
    fixture: fixture.path,
    fixtureBytes: fixture.size,
    wallMs: Date.now() - startedAt,
    canvas,
    pageShot,
    frameShot: frameShotSaved ? frameShot : null,
  };
}

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  if (!existsSync(SITE_EDITOR)) {
    throw new Error(`сайт не собран (нет ${SITE_EDITOR}) — запустите node scripts/assemble.mjs`);
  }
  const docxFixture = { path: DOCX_FIXTURE, size: statSync(DOCX_FIXTURE).size };
  const xlsxFixture = smallestXlsx();
  const result = { startedAt: new Date().toISOString(), outDir: OUT_DIR, fixtures: { docx: docxFixture.path, xlsx: xlsxFixture.path } };
  const consoleLines = [];
  let server = null;
  let browser = null;
  let page = null;

  try {
    const chromium = loadChromium();
    server = await ensureServer();
    result.url = server.url;
    console.log(`страница: ${server.url}/viewer.html — сервер ${server.reused ? 'поднят заранее' : 'поднял скрипт'}`);

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

    await page.goto(`${server.url}/viewer.html`, { waitUntil: 'domcontentloaded' });
    console.log(`жду готовности редактора (document:ready, до ${READY_TIMEOUT_MS / 1000} с)…`);
    await page.waitForSelector('#viewer-status[data-state="ready"]', { timeout: READY_TIMEOUT_MS });
    console.log('редактор готов');

    result.docx = await checkDocument(page, docxFixture, 'docx');
    result.xlsx = await checkDocument(page, xlsxFixture, 'xlsx');

    if (consoleLines.length > 0) {
      console.log(`\nсообщения консоли (${consoleLines.length}, последние 40):`);
      for (const line of consoleLines.slice(-40)) console.log(`  ${line}`);
    }
    result.console = consoleLines.slice(-40);

    result.ok = result.docx.ok && result.xlsx.ok;
  } catch (error) {
    result.failure = String(error?.message ?? error);
    // Строка статуса страницы (если она успела появиться) — самое полезное при разборе:
    // в ней текст document:error с расшифровкой кода.
    result.statusOnFailure = null;
    if (page !== null) {
      result.statusOnFailure = await page
        .locator('#viewer-status')
        .textContent({ timeout: 1000 })
        .then((text) => text.trim())
        .catch(() => null);
    }
    console.log(`ПРОВАЛ: ${result.failure}`);
    if (result.statusOnFailure) console.log(`  статус страницы: ${result.statusOnFailure}`);
    result.ok = false;
  } finally {
    await browser?.close();
    server?.child?.kill();
  }

  writeFileSync(join(OUT_DIR, 'viewer-check.json'), JSON.stringify(result, null, 2));
  console.log(`сводка: ${join(OUT_DIR, 'viewer-check.json')}`);
  console.log(`скриншоты: ${OUT_DIR}/*.png`);
  console.log(result.ok === true ? 'OK: docx и xlsx открылись, canvas отрисован' : 'ПРОВАЛ: подробности выше и в сводке');
  process.exit(result.ok === true ? 0 : 1);
}

await main();
