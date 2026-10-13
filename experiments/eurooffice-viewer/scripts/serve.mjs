// Статический сервер собранного сайта (site/): zero-dep, порт из аргумента/env.
// Особенности: clean URL (/editor → editor.html), MIME-карта, .br-файлы отдаются
// как есть с Content-Encoding: br, а если запрошен x2t.wasm.br, но на диске
// только несжатый x2t.wasm — отдаётся он и без Content-Encoding (так части и
// публикуются: URL несёт .br, байты — нет).
//
// Запуск: node scripts/serve.mjs [порт]   (или VIEWER_PORT=5271 node scripts/serve.mjs)

import { createServer } from 'node:http';
import { createReadStream, existsSync, statSync } from 'node:fs';
import { extname, join, normalize, resolve, sep, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const SPIKE_DIR = resolve(SCRIPT_DIR, '..');
const SITE_DIR = join(SPIKE_DIR, 'site');

const port = Number(process.argv[2] ?? process.env.VIEWER_PORT ?? process.env.PORT ?? 5271);
if (!Number.isInteger(port) || port <= 0) {
  console.error(`порт должен быть целым числом, а не «${process.argv[2] ?? process.env.VIEWER_PORT}»`);
  process.exit(1);
}
if (!existsSync(join(SITE_DIR, 'editor.html'))) {
  console.error(`site/ не собран (нет ${join(SITE_DIR, 'editor.html')}) — запустите node scripts/assemble.mjs`);
  process.exit(1);
}

const MIME = new Map(
  Object.entries({
    '.html': 'text/html; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8',
    '.mjs': 'text/javascript; charset=utf-8',
    '.wasm': 'application/wasm',
    '.css': 'text/css; charset=utf-8',
    '.json': 'application/json; charset=utf-8',
    '.ttf': 'font/ttf',
    '.otf': 'font/otf',
    '.woff': 'font/woff',
    '.woff2': 'font/woff2',
    '.svg': 'image/svg+xml',
    '.png': 'image/png',
    '.ico': 'image/x-icon',
    '.jpg': 'image/jpeg',
    '.jpeg': 'image/jpeg',
    '.gif': 'image/gif',
    '.webp': 'image/webp',
    '.map': 'application/json; charset=utf-8',
    '.txt': 'text/plain; charset=utf-8',
    '.xml': 'application/xml',
    '.pdf': 'application/pdf',
  }),
);

function fileIfExists(path) {
  return existsSync(path) && statSync(path).isFile() ? path : null;
}

/**
 * Путь запроса → файл на диске. Directory listing не делаем: каталог — только
 * index.html внутри. Clean URL: путь без расширения и с существующим <path>.html
 * (так работает /editor).
 */
function resolveFile(pathname) {
  const relative = normalize(pathname).replace(/^([/\\])+/, '');
  const target = resolve(SITE_DIR, relative);
  if (target !== SITE_DIR && !target.startsWith(SITE_DIR + sep)) return null; // выход из site/

  const direct = fileIfExists(target);
  if (direct !== null) return direct;

  if (existsSync(target) && statSync(target).isDirectory()) {
    return fileIfExists(join(target, 'index.html'));
  }

  if (extname(target) === '') {
    const asHtml = fileIfExists(`${target}.html`);
    if (asHtml !== null) return asHtml;
  }

  // x2t.wasm.br: сайт просит .br, части публикуют несжатый x2t.wasm — отдаём
  // его как есть (без Content-Encoding: bytes не brotli). Если assemble успел
  // сделать brotli-копию, сюда не попадём — она найдётся выше и уйдёт с br.
  if (target.endsWith('.br')) {
    const base = fileIfExists(target.slice(0, -3));
    if (base !== null) return base;
  }

  return null;
}

function sendFile(response, file, headOnly) {
  const isBrotli = file.endsWith('.br');
  const typeSource = isBrotli ? file.slice(0, -3) : file;
  const headers = {
    'Content-Type': MIME.get(extname(typeSource).toLowerCase()) ?? 'application/octet-stream',
    'Content-Length': statSync(file).size,
  };
  if (isBrotli) headers['Content-Encoding'] = 'br';
  response.writeHead(200, headers);
  if (headOnly) {
    response.end();
    return;
  }
  createReadStream(file).pipe(response);
}

function sendText(response, status, text, headOnly) {
  const body = Buffer.from(text, 'utf8');
  response.writeHead(status, { 'Content-Type': 'text/plain; charset=utf-8', 'Content-Length': body.length });
  response.end(headOnly ? undefined : body);
}

const server = createServer((request, response) => {
  const headOnly = request.method === 'HEAD';
  if (request.method !== 'GET' && !headOnly) {
    sendText(response, 405, 'только GET/HEAD', headOnly);
    return;
  }
  let pathname;
  try {
    pathname = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname);
  } catch {
    sendText(response, 400, 'битый путь', headOnly);
    return;
  }
  const file = resolveFile(pathname);
  if (file === null) {
    sendText(response, 404, `не найдено: ${pathname}`, headOnly);
    return;
  }
  sendFile(response, file, headOnly);
});

server.on('error', (error) => {
  if (error.code === 'EADDRINUSE') {
    console.error(`порт ${port} занят — укажите другой: node scripts/serve.mjs 5272`);
  } else {
    console.error(`сервер упал: ${error.message}`);
  }
  process.exit(1);
});

server.listen(port, '127.0.0.1', () => {
  console.log(`Euro-Office на http://127.0.0.1:${port}/ (просмотрщик: http://127.0.0.1:${port}/viewer.html, редактор: /editor)`);
});
