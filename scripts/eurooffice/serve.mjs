// Статический сервер собранного сайта (build/eurooffice/site/): zero-dep, порт
// из аргумента/env. Особенности:
//   - clean URL (/editor → editor.html), MIME-карта, никаких листингов, 404
//     текстом, HEAD — так его проверяет curl;
//   - Cross-Origin-Resource-Policy: cross-origin и Cross-Origin-Embedder-Policy:
//     require-corp на всех ответах: в дев-режиме страница демо живёт на другом
//     origin (http://localhost:5178) и сама изолирована по COEP require-corp, а
//     Chromium пускает в такую страницу чужой iframe, только если тот отдаёт оба
//     заголовка — одного CORP мало, ответ падает с net::ERR_BLOCKED_BY_RESPONSE.
//     В проде всё same-origin, и оба заголовка просто безвредны;
//   - .br определяется по содержимому, а не по имени: после
//     EUROOFFICE_X2T_BR=copy под именем x2t.wasm.br лежат несжатые байты, и
//     ответ с Content-Encoding: br сломал бы редактор молча.
//
// Запуск: node scripts/eurooffice/serve.mjs [порт]   (или VIEWER_PORT=5271 node …)

import { createServer } from 'node:http';
import { closeSync, createReadStream, existsSync, openSync, readSync, statSync } from 'node:fs';
import { extname, join, normalize, resolve, sep, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
/** Корень репозитория — от расположения скрипта, а не от cwd: запускать можно откуда угодно. */
const ROOT_DIR = resolve(SCRIPT_DIR, '..', '..');
const SITE_DIR = join(ROOT_DIR, 'build', 'eurooffice', 'site');

/** Сигнатура wasm-модуля: по ней узнаём plain-байты под именем .br. */
const WASM_MAGIC = Buffer.from([0x00, 0x61, 0x73, 0x6d]);

const port = Number(process.argv[2] ?? process.env.VIEWER_PORT ?? process.env.PORT ?? 5271);
if (!Number.isInteger(port) || port <= 0) {
  console.error(`порт должен быть целым числом, а не «${process.argv[2] ?? process.env.VIEWER_PORT}»`);
  process.exit(1);
}
if (!existsSync(join(SITE_DIR, 'editor.html'))) {
  console.error(`сайт не собран (нет ${join(SITE_DIR, 'editor.html')}) — запустите node scripts/eurooffice/assemble.mjs`);
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

/** Первые байты файла без чтения всего файла: x2t.wasm — 35,8 МиБ, читать его ради сигнатуры незачем. */
function headBytes(path, count) {
  const fd = openSync(path, 'r');
  try {
    const buffer = Buffer.alloc(count);
    const read = readSync(fd, buffer, 0, count, 0);
    return buffer.subarray(0, read);
  } finally {
    closeSync(fd);
  }
}

/**
 * Файл с именем .br, который надо отдать с Content-Encoding: br. Имя здесь не
 * доказательство: `EUROOFFICE_X2T_BR=copy` кладёт под ним plain-байты x2t.wasm
 * (так публикует GitHub Pages — она не отдаёт br), и заголовок кодирования на
 * них — молчаливая поломка редактора. Отличий ровно одно: сигнатура wasm.
 */
function isBrotliEncoded(file) {
  return file.endsWith('.br') && !headBytes(file, 4).equals(WASM_MAGIC);
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

  // Страховка: сайт просит .br, а на диске только несжатый x2t.wasm (например,
  // assemble ещё не запускали, а части уже распакованы) — отдаём base как есть,
  // без Content-Encoding. Если копия есть, сюда не попадём: она найдётся выше,
  // а сжата ли она — решает isBrotliEncoded.
  if (target.endsWith('.br')) {
    const base = fileIfExists(target.slice(0, -3));
    if (base !== null) return base;
  }

  return null;
}

/**
 * Заголовки кросс-доменного встраивания — на все ответы, включая 404: странице
 * демо на другом origin'е нужен именно этот набор (см. шапку файла).
 */
const CROSS_ORIGIN_HEADERS = {
  'Cross-Origin-Resource-Policy': 'cross-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

function sendFile(response, file, headOnly) {
  const brotli = isBrotliEncoded(file);
  // Тип берётся у имени без .br: x2t.wasm.br — это wasm, а не «br».
  const typeSource = file.endsWith('.br') ? file.slice(0, -3) : file;
  const headers = {
    'Content-Type': MIME.get(extname(typeSource).toLowerCase()) ?? 'application/octet-stream',
    'Content-Length': statSync(file).size,
    ...CROSS_ORIGIN_HEADERS,
  };
  if (brotli) headers['Content-Encoding'] = 'br';
  response.writeHead(200, headers);
  if (headOnly) {
    response.end();
    return;
  }
  createReadStream(file).pipe(response);
}

function sendText(response, status, text, headOnly) {
  const body = Buffer.from(text, 'utf8');
  response.writeHead(status, {
    'Content-Type': 'text/plain; charset=utf-8',
    'Content-Length': body.length,
    ...CROSS_ORIGIN_HEADERS,
  });
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
    console.error(`порт ${port} занят — укажите другой: node scripts/eurooffice/serve.mjs 5272`);
  } else {
    console.error(`сервер упал: ${error.message}`);
  }
  process.exit(1);
});

server.listen(port, '127.0.0.1', () => {
  console.log(`редактор Euro-Office: http://127.0.0.1:${port}/editor (статика из ${SITE_DIR})`);
});
