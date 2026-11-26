#!/usr/bin/env node
// Замер геометрии кадров DOCX print-layout (модель «страница + поля») и
// XLSX normal (начало A1) по actual-кадрам оракула; пишет geometry.json,
// который читает `strict-viewer-references-online.mjs capture --geometry`.
// Файла нет (чистый checkout) — он создаётся с нуля; есть — записи
// замеренных фикстур обновляются, остальные сохраняются.
//
// Использование:
//   node scripts/measure-docx-stack.mjs <out-оракула> --write <geometry.json>
//
// На страницу замеряются: left/width (белый скан строки), height (низ
// страницы по прозрачному зазору до следующей страницы; у последней
// страницы наследуется от соседней — продолжение листа делает низ
// невидимым), nextTop (первый ряд следующей страницы после зазора).
// Схема geometry.json: <slug>/print-layout = { frame, pages: { page-N:
// { left, width, height, nextTop } } }; <slug>/normal = { a1: { a1x, a1y,
// frameWidth, frameHeight } }.

import { createRequire } from 'node:module';
import { mkdir, readFile, writeFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const requireFromOracle = createRequire(
  path.join(REPO_ROOT, 'tools', 'visual-oracle', 'package.json'),
);
const { PNG } = requireFromOracle('pngjs');

const [, , actualsDir, geoFlag, geoPath] = process.argv;
if (!actualsDir || geoFlag !== '--write' || !geoPath) {
  process.stderr.write('usage: measure-docx-stack.mjs <out-оракула> --write <geometry.json>\n');
  process.exit(2);
}

/** slug фикстуры из source.path — тот же алгоритм, что в оракуле и в .mjs. */
function fixtureSlug(sourcePath) {
  const stem = sourcePath.replace(/^.*\//, '').replace(/\.[^.]+$/, '');
  const slug = stem
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, '-')
    .replace(/^-+|-+$/g, '');
  return slug.length > 0 ? slug : 'document';
}

/**
 * Начало содержимого A1 из actual-кадра XLSX: левая колонка — заголовки строк
 * (заливка 245), дальше белые ячейки; высота строки заголовков колонок ищется
 * по той же границе в первой колонке.
 */
function measureXlsxA1Origin(png) {
  const img = PNG.sync.read(png);
  const at = (x, y) => {
    const i = (y * img.width + x) * 4;
    return [img.data[i], img.data[i + 1], img.data[i + 2], img.data[i + 3]];
  };
  const isHeader = (x, y) => {
    const [r, g, b] = at(x, y);
    return r >= 235 && r <= 250 && Math.abs(r - g) < 6 && Math.abs(g - b) < 6;
  };
  // Сканируем строку внутри первой ячейки по вертикали: до границы — заголовок
  // строки (серый), после — белая ячейка. Берём y чуть ниже середины кадра,
  // где точно ячейки, а не закреплённые области.
  const y = Math.min(200, img.height - 1);
  let a1x = -1;
  for (let x = 0; x < img.width; x += 1) {
    if (!isHeader(x, y)) {
      a1x = x;
      break;
    }
  }
  // Высота заголовков колонок: по первой колонке поиска A1 нет — сканируем
  // колонку левее a1x (внутри заголовков строк): сверху угол и заголовок
  // колонок, граница — первая горизонтальная линия сетки под ними. Проще:
  // идём по x = a1x + 2 (внутри ячеек) сверху вниз: заголовок колонок серый,
  // первая ячейка белая.
  let a1y = -1;
  for (let yy = 0; yy < img.height; yy += 1) {
    if (!isHeader(a1x + 2, yy)) {
      a1y = yy;
      break;
    }
  }
  if (a1x < 0 || a1y < 0) {
    throw new Error('не удалось найти начало A1 в actual-кадре');
  }
  return { a1x, a1y, frameWidth: img.width, frameHeight: img.height };
}

function measurePage(pngBuffer) {
  const img = PNG.sync.read(pngBuffer);
  const alphaAt = (x, y) => img.data[(y * img.width + x) * 4 + 3];
  const whiteAt = (x, y) => {
    const i = (y * img.width + x) * 4;
    return (
      img.data[i + 3] > 0 && img.data[i] > 240 && img.data[i + 1] > 240 && img.data[i + 2] > 240
    );
  };
  // Ширина страницы — максимальная белая протяжённость строки в верхней
  // части кадра: контент может выступать в поля (sections), но страница
  // белая во всю свою ширину на каждой строке. Страница центрирована,
  // поэтому left = (кадр − ширина) / 2.
  let maxSpan = -1;
  let left = -1;
  for (let y = 0; y < Math.min(400, img.height); y += 1) {
    let first = -1;
    let last = -1;
    for (let x = 0; x < img.width; x += 1) {
      if (whiteAt(x, y)) {
        first = x;
        break;
      }
    }
    if (first >= 0) {
      for (let x = img.width - 1; x >= 0; x -= 1) {
        if (whiteAt(x, y)) {
          last = x;
          break;
        }
      }
    }
    if (first >= 0 && last - first + 1 > maxSpan) {
      maxSpan = last - first + 1;
      left = Math.round((img.width - maxSpan) / 2);
    }
  }
  if (maxSpan < 500) {
    throw new Error(`не найдена белая страница в кадре (max span ${maxSpan})`);
  }
  const right = left + maxSpan - 1;
  const frame = { width: img.width, height: img.height };
  // Зазор между страницами — полностью прозрачные ряды; текстовые ряды
  // прозрачными не бывают, поэтому граница страницы определяется надёжно.
  const x0 = left + 8;
  const x1 = right - 8;
  const rowHasInk = (y) => {
    for (let x = x0; x <= x1; x += 1) {
      if (alphaAt(x, y) > 0) return true;
    }
    return false;
  };
  // firstBottom — последний ряд первой страницы (низ), nextTop — первый ряд
  // следующей страницы после зазора; зазор — 3+ полностью прозрачных ряда.
  let firstBottom = -1;
  let nextTop = null;
  let gapLen = 0;
  let inGap = false;
  for (let y = 0; y < img.height; y += 1) {
    if (rowHasInk(y)) {
      if (inGap) {
        if (nextTop === null) nextTop = y;
      } else {
        firstBottom = y;
      }
      gapLen = 0;
    } else {
      gapLen += 1;
      if (gapLen >= 3) inGap = true;
    }
  }
  if (firstBottom < 0) throw new Error('страница пуста');
  return {
    left,
    width: right - left + 1,
    height: null, // заполняется после: низ последней страницы невидим
    nextTop,
    firstBottom,
    frame,
  };
}

const corpus = JSON.parse(
  await readFile(path.join(REPO_ROOT, 'test-fixtures', 'strict-viewer', 'corpus.json'), 'utf8'),
);
const slugById = new Map(
  corpus.fixtures.map((f) => [f.id, fixtureSlug(f.source.path)]),
);

const geometry = await readFile(geoPath, 'utf8').then(
  (text) => JSON.parse(text),
  (error) => {
    if (error?.code === 'ENOENT') return {};
    throw error;
  },
);

let measured = 0;
let measuredXlsx = 0;
for (const entry of await readdir(actualsDir, { withFileTypes: true })) {
  if (!entry.isDirectory()) continue;
  const fixtureId = entry.name;
  const sheetPath = path.join(actualsDir, fixtureId, 'normal', 'sheet-0.png');
  const sheet = await readFile(sheetPath).catch(() => null);
  if (sheet !== null && slugById.has(fixtureId)) {
    const a1 = measureXlsxA1Origin(sheet);
    geometry[`${slugById.get(fixtureId)}/normal`] = { a1 };
    measuredXlsx += 1;
    process.stdout.write(
      `${slugById.get(fixtureId)}: A1@(${a1.a1x},${a1.a1y}), кадр ${a1.frameWidth}×${a1.frameHeight}\n`,
    );
  }
  const modeDir = path.join(actualsDir, fixtureId, 'print-layout');
  const pageFiles = (await readdir(modeDir).catch(() => []))
    .filter((name) => /^page-\d+\.png$/.test(name))
    .sort((a, b) => Number(a.match(/\d+/)[0]) - Number(b.match(/\d+/)[0]));
  if (pageFiles.length === 0) continue;
  const slug = slugById.get(fixtureId);
  if (slug === undefined) {
    process.stderr.write(`[warn] ${fixtureId}: нет в корпусе — пропускаю\n`);
    continue;
  }
  const pages = {};
  for (const file of pageFiles) {
    const m = measurePage(await readFile(path.join(modeDir, file)));
    pages[file.replace('.png', '')] = m;
  }
  // Высота страницы: низ не-последних страниц виден по зазору; последняя
  // наследует размер соседней (все страницы документа одного размера, кроме
  // sections — там разнобой поймает пилот).
  let siblingHeight = null;
  for (const key of Object.keys(pages)) {
    if (pages[key].nextTop !== null) {
      siblingHeight = pages[key].firstBottom + 1;
      break;
    }
  }
  for (const key of Object.keys(pages)) {
    const p = pages[key];
    if (p.nextTop !== null) {
      p.height = p.firstBottom + 1;
    } else {
      p.height = siblingHeight;
    }
    delete p.firstBottom;
  }
  const first = pages['page-1'];
  // Кадр — размер PNG постранично (у sections страницы разного размера, а
  // у дефектов пагинации вьюера кадр может не равняться странице + 2×pad).
  geometry[`${slug}/print-layout`] = {
    frame: first.frame,
    pages,
  };
  measured += 1;
  process.stdout.write(
    `${slug}: ${Object.keys(pages).length} стр., left=${first.left}, width=${first.width}, height=${first.height ?? '?'}, кадр ${first.frame.width}×${first.frame.height}\n`,
  );
}

await mkdir(path.dirname(path.resolve(geoPath)), { recursive: true });
await writeFile(geoPath, `${JSON.stringify(geometry, null, 2)}\n`);
process.stdout.write(
  `${geoPath} обновлён: ${measured} фикстур docx, ${measuredXlsx} фикстур xlsx\n`,
);
