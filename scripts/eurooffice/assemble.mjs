// Сборка статического сайта Euro-Office из npm-частей @kreijstal/eurooffice-*.
//
// Что делает: берёт у реестра npm метаданные каждой части (версия пинится
// константой), скачивает тарболы в build/eurooffice/.cache/ (повторные запуски
// берут их оттуда), распаковывает в build/eurooffice/site/ overlay'ем —
// подчинённые части первыми, главный пакет последним, — зеркалирует каталог
// движковых шрифтов fonts/ из репозитория ranuts/document (npm-части
// eurooffice-fonts-1/-2 не опубликованы; сеть не должна ронять сборку — при
// стойкой неудаче громкое предупреждение и продолжение), готовит x2t.wasm.br в
// режиме EUROOFFICE_X2T_BR (см. ensureX2tBr) и проверяет, что обязательные пути
// сайта на месте.
//
// Страницы просмотрщика здесь нет: она живёт в примере examples/viewer и
// публикуется отдельно — scripts/pages/stage.mjs.
//
// Запуск: node scripts/eurooffice/assemble.mjs
//         EUROOFFICE_X2T_BR=copy node scripts/eurooffice/assemble.mjs   (CI: plain-байты под именем .br)

import { execFileSync } from 'node:child_process';
import {
  closeSync,
  copyFileSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  readSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { once } from 'node:events';
import { Readable } from 'node:stream';
import { brotliCompress, constants as zlibConstants } from 'node:zlib';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
/** Корень репозитория — от расположения скрипта, а не от cwd: запускать можно откуда угодно. */
const ROOT_DIR = resolve(SCRIPT_DIR, '..', '..');
const SITE_DIR = join(ROOT_DIR, 'build', 'eurooffice', 'site');
const CACHE_DIR = join(ROOT_DIR, 'build', 'eurooffice', '.cache');

/** Версия всех частей одна: части собираются из общего dist сайта. */
const VERSION = '9.3.5-rc.1.build.1';
const SCOPE = '@kreijstal';
const REGISTRY = 'https://registry.npmjs.org';

/** API каталога шрифтов: зеркало public/fonts/ из ranuts/document (см. mirrorFonts). */
const FONTS_API = 'https://api.github.com/repos/ranuts/document/contents/public/fonts';
/** Конкурентность скачивания файлов каталога шрифтов. */
const FONTS_PARALLELISM = 8;

/** Сигнатура wasm-модуля: по ней отличаем сжатый x2t.wasm.br от plain-байтов. */
const WASM_MAGIC = Buffer.from([0x00, 0x61, 0x73, 0x6d]);

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

/** Файл начинается с сигнатуры wasm — то есть под именем .br лежат несжатые байты. */
function isPlainWasm(path) {
  return headBytes(path, 4).equals(WASM_MAGIC);
}

/**
 * Режим x2t.wasm.br из EUROOFFICE_X2T_BR:
 *
 * - `brotli` (по умолчанию) — локальная разработка: сжатая копия x2t.wasm.br,
 *   сервер отдаёт её с Content-Encoding: br;
 * - `copy` — для CI: `.br` получает несжатые байты x2t.wasm. GitHub Pages не
 *   отдаёт Content-Encoding: br, поэтому в публикуемом артефакте под именем
 *   `.br` обязаны лежать plain-байты — иначе браузер попытается разжать wasm
 *   как brotli и редактор не поднимется. Сжатие там же — только потерянные
 *   2–4 минуты сборки.
 *
 * Значение проверяется до скачивания частей: опечатка не должна ждать
 * пятиминутной распаковки, чтобы упасть.
 */
function x2tBrMode() {
  const raw = (process.env.EUROOFFICE_X2T_BR ?? '').trim().toLowerCase();
  if (raw === '' || raw === 'brotli') return 'brotli';
  if (raw === 'copy') return 'copy';
  console.error(
    `EUROOFFICE_X2T_BR=«${process.env.EUROOFFICE_X2T_BR}» — неизвестный режим; допустимые значения: brotli (по умолчанию), copy`,
  );
  process.exit(1);
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

/**
 * x2t.wasm.br — то, что редактор реально просит (WASM_PATH в assets/document-*.js).
 * В опубликованных частях есть только несжатый x2t.wasm (35,8 МиБ), поэтому
 * копию делаем сами. Режим `brotli` сжимает тем же zlib из Node (иначе
 * first-save тянет 35 МиБ вместо ~7) и уже готовую копию не пересчитывает —
 * сборка идемпотентна. Режим `copy` кладёт несжатые байты: см. x2tBrMode().
 */
async function ensureX2tBr(mode) {
  const wasm = join(SITE_DIR, 'sdkjs', 'common', 'wasm', 'x2t', 'x2t.wasm');
  const br = `${wasm}.br`;
  if (!existsSync(wasm)) {
    // Копировать и сжимать нечего; что файла нет, поймает checkSite() ниже.
    console.log('\nx2t.wasm.br: x2t.wasm в собранном сайте не найден — делать нечего');
    return;
  }
  const wasmSize = statSync(wasm).size;

  if (mode === 'copy') {
    // Безусловная перезапись: после brotli-прогона под этим именем лежит
    // сжатая копия, и «уже есть» здесь означало бы сломать публикацию.
    copyFileSync(wasm, br);
    console.log(
      `\nx2t.wasm.br: режим copy — несжатые байты x2t.wasm (${formatBytes(wasmSize)}), Content-Encoding не нужен`,
    );
    return;
  }

  if (existsSync(br)) {
    const brSize = statSync(br).size;
    // Проверка «размер > 1 МиБ» одна пропускает главную ловушку: остаток от
    // copy-прогона (35,8 МиБ plain-байтов). Сигнатура была — копию надо сжать.
    if (brSize > 1024 * 1024 && !isPlainWasm(br)) {
      console.log(`\nx2t.wasm.br: режим brotli — готовая копия на месте (${formatBytes(brSize)}), не пересчитываю`);
      return;
    }
    console.log(`\nx2t.wasm.br: режим brotli — прежняя копия не сжата (${formatBytes(brSize)}), пересчитываю…`);
  }

  const source = readFileSync(wasm);
  console.log(`\nx2t.wasm.br: режим brotli — в частях только несжатый x2t.wasm (${formatBytes(source.length)}), сжимаю…`);
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
    `x2t.wasm.br: режим brotli — ${formatBytes(compressed.length)} за ${((Date.now() - started) / 1000).toFixed(0)} с,` +
      ` сервер отдаёт её с Content-Encoding: br`,
  );
}

/** МБ для прогресса каталога шрифтов. */
function formatMiB(bytes) {
  return `${(bytes / (1024 * 1024)).toFixed(1)} МБ`;
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

/**
 * Список файлов каталога шрифтов из GitHub API (без токена — публичный репозиторий).
 * Три попытки с паузой: rate-limit/сбой сети — штатные случаи. При стойкой
 * неудаче бросает — вызывающий печатает громкое предупреждение и продолжает.
 */
async function fetchFontsListing() {
  let lastError;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try {
      const response = await fetch(FONTS_API);
      if (!response.ok) {
        throw new Error(`GitHub API: HTTP ${response.status}`);
      }
      const entries = await response.json();
      if (!Array.isArray(entries)) {
        throw new Error('GitHub API: ожидался список файлов, пришло что-то другое');
      }
      const files = [];
      for (const entry of entries) {
        if (entry?.type !== 'file') continue;
        const name = entry.name;
        const url = entry.download_url;
        // Имя обязано быть плоским и безопасным (в каталоге — числа «050»…«281»).
        if (typeof name !== 'string' || !/^[\w.-]+$/.test(name) || typeof url !== 'string') continue;
        files.push({ name, url, size: Number(entry.size) || 0 });
      }
      return files;
    } catch (error) {
      lastError = error;
      if (attempt < 3) {
        console.warn(`  список шрифтов: попытка ${attempt}/3 не удалась — ${error.message}; пауза и ещё раз…`);
        await sleep(3000);
      }
    }
  }
  throw lastError;
}

/** Один файл каталога в кэш: три попытки, .tmp переименовывается только после успеха. */
async function downloadFontFile(file, cachePath) {
  const tmp = `${cachePath}.tmp`;
  let lastError;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try {
      const response = await fetch(file.url);
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      const out = createWriteStream(tmp);
      for await (const chunk of Readable.fromWeb(response.body)) {
        if (!out.write(chunk)) await once(out, 'drain');
      }
      await new Promise((resolveEnd, rejectEnd) => out.end((error) => (error ? rejectEnd(error) : resolveEnd())));
      renameSync(tmp, cachePath);
      return;
    } catch (error) {
      lastError = error;
      rmSync(tmp, { force: true });
      if (attempt < 3) await sleep(500 * attempt);
    }
  }
  throw lastError;
}

/** Ограниченный пул воркеров поверх массива. */
async function mapWithConcurrency(items, limit, worker) {
  const results = new Array(items.length);
  let nextIndex = 0;
  const runners = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (nextIndex < items.length) {
      const index = nextIndex;
      nextIndex += 1;
      results[index] = await worker(items[index]);
    }
  });
  await Promise.all(runners);
  return results;
}

/** Громкое предупреждение: шрифты не собраны, но сборка продолжается (exit 0). */
function loudFontsWarning(reason) {
  console.warn('\n!!! ШРИФТЫ НЕ СКАЧАНЫ — ТАБЛИЧНЫЙ РЕДАКТОР НЕ БУДЕТ РИСОВАТЬ !!!');
  console.warn(`причина: ${reason}`);
  console.warn('сборка продолжается; поломку поймает браузерная приёмка (подробности в README)\n');
}

/**
 * Каталог движковых шрифтов (site/fonts/): редактор запрашивает /fonts/<индекс>
 * по одному файлу, и табличный редактор без него не рисует. npm-части
 * eurooffice-fonts-1/-2, которые должны нести каталог, в реестре не
 * опубликованы (404), поэтому каталог зеркалируем из репозитория
 * ranuts/document (public/fonts/ — те же файлы, что на живом сайте). Файлы
 * намеренно XOR-обфусцированы: де-XOR делает код приложения в браузере, так
 * что копируем их как есть, без обработки.
 *
 * Сеть не роняет сборку: при стойкой неудаче API или скачивания — громкое
 * предупреждение и продолжение. Кэш .cache/fonts/ делает перезапуск быстрым:
 * файл берётся оттуда, если он есть и ненулевой.
 */
async function mirrorFonts() {
  const cacheFonts = join(CACHE_DIR, 'fonts');
  const siteFonts = join(SITE_DIR, 'fonts');
  mkdirSync(cacheFonts, { recursive: true });
  mkdirSync(siteFonts, { recursive: true });

  let listing;
  try {
    listing = await fetchFontsListing();
  } catch (error) {
    loudFontsWarning(`каталог не получен с GitHub API: ${error.message}`);
    return;
  }
  if (listing.length === 0) {
    loudFontsWarning('GitHub API вернул пустой каталог public/fonts');
    return;
  }

  const apiBytes = listing.reduce((sum, file) => sum + file.size, 0);
  console.log(`\nкаталог шрифтов: зеркало public/fonts/ из ranuts/document — ${listing.length} файлов, ${formatMiB(apiBytes)} по API-списку`);
  const started = Date.now();
  let done = 0;
  let bytesDone = 0;
  let cached = 0;
  let downloaded = 0;
  let failed = 0;

  await mapWithConcurrency(listing, FONTS_PARALLELISM, async (file) => {
    try {
      const cachePath = join(cacheFonts, file.name);
      const exists = existsSync(cachePath) && statSync(cachePath).size > 0;
      if (!exists) {
        await downloadFontFile(file, cachePath);
        downloaded += 1;
      } else {
        cached += 1;
      }
      copyFileSync(cachePath, join(siteFonts, file.name));
      done += 1;
      bytesDone += statSync(cachePath).size;
      process.stdout.write(`\r  шрифты: ${done}/${listing.length}, ${formatMiB(bytesDone)}   `);
    } catch (error) {
      failed += 1;
      console.warn(`\n  шрифт ${file.name} не скачался: ${error.message}`);
    }
  });

  // Чистка: файлы site/fonts/, которых нет в списке каталога, — мусор прежних
  // сборок. Список получен — удаляем и их, и каталоги (в каталоге только файлы).
  const keep = new Set(listing.map((file) => file.name));
  for (const entry of readdirSync(siteFonts, { withFileTypes: true })) {
    if (!keep.has(entry.name)) {
      rmSync(join(siteFonts, entry.name), { recursive: true, force: true });
    }
  }

  const onDisk = readdirSync(siteFonts).length;
  if (onDisk === 0) {
    loudFontsWarning('после зеркалирования в site/fonts/ ноль файлов');
  } else if (onDisk < listing.length) {
    console.warn(
      `предупреждение: в site/fonts/ ${onDisk} файлов из ${listing.length} — часть шрифтов не скачалась, табличный редактор может недорисовывать`,
    );
  }
  process.stdout.write(
    `\r  шрифты: готово — ${done}/${listing.length} файлов, ${formatMiB(bytesDone)} за ${((Date.now() - started) / 1000).toFixed(0)} с` +
      ` (скачано ${downloaded}, из кэша ${cached}${failed > 0 ? `, не удалось ${failed}` : ''}); site/fonts/: ${onDisk} файл(ов)\n`,
  );
}

/**
 * Обязательные пути собранного сайта. Возвращает список проблем; пустой список —
 * сайт цел. Это проверка «сборка не развалилась», а не приёмка артефакта:
 * строгие гейты публикуемого сайта (размеры, маршруты, запрет лишнего) живут в
 * scripts/pages/stage.mjs, куда сайт и уезжает целиком.
 *
 * Каталог fonts/ (движковый каталог шрифтов) здесь не проверяется: его
 * наполняет шаг зеркалирования, и его отсутствие — предупреждение, а не отказ
 * сборки (редактор запросы к /fonts/* переносит).
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
  const brMode = x2tBrMode();
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

  await mirrorFonts();
  await ensureX2tBr(brMode);

  const problems = checkSite();
  const stats = siteStats();
  console.log(`\nsite/: файлов ${stats.files}, вес ${formatBytes(stats.bytes)}`);
  if (skipped.length > 0) {
    console.log(
      `пропущены неопубликованные части: ${skipped.join(', ')} — ` +
        `каталог site/fonts/ вместо них зеркалируется из ranuts/document (шаг выше); подробности в README`,
    );
  }
  if (problems.length > 0) {
    throw new Error(`собранный сайт неполон:\n  - ${problems.join('\n  - ')}`);
  }
  console.log('проверка состава: обязательные пути на месте');
  console.log(`готово: node scripts/eurooffice/serve.mjs — http://127.0.0.1:5271/editor (локальная разработка)`);
  console.log('       node scripts/pages/stage.mjs — сборка артефакта публикации (.pages/)');
}

await main();
