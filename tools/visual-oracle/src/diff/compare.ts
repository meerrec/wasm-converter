// Сравнение изображений по профилю `visual-v1`
// (`docs/strict-viewer/certification-matrix.md` §2.3).
//
// Профиль реализован буквально, потому что он — release gate, а не эвристика:
//
// 1. Размеры обязаны совпадать. Иначе `page-mismatch`, и метрики пикселей не
//    считаются вовсе: расхождение размеров нельзя «оправдать» высоким SSIM.
// 2. Вне масок сравниваются все пиксели RGBA. Цветовой допуск —
//    `max(|dR|, |dG|, |dB|) <= 2`, alpha обязана совпадать точно.
// 3. `match` требует ровно нуля расходящихся пикселей вне масок.
// 4. SSIM считается только как диагностика: он не смягчает правило нуля.
// 5. Маски исключаются из pixel diff и SSIM, но не из структурных проверок —
//    структурные проверки живут вне этого модуля и масок не знают.
//
// `diffRatio` — доля расходящихся пикселей среди сравнённых, то есть вне
// масок. Делить на всю площадь было бы нечестно: полностью замаскированный
// кадр давал бы ноль расхождений при любом содержимом.
//
// `maxChannelDelta` — максимум по всем четырём каналам, включая alpha:
// диагностика должна показывать и прозрачность, хотя вердикт по ней уже
// вынесен отдельно (alpha сравнивается точно).
//
// SSIM — глобальный по яркости, окна 8×8 без перекрытия, стандартные
// константы C1/C2. Окно, задетое маской хотя бы одним пикселем, пропускается:
// частично замаскированное окно смешало бы динамическую область со статикой.
// Остаток по краям (ширина или высота не кратна 8) в SSIM не входит.
//
// Регионы расхождения — грубая сетка 8×8 пикселей: занятые ячейки
// объединяются по четырёхсвязности, прямоугольник компоненты становится
// регионом. Компоненты с менее чем 4 расходящимися пикселями отбрасываются
// как шум, список обрезается до 50 регионов по убыванию площади — отчёт
// должен оставаться читаемым, а полный список пикселей всё равно есть в
// метриках.

import type { DiffMetrics, Rect } from '../types.js';
import type { RgbaImage } from './png.js';

/** Цветовой допуск профиля `visual-v1` по каналам R, G, B. */
export const COLOR_TOLERANCE = 2;

/** Сторона ячейки грубой сетки регионов, в пикселях. */
export const REGION_CELL = 8;

/** Минимум расходящихся пикселей в компоненте, ниже — шум. */
export const MIN_REGION_PIXELS = 4;

/** Предел числа регионов в отчёте. */
export const MAX_REGIONS = 50;

/** Размер изображения. */
export interface ImageSize {
  width: number;
  height: number;
}

/** Результат сравнения: либо размеры не сошлись, либо полный набор метрик. */
export type CompareOutcome =
  | {
      kind: 'page-mismatch';
      reason: 'dimensions';
      reference: ImageSize;
      actual: ImageSize;
    }
  | {
      kind: 'compared';
      width: number;
      height: number;
      metrics: DiffMetrics;
      regions: Rect[];
      /**
       * Точный bounding box всех расходящихся пикселей либо `null`, если их
       * нет. Регионы квантованы сеткой и обрезаны по числу, поэтому для
       * правил классификации нужен именно он.
       */
      diffBounds: Rect | null;
      /** Визуализация: эталон в градациях серого, расхождения красным, маски синим. */
      overlay: RgbaImage;
    };

/**
 * Сравнить эталон с захватом.
 *
 * `masks` — прямоугольники динамических областей из manifest; они
 * применяются одинаково к обоим изображениям и исключаются из pixel diff и
 * SSIM. Пустой список означает сравнение всей площади.
 */
export function compareImages(
  reference: RgbaImage,
  actual: RgbaImage,
  masks: readonly Rect[] = [],
): CompareOutcome {
  if (reference.width !== actual.width || reference.height !== actual.height) {
    return {
      kind: 'page-mismatch',
      reason: 'dimensions',
      reference: { width: reference.width, height: reference.height },
      actual: { width: actual.width, height: actual.height },
    };
  }

  const width = reference.width;
  const height = reference.height;
  const pixelCount = width * height;
  const mask = buildMaskBitmap(width, height, masks);
  const diffFlags = new Uint8Array(pixelCount);
  const cellColumns = Math.ceil(width / REGION_CELL);
  const cellRows = Math.ceil(height / REGION_CELL);
  const cellDiff = new Uint32Array(cellColumns * cellRows);

  let diffPixels = 0;
  let comparedPixels = 0;
  let maxChannelDelta = 0;
  let minX = width;
  let minY = height;
  let maxX = -1;
  let maxY = -1;

  for (let y = 0; y < height; y += 1) {
    const rowCell = Math.floor(y / REGION_CELL) * cellColumns;
    for (let x = 0; x < width; x += 1) {
      const pixel = y * width + x;
      if (mask !== null && mask[pixel] === 1) continue;
      comparedPixels += 1;

      const offset = pixel * 4;
      const dr = Math.abs((reference.data[offset] ?? 0) - (actual.data[offset] ?? 0));
      const dg = Math.abs((reference.data[offset + 1] ?? 0) - (actual.data[offset + 1] ?? 0));
      const db = Math.abs((reference.data[offset + 2] ?? 0) - (actual.data[offset + 2] ?? 0));
      const da = Math.abs((reference.data[offset + 3] ?? 0) - (actual.data[offset + 3] ?? 0));

      const colorDelta = Math.max(dr, dg, db);
      const channelDelta = Math.max(colorDelta, da);
      if (channelDelta > maxChannelDelta) maxChannelDelta = channelDelta;

      if (colorDelta > COLOR_TOLERANCE || da !== 0) {
        diffPixels += 1;
        diffFlags[pixel] = 1;
        const cell = rowCell + Math.floor(x / REGION_CELL);
        cellDiff[cell] = (cellDiff[cell] ?? 0) + 1;
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }

  const metrics: DiffMetrics = {
    diffPixels,
    diffRatio: comparedPixels === 0 ? 0 : diffPixels / comparedPixels,
    maxChannelDelta,
    ssim: computeSsim(reference, actual, mask, width, height),
  };

  return {
    kind: 'compared',
    width,
    height,
    metrics,
    regions: collectRegions(cellDiff, cellColumns, cellRows, width, height),
    diffBounds:
      maxX < 0
        ? null
        : { x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1 },
    overlay: paintOverlay(reference, mask, diffFlags, width, height),
  };
}

/**
 * Битовая карта масок: 1 — пиксель исключён из сравнения.
 *
 * Прямоугольники обрезаются по границам изображения: manifest проверен
 * схемой, но выходить за кадр маска всё равно не должна.
 */
function buildMaskBitmap(
  width: number,
  height: number,
  masks: readonly Rect[],
): Uint8Array | null {
  if (masks.length === 0) return null;
  const bitmap = new Uint8Array(width * height);
  for (const rect of masks) {
    const x0 = Math.max(0, Math.min(width, Math.floor(rect.x)));
    const y0 = Math.max(0, Math.min(height, Math.floor(rect.y)));
    const x1 = Math.max(x0, Math.min(width, Math.floor(rect.x + rect.width)));
    const y1 = Math.max(y0, Math.min(height, Math.floor(rect.y + rect.height)));
    for (let y = y0; y < y1; y += 1) {
      bitmap.fill(1, y * width + x0, y * width + x1);
    }
  }
  return bitmap;
}

/**
 * Глобальный SSIM по яркости.
 *
 * Возвращает `null`, если изображение меньше одного окна или все окна задеты
 * масками: подставлять 1 вместо «не измерено» значило бы врать в отчёте.
 */
function computeSsim(
  reference: RgbaImage,
  actual: RgbaImage,
  mask: Uint8Array | null,
  width: number,
  height: number,
): number | null {
  const window = REGION_CELL;
  const columns = Math.floor(width / window);
  const rows = Math.floor(height / window);
  if (columns === 0 || rows === 0) return null;

  const c1 = (0.01 * 255) ** 2;
  const c2 = (0.03 * 255) ** 2;
  let total = 0;
  let windows = 0;

  for (let row = 0; row < rows; row += 1) {
    for (let column = 0; column < columns; column += 1) {
      let sumRef = 0;
      let sumAct = 0;
      let sumRefSq = 0;
      let sumActSq = 0;
      let sumCross = 0;
      let masked = false;

      for (let y = row * window; y < (row + 1) * window && !masked; y += 1) {
        for (let x = column * window; x < (column + 1) * window; x += 1) {
          const pixel = y * width + x;
          if (mask !== null && mask[pixel] === 1) {
            masked = true;
            break;
          }
          const offset = pixel * 4;
          const lumaRef =
            0.299 * (reference.data[offset] ?? 0) +
            0.587 * (reference.data[offset + 1] ?? 0) +
            0.114 * (reference.data[offset + 2] ?? 0);
          const lumaAct =
            0.299 * (actual.data[offset] ?? 0) +
            0.587 * (actual.data[offset + 1] ?? 0) +
            0.114 * (actual.data[offset + 2] ?? 0);
          sumRef += lumaRef;
          sumAct += lumaAct;
          sumRefSq += lumaRef * lumaRef;
          sumActSq += lumaAct * lumaAct;
          sumCross += lumaRef * lumaAct;
        }
      }
      if (masked) continue;

      const count = window * window;
      const meanRef = sumRef / count;
      const meanAct = sumAct / count;
      const varianceRef = sumRefSq / count - meanRef * meanRef;
      const varianceAct = sumActSq / count - meanAct * meanAct;
      const covariance = sumCross / count - meanRef * meanAct;
      const numerator = (2 * meanRef * meanAct + c1) * (2 * covariance + c2);
      const denominator =
        (meanRef * meanRef + meanAct * meanAct + c1) * (varianceRef + varianceAct + c2);
      total += denominator === 0 ? 1 : numerator / denominator;
      windows += 1;
    }
  }

  if (windows === 0) return null;
  // Шесть знаков: отчёт должен быть воспроизводимым побайтово.
  return Math.round((total / windows) * 1e6) / 1e6;
}

/**
 * Собрать регионы расхождения из сетки занятых ячеек.
 *
 * Компоненты обходятся в порядке строк, поэтому результат детерминирован;
 * сортировка — по площади, затем по координатам.
 */
function collectRegions(
  cellDiff: Uint32Array,
  cellColumns: number,
  cellRows: number,
  width: number,
  height: number,
): Rect[] {
  const visited = new Uint8Array(cellColumns * cellRows);
  const regions: { rect: Rect; pixels: number }[] = [];
  const stack: number[] = [];

  for (let start = 0; start < cellDiff.length; start += 1) {
    if ((cellDiff[start] ?? 0) === 0 || visited[start] === 1) continue;
    visited[start] = 1;
    stack.push(start);

    let minColumn = cellColumns;
    let maxColumn = -1;
    let minRow = cellRows;
    let maxRow = -1;
    let pixels = 0;

    while (stack.length > 0) {
      const cell = stack.pop() ?? 0;
      const column = cell % cellColumns;
      const row = Math.floor(cell / cellColumns);
      pixels += cellDiff[cell] ?? 0;
      if (column < minColumn) minColumn = column;
      if (column > maxColumn) maxColumn = column;
      if (row < minRow) minRow = row;
      if (row > maxRow) maxRow = row;

      const neighbours = [
        column > 0 ? cell - 1 : -1,
        column + 1 < cellColumns ? cell + 1 : -1,
        row > 0 ? cell - cellColumns : -1,
        row + 1 < cellRows ? cell + cellColumns : -1,
      ];
      for (const neighbour of neighbours) {
        if (neighbour < 0) continue;
        if ((cellDiff[neighbour] ?? 0) === 0 || visited[neighbour] === 1) continue;
        visited[neighbour] = 1;
        stack.push(neighbour);
      }
    }

    if (pixels < MIN_REGION_PIXELS) continue;
    const x = minColumn * REGION_CELL;
    const y = minRow * REGION_CELL;
    regions.push({
      rect: {
        x,
        y,
        width: Math.min(width, (maxColumn + 1) * REGION_CELL) - x,
        height: Math.min(height, (maxRow + 1) * REGION_CELL) - y,
      },
      pixels,
    });
  }

  regions.sort(
    (a, b) =>
      b.rect.width * b.rect.height - a.rect.width * a.rect.height ||
      a.rect.y - b.rect.y ||
      a.rect.x - b.rect.x,
  );
  return regions.slice(0, MAX_REGIONS).map((region) => region.rect);
}

/**
 * Нарисовать диагностическую картинку.
 *
 * Эталон обесцвечивается, расходящиеся пиксели заливаются красным, маски —
 * синим вполсилы. Так оператор видит и место расхождения, и то, что было
 * исключено из сравнения.
 */
function paintOverlay(
  reference: RgbaImage,
  mask: Uint8Array | null,
  diffFlags: Uint8Array,
  width: number,
  height: number,
): RgbaImage {
  const data = new Uint8Array(width * height * 4);
  for (let pixel = 0; pixel < width * height; pixel += 1) {
    const offset = pixel * 4;
    const luma = Math.round(
      0.299 * (reference.data[offset] ?? 0) +
        0.587 * (reference.data[offset + 1] ?? 0) +
        0.114 * (reference.data[offset + 2] ?? 0),
    );
    let red = luma;
    let green = luma;
    let blue = luma;
    if (mask !== null && mask[pixel] === 1) {
      red = Math.round(luma * 0.5);
      green = Math.round(luma * 0.5);
      blue = Math.round(127.5 + luma * 0.5);
    } else if (diffFlags[pixel] === 1) {
      red = 255;
      green = 0;
      blue = 0;
    }
    data[offset] = red;
    data[offset + 1] = green;
    data[offset + 2] = blue;
    data[offset + 3] = 255;
  }
  return { width, height, data };
}
