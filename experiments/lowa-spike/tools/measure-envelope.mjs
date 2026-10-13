// Замер «конверта» LOWA по гипотезам H1–H5 спайка sprint-13.
//
// Предмет: публичное демо конвертации DOCX→PDF на сборке ZetaOffice (LOWA/WASM),
// https://zetaoffice.net/demos/convertpdf/. Страница отдаётся с COOP/COEP
// (cross-origin-opener-policy: same-origin, cross-origin-embedder-policy: require-corp),
// поэтому crossOriginIsolated === true и доступны SAB и
// performance.measureUserAgentSpecificMemory().
//
// Запуск:
//   node measure-envelope.mjs                       # 2 холодных прогона + тёплые
//   node measure-envelope.mjs --runs 1 --out /tmp/lowa-measure
//
// Playwright берётся из examples/viewer через createRequire: в самом
// experiments/lowa-spike/tools/ пакета нет.
//
// РЕЗУЛЬТАТ: <out>/raw.json + печать сводки в stdout, PDF — <out>/sus-utp-zetaoffice.pdf.

import { createRequire } from 'node:module';
import { execFile } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const require = createRequire(
  '/Users/irek/GitHub/wasm-converter/doc-converter/examples/viewer/package.json',
);
const { chromium } = require('@playwright/test');

// --- параметры слайса (менять нельзя: это пороги из плана спринта) ---
const DEMO = 'https://zetaoffice.net/demos/convertpdf/';
const FIXTURE =
  '/Users/irek/GitHub/wasm-converter/doc-converter/test-fixtures/СУС УТП_v2.0_31.10.25.docx';
const ARTIFACT = /soffice\.(js|wasm|data)$/;
const THRESHOLDS = {
  H1: { yes: 25 * 1024 * 1024, no: 40 * 1024 * 1024, unit: 'bytes' },
  H2: { yes: 8, no: 20, unit: 's' },
  H3: { yes: 3, no: 8, unit: 's' },
  H4: { yes: 800, no: 1500, unit: 'MB' },
  H5: { yes: 20, no: 60, unit: 's' },
};

const argv = process.argv.slice(2);
const argOf = (name, dflt) => {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : dflt;
};
const OUT_DIR = argOf('--out', '/tmp/lowa-measure');
const RUNS = Number(argOf('--runs', '2'));
const BLOCK = ['*soffice*'];
mkdirSync(OUT_DIR, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const mib = (b) => Number((b / 1048576).toFixed(2));
const rep = [];
const log = (s) => process.stderr.write(s + '\n');

// ---------------------------------------------------------------------------
// Пик RSS процессов Chromium — память вкладки на уровне ОС.
// Нужен потому, что performance.measureUserAgentSpecificMemory() учитывает
// wasm-память как зарезервированный диапазон, а RSS — реально тронутые страницы.
//
// Процессы опознаются диффом: снимок до launch() даёт baseline, всё новое с
// --type= — наши. Browser.getBrowserCommandLine тут недоступен (Playwright не
// выставляет --enable-automation), а browser.process() в 1.63 нет.
// ---------------------------------------------------------------------------
async function psSnapshot() {
  return new Promise((resolve) => {
    execFile(
      'ps',
      ['-axww', '-o', 'pid,ppid,rss,args'],
      { maxBuffer: 64 * 1024 * 1024 },
      (err, out) => {
        if (err) return resolve(null);
        const procs = [];
        for (const line of out.split('\n')) {
          const m = line.match(/^\s*(\d+)\s+(\d+)\s+(\d+)\s+(.*)$/);
          if (!m) continue;
          procs.push({
            pid: Number(m[1]),
            ppid: Number(m[2]),
            rssMb: Number(m[3]) / 1024,
            args: m[4],
          });
        }
        resolve(procs);
      },
    );
  });
}

function rssOf(procs, baseline) {
  const fresh = procs.filter((p) => !baseline.has(p.pid));
  const empty = { rendererMb: 0, rendererCount: 0, renderers: [], others: [], fresh: 0 };
  if (!fresh.length) return empty;
  // Главный процесс браузера — новый процесс без --type=; рендереры — его потомки.
  const mainPids = new Set(fresh.filter((p) => !/--type=/.test(p.args)).map((p) => p.pid));
  if (!mainPids.size) return { ...empty, fresh: fresh.length };
  const byPid = new Map(fresh.map((p) => [p.pid, p]));
  const isMine = (p) => {
    let cur = byPid.get(p.ppid);
    for (let d = 0; cur && d < 4; d++) {
      if (mainPids.has(cur.pid)) return true;
      cur = byPid.get(cur.ppid);
    }
    return false;
  };
  const renderers = [];
  const others = [];
  let rendererMb = 0;
  for (const p of fresh) {
    if (!isMine(p)) continue;
    const type = (p.args.match(/--type=([\w-]+)/) || [, 'browser'])[1];
    if (type === 'renderer') {
      rendererMb += p.rssMb;
      renderers.push({ pid: p.pid, rssMb: Number(p.rssMb.toFixed(1)) });
    } else {
      others.push({ pid: p.pid, type, rssMb: Number(p.rssMb.toFixed(1)) });
    }
  }
  return {
    rendererMb: Number(rendererMb.toFixed(1)),
    rendererCount: renderers.length,
    renderers,
    others,
    fresh: fresh.length,
  };
}

function startRssSampler(baseline) {
  const state = { peakMb: 0, samples: 0, at: null, lastDetail: null, stop: false };
  (async () => {
    while (!state.stop) {
      const snap = await psSnapshot();
      if (snap) {
        const r = rssOf(snap, baseline);
        state.samples++;
        state.lastDetail = r;
        if (r.rendererMb > state.peakMb) {
          state.peakMb = r.rendererMb;
          state.at = new Date().toISOString();
        }
      }
      await sleep(200);
    }
  })();
  return state;
}

// ---------------------------------------------------------------------------
// Init-скрипт страницы. IIFE (а не «функция строкой»): Playwright evaluate со
// строкой вычисляет выражение и не вызывает его.
// Все метки — в часах performance.now(), то есть в одной шкале с Resource
// Timing, поэтому «сеть» и «инициализация CPU» разделяются без выравнивания
// часов CDP.
// ---------------------------------------------------------------------------
const INIT_SCRIPT = `(() => {
  if (window.__lowa) return;
  const L = { events: [], pdfBlobs: [], t0: performance.now(), timeOrigin: performance.timeOrigin };
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
  } catch (e) { mark('patchFailed', { what: 'mutationObserver', err: String(e) }); }
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
  } catch {}

  // Время отдачи команды в офисный воркер и время ответа 'converted'.
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
  } catch (e) { mark('patchFailed', { what: 'messagePort', err: String(e) }); }

  // Забираем PDF-блоб: URL.revokeObjectURL нейтрализуем, иначе блоб не прочитать.
  // Побочный эффект — блоб остаётся живым; учтено в отчёте по H4.
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
  } catch (e) { mark('patchFailed', { what: 'createObjectURL', err: String(e) }); }
})();`;

const MEASURE_FN = `(async () => {
  const out = {
    elapsed: performance.now(),
    crossOriginIsolated,
    jsHeapUsedMb: performance.memory ? +(performance.memory.usedJSHeapSize / 1048576).toFixed(1) : null,
    jsHeapTotalMb: performance.memory ? +(performance.memory.totalJSHeapSize / 1048576).toFixed(1) : null,
  };
  try {
    const m = await performance.measureUserAgentSpecificMemory();
    out.uaBytes = m.bytes;
    out.uaMb = +(m.bytes / 1048576).toFixed(1);
    out.uaBreakdown = m.breakdown.map((b) => ({
      mb: +(b.bytes / 1048576).toFixed(1),
      types: b.types,
      urls: (b.attribution || []).map((a) => a.url).filter(Boolean).slice(0, 2),
    }));
  } catch (e) { out.uaError = String(e); }
  return out;
})()`;

async function measureMemory(page, label) {
  try {
    const r = await page.evaluate(MEASURE_FN);
    return { ...(r ?? {}), label };
  } catch (e) {
    return { label, error: String(e) };
  }
}

// ---------------------------------------------------------------------------
// Один холодный прогон в свежем браузере (свой user-data-dir => пустой HTTP-кэш)
// ---------------------------------------------------------------------------
async function coldRun(runIndex, { doConvert }) {
  const baselineProcs = await psSnapshot();
  const baseline = new Set((baselineProcs ?? []).map((p) => p.pid));

  const browser = await chromium.launch({ channel: 'chromium', headless: true });
  const version = browser.version();
  const rss = startRssSampler(baseline);
  const context = await browser.newContext();
  const page = await context.newPage();

  const browserConsole = [];
  const errors = [];
  const badResponses = [];
  const failedReqs = [];
  page.on('console', (m) =>
    browserConsole.push({ type: m.type(), text: m.text().slice(0, 400) }),
  );
  page.on('pageerror', (e) => errors.push({ kind: 'pageerror', text: String(e).slice(0, 600) }));
  page.on('response', (r) => {
    if (r.status() >= 400) badResponses.push({ status: r.status(), url: r.url() });
  });
  page.on('requestfailed', (r) => failedReqs.push({ url: r.url(), err: r.failure()?.errorText }));

  // --- CDP: сеть на сессии страницы. Артефакты LOWA грузит главный поток
  //     (soffice.js — <script>, wasm/data — fetch и XHR эмскриптена); это
  //     проверено разведкой: worker-сессия видит только cache-hit'ы на 0 байт.
  const cdp = await context.newCDPSession(page);
  const reqUrl = new Map();
  const reqInfo = new Map();
  let phase = `cold${runIndex}`;
  const finished = [];
  const dataReceived = new Map();
  const servedFromCache = new Set();
  const reqLog = [];

  cdp.on('Network.requestWillBeSent', (e) => {
    reqUrl.set(e.requestId, e.request.url);
    const h = e.request.headers ?? {};
    const key = (n) => Object.keys(h).find((k) => k.toLowerCase() === n);
    reqLog.push({
      phase,
      url: e.request.url,
      type: e.type,
      headers: h,
      requestCacheControl: h[key('cache-control')] ?? null,
      pragma: h[key('pragma')] ?? null,
      range: h[key('range')] ?? null,
    });
  });
  cdp.on('Network.responseReceived', (e) => {
    reqUrl.set(e.requestId, e.response.url);
    reqInfo.set(e.requestId, {
      status: e.response.status,
      mimeType: e.response.mimeType,
      contentLength: e.response.headers?.['content-length'] ?? null,
      contentEncoding: e.response.headers?.['content-encoding'] ?? null,
      cacheControl: e.response.headers?.['cache-control'] ?? null,
      fromDiskCache: !!e.response.fromDiskCache,
      fromPrefetchCache: !!e.response.fromPrefetchCache,
      fromServiceWorker: !!e.response.fromServiceWorker,
      protocol: e.response.protocol,
    });
  });
  cdp.on('Network.requestServedFromCache', (e) => servedFromCache.add(e.requestId));
  cdp.on('Network.dataReceived', (e) =>
    dataReceived.set(e.requestId, (dataReceived.get(e.requestId) ?? 0) + e.dataLength),
  );
  cdp.on('Network.loadingFinished', (e) =>
    finished.push({ url: reqUrl.get(e.requestId) ?? '?', len: e.encodedDataLength, phase }),
  );
  cdp.on('Network.loadingFailed', (e) =>
    finished.push({ url: reqUrl.get(e.requestId) ?? '?', len: 0, phase, failed: e.errorText }),
  );
  await cdp.send('Network.enable');

  await context.addInitScript(INIT_SCRIPT);

  const run = {
    runIndex,
    browserVersion: version,
    points: {},
    console: browserConsole,
    errors,
    badResponses,
    failedReqs,
  };
  const snapshotPoint = (v) => ({ ...v, rss: rss.lastDetail });

  // --- P0: базовая линия. Та же страница демо, но загрузка артефактов LOWA
  //     запрещена CDP, поэтому измеряется оболочка без модуля. Иначе «до старта»
  //     не поймать: measureUserAgentSpecificMemory() сам по себе не мгновенный,
  //     и на живом документе он попадает уже после начала загрузки.
  const basePage = await context.newPage();
  try {
    const baseCdp = await context.newCDPSession(basePage);
    await baseCdp.send('Network.enable');
    await baseCdp.send('Network.setBlockedURLs', { urls: BLOCK });
    await basePage.goto(DEMO, { waitUntil: 'domcontentloaded', timeout: 60_000 }).catch(() => {});
    run.points.p0 = snapshotPoint(
      await measureMemory(basePage, `P0 оболочка демо без модуля (заблокировано: ${BLOCK.join(',')})`),
    );
    run.p0Ready = await basePage
      .evaluate(() => {
        const i = document.getElementById('input');
        return { inputExists: !!i, inputDisabled: i?.disabled ?? null };
      })
      .catch((e) => ({ error: String(e).slice(0, 200) }));
  } catch (e) {
    run.points.p0 = { label: 'P0', error: String(e).slice(0, 300) };
  } finally {
    await basePage.close().catch(() => {});
  }

  // --- Основная навигация: холодный старт ---
  phase = `cold${runIndex}`;
  await page.goto(DEMO, { waitUntil: 'commit', timeout: 120_000 });

  // --- P1: готовность ---
  const tReadyWall = Date.now();
  await page.waitForFunction(
    () => (window.__lowa?.events ?? []).some((e) => e.name === 'inputEnabled'),
    null,
    { timeout: 300_000, polling: 100 },
  );
  run.wallGotoToReadyMs = Date.now() - tReadyWall;
  run.points.p1 = snapshotPoint(await measureMemory(page, 'P1 после готовности'));

  // --- P2: конвертация фикстуры ---
  if (doConvert) {
    const before = await page.evaluate(() => window.__lowa.pdfBlobs.length);
    await page.setInputFiles('#input', FIXTURE);
    await page
      .waitForFunction((n) => (window.__lowa?.pdfBlobs?.length ?? 0) > n, before, {
        timeout: 900_000,
        polling: 100,
      })
      .catch((e) => errors.push({ kind: 'waitPdfTimeout', text: String(e).slice(0, 200) }));
    run.points.p2 = snapshotPoint(await measureMemory(page, 'P2 после конвертации'));
    run.pdf = await extractPdf(page);
    run.points.p3 = snapshotPoint(await measureMemory(page, 'P3 после выгрузки PDF'));
  }

  // --- сырые данные страницы ---
  const pageData = await page.evaluate(() => ({
    events: window.__lowa.events,
    resourceTiming: performance.getEntriesByType('resource').map((e) => ({
      name: e.name,
      initiatorType: e.initiatorType,
      startTime: e.startTime,
      responseEnd: e.responseEnd,
      duration: e.duration,
      transferSize: e.transferSize,
      encodedBodySize: e.encodedBodySize,
      decodedBodySize: e.decodedBodySize,
      nextHopProtocol: e.nextHopProtocol,
    })),
    navigator: {
      userAgent: navigator.userAgent,
      deviceMemory: navigator.deviceMemory,
      hardwareConcurrency: navigator.hardwareConcurrency,
    },
    crossOriginIsolated,
  }));
  run.pageEvents = pageData.events;
  run.navigator = pageData.navigator;
  run.crossOriginIsolated = pageData.crossOriginIsolated;

  // --- H1: cold-фаза, сумма encodedDataLength по артефактам ---
  const aggByUrl = (list) => {
    const m = new Map();
    for (const f of list) m.set(f.url, (m.get(f.url) ?? 0) + f.len);
    return m;
  };
  const coldFinished = finished.filter((f) => f.phase === `cold${runIndex}`);
  const byUrl = aggByUrl(coldFinished.filter((f) => ARTIFACT.test(f.url)));
  const headersByUrl = new Map();
  const decodedByUrl = new Map();
  for (const [reqId, u] of reqUrl) {
    if (!ARTIFACT.test(u)) continue;
    const info = reqInfo.get(reqId);
    if (info && !headersByUrl.has(u)) headersByUrl.set(u, info);
    if (coldFinished.some((f) => f.url === u)) {
      decodedByUrl.set(u, (decodedByUrl.get(u) ?? 0) + (dataReceived.get(reqId) ?? 0));
    }
  }
  const artifactRows = [...byUrl.entries()].map(([url, len]) => ({
    url,
    requests: coldFinished.filter((f) => f.url === url).length,
    encodedBytes: len,
    decodedBytes: decodedByUrl.get(url) ?? null,
    contentLengthHeader: headersByUrl.get(url)?.contentLength ?? null,
    contentEncoding: headersByUrl.get(url)?.contentEncoding ?? null,
    cacheControl: headersByUrl.get(url)?.cacheControl ?? null,
    protocol: headersByUrl.get(url)?.protocol ?? null,
    status: headersByUrl.get(url)?.status ?? null,
    fromDiskCache: headersByUrl.get(url)?.fromDiskCache ?? null,
  }));
  const h1Total = artifactRows.reduce((s, r) => s + r.encodedBytes, 0);
  const auxByUrl = aggByUrl(
    coldFinished.filter(
      (f) => !ARTIFACT.test(f.url) && !f.url.startsWith('blob:') && !f.url.startsWith('data:'),
    ),
  );
  run.h1 = {
    phase: `cold${runIndex}`,
    artifacts: artifactRows,
    totalEncodedBytes: h1Total,
    totalEncodedMiB: mib(h1Total),
    allRequestsEncodedMiB: mib(coldFinished.reduce((s, f) => s + Math.max(0, f.len), 0)),
    auxiliary: [...auxByUrl.entries()].map(([url, encodedBytes]) => ({ url, encodedBytes })),
    requestHeaders: reqLog
      .filter((r) => ARTIFACT.test(r.url))
      .map((r) => ({
        phase: r.phase,
        url: r.url.split('/').pop(),
        requestCacheControl: r.requestCacheControl,
        pragma: r.pragma,
        range: r.range,
        headers: r.headers,
      })),
    servedFromCacheRequests: servedFromCache.size,
  };

  // --- H2: разбор времени ---
  const ev = Object.fromEntries(pageData.events.map((e) => [e.name, e]));
  const artRT = pageData.resourceTiming.filter((r) => ARTIFACT.test(r.name));
  const lastArtifactEnd = artRT.length ? Math.max(...artRT.map((r) => r.responseEnd)) : null;
  const firstArtifactStart = artRT.length ? Math.min(...artRT.map((r) => r.startTime)) : null;
  run.h2 = {
    readyMs: ev.inputEnabled?.t ?? null,
    readyVia: ev.inputEnabled?.via ?? null,
    domContentLoadedMs: ev.domContentLoaded?.t ?? null,
    loadMs: ev.load?.t ?? null,
    firstArtifactStartMs: firstArtifactStart,
    lastArtifactResponseEndMs: lastArtifactEnd,
    networkPhaseMs:
      firstArtifactStart != null && lastArtifactEnd != null ? lastArtifactEnd - firstArtifactStart : null,
    cpuInitPhaseMs:
      ev.inputEnabled && lastArtifactEnd != null ? ev.inputEnabled.t - lastArtifactEnd : null,
    wallMsGotoToReady: run.wallGotoToReadyMs,
    artifacts: artRT.map((r) => ({
      url: r.name.split('/').pop(),
      startMs: Math.round(r.startTime),
      endMs: Math.round(r.responseEnd),
      durationMs: Math.round(r.duration),
      transferSize: r.transferSize,
      encodedBodySize: r.encodedBodySize,
      decodedBodySize: r.decodedBodySize,
      initiatorType: r.initiatorType,
      protocol: r.nextHopProtocol,
    })),
  };

  // --- H5: конвертация ---
  if (doConvert) {
    const ch = ev.change?.t ?? null;
    const cp = ev.convertPosted?.t ?? null;
    const cv = ev.converted?.t ?? null;
    const pb = ev.pdfBlob?.t ?? null;
    run.h5 = {
      changeToConvertPostedMs: ch != null && cp != null ? cp - ch : null,
      convertInWorkerMs: cp != null && cv != null ? cv - cp : null,
      changeToConvertedMs: ch != null && cv != null ? cv - ch : null,
      changeToPdfBlobMs: ch != null && pb != null ? pb - ch : null,
      pdfBlobBytes: ev.pdfBlob?.size ?? null,
    };
  }

  // --- H3: тёплый старт. Перезагрузка в том же контексте: HTTP-кэш и
  //     wasm code cache прогреты, документ и воркеры стартуют заново. ---
  run.warm = [];
  for (let w = 0; w < 2; w++) {
    await page.evaluate(() => {
      window.__lowa = null;
    });
    phase = `warm${runIndex}-${w + 1}`;
    const markIdx = finished.length;
    const tW = Date.now();
    try {
      // domcontentloaded, а не commit: время готовности берётся из
      // performance.now() самой страницы, поэтому задержка Node не влияет.
      await page.goto(DEMO, { waitUntil: 'domcontentloaded', timeout: 120_000 });
      await page.waitForFunction(
        () => (window.__lowa?.events ?? []).some((e) => e.name === 'inputEnabled'),
        null,
        { timeout: 300_000, polling: 100 },
      );
      const pd = await page.evaluate(() => ({
        events: window.__lowa.events,
        rt: performance.getEntriesByType('resource')
          .filter((r) => /soffice\.(js|wasm|data)$/.test(r.name))
          .map((r) => ({
            name: r.name.split('/').pop(),
            startTime: r.startTime,
            responseEnd: r.responseEnd,
            encodedBodySize: r.encodedBodySize,
            decodedBodySize: r.decodedBodySize,
          })),
      }));
      const ev2 = Object.fromEntries(pd.events.map((e) => [e.name, e]));
      const lastEnd = pd.rt.length ? Math.max(...pd.rt.map((r) => r.responseEnd)) : null;
      const wFinished = finished.slice(markIdx).filter((f) => f.phase === phase);
      const wArtifact = aggByUrl(wFinished.filter((f) => ARTIFACT.test(f.url)));
      run.warm.push({
        iteration: w + 1,
        readyMs: ev2.inputEnabled?.t ?? null,
        wallMs: Date.now() - tW,
        lastArtifactResponseEndMs: lastEnd,
        cpuInitPhaseMs: ev2.inputEnabled && lastEnd != null ? ev2.inputEnabled.t - lastEnd : null,
        artifactEncodedBytes: [...wArtifact.entries()].map(([url, encodedBytes]) => ({
          url: url.split('/').pop(),
          encodedBytes,
        })),
        artifactTotalEncodedBytes: [...wArtifact.values()].reduce((s, n) => s + n, 0),
        artifacts: pd.rt.map((r) => ({
          ...r,
          startTime: Math.round(r.startTime),
          responseEnd: Math.round(r.responseEnd),
        })),
      });
    } catch (e) {
      run.warm.push({ iteration: w + 1, error: String(e).slice(0, 300) });
    }
  }

  // --- итоги H4 и скриншот неудачи ---
  await sleep(400);
  rss.stop = true;
  await sleep(400);
  run.rss = {
    peakMb: rss.peakMb,
    samples: rss.samples,
    at: rss.at,
    last: rss.lastDetail,
  };
  run.pdfOk = run.pdf ? run.pdf.bytes > 10 * 1024 : null;
  if (doConvert && !run.pdfOk) {
    run.screenshot = join(OUT_DIR, `failure-cold${runIndex}.png`);
    await page.screenshot({ path: run.screenshot }).catch(() => {});
  }

  // --- Диагностика окружения: командная строка браузера (нужна, чтобы
  //     объяснить, почему тёплый старт не берёт артефакты из кэша) ---
  if (runIndex === 1) {
    try {
      const vp = await context.newPage();
      await vp.goto('chrome://version', { waitUntil: 'domcontentloaded', timeout: 30_000 });
      run.browserCommandLine = await vp.evaluate(() =>
        document.getElementById('command_line')?.textContent ?? null,
      );
      await vp.close();
    } catch (e) {
      run.browserCommandLine = `не снято: ${String(e).slice(0, 200)}`;
    }
  }

  await browser.close();
  return run;
}

// --- выгрузка PDF порциями: целый 5-МБ блоб одним evaluate не пролезет ---
async function extractPdf(page) {
  const meta = await page.evaluate(() => {
    const b = window.__lowa?.pdfBlobs?.[0];
    return b ? { size: b.size, type: b.type } : null;
  });
  if (!meta) return { error: 'no pdf blob captured' };
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
  const path = join(OUT_DIR, 'sus-utp-zetaoffice.pdf');
  writeFileSync(path, buf);
  return {
    path,
    bytes: buf.length,
    blobSize: meta.size,
    blobType: meta.type,
    head: buf.subarray(0, 8).toString('latin1'),
  };
}

// ---------------------------------------------------------------------------
const t0 = Date.now();
for (let i = 1; i <= RUNS; i++) {
  const doConvert = i === 1;
  log(`\n=== холодный прогон ${i}/${RUNS} (convert=${doConvert}) ===`);
  const r = await coldRun(i, { doConvert });
  rep.push(r);
  log(
    `  H2 ready=${Math.round(r.h2.readyMs)} ms (сеть ${Math.round(r.h2.networkPhaseMs)} + init ${Math.round(r.h2.cpuInitPhaseMs)})\n` +
      `  H1 total=${r.h1.totalEncodedMiB} MiB\n` +
      `  H3 warm=${r.warm.map((w) => (w.readyMs != null ? Math.round(w.readyMs) : w.error)).join(', ')} ms\n` +
      `  H4 peakRSS=${r.rss.peakMb} MB (${r.rss.samples} сэмплов)\n` +
      (r.h5
        ? `  H5 convert=${Math.round(r.h5.convertInWorkerMs)} ms, total=${Math.round(r.h5.changeToConvertedMs)} ms, pdf=${r.pdf?.bytes}\n`
        : ''),
  );
  if (r.errors.length) log(`  ошибки страницы: ${JSON.stringify(r.errors.slice(0, 3))}`);
  if (r.badResponses.length) log(`  HTTP>=400: ${JSON.stringify(r.badResponses.slice(0, 5))}`);
}

writeFileSync(
  join(OUT_DIR, 'raw.json'),
  JSON.stringify(
    {
      startedAt: new Date(t0).toISOString(),
      finishedAt: new Date().toISOString(),
      demo: DEMO,
      fixture: FIXTURE,
      playwright: require('@playwright/test/package.json').version,
      thresholds: THRESHOLDS,
      runs: rep,
    },
    null,
    2,
  ),
);

const p = (s) => process.stdout.write(s + '\n');
p('\n================ СВОДКА ================');
for (const r of rep) {
  p(`\n--- прогон ${r.runIndex} (Chromium ${r.browserVersion}) ---`);
  p(`H1 артефакты по сети (CDP encodedDataLength, фаза ${r.h1.phase}): ${r.h1.totalEncodedMiB} MiB`);
  for (const a of r.h1.artifacts) {
    p(
      `   ${a.url.split('/').pop().padEnd(13)} req=${a.requests} encoded=${String(a.encodedBytes).padStart(10)}B` +
        ` content-length=${a.contentLengthHeader ?? '—'} enc=${a.contentEncoding ?? '—'} decode=${a.decodedBytes ?? '—'} ${a.protocol ?? ''} cache="${a.cacheControl ?? '—'}"`,
    );
  }
  p(`   вспомогательные: ${r.h1.auxiliary.map((a) => `${a.url.split('/').pop() || a.url}=${a.encodedBytes}`).join(' ')}`);
  p(
    `H2 холодный старт: ${Math.round(r.h2.readyMs)} ms = сеть ${Math.round(r.h2.networkPhaseMs)} ms + CPU-init ${Math.round(r.h2.cpuInitPhaseMs)} ms`,
  );
  for (const a of r.h2.artifacts)
    p(`     ${a.url.padEnd(13)} ${a.startMs}→${a.endMs} ms (${a.durationMs} ms, ${a.initiatorType})`);
  for (const w of r.warm)
    p(
      `H3 тёплый #${w.iteration}: ${w.readyMs != null ? Math.round(w.readyMs) + ' ms' : w.error}` +
        (w.artifactTotalEncodedBytes != null
          ? `, повторно по сети ${w.artifactTotalEncodedBytes} Б, init ${Math.round(w.cpuInitPhaseMs)} ms, [${w.artifacts.map((x) => x.name + ':' + x.startTime + '→' + x.responseEnd).join(', ')}]`
          : ''),
    );
  p(
    `H4 UA-память: P0=${r.points.p0?.uaMb ?? r.points.p0?.uaError ?? '—'} МБ, P1=${r.points.p1?.uaMb ?? '—'} МБ, P2=${r.points.p2?.uaMb ?? '—'} МБ, P3=${r.points.p3?.uaMb ?? '—'} МБ`,
  );
  p(`   пик RSS рендереров: ${r.rss.peakMb} МБ (${r.rss.samples} сэмплов; в моменте: ${JSON.stringify(r.rss.last?.renderers)})`);
  if (r.points.p1?.uaBreakdown)
    p(`   разбор P1: ${r.points.p1.uaBreakdown.map((b) => `${b.mb}МБ[${b.types.join('/')}]`).join(' ')}`);
  if (r.h5)
    p(
      `H5 конвертация: ${Math.round(r.h5.convertInWorkerMs)} ms в воркере, ${Math.round(r.h5.changeToConvertedMs)} ms от change; PDF ${r.pdf?.bytes ?? '—'} Б`,
    );
  if (r.pdf?.error) p(`   PDF НЕ ПОЛУЧЕН: ${r.pdf.error}`);
  p(`   crossOriginIsolated=${r.crossOriginIsolated}`);
  p(`   console.error: ${r.console.filter((c) => c.type === 'error').length}, pageerror: ${r.errors.length}, HTTP>=400: ${r.badResponses.length}`);
}
p(`\nraw.json: ${join(OUT_DIR, 'raw.json')}`);
p(`всего времени: ${((Date.now() - t0) / 1000).toFixed(0)} с`);
