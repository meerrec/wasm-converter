// Сборка статического сайта Euro-Office из npm-частей @kreijstal/eurooffice-*.
//
// Что делает: берёт у реестра npm метаданные каждой части (версия пинится
// константой), скачивает тарболы в .cache/ (повторные запуски берут их оттуда),
// распаковывает в site/ overlay'ем — подчинённые части первыми, главный пакет
// последним, — копирует наш просмотрщик (viewer.html и собранную TS-оболочку
// dist/, см. pnpm run build) и готовит brotli-копию x2t.wasm.br (в частях
// лежит только несжатый x2t.wasm, а сайт ходит за .br), после чего проверяет
// обязательные пути собранного сайта.
//
// Запуск: node scripts/assemble.mjs

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, statSync, createWriteStream, copyFileSync, writeFileSync, rmSync } from 'node:fs';
import { once } from 'node:events';
import { Readable } from 'node:stream';
import { brotliCompress, constants as zlibConstants } from 'node:zlib';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const SPIKE_DIR = resolve(SCRIPT_DIR, '..');
const SITE_DIR = join(SPIKE_DIR, 'site');
const CACHE_DIR = join(SPIKE_DIR, '.cache');

/** Версия всех частей одна: части собираются из общего dist сайта. */
const VERSION = '9.3.5-rc.1.build.1';
const SCOPE = '@kreijstal';
const REGISTRY = 'https://registry.npmjs.org';

/**
 * Порядок overlay: первыми ложатся подчинённые части, последним — главный
 * пакет. Подтверждено по факту: у web-apps-1 и web-apps-2 пересекаются 190
 * файлов, и все они побайтово одинаковы, а с главным пакетом пересекаются
 * только README/SHA256SUMS/LICENSE — их и должен перекрывать главный.
 */
const PARTS = [
  'eurooffice-fonts-1',
  'eurooffice-fonts-2',
  'eurooffice-sdkjs-common',
  'eurooffice-web-apps-1',
  'eurooffice-web-apps-2',
  'eurooffice',
];

function formatBytes(size) {
  if (size < 1024) return `${size} Б`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} КиБ`;
  return `${(size / (1024 * 1024)).toFixed(1)} МиБ`;
}

/**
 * Метаданные части. `null` — части нет в реестре (404): это штатный случай для
 * ещё не опубликованных частей, а не ошибка сборки.
 */
async function fetchTarballUrl(part) {
  const url = `${REGISTRY}/${encodeURIComponent(`${SCOPE}/${part}`)}/${VERSION}`;
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(`реестр ответил HTTP ${response.status} на ${url}`);
  }
  const meta = await response.json();
  const tarball = meta?.dist?.tarball;
  if (typeof tarball !== 'string') {
    throw new Error(`в метаданных ${part} нет dist.tarball`);
  }
  return tarball;
}

/** Тарбол из кэша или из реестра; после скачивания переименовывается из .tmp. */
async function fetchTarball(part, url) {
  const tgz = join(CACHE_DIR, `${part}.tgz`);
  if (existsSync(tgz) && statSync(tgz).size > 0) {
    console.log(`  тарбол из кэша: .cache/${part}.tgz (${formatBytes(statSync(tgz).size)})`);
    return tgz;
  }
  const tmp = `${tgz}.tmp`;
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`скачивание ${part}: HTTP ${response.status} (${url})`);
  }
  const total = Number(response.headers.get('content-length')) || 0;
  let received = 0;
  let lastPrint = 0;
  process.stdout.write(`  скачиваю ${part}…`);
  const out = createWriteStream(tmp);
  for await (const chunk of Readable.fromWeb(response.body)) {
    received += chunk.length;
    if (!out.write(chunk)) await once(out, 'drain');
    const now = Date.now();
    if (now - lastPrint > 1500) {
      lastPrint = now;
      // Строка прогресса на месте, без перевода: строку затрёт финальный лог.
      process.stdout.write(`\r  скачиваю ${part}: ${formatBytes(received)}${total ? ` из ${formatBytes(total)}` : ''}   `);
    }
  }
  await new Promise((resolveEnd, rejectEnd) => out.end((error) => (error ? rejectEnd(error) : resolveEnd())));
  process.stdout.write(`\r  скачано: .cache/${part}.tgz — ${formatBytes(received)} (перезапуск возьмёт из кэша)\n`);
  renameSync(tmp, tgz);
  return tgz;
}

/** Распаковка overlay'ем: package/ — общий префикс тарболов npm, его срезаем. */
function extractTarball(tgz) {
  execFileSync('tar', ['-xzf', tgz, '-C', SITE_DIR, '--strip-components=1']);
}

/** Рекурсивная копия каталога: целевой затирается, чтобы не копить устаревшие файлы. */
function copyDir(source, target) {
  rmSync(target, { recursive: true, force: true });
  mkdirSync(target, { recursive: true });
  for (const entry of readdirSync(source, { withFileTypes: true })) {
    const from = join(source, entry.name);
    const to = join(target, entry.name);
    if (entry.isDirectory()) copyDir(from, to);
    else copyFileSync(from, to);
  }
}

/**
 * x2t.wasm.br — то, что редактор реально просит (WASM_PATH в assets/document-*.js).
 * В опубликованных частях есть только несжатый x2t.wasm (35,8 МиБ), поэтому
 * brotli-копию делаем сами тем же zlib из Node: иначе first-save тянет 35 МиБ
 * вместо ~7. Уже существующую копию не трогаем — сборка идемпотентна.
 */
async function ensureX2tBrotli() {
  const wasm = join(SITE_DIR, 'sdkjs', 'common', 'wasm', 'x2t', 'x2t.wasm');
  if (!existsSync(wasm)) return;
  const br = `${wasm}.br`;
  if (existsSync(br) && statSync(br).size > 1024 * 1024) {
    console.log(`\nbrotli-копия x2t.wasm.br на месте (${formatBytes(statSync(br).size)})`);
    return;
  }
  const source = readFileSync(wasm);
  console.log(`\nbrotli-копия x2t.wasm.br: в частях только несжатый x2t.wasm (${formatBytes(source.length)}), сжимаю…`);
  const started = Date.now();
  const compressed = await new Promise((resolveCompress, rejectCompress) => {
    brotliCompress(
      source,
      {
        params: {
          // Максимальное качество: таргет — размер и время скачивания в браузере, не сборки.
          [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
          [zlibConstants.BROTLI_PARAM_SIZE_HINT]: source.length,
        },
      },
      (error, result) => (error ? rejectCompress(error) : resolveCompress(result)),
    );
  });
  writeFileSync(br, compressed);
  console.log(
    `brotli-копия готова: ${formatBytes(compressed.length)} за ${((Date.now() - started) / 1000).toFixed(0)} с — сервер отдаёт её с Content-Encoding: br`,
  );
}

/**
 * Обязательные пути собранного сайта. Возвращает список проблем; пустой список —
 * сайт цел. Каталог fonts/ (движковый каталог шрифтов, ~267 записей) проверяется
 * отдельно: его несут неопубликованные части eurooffice-fonts-1/-2, и его
 * отсутствие — предупреждение, а не отказ (редактор запросы к /fonts/* переносит).
 */
function checkSite() {
  const problems = [];
  const sitePath = (...segments) => join(SITE_DIR, ...segments);
  const requireFile = (rel) => {
    const path = sitePath(rel);
    if (!existsSync(path) || !statSync(path).isFile()) problems.push(`нет файла ${rel}`);
  };
  const requireDirNonEmpty = (rel) => {
    const path = sitePath(rel);
    if (!existsSync(path) || readdirSync(path).length === 0) problems.push(`пуст или отсутствует каталог ${rel}/`);
  };

  requireFile('editor.html');
  requireFile('sdkjs/common/wasm/x2t/x2t.js');
  requireFile('sdkjs/common/wasm/x2t/x2t.wasm.br');
  requireFile('ran-fonts/fonts.css');
  requireFile('dist/main.js');
  requireFile('dist/style.css');
  requireDirNonEmpty('sdkjs/word');
  requireDirNonEmpty('ran-fonts');

  const assetsDir = sitePath('assets');
  const editorAssets = existsSync(assetsDir) ? readdirSync(assetsDir).filter((name) => /^editor.*\.js$/.test(name)) : [];
  if (editorAssets.length === 0) problems.push('нет assets/editor*.js');

  return problems;
}

/** Файлы и вес site/ — в сводку сборки. */
function siteStats() {
  let files = 0;
  let bytes = 0;
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile()) {
        files += 1;
        bytes += statSync(path).size;
      }
    }
  };
  walk(SITE_DIR);
  return { files, bytes };
}

async function main() {
  mkdirSync(CACHE_DIR, { recursive: true });
  mkdirSync(SITE_DIR, { recursive: true });
  console.log(`сборка сайта Euro-Office ${VERSION} — ${PARTS.length} частей, overlay в ${SITE_DIR}`);

  const skipped = [];
  for (const [index, part] of PARTS.entries()) {
    const tag = `[${index + 1}/${PARTS.length}] ${part}`;
    const tarball = await fetchTarballUrl(part);
    if (tarball === null) {
      console.log(`${tag} — части нет в реестре npm (HTTP 404), пропускаю`);
      skipped.push(part);
      continue;
    }
    console.log(`${tag} — ${tarball}`);
    const tgz = await fetchTarball(part, tarball);
    console.log(`  распаковка overlay'ем в site/…`);
    extractTarball(tgz);
  }

  const viewerSource = join(SPIKE_DIR, 'viewer.html');
  copyFileSync(viewerSource, join(SITE_DIR, 'viewer.html'));
  console.log(`\nнаш просмотрщик: viewer.html → site/viewer.html`);

  const distSource = join(SPIKE_DIR, 'dist');
  if (!existsSync(join(distSource, 'main.js')) || !existsSync(join(distSource, 'style.css'))) {
    throw new Error(`dist/ не собран (нет dist/main.js или dist/style.css) — запустите pnpm run build`);
  }
  copyDir(distSource, join(SITE_DIR, 'dist'));
  console.log(`наш просмотрщик: dist/ → site/dist/`);

  await ensureX2tBrotli();

  const problems = checkSite();
  const stats = siteStats();
  console.log(`\nsite/: файлов ${stats.files}, вес ${formatBytes(stats.bytes)}`);
  if (skipped.length > 0) {
    console.log(
      `пропущены неопубликованные части: ${skipped.join(', ')} — ` +
        `без eurooffice-fonts-1/-2 нет каталога site/fonts/ (движковые шрифты); подробности в README`,
    );
  }
  if (!existsSync(join(SITE_DIR, 'fonts'))) {
    console.log('предупреждение: site/fonts/ (движковый каталог шрифтов) отсутствует — редактор подтянет только встроенные шрифты');
  }
  if (problems.length > 0) {
    throw new Error(`собранный сайт неполон:\n  - ${problems.join('\n  - ')}`);
  }
  console.log('проверка состава: обязательные пути на месте');
  console.log('готово: node scripts/serve.mjs, затем http://127.0.0.1:5271/viewer.html');
}

await main();
