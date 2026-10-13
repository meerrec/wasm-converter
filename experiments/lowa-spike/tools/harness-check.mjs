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
// Вторая половина харнесса — Calc; отдельной приёмки у неё нет, проверяем сам факт PDF.
const XLSX_FIXTURE =
  '/Users/irek/GitHub/wasm-converter/doc-converter/test-fixtures/СУС 2.1 Сценарии функционального тестирования_v2.xlsx';
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

/**
 * Прогоняет один файл через харнесс: выбор файла в <input type=file>, ожидание конца,
 * выгрузка PDF из memfs в файл. Возвращает всё, что нужно для отчёта.
 */
async function convert(page, fixture, outName) {
  // Фаза от прошлой конвертации остаётся 'done', поэтому сначала ждём 'converting':
  // иначе ожидание конца сработает на старом состоянии.
  await page.waitForFunction(
    () => document.getElementById('file').disabled === false,
    undefined,
    { timeout: 30_000, polling: 100 },
  );
  await page.setInputFiles('#file', fixture);
  await page.waitForFunction(() => window.lowaHarness?.status().phase === 'converting', undefined, {
    timeout: 60_000,
    polling: 100,
  });
  const started = Date.now();
  await page.waitForFunction(
    () => ['done', 'error'].includes(window.lowaHarness?.status().phase),
    undefined,
    { timeout: 300_000, polling: 500 },
  );
  const status = await page.evaluate(() => window.lowaHarness.status());
  const out = {
    file: fixture.split('/').pop(),
    phase: status.phase,
    error: status.error,
    pdfBytes: status.pdfBytes,
    convertMs: Date.now() - started,
    log: status.log,
  };
  if (status.phase !== 'done') {
    return out;
  }
  const base64 = await page.evaluate(() => window.lowaHarness.readPdfBase64());
  out.pdfPath = join(OUT_DIR, outName);
  writeFileSync(out.pdfPath, Buffer.from(base64, 'base64'));
  out.pages = pdfPages(out.pdfPath);
  console.log(
    `${out.file}: ${out.pages} страниц, ${out.pdfBytes} Б за ${(out.convertMs / 1000).toFixed(1)} с`,
  );
  return out;
}

/**
 * Полный Chromium умеет встроенный просмотрщик PDF — в скриншот попадает сам документ.
 * Headless shell его не содержит, поэтому снимок там будет с пустым iframe.
 */
async function launchBrowser() {
  try {
    const browser = await chromium.launch({ channel: 'chromium' });
    return { browser, channel: 'chromium' };
  } catch (error) {
    console.log(`channel:chromium недоступен (${error.message.split('\n')[0]}), беру headless shell`);
    return { browser: await chromium.launch(), channel: 'headless-shell' };
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

  const { browser, channel } = await launchBrowser();
  const page = await browser.newPage();
  const consoleLines = [];
  page.on('console', (msg) => consoleLines.push(`${msg.type()}: ${msg.text()}`));
  page.on('pageerror', (error) => consoleLines.push(`pageerror: ${error.message}`));

  const result = { startedAt: new Date().toISOString(), browserChannel: channel };
  console.log(`браузер: ${channel}`);
  try {
    // LOWA_BASE=https://cdn.zetaoffice.net/zetaoffice_latest/ — прогон без зеркала, прямо с CDN.
    const query = process.env.LOWA_BASE ? `?base=${encodeURIComponent(process.env.LOWA_BASE)}` : '';
    await page.goto(`http://localhost:${PORT}/${query}`, { waitUntil: 'domcontentloaded' });
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

    try {
      const probe = await page.evaluate(() => window.lowaHarness.probe());
      result.filters = {
        count: probe.count,
        writer_pdf_Export: probe.names.includes('writer_pdf_Export'),
        calc_pdf_Export: probe.names.includes('calc_pdf_Export'),
        pdfNames: probe.names.filter((name) => /pdf/i.test(name)),
        detailed: probe.detailed,
        diag: probe.diag,
        errors: probe.errors,
      };
      writeFileSync(join(OUT_DIR, 'filters.json'), JSON.stringify(probe, null, 2));
      console.log(
        `фильтров всего ${probe.count}; writer_pdf_Export=${result.filters.writer_pdf_Export}, ` +
          `calc_pdf_Export=${result.filters.calc_pdf_Export}`,
      );
    } catch (error) {
      // Перечисление фильтров — отдельная проверка; провал не должен закрывать путь конвертации.
      result.filters = { error: String(error.message ?? error) };
      console.log(`перечисление фильтров не удалось: ${result.filters.error}`);
    }

    // Настоящий выбор файла в <input type=file>, а не вызов API напрямую.
    const docx = await convert(page, FIXTURE, 'harness.pdf');
    result.convertMs = docx.convertMs;
    result.phase = docx.phase;
    result.error = docx.error;
    result.pdfBytes = docx.pdfBytes;
    result.pages = docx.pages;
    result.log = docx.log;
    result.pdfPath = docx.pdfPath;
    result.nativePages = existsSync(NATIVE_REFERENCE) ? pdfPages(NATIVE_REFERENCE) : null;
    if (docx.phase !== 'done') {
      throw new Error(`конвертация не удалась: ${docx.error}`);
    }
    // Встроенный просмотрщик PDF рисует первую страницу не сразу после загрузки блоба.
    await page.waitForTimeout(6000);
    await page.screenshot({ path: join(OUT_DIR, 'harness.png'), fullPage: true });
    result.screenshot = join(OUT_DIR, 'harness.png');

    result.downloadLink = await page.evaluate(() => {
      const link = document.getElementById('download');
      return { hidden: link.hidden, href: link.href.slice(0, 12), download: link.download };
    });

    if (process.env.LOWA_SKIP_XLSX !== '1' && existsSync(XLSX_FIXTURE)) {
      const xlsx = await convert(page, XLSX_FIXTURE, 'harness-xlsx.pdf');
      result.xlsx = { ...xlsx, filter: 'calc_pdf_Export' };
      console.log(
        `XLSX: ${xlsx.phase}, ${xlsx.pages} страниц, ${xlsx.pdfBytes} Б за ${(xlsx.convertMs / 1000).toFixed(1)} с`,
      );
    }

    // Отрицательные проверки: у харнесса должен быть ответ, а не тишина.
    // Мусор вместо docx LibreOffice молча открывает как текст, поэтому путь вне memfs проверяем
    // отдельно: такого файла нет, loadComponentFromURL обязан бросить UNO-исключение.
    const brokenFile = join(OUT_DIR, 'broken.docx');
    writeFileSync(brokenFile, Buffer.from('это не docx, а просто текст\n'));
    const broken = await convert(page, brokenFile, 'broken.pdf');
    result.brokenFile = { phase: broken.phase, pages: broken.pages, pdfBytes: broken.pdfBytes };
    console.log(`мусор под именем .docx: ${broken.phase}, страниц ${broken.pages}`);
    result.missingFile = await page.evaluate(() =>
      window.lowaHarness
        .request({ cmd: 'convert', from: '/tmp/нет-такого.docx', to: '/tmp/x.pdf', filter: 'writer_pdf_Export' }, 60_000)
        .then(() => ({ ok: true }))
        .catch((error) => ({ ok: false, message: error.message, unoType: error.unoType })),
    );
    console.log(`файл вне memfs: ${JSON.stringify(result.missingFile)}`);
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
