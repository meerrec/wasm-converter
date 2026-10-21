// Замер пика памяти main-потока для двух путей открытия DOCX (ADR-0031 §3):
// через ArrayBuffer и через ReadableStream. Метрика — `performance.memory`
// (только Chromium): JS-куча main-потока. Ограничение метрики: внешние
// ArrayBuffer (крупные буферы выделяются вне кучи) в неё не входят, поэтому
// детач 57-МиБ буфера при трансфере стрим-путь не «удешевляет» в числах —
// проба держит: (1) smoke стрим-пути на тяжёлой фикстуре (разбор 488 страниц
// из потока), (2) регрессию heap-уровня (чанковая бухгалтерия не раздувает
// кучу). Память воркера из JS недоступна; его пик stream-пути равен пути
// через буфер по построению (предвыделенный `sourceBytes` + копия в wasm на
// время парса).
//
// Usage:
//   pnpm -F @meerrec/example-viewer exec node ../../scripts/stream-memory-probe.mjs \
//     [фикстура] [предел-МиБ]
//
// Фикстура по умолчанию — target/fixtures/docx-large/memory_50mib.docx
// (пишется `pnpm gen:docx-fixtures --large`). Сборки пакетов должны быть
// свежими: `pnpm build:wasm && pnpm turbo run build`. Страницу и сборки
// раздаёт статический сервер: единственный bare-импорт сборок — `@meerrec/wasm`
// в worker.js — переписывается на абсолютный путь, остальные бандлы
// самодостаточны.
//
// Выход: 0 — стрим уложился в предел; 1 — прогон упал; 2 — нет инструментов
// (Chromium не установлен); 3 — превышение бюджета.
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';

const ROOT = normalize(join(import.meta.dirname ?? '.', '..'));
const fixtureArg = process.argv[2] ?? 'target/fixtures/docx-large/memory_50mib.docx';
const limitMiB = Number(process.argv[3] ?? 32);

let playwrightRequire;
try {
  playwrightRequire = createRequire(join(ROOT, 'examples/viewer/package.json'));
} catch {
  console.error('нет @playwright/test — запускайте из examples/viewer (см. Usage)');
  process.exit(2);
}
// `playwright` — транзитивная зависимость @playwright/test и наружу из
// examples/viewer не видна (pnpm), поэтому резолвим от файла самого пакета.
const { chromium } = createRequire(playwrightRequire.resolve('@playwright/test'))('playwright');

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.wasm': 'application/wasm',
  '.docx': 'application/octet-stream',
};

const PROBE_HTML = `<!doctype html>
<html>
<body>
<div id="host"></div>
<pre id="log"></pre>
<script type="module">
const logLine = (text) => {
  const log = document.getElementById('log');
  log.textContent += text + '\\n';
};
window.addEventListener('error', (e) => logLine('error: ' + e.message));
window.addEventListener('unhandledrejection', (e) => logLine('unhandled: ' + String(e.reason)));
try {
  const { createDocxViewer } = await import('/packages/core/dist/index.js');
  logLine('core imported');
  const host = document.getElementById('host');
  const fixtureUrl = new URLSearchParams(location.search).get('fixture');
  const viewer = await createDocxViewer(host, {
    workerUrl: '/packages/core/dist/worker.js',
    wasmUrl: '/packages/wasm/pkg/doc_converter_wasm_bg.wasm',
    docxWasm: {
      module: '/packages/wasm-docx/pkg/doc_converter_docx_wasm.js',
      binary: '/packages/wasm-docx/pkg/doc_converter_docx_wasm_bg.wasm',
    },
  });
  logLine('viewer created');

  // Пик JS-кучи main-потока за время операции: семплируем каждые 5 мс —
  // разовое значение ничего не говорит, а interval ловит пик и fetch, и open.
  async function measure(run) {
    let peak = 0;
    const timer = setInterval(() => {
      const used = performance.memory?.usedJSHeapSize ?? 0;
      if (used > peak) peak = used;
    }, 5);
    try {
      const result = await run();
      clearInterval(timer);
      return { ok: true, peak, pages: result.length };
    } catch (e) {
      clearInterval(timer);
      return { ok: false, peak, error: String(e) };
    }
  }

  const bufferPath = await measure(async () => {
    const response = await fetch(fixtureUrl);
    const bytes = await response.arrayBuffer();
    return viewer.open(bytes);
  });
  logLine('buffer path done: ok=' + bufferPath.ok + ' peak=' + bufferPath.peak);

  const streamPath = await measure(async () => {
    const response = await fetch(fixtureUrl);
    const total = Number(response.headers.get('content-length'));
    return viewer.open(response.body, { size: total });
  });
  logLine('stream path done: ok=' + streamPath.ok + ' peak=' + streamPath.peak);

  viewer.destroy();
  window.__probe = { bufferPath, streamPath };
} catch (e) {
  logLine('fatal: ' + String(e));
}
</script>
</body>
</html>`;

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    let path = decodeURIComponent(url.pathname);
    if (path === '/' || path === '/probe.html') path = '/target/stream-memory-probe.html';
    const file = normalize(join(ROOT, path));
    if (!file.startsWith(ROOT)) throw new Error('вне корня');
    const info = await stat(file);
    if (!info.isFile()) throw new Error('не файл');
    let body = await readFile(file);
    // Единственный bare-импорт сборок: worker.js тянет @meerrec/wasm, а
    // статический сервер bare-спекификаторы не резолвит.
    if (path.endsWith('/dist/worker.js')) {
      body = Buffer.from(
        body.toString().replace("from '@meerrec/wasm'", "from '/packages/wasm/pkg/doc_converter_wasm.js'"),
      );
    }
    res.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end();
  }
});

await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
const base = `http://127.0.0.1:${port}`;

try {
  const fixtureInfo = await stat(join(ROOT, fixtureArg)).catch(() => null);
  if (!fixtureInfo) {
    console.error(`фикстура не найдена: ${fixtureArg}`);
    process.exit(1);
  }
  console.log(`фикстура: ${fixtureArg} (${(fixtureInfo.size / 1024 / 1024).toFixed(1)} МиБ)`);

  // Файл пробы кладём в target/ — каталог под .gitignore, корень не засоряем.
  const { writeFile, mkdir } = await import('node:fs/promises');
  await mkdir(join(ROOT, 'target'), { recursive: true });
  await writeFile(join(ROOT, 'target/stream-memory-probe.html'), PROBE_HTML);

  const browser = await chromium.launch();
  const page = await browser.newPage();
  page.on('console', (message) => console.log(`[page] ${message.type()}: ${message.text()}`));
  page.on('pageerror', (error) => console.error(`[page] pageerror: ${error}`));
  const fixtureUrl = `${base}/${fixtureArg}`;
  const pageUrl = `${base}/probe.html?fixture=${encodeURIComponent(fixtureUrl)}`;
  const response = await page.goto(pageUrl, { waitUntil: 'commit' });
  if (!response || !response.ok()) {
    console.error(`[probe] страница не открылась: ${response?.status()}`);
  }
  try {
    await page.waitForFunction(() => window.__probe !== undefined, undefined, {
      timeout: 300_000,
    });
  } catch {
    console.error('[probe] таймаут ожидания результата; лог страницы:');
    console.error(await page.evaluate(() => document.getElementById('log')?.textContent ?? '(пусто)'));
    await browser.close();
    process.exit(1);
  }
  const probe = await page.evaluate(() => window.__probe);
  await browser.close();

  for (const [name, path] of Object.entries(probe)) {
    if (!path.ok) {
      console.error(`${name}: упал — ${path.error}`);
      process.exit(1);
    }
    console.log(
      `${name}: пик ${(path.peak / 1024 / 1024).toFixed(1)} МиБ, страниц ${path.pages}`,
    );
  }

  const extraMiB = (probe.streamPath.peak - probe.bufferPath.peak) / 1024 / 1024;
  console.log(`стрим против буфера: ${extraMiB >= 0 ? '+' : ''}${extraMiB.toFixed(1)} МиБ (предел +${limitMiB})`);
  if (extraMiB > limitMiB) {
    console.error('::error:: пик стрима выше пика буферного пути больше допустимого');
    process.exit(3);
  }
  console.log('OK: пик стрима в пределах бюджета');
} finally {
  server.close();
}
