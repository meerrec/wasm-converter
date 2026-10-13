// Приёмка харнесса спайка LOWA: поднимает его dev-сервер, гоняет реальную конвертацию
// в Chromium и сверяет результат с эталоном нативного LibreOffice.
//
// Запуск:
//   node experiments/lowa-spike/tools/harness-check.mjs
//
// Playwright и (запасной) vite берутся из examples/viewer через createRequire: в самом
// experiments/lowa-spike/ из зависимостей только vite.
//
// РЕЗУЛЬТАТ: /tmp/lowa-measure/harness.pdf, harness.png, filters.json + сводка в stdout.

import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const SPIKE_DIR = '/Users/irek/GitHub/wasm-converter/doc-converter/experiments/lowa-spike';
const VIEWER_DIR = '/Users/irek/GitHub/wasm-converter/doc-converter/examples/viewer';
const FIXTURE =
  '/Users/irek/GitHub/wasm-converter/doc-converter/test-fixtures/СУС УТП_v2.0_31.10.25.docx';
const NATIVE_REFERENCE = '/tmp/lowa-measure-native/sus-utp-native.pdf';
const OUT_DIR = process.env.LOWA_OUT ?? '/tmp/lowa-measure';
const PORT = Number(process.env.LOWA_PORT ?? 5199);
const EXPECTED_PAGES = 161;

const requireViewer = createRequire(join(VIEWER_DIR, 'package.json'));
const { chromium } = requireViewer('@playwright/test');

function requireVite() {
  const local = join(SPIKE_DIR, 'node_modules', 'vite', 'package.json');
  if (existsSync(local)) {
    return createRequire(join(SPIKE_DIR, 'package.json'))('vite');
  }
  return requireViewer('vite');
}

/** Число страниц через pdfinfo; null, если утилиты нет (локально она есть, в CI бывает нет). */
function pdfPages(file) {
  try {
    const out = execFileSync('pdfinfo', [file], { encoding: 'utf8' });
    const m = /^Pages:\s+(\d+)/m.exec(out);
    return m ? Number(m[1]) : null;
  } catch {
    return null;
  }
}

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  const { createServer } = requireVite();
  const server = await createServer({
    root: SPIKE_DIR,
    configFile: join(SPIKE_DIR, 'vite.config.ts'),
    server: { port: PORT, strictPort: true },
  });
  await server.listen();

  const browser = await chromium.launch();
  const page = await browser.newPage();
  const consoleLines = [];
  page.on('console', (msg) => consoleLines.push(`${msg.type()}: ${msg.text()}`));
  page.on('pageerror', (error) => consoleLines.push(`pageerror: ${error.message}`));

  const result = { startedAt: new Date().toISOString() };
  try {
    await page.goto(`http://localhost:${PORT}/`, { waitUntil: 'domcontentloaded' });
    result.crossOriginIsolated = await page.evaluate(() => crossOriginIsolated);
    console.log(`crossOriginIsolated: ${result.crossOriginIsolated}`);
    if (!result.crossOriginIsolated) {
      throw new Error('страница не cross-origin isolated: sharedArrayBuffer недоступен');
    }

    // Холодный старт: ~50 МБ артефактов и ~30 с в настоящем браузере — таймаут с запасом.
    const bootStart = Date.now();
    await page.waitForFunction(
      () => window.lowaHarness?.status().phase === 'ready',
      undefined,
      { timeout: 300_000, polling: 500 },
    );
    result.bootMs = Date.now() - bootStart;
    console.log(`офис поднялся за ${(result.bootMs / 1000).toFixed(1)} с`);

    const probe = await page.evaluate(() => window.lowaHarness.probe());
    result.filters = {
      count: probe.count,
      writer_pdf_Export: probe.names.includes('writer_pdf_Export'),
      calc_pdf_Export: probe.names.includes('calc_pdf_Export'),
      pdfNames: probe.names.filter((name) => /pdf/i.test(name)),
      detailed: probe.detailed,
      errors: probe.errors,
    };
    writeFileSync(join(OUT_DIR, 'filters.json'), JSON.stringify(probe, null, 2));
    console.log(
      `фильтров всего ${probe.count}; writer_pdf_Export=${result.filters.writer_pdf_Export}, ` +
        `calc_pdf_Export=${result.filters.calc_pdf_Export}`,
    );

    // Настоящий выбор файла в <input type=file>, а не вызов API напрямую.
    await page.setInputFiles('#file', FIXTURE);
    const convertStart = Date.now();
    await page.waitForFunction(
      () => ['done', 'error'].includes(window.lowaHarness?.status().phase),
      undefined,
      { timeout: 300_000, polling: 500 },
    );
    result.convertMs = Date.now() - convertStart;
    const status = await page.evaluate(() => window.lowaHarness.status());
    result.phase = status.phase;
    result.error = status.error;
    if (status.phase !== 'done') {
      throw new Error(`конвертация не удалась: ${status.error}`);
    }
    result.pdfBytes = status.pdfBytes;
    result.log = status.log;

    const base64 = await page.evaluate(() => window.lowaHarness.readPdfBase64());
    const pdfPath = join(OUT_DIR, 'harness.pdf');
    writeFileSync(pdfPath, Buffer.from(base64, 'base64'));
    result.pdfPath = pdfPath;
    result.pages = pdfPages(pdfPath);
    result.nativePages = existsSync(NATIVE_REFERENCE) ? pdfPages(NATIVE_REFERENCE) : null;
    await page.screenshot({ path: join(OUT_DIR, 'harness.png'), fullPage: true });
    result.screenshot = join(OUT_DIR, 'harness.png');
  } catch (error) {
    result.failure = String(error.message ?? error);
    try {
      await page.screenshot({ path: join(OUT_DIR, 'harness.png'), fullPage: true });
      const status = await page.evaluate(() => window.lowaHarness?.status() ?? null);
      result.log = status?.log ?? null;
    } catch {
      /* страница могла не подняться вовсе — снимок не обязателен */
    }
  } finally {
    result.console = consoleLines.slice(-80);
    await browser.close();
    await server.close();
  }

  writeFileSync(join(OUT_DIR, 'harness-check.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));

  const ok =
    result.failure === undefined &&
    result.phase === 'done' &&
    result.pages === EXPECTED_PAGES &&
    result.filters?.writer_pdf_Export === true &&
    result.filters?.calc_pdf_Export === true;
  console.log(
    ok
      ? `OK: ${result.pages} страниц, ${result.pdfBytes} Б, ${(result.convertMs / 1000).toFixed(1)} с`
      : `ПРОВАЛ: см. ${join(OUT_DIR, 'harness-check.json')}`,
  );
  process.exit(ok ? 0 : 1);
}

await main();
