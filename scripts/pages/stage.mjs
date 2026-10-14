// Компоновка публикуемого артефакта GitHub Pages: сайт из двух уровней — в корне
// self-host движка Euro-Office, в подкаталоге /viewer/ — демо examples/viewer.
//
// Запуск:
//   node scripts/pages/stage.mjs [--out .pages] [--serve] [--port 5272]
//
// Почему артефакт собирается отдельным шагом, а не публикуется build/eurooffice/site
// как есть:
//   - вендорный лендинг движка публиковать нельзя (страницы про продукты, локали,
//     SEO-файлы), а вырезать его нужно по правилу «удаляем только то, на что нет
//     ссылок»: это знание о чужой сборке, и место ему здесь, а не в CI-шаге;
//   - GitHub Pages не отдаёт Content-Encoding: br для готовых .br-файлов (жмёт
//     только на лету, gzip), а движок ходит ровно за
//     sdkjs/common/wasm/x2t/x2t.wasm.br — под этим именем обязаны лежать
//     plain-байты, см. ensureX2tPlain;
//   - здесь же живут гейты публикуемого: сломанный артефакт не должен доехать до
//     upload-pages-artifact.
//
// Правило чистки: удаляем только то, на что нет ссылок из editor.html,
// assets/*.js и web-apps/apps/{documenteditor,spreadsheeteditor}/main/index.html.
// Сомнительное оставляем: лишний файл стоит места, а недостающий ломает редактор
// молча. Имена из PRUNE_* проверены grep'ом по этим трём источникам на сборке
// Euro-Office 9.3.5-rc.1.build.1: ни одно в них не встречается.
//
// Гейты. FATAL печатаются все разом и дают код возврата 1 — публиковать неполный
// сайт нельзя. WARN печатаются аннотацией ::warning:: (её подхватывает GitHub
// Actions) и, если задан GITHUB_STEP_SUMMARY, уходят строкой в сводку шага; код
// возврата при них нулевой.

import {
  appendFileSync, closeSync, copyFileSync, createReadStream, existsSync, mkdirSync, openSync,
  readdirSync, readFileSync, readSync, rmSync, statSync,
} from 'node:fs';
import { createServer } from 'node:http';
import { dirname, extname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
/** Корень репозитория — от расположения скрипта, а не от cwd: запускать можно откуда угодно. */
const ROOT_DIR = resolve(SCRIPT_DIR, '..', '..');
/** Источник корня сайта: распакованные части движка (scripts/eurooffice/assemble.mjs). */
const ENGINE_DIR = join(ROOT_DIR, 'build', 'eurooffice', 'site');
/** Источник /viewer/: собранное демо (pnpm -F @doc-converter/example-viewer build). */
const DEMO_DIR = join(ROOT_DIR, 'examples', 'viewer', 'dist');

/** Сигнатура wasm-модуля: по ней отличаем plain-байты x2t.wasm от brotli-копии. */
const WASM_MAGIC = Buffer.from([0x00, 0x61, 0x73, 0x6d]);

/** Префикс, под которым сайт живёт на Pages (имя репозитория): с ним запечена база демо. */
const SITE_PREFIX = '/wasm-converter/';

/** Лимит сайта GitHub Pages; до него артефакт упирается не только в запрет, но и в 10 минут деплоя. */
const SIZE_LIMIT = 1000 * 1024 * 1024;
/** Порог беспокойства: запас до лимита тает, а upload/deploy упирается в 10 минут. */
const SIZE_WARN = 800 * 1024 * 1024;

/** Дев-адрес хоста редактора: в прод-артефакте ему делать нечего. */
const DEV_ADDRESSES = ['127.0.0.1:5271', 'localhost:5271'];

/** Расширения бинарников демо: дев-адрес в них не ищем, читать 36 МиБ wasm ради grep'а незачем. */
const BINARY_EXT = new Set([
  '.wasm', '.br', '.docx', '.xlsx', '.pdf', '.zip', '.png', '.jpg', '.jpeg',
  '.gif', '.ico', '.webp', '.woff', '.woff2', '.ttf', '.otf',
]);

/**
 * Каталоги корня сайта, которые не публикуем: страницы вендорного лендинга
 * (open/, convert/ — его же конвертеры по файловым расширениям, help/ — его
 * документация), локализации лендинга, его IIFE-компоненты и сборочные рецепты.
 * `convert/` — не наш маршрут: демо конвертирует на `#/convert` у себя.
 */
const PRUNE_DIRS = new Set(['de', 'es', 'ja', 'ko', 'pt', 'zh-CN', 'open', 'convert', 'help', 'ranui-iife', 'build']);

/** Файлы корня сайта, которые не публикуем (по именам из сборки движка). */
const PRUNE_FILES = new Set([
  'home.css', 'open-local.js', 'history-recent.js', 'lang-switch.js', 'sw-register.js', 'sw.js',
  'robots.txt', 'sitemap.xml', 'llms.txt', 'llms-full.txt', '_headers', '_redirects',
]);

/** Лендинг приходит хеш-именами и обновляется от сборки к сборке — ловим по префиксу. */
const PRUNE_LANDING = /^landing.*\.(?:js|css)$/;

/** Сработавшие WARN: печатаются сразу, в конце сводятся списком. */
const warnings = [];

function warn(text) {
  warnings.push(text);
  console.log(`::warning::${text}`);
  const summary = process.env.GITHUB_STEP_SUMMARY;
  if (summary) appendFileSync(summary, `- WARN: ${text}\n`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function hasFile(path) {
  return existsSync(path) && statSync(path).isFile();
}

function dirNames(dir) {
  return existsSync(dir) ? readdirSync(dir) : [];
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} Б`;
  const units = ['КиБ', 'МиБ', 'ГиБ'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** Первые байты файла: x2t.wasm — 35,8 МиБ, читать его целиком ради сигнатуры незачем. */
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

function isPlainWasm(path) {
  return headBytes(path, 4).equals(WASM_MAGIC);
}

/**
 * Есть ли needle в файле; читаем кусками, а не целиком. Перехлёст на длину
 * needle закрывает стык кусков, а сравнение идёт по байтам: в UTF-8 многобайтный
 * символ мог бы разъехаться по границе куска.
 */
function fileContains(file, needle) {
  const needleBytes = Buffer.from(needle, 'utf8');
  const chunkSize = 64 * 1024;
  const buffer = Buffer.alloc(chunkSize);
  const fd = openSync(file, 'r');
  try {
    let offset = 0;
    let carry = Buffer.alloc(0);
    for (;;) {
      const read = readSync(fd, buffer, 0, chunkSize, offset);
      if (read === 0) return false;
      const chunk = carry.length === 0 ? buffer.subarray(0, read) : Buffer.concat([carry, buffer.subarray(0, read)]);
      if (chunk.includes(needleBytes)) return true;
      carry = Buffer.from(chunk.subarray(Math.max(0, chunk.length - (needleBytes.length - 1))));
      offset += read;
    }
  } finally {
    closeSync(fd);
  }
}

function parseArgs(argv) {
  const options = { out: '.pages', serve: false, port: 5272 };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--out') {
      index += 1;
      if (argv[index] === undefined) fail('--out без значения: укажите каталог, например --out .pages');
      options.out = argv[index];
    } else if (arg === '--serve') {
      options.serve = true;
    } else if (arg === '--port') {
      index += 1;
      const port = Number(argv[index]);
      if (!Number.isInteger(port) || port <= 0) fail(`--port ждёт целое число, а не «${argv[index] ?? ''}»`);
      options.port = port;
    } else {
      fail(`неизвестный аргумент «${arg}»; допустимые: --out <каталог>, --serve, --port <число>`);
    }
  }
  return options;
}

/**
 * Что не публикуем в корне сайта. Правило одно: здесь перечислено ровно то, на
 * что не ссылается движок, — страницы лендинга и его же обвязка. Файлы, которые
 * движок упоминает (раньше всего ran-tokens.<хеш>.css из editor.html), остаются
 * на месте, даже если по имени похожи на лендинг.
 *
 * Все перечисленные ниже правила задевают только корень. Вложенное не трогаем:
 * приложения движка — это не лендинг, и вырезать в них HTML значит отдать
 * посетителю сайт без редактора.
 */
function pruneEngine(rel, isDirectory) {
  const slash = rel.indexOf('/');
  // Первый сегмент — для самого каталога и для всего, что внутри: иначе
  // каталог из списка скопировался бы пустым (его дети отсеялись бы, а он нет).
  const first = slash === -1 ? rel : rel.slice(0, slash);
  if (PRUNE_DIRS.has(first)) return true;
  if (isDirectory) return false;
  if (rel === 'editor.html') return false;
  // HTML вырезаем только в корне сайта: остальные корневые страницы — вендорный
  // лендинг. Вложенный HTML — это приложения движка (web-apps/apps/*/main/
  // index.html и обвязка вокруг них): вырезав их, мы оставим сайт без редактора,
  // а он и есть корневая страница (editor.html грузит их во фрейме).
  if (slash === -1 && rel.endsWith('.html')) return true;
  return PRUNE_FILES.has(rel) || PRUNE_LANDING.test(rel);
}

/**
 * Копирование дерева файловым потоком: copyFileSync на каждый файл, содержимое
 * в память не читается (600 МиБ на 2790 файлов). `skip` получает путь
 * относительно источника, всегда через «/».
 */
function copyTree(fromDir, toDir, rel, skip) {
  mkdirSync(toDir, { recursive: true });
  for (const entry of readdirSync(fromDir, { withFileTypes: true })) {
    const childRel = rel === '' ? entry.name : `${rel}/${entry.name}`;
    if (skip(childRel, entry.isDirectory())) continue;
    const from = join(fromDir, entry.name);
    const to = join(toDir, entry.name);
    if (entry.isDirectory()) copyTree(from, to, childRel, skip);
    else if (entry.isFile()) copyFileSync(from, to);
    // Символических ссылок в сборках нет; молча пропустить их значило бы
    // опубликовать сайт без файла, который есть в источнике.
    else throw new Error(`${from}: не файл и не каталог — артефакт собрался бы неполным`);
  }
}

function treeStats(dir) {
  let files = 0;
  let bytes = 0;
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile()) {
        files += 1;
        bytes += statSync(path).size;
      }
    }
  };
  walk(dir);
  return { files, bytes };
}

/**
 * Под именем x2t.wasm.br в публикуемом артефакте обязаны лежать несжатые байты
 * x2t.wasm: Pages не отдаёт Content-Encoding: br, и сжатая копия молча ломает
 * первое сохранение в редакторе. Локальный assemble по умолчанию сжимает
 * (режим brotli), CI собирает с EUROOFFICE_X2T_BR=copy — и там переписывать
 * нечего; если переписали, значит сайт собран из дев-состояния.
 */
function ensureX2tPlain(out) {
  const br = join(out, 'sdkjs', 'common', 'wasm', 'x2t', 'x2t.wasm.br');
  const wasm = join(out, 'sdkjs', 'common', 'wasm', 'x2t', 'x2t.wasm');
  if (!hasFile(br) || isPlainWasm(br) || !hasFile(wasm)) return;
  copyFileSync(wasm, br);
  warn(
    `x2t.wasm.br перезаписан несжатыми байтами x2t.wasm (${formatBytes(statSync(wasm).size)}): ` +
      'в источнике лежала brotli-копия, а Pages её как сжатую не отдаст — сайт собран дев-режимом assemble ' +
      '(EUROOFFICE_X2T_BR=brotli); в CI стоит copy, и этой перезаписи там не бывает',
  );
}

/** Текстовые файлы демо: бинарники пропускаем, дев-адрес в них не живёт. */
function textFiles(dir) {
  const found = [];
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile() && !BINARY_EXT.has(extname(entry.name).toLowerCase())) found.push(path);
    }
  };
  walk(dir);
  return found;
}

/**
 * Гейты артефакта. Возвращает список FATAL-проблем (пустой — публиковать можно);
 * WARN печатает сам, через warn().
 */
function gates(out) {
  const problems = [];
  const at = (rel) => join(out, rel);
  const requireFile = (rel, why) => {
    if (!hasFile(at(rel))) problems.push(`нет ${rel}${why === undefined ? '' : ` — ${why}`}`);
  };

  // ── корень: движок ────────────────────────────────────────────────────────
  requireFile('editor.html', 'без него демо не откроет редактор');
  if (!dirNames(at('assets')).some((name) => /^editor.*\.js$/.test(name))) {
    problems.push('в assets/ нет editor*.js — бандл редактора не собрался');
  }
  requireFile('sdkjs/common/wasm/x2t/x2t.js', 'рантайм конвертера не доехал');
  requireFile('sdkjs/word/sdk-all.js', 'ядро текстового редактора не доехало');
  requireFile('ran-fonts/fonts.css');
  requireFile('manifest.json');
  requireFile('document_editor_service_worker.js');

  const br = at('sdkjs/common/wasm/x2t/x2t.wasm.br');
  if (!hasFile(br)) problems.push('нет sdkjs/common/wasm/x2t/x2t.wasm.br — движок ходит именно за этим именем');
  else if (!isPlainWasm(br)) {
    problems.push('sdkjs/common/wasm/x2t/x2t.wasm.br начинается не с \\0asm — под .br лежит сжатая копия, а Pages не отдаёт Content-Encoding: br');
  }

  // Лицензия AGPL движка обязана уехать вместе с ним: публикуем чужой код.
  const license = at('LICENSES/euro-office-LICENSE.txt');
  if (!hasFile(license)) problems.push('нет LICENSES/euro-office-LICENSE.txt');
  else if (!readFileSync(license, 'utf8').includes('AGPL')) {
    problems.push('LICENSES/euro-office-LICENSE.txt не похож на AGPL-лицензию движка');
  }

  // ── корень: наши файлы ────────────────────────────────────────────────────
  const ownIndex = at('index.html');
  if (!hasFile(ownIndex)) problems.push('нет index.html — страницы-указателя в корне нет');
  else if (!readFileSync(ownIndex, 'utf8').includes('./viewer/')) {
    problems.push('index.html в корне не наш: в нём нет ссылки ./viewer/');
  }
  // Корневой sw.js — единственное место, где движку ставятся заголовки изоляции
  // (COEP родителя не впустит кадр редактора без COEP у самого кадра, а серверных
  // настроек на Pages нет). Потерять эту строку можно тихо — воркер останется
  // рабочим, просто перестанет добавлять заголовок, — а браузерная приёмка
  // (scripts/pages/check-route.mjs) в CI не гоняется, так что в пайплайне это
  // дешёвая и единственная страховка от потери изоляции: без заголовков страница
  // #/eurooffice на Pages мертва (iframe редактора блокируется), а вкладка при
  // этом выглядит живой.
  // Строка же и опознаёт наш воркер: у вендорного PWA-воркера её нет, и подмена
  // (наши файлы копируются последними поверх вендорных) не пройдёт молча.
  const ownSw = at('sw.js');
  if (!hasFile(ownSw)) problems.push('нет sw.js в корне — старые кэши и заголовки изоляции движка снять нечем');
  else if (!readFileSync(ownSw, 'utf8').includes('Cross-Origin-Embedder-Policy')) {
    problems.push('sw.js в корне не ставит Cross-Origin-Embedder-Policy — изолированная страница демо не впустит кадр редактора');
  }

  // ── /viewer/: демо ────────────────────────────────────────────────────────
  const demoIndex = at('viewer/index.html');
  if (!hasFile(demoIndex)) problems.push('нет viewer/index.html — демо не скомпоновалось');
  else {
    const html = readFileSync(demoIndex, 'utf8');
    // База демо запекается на сборке (VITE_BASE=/<repo>/viewer/). Незапечённая
    // база — это голый /assets/ в src/href: такой билд поехал бы за своими
    // ассетами в корень сайта, к движку.
    if (/(?:src|href)="\/assets\//.test(html)) {
      problems.push('viewer/index.html ссылается на /assets/ без префикса — база запечена от корня, а ассеты демо лежат в /viewer/assets/');
    }
    if (!/(?:src|href)="\/[^"]*\/assets\//.test(html)) {
      problems.push('viewer/index.html не ссылается ни на один /<префикс>/assets/… — база не запечена вовсе');
    }
  }

  const demoAssets = dirNames(at('viewer/assets'));
  if (!demoAssets.some((name) => name.endsWith('.js'))) problems.push('в viewer/assets/ нет ни одного .js');
  for (const page of ['DocxPage', 'XlsxPage', 'ConvertPage', 'EuroofficePage']) {
    if (!demoAssets.some((name) => name.startsWith(`${page}-`) && name.endsWith('.js'))) {
      problems.push(`в viewer/assets/ нет чанка ${page}-*.js — страница не попала в сборку`);
    }
  }

  // Гейта про viewer/coi-serviceworker.js здесь нет намеренно: вендоренный воркер
  // из демо удалён, изоляцию даёт корневой. Навигацию кадра обслуживает воркер-
  // контроллер родителя, а не тот, чья область совпадает с URL кадра, — поэтому
  // воркера внутри /viewer/ демо и не требует.

  const x2t = at('viewer/x2t/x2t.wasm');
  if (!hasFile(x2t)) problems.push('нет viewer/x2t/x2t.wasm — конвертеру демо нечем работать');
  else if (statSync(x2t).size < 1024 * 1024) {
    problems.push(`viewer/x2t/x2t.wasm весит ${formatBytes(statSync(x2t).size)} — в сборку уехал LFS-пойнтер вместо бинарника`);
  }

  // Дев-адрес хоста редактора не должен доехать до прода: .env.development
  // читается только дев-сервером Vite, и литерал здесь — след такой сборки.
  if (existsSync(at('viewer'))) {
    for (const file of textFiles(at('viewer'))) {
      for (const needle of DEV_ADDRESSES) {
        if (fileContains(file, needle)) {
          problems.push(`в viewer/${relative(at('viewer'), file)} остался дев-адрес «${needle}» — сборка сделана с .env.development`);
        }
      }
    }
  }

  // ── WARN ─────────────────────────────────────────────────────────────────
  // Каталог шрифтов движка зеркалится из чужого GitHub API (assemble.mjs): его
  // неполнота — это недорисованный табличный редактор, а не сломанный сайт.
  const fonts = dirNames(at('fonts')).length;
  if (fonts < 203) {
    warn(`fonts/: ${fonts} файлов вместо 203 — каталог шрифтов зеркалится из чужого GitHub API, и его неполнота означает, что табличный редактор не дорисует шрифты; демо при этом рабочее`);
  }

  const stats = treeStats(out);
  if (stats.bytes >= SIZE_LIMIT) {
    problems.push(`артефакт ${formatBytes(stats.bytes)} — лимит сайта GitHub Pages 1 ГБ, деплой его не примет`);
  } else if (stats.bytes > SIZE_WARN) {
    warn(`артефакт ${formatBytes(stats.bytes)} — запас до лимита в 1 ГБ тает, а upload/deploy упирается в 10 минут`);
  }

  return { problems, stats };
}

// ── Локальный сервер (--serve) ───────────────────────────────────────────────

/**
 * MIME-карта как у scripts/eurooffice/serve.mjs, без дев-надстроек: заголовков
 * кодирования сервер не ставит намеренно — Pages их не ставит, и проверять
 * артефакт нужно ровно так, как его отдадут посетителю.
 */
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

/**
 * Путь после префикса → файл на диске. Листингов не делаем, clean URL не
 * разворачиваем: Pages тоже отдаёт /editor только как 404, и повторять здесь
 * чужую поблажку значило бы проверять не тот сайт, что публикуется.
 */
function resolveSiteFile(root, pathname) {
  const target = resolve(root, pathname.replace(/^[/\\]+/, ''));
  if (target !== root && !target.startsWith(root + sep)) return null;
  if (hasFile(target)) return target;
  if (existsSync(target) && statSync(target).isDirectory()) {
    const index = join(target, 'index.html');
    return hasFile(index) ? index : null;
  }
  return null;
}

function sendText(response, status, text, headOnly) {
  const body = Buffer.from(text, 'utf8');
  response.writeHead(status, { 'Content-Type': 'text/plain; charset=utf-8', 'Content-Length': body.length });
  response.end(headOnly ? undefined : body);
}

function startServer(root, port) {
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
    // Вне префикса — 404: на Pages корень домена занят другими проектами, а
    // артефакт живёт в /<имя-репозитория>/; сервер обязан повторять это, иначе
    // проверка «по префиксу» не проверяла бы ничего.
    if (!pathname.startsWith(SITE_PREFIX)) {
      sendText(response, 404, `вне префикса ${SITE_PREFIX}: ${pathname}`, headOnly);
      return;
    }
    const file = resolveSiteFile(root, pathname.slice(SITE_PREFIX.length));
    if (file === null) {
      sendText(response, 404, `не найдено: ${pathname}`, headOnly);
      return;
    }
    // Тип берётся у имени без .br: x2t.wasm.br — это wasm, а не «br». Заголовков
    // кодирования нет: в артефакте под .br лежат plain-байты (см. ensureX2tPlain).
    const typeSource = file.endsWith('.br') ? file.slice(0, -3) : file;
    response.writeHead(200, {
      'Content-Type': MIME.get(extname(typeSource).toLowerCase()) ?? 'application/octet-stream',
      'Content-Length': statSync(file).size,
    });
    if (headOnly) response.end();
    else createReadStream(file).pipe(response);
  });

  server.on('error', (error) => {
    if (error.code === 'EADDRINUSE') {
      console.error(`порт ${port} занят — укажите другой: node scripts/pages/stage.mjs --serve --port 5273`);
    } else {
      console.error(`сервер упал: ${error.message}`);
    }
    process.exitCode = 1;
  });

  server.listen(port, '127.0.0.1', () => {
    console.log(`\nстатика артефакта на http://127.0.0.1:${port}${SITE_PREFIX} (как на GitHub Pages: без заголовков кодирования)`);
    console.log(`  корень:      http://127.0.0.1:${port}${SITE_PREFIX}`);
    console.log(`  демо:        http://127.0.0.1:${port}${SITE_PREFIX}viewer/`);
    console.log(`  Euro-Office: http://127.0.0.1:${port}${SITE_PREFIX}viewer/#/eurooffice`);
    console.log('  остановить: Ctrl+C');
  });
}

// ── Основной ход ─────────────────────────────────────────────────────────────

function requireSources() {
  if (!hasFile(join(ENGINE_DIR, 'editor.html'))) {
    fail(`нет собранного сайта движка (${ENGINE_DIR}) — запустите: node scripts/eurooffice/assemble.mjs`);
  }
  if (!hasFile(join(DEMO_DIR, 'index.html'))) {
    fail(`нет сборки демо (${DEMO_DIR}) — запустите: pnpm -F @doc-converter/example-viewer build`);
  }
  for (const name of ['index.html', 'sw.js']) {
    if (!hasFile(join(SCRIPT_DIR, name))) fail(`нет ${join(SCRIPT_DIR, name)} — без него артефакт неполон`);
  }
}

/** Артефакт собирается с нуля: остатки прошлого прогона в него не попадают. */
function prepareOut(out) {
  if (out === ROOT_DIR || ROOT_DIR.startsWith(out + sep) || out === ENGINE_DIR || ENGINE_DIR.startsWith(out + sep)) {
    fail(`--out ${out} накрывает исходники: укажите каталог вне ${ROOT_DIR}`);
  }
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const out = resolve(ROOT_DIR, options.out);
  requireSources();
  prepareOut(out);

  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, '\n## pages: stage.mjs\n');

  copyTree(ENGINE_DIR, out, '', pruneEngine);
  copyTree(DEMO_DIR, join(out, 'viewer'), '', () => false);
  // Наши файлы кладутся последними и перекрывают одноимённые: index.html встаёт
  // на место вендорного лендинга, sw.js — на место вендорного PWA-воркера.
  copyFileSync(join(SCRIPT_DIR, 'index.html'), join(out, 'index.html'));
  copyFileSync(join(SCRIPT_DIR, 'sw.js'), join(out, 'sw.js'));

  // Движок регистрирует свой document_editor_service_worker.js из кадров редактора,
  // и областью ему служит корень сайта — тот же, что у нашего sw.js: вендорная
  // заглушка (160 байт, без заголовков изоляции) вытеснила бы наш воркер. Поэтому
  // под этим именем лежит ровно наш скрипт, побайтово: при регистрации браузер
  // сравнивает скрипт с сохранённым и, если он не изменился, переустановки не
  // делает — так две регистрации в одном scope не дёргают друг друга.
  // Подменяем только существующий файл: если движок переименует регистрируемый
  // скрипт, об этом скажет гейт, а не тихая потеря изоляции в артефакте.
  const vendorWorker = join(out, 'document_editor_service_worker.js');
  if (hasFile(vendorWorker)) copyFileSync(join(SCRIPT_DIR, 'sw.js'), vendorWorker);

  ensureX2tPlain(out);

  const { problems, stats } = gates(out);
  console.log(`\nартефакт: ${relative(ROOT_DIR, out)} — файлов ${stats.files}, вес ${formatBytes(stats.bytes)}`);

  if (problems.length > 0) {
    console.error(`\nартефакт неполон — публиковать нельзя (${problems.length}):`);
    for (const problem of problems) console.error(`  - ${problem}`);
    process.exitCode = 1;
    return;
  }

  console.log(warnings.length === 0 ? 'гейты пройдены, WARN нет' : `гейты пройдены, WARN (${warnings.length}):`);
  for (const text of warnings) console.log(`  - ${text}`);

  if (options.serve) startServer(out, options.port);
  else console.log(`готово: node scripts/pages/stage.mjs --out ${options.out} --serve — статика по ${SITE_PREFIX}`);
}

main();
