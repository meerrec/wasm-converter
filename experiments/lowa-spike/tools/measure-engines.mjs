// Замер H7 спайка sprint-13: работает ли LOWA-демо не только в Chromium,
// но и в Firefox и WebKit.
//
// Предмет тот же, что у measure-envelope.mjs: публичное демо
// https://zetaoffice.net/demos/convertpdf/ (COOP/COEP => crossOriginIsolated).
// Отличие: движков три, а CDP доступен только Chromium. Поэтому объём по сети
// снимается переносимым `request.sizes()` (в 1.63 работает и в FF, и в WK —
// проверено пробником), а не CDP-событиями.
//
// Chromium здесь не гоняется: его числа берутся из /tmp/lowa-measure/raw.json
// (их снял слайс measure-envelope.mjs).
//
// Запуск:
//   node measure-engines.mjs                       # firefox + webkit
//   node measure-engines.mjs --engines firefox
//
// Playwright берётся из examples/viewer через createRequire: в самом
// experiments/lowa-spike/tools/ пакета нет.
//
// РЕЗУЛЬТАТ: <out>/engines-raw.json, PDF'ы <out>/sus-utp-<движок>.pdf,
// скриншоты <out>/engines-<движок>-<этап>.png.

import { createRequire } from 'node:module';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const require = createRequire(
  '/Users/irek/GitHub/wasm-converter/doc-converter/examples/viewer/package.json',
);
const { chromium, firefox, webkit } = require('@playwright/test');

const DEMO = 'https://zetaoffice.net/demos/convertpdf/';
const FIXTURE =
  '/Users/irek/GitHub/wasm-converter/doc-converter/test-fixtures/СУС УТП_v2.0_31.10.25.docx';
const ARTIFACT = /soffice\.(js|wasm|data)$/;

const argv = process.argv.slice(2);
const argOf = (name, dflt) => {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : dflt;
};
const OUT_DIR = argOf('--out', '/tmp/lowa-measure');
const ENGINES = argOf('--engines', 'firefox,webkit').split(',').filter(Boolean);
// Старт: 180 с — в Chromium было 28 с, в других движках ждём дольше, но не
// считаем долгий старт отказом: отличаем «долго» (идёт закачка) от «не работает».
const READY_TIMEOUT_MS = Number(argOf('--ready-timeout', '180000'));
const READY_EXTENSION_MS = Number(argOf('--ready-extension', '180000'));
const PDF_TIMEOUT_MS = Number(argOf('--pdf-timeout', '300000'));
mkdirSync(OUT_DIR, { recursive: true });

const TYPES = { chromium, firefox, webkit };
const mib = (b) => (b == null ? null : Number((b / 1048576).toFixed(2)));
const log = (s) => process.stderr.write(s + '\n');

// ---------------------------------------------------------------------------
// Init-скрипт страницы — приёмы перенесены из measure-envelope.mjs (тот же
// демо-процесс: disabled#input как признак готовности, patch createObjectURL
// для выхватывания PDF-блоба, метки postMessage вокруг cmd:'convert').
// Метки идут в часах performance.now(), в одной шкале с Resource Timing.
// ---------------------------------------------------------------------------
const INIT_SCRIPT = `(() => {
  if (window.__lowa) return;
  const L = { events: [], pdfBlobs: [], errors: [] };
  window.__lowa = L;
  const mark = (name, extra) => L.events.push(Object.assign({ name, t: performance.now() }, extra || {}));
  mark('initScript');

  // Готовность демо = снятие disabled с #input (office_thread.js шлёт cmd:'start').
  let readyMarked = false;
  const markReady = (via) => { if (!readyMarked) { readyMarked = true; mark('inputEnabled', { via }); } };
  try {
    new MutationObserver(() => {
      const el = document.getElementById('input');
      if (el && !el.disabled) markReady('mutationObserver');
    }).observe(document, { childList: true, subtree: true, attributes: true, attributeFilter: ['disabled'] });
  } catch (e) { L.errors.push('mutationObserver: ' + e); }
  const poll = () => {
    const el = document.getElementById('input');
    if (el && !el.disabled) markReady('rafPoll');
    requestAnimationFrame(poll);
  };
  requestAnimationFrame(poll);

  try {
    document.addEventListener('DOMContentLoaded', () => mark('domContentLoaded'));
    window.addEventListener('load', () => mark('load'));
    document.addEventListener('change', (e) => mark('change', { id: e.target && e.target.id }), true);
  } catch (e) { L.errors.push('listeners: ' + e); }

  // Отдача команды офисному воркеру и ответ 'converted'.
  try {
    const origPost = MessagePort.prototype.postMessage;
    MessagePort.prototype.postMessage = function (msg, ...rest) {
      try { if (msg && msg.cmd === 'convert') mark('convertPosted', { from: msg.from }); } catch {}
      return origPost.call(this, msg, ...rest);
    };
    const desc = Object.getOwnPropertyDescriptor(MessagePort.prototype, 'onmessage');
    if (desc && desc.set) {
      Object.defineProperty(MessagePort.prototype, 'onmessage', {
        configurable: true,
        enumerable: desc.enumerable,
        get() { return desc.get.call(this); },
        set(fn) {
          if (typeof fn !== 'function') return desc.set.call(this, fn);
          return desc.set.call(this, function (e) {
            try { if (e && e.data && e.data.cmd === 'converted') mark('converted', { to: e.data.to }); } catch {}
            return fn.apply(this, arguments);
          });
        },
      });
    }
  } catch (e) { L.errors.push('messagePort: ' + e); }

  // PDF-блоб: revokeObjectURL нейтрализуем, иначе блоб не прочитать.
  try {
    const origCreate = URL.createObjectURL.bind(URL);
    URL.createObjectURL = function (obj) {
      const url = origCreate(obj);
      try {
        if (obj && obj.type === 'application/pdf') {
          L.pdfBlobs.push(obj);
          mark('pdfBlob', { size: obj.size });
        }
      } catch {}
      return url;
    };
    URL.revokeObjectURL = function () { mark('revokeSuppressed'); };
  } catch (e) { L.errors.push('createObjectURL: ' + e); }

  // Сеть: сколько байт артефактов уже дошло — нужен, чтобы на таймауте
  // отличить «ещё качается» от «встало».
  try {
    const seen = {};
    const rec = (n) => {
      if (!/soffice\\.(js|wasm|data)/.test(n)) return;
      seen[n] = (seen[n] || 0) + 1;
      mark('artifactFetch', { url: n, n: seen[n] });
    };
    performance.getEntriesByType('resource').forEach((e) => rec(e.name));
    new PerformanceObserver((list) => list.getEntries().forEach((e) => rec(e.name)))
      .observe({ type: 'resource', buffered: true });
  } catch (e) { L.errors.push('perfObserver: ' + e); }
})();`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------
// Один прогон движка: свежий браузер, пустой кэш.
// ---------------------------------------------------------------------------
async function runEngine(engine) {
  const type = TYPES[engine];
  if (!type) throw new Error(`неизвестный движок: ${engine}`);

  const run = {
    engine,
    browserVersion: null,
    crossOriginIsolated: null,
    ready: { ok: false, wallMs: null, pageReadyMs: null, via: null, extended: false, reason: null },
    conversion: { ok: false, wallMs: null, changeToConvertedMs: null, blobBytes: null, pdfPath: null, reason: null },
    network: { artifacts: [], artifactBytes: null, artifactMiB: null, allRequestsBodyBytes: 0, allRequestsMiB: null, requests: 0 },
    console: [],
    errors: [],
    badResponses: [],
    failedReqs: [],
    pageErrors: [],
    pageData: null,
    screenshot: null,
    outcome: 'unknown',
  };

  log(`[${engine}] launch`);
  const browser = await type.launch({ headless: true });
  run.browserVersion = browser.version();

  // Переносимый учёт объёма: request.sizes() работает во всех трёх движках
  // (проверено пробником). Промисы разрешаем после прогона, а не в обработчике, —
  // иначе закрытие страницы может оборвать незавершённый RPC.
  const sizeJobs = [];
  const sizeByUrl = new Map();
  run.artifactHeaders = [];

  try {
    const context = await browser.newContext();
    const page = await context.newPage();

    page.on('console', (m) => {
      if (run.console.length < 400) run.console.push({ type: m.type(), text: m.text().slice(0, 400) });
    });
    page.on('pageerror', (e) => run.pageErrors.push({ kind: 'pageerror', text: String(e).slice(0, 600) }));
    // Заголовки артефактов: объём по сети зависит от согласованного сжатия
    // (Chromium договаривается на zstd, Firefox/WebKit — нет), и без
    // content-encoding разница в размерах `soffice.js` выглядит загадкой.
    page.on('response', (r) => {
      if (r.status() >= 400) run.badResponses.push({ status: r.status(), url: r.url() });
      if (ARTIFACT.test(r.url())) {
        // headers() в 1.63 синхронный (в отличие от allHeaders())
        try {
          const h = r.headers();
          run.artifactHeaders.push({
            url: r.url(),
            status: r.status(),
            contentEncoding: h['content-encoding'] ?? null,
            contentLength: h['content-length'] ?? null,
          });
        } catch {}
      }
    });
    page.on('requestfailed', (r) =>
      run.failedReqs.push({ url: r.url(), err: r.failure()?.errorText ?? null }),
    );

    page.on('requestfinished', (req) => {
      run.network.requests++;
      const p = req
        .sizes()
        .then((s) => {
          const prev = sizeByUrl.get(req.url()) ?? 0;
          sizeByUrl.set(req.url(), prev + (s.responseBodySize ?? 0));
        })
        .catch(() => {});
      sizeJobs.push(p);
    });

    await context.addInitScript(INIT_SCRIPT);

    // --- старт ---
    log(`[${engine}] goto`);
    await page.goto(DEMO, { waitUntil: 'commit', timeout: 120_000 });
    const tReady = Date.now();

    // --- готовность. Первый отрезок — READY_TIMEOUT_MS; если он истёк, но
    //     закачка артефактов ещё идёт, продлеваем: это «долго», а не «сломано».
    const waitReady = (ms) =>
      page
        .waitForFunction(() => (window.__lowa?.events ?? []).some((e) => e.name === 'inputEnabled'), null, {
          timeout: ms,
          polling: 100,
        })
        .then(() => true)
        .catch(() => false);

    log(`[${engine}] wait ready (${READY_TIMEOUT_MS} ms)…`);
    let ok = await waitReady(READY_TIMEOUT_MS);
    if (!ok) {
      const st = await artifactState(page, sizeByUrl);
      const progress = st.done < st.seen; // не все артефакты доехали => идёт закачка
      log(`[${engine}] not ready after ${READY_TIMEOUT_MS} ms: ${JSON.stringify(st)}`);
      if (progress) {
        run.ready.extended = true;
        ok = await waitReady(READY_EXTENSION_MS);
      } else {
        run.ready.reason = `артефакты доехали (${JSON.stringify(st)}), готовности нет — модуль не стартовал`;
      }
      if (!ok && !run.ready.reason) {
        run.ready.reason = `готовности нет и после продления (${JSON.stringify(await artifactState(page, sizeByUrl))})`;
      }
    }
    run.ready.ok = ok;
    run.ready.wallMs = ok ? Date.now() - tReady : null;
    if (!ok) {
      run.outcome = 'ready-timeout';
      await collectEvidence(page, run, 'notready');
      return run;
    }
    log(`[${engine}] ready in ${run.ready.wallMs} ms (extended=${run.ready.extended})`);

    const pre = await page.evaluate(() => ({
      crossOriginIsolated,
      events: window.__lowa?.events ?? [],
      initErrors: window.__lowa?.errors ?? [],
      navigator: { userAgent: navigator.userAgent, hardwareConcurrency: navigator.hardwareConcurrency },
      resourceTiming: performance.getEntriesByType('resource').map((e) => ({
        name: e.name,
        initiatorType: e.initiatorType,
        startTime: e.startTime,
        responseEnd: e.responseEnd,
        transferSize: e.transferSize,
        encodedBodySize: e.encodedBodySize,
        decodedBodySize: e.decodedBodySize,
      })),
    }));
    run.crossOriginIsolated = pre.crossOriginIsolated;
    run.initErrors = pre.initErrors;
    run.navigator = pre.navigator;

    const ev = Object.fromEntries(pre.events.map((e) => [e.name, e]));
    run.ready.pageReadyMs = ev.inputEnabled?.t ?? null;
    run.ready.via = ev.inputEnabled?.via ?? null;

    // --- конвертация ---
    log(`[${engine}] convert`);
    const before = await page.evaluate(() => window.__lowa.pdfBlobs.length);
    const tConv = Date.now();
    await page.setInputFiles('#input', FIXTURE);
    ok = await page
      .waitForFunction((n) => (window.__lowa?.pdfBlobs?.length ?? 0) > n, before, {
        timeout: PDF_TIMEOUT_MS,
        polling: 200,
      })
      .then(() => true)
      .catch(() => false);
    run.conversion.wallMs = Date.now() - tConv;
    if (!ok) {
      run.conversion.reason = `PDF не появился за ${PDF_TIMEOUT_MS} ms`;
      run.outcome = 'no-pdf';
      await collectEvidence(page, run, 'nopdf');
      return run;
    }

    // --- выгрузка PDF порциями: целый 5-МБ блоб одним evaluate не пролезет ---
    const meta = await page.evaluate(() => {
      const b = window.__lowa.pdfBlobs[0];
      return b ? { size: b.size, type: b.type } : null;
    });
    const CHUNK = 4 * 1024 * 1024;
    const parts = [];
    for (let off = 0; off < meta.size; off += CHUNK) {
      const b64 = await page.evaluate(async ([o, len]) => {
        const blob = window.__lowa.pdfBlobs[0];
        const buf = await blob.slice(o, o + len).arrayBuffer();
        const bytes = new Uint8Array(buf);
        let s = '';
        for (let i = 0; i < bytes.length; i += 0x8000) {
          s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
        }
        return btoa(s);
      }, [off, CHUNK]);
      parts.push(Buffer.from(b64, 'base64'));
    }
    const buf = Buffer.concat(parts);
    const pdfPath = join(OUT_DIR, `sus-utp-${engine}.pdf`);
    writeFileSync(pdfPath, buf);
    run.conversion.ok = true;
    run.conversion.pdfPath = pdfPath;
    run.conversion.blobBytes = meta.size;
    run.conversion.blobType = meta.type;
    run.conversion.pdfBytes = buf.length;
    run.conversion.pdfHead = buf.subarray(0, 8).toString('latin1');
    run.conversion.changeToConvertedMs =
      ev.change && ev.converted ? ev.converted.t - ev.change.t : null;

    const post = await page.evaluate(() => ({
      events: window.__lowa?.events ?? [],
      resourceTiming: performance.getEntriesByType('resource').map((e) => ({
        name: e.name,
        startTime: e.startTime,
        responseEnd: e.responseEnd,
        transferSize: e.transferSize,
      })),
    }));
    run.pageData = { pre, post };
    run.outcome = 'ok';
    return run;
  } catch (e) {
    run.errors.push({ kind: 'runThrew', text: String(e).slice(0, 600) });
    run.outcome = run.ready.ok ? 'error-after-ready' : 'launch-or-nav-error';
    return run;
  } finally {
    // добираем sizes() до закрытия страницы
    await Promise.allSettled(sizeJobs);
    summarizeNetwork(run, sizeByUrl);
    await browser.close().catch(() => {});
  }
}

// Сколько артефактов доехало: сравнение с числом стартов из performance-записей.
async function artifactState(page, sizeByUrl) {
  let seen = null;
  try {
    seen = await page.evaluate(() => {
      const names = performance
        .getEntriesByType('resource')
        .map((e) => e.name)
        .filter((n) => /soffice\.(js|wasm|data)$/.test(n));
      return [...new Set(names)].length;
    });
  } catch {
    seen = null;
  }
  let done = 0;
  let bytes = 0;
  for (const [url, b] of sizeByUrl) {
    if (ARTIFACT.test(url)) {
      done++;
      bytes += b;
    }
  }
  return { seen, done, bytes, mib: mib(bytes) };
}

async function collectEvidence(page, run, stage) {
  run.screenshot = join(OUT_DIR, `engines-${run.engine}-${stage}.png`);
  await page.screenshot({ path: run.screenshot, fullPage: false }).catch(() => {});
  try {
    run.capture = await page.evaluate(() => ({
      crossOriginIsolated,
      url: location.href,
      title: document.title,
      readyState: document.readyState,
      input: (() => {
        const el = document.getElementById('input');
        return el ? { disabled: el.disabled, type: el.type, hidden: el.offsetParent === null } : null;
      })(),
      bodyText: (document.body?.innerText ?? '').slice(0, 400),
      blobCount: window.__lowa?.pdfBlobs?.length ?? null,
      events: (window.__lowa?.events ?? []).map((e) => e.name),
    }));
  } catch (e) {
    run.capture = { error: String(e).slice(0, 300) };
  }
  // объём по сети на момент отказа доберёт summarizeNetwork в finally
}

function summarizeNetwork(run, sizeByUrl) {
  const rows = [];
  let artifactBytes = 0;
  for (const [url, bytes] of sizeByUrl) {
    const isArtifact = ARTIFACT.test(url);
    if (isArtifact) artifactBytes += bytes;
    rows.push({ url, bodyBytes: bytes, artifact: isArtifact });
  }
  run.network.artifacts = rows.filter((r) => r.artifact);
  run.network.artifactBytes = artifactBytes;
  run.network.artifactMiB = mib(artifactBytes);
  run.network.allRequestsBodyBytes = rows.reduce((s, r) => s + r.bodyBytes, 0);
  run.network.allRequestsMiB = mib(run.network.allRequestsBodyBytes);
  run.network.rows = rows;
}

// ---------------------------------------------------------------------------
const RUNS = Number(argOf('--runs', '1'));
const results = [];
for (const engine of ENGINES) {
  const runs = [];
  for (let i = 1; i <= RUNS; i++) {
    if (RUNS > 1) log(`[${engine}] прогон ${i}/${RUNS}`);
    const r = await runEngine(engine);
    r.runIndex = i;
    runs.push(r);
    log(`[${engine}#${i}] outcome=${r.outcome} ready=${r.ready.wallMs} ms pdf=${r.conversion.blobBytes ?? '—'}`);
  }
  results.push({ engine, browserVersion: runs[0].browserVersion, runs });
}

// Слияние с прошлым запуском: движки гоняются по отдельности, и повторный
// прогон одного не должен затирать результаты другого.
const OUT_JSON = join(OUT_DIR, 'engines-raw.json');
let prev = [];
try {
  prev = JSON.parse(readFileSync(OUT_JSON, 'utf8')).engines ?? [];
} catch {
  prev = [];
}
const merged = [...prev.filter((r) => !ENGINES.includes(r.engine)), ...results];
const out = {
  startedAt: new Date().toISOString(),
  demo: DEMO,
  fixture: FIXTURE,
  thresholds: { readyTimeoutMs: READY_TIMEOUT_MS, pdfTimeoutMs: PDF_TIMEOUT_MS },
  engines: merged,
};
writeFileSync(OUT_JSON, JSON.stringify(out, null, 2));
log(`written ${join(OUT_DIR, 'engines-raw.json')}`);
