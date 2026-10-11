// Замеры по кадру холста: своего декодера PNG в Node нет, поэтому картинку
// разбирает сама страница через `ImageBitmap`, а тест получает числа.
//
// Один декодер на оба вида, потому что устроен он одинаково: воркер отдаёт
// PNG, браузер его декодирует, а наружу уходит маленький объект — не RGBA.
// Различаются лишь смыслы долей, и оба смысла тут сохранены дословно:
//
// - `inkRatio` — смысл DOCX: доля тёмных среди непрозрачных. У DOCX холст залит
//   не весь: белый лист лежит на прозрачном фоне, и «самый частый цвет» тут
//   ничего не значит — у прозрачных пикселей каналы тоже нули. Поэтому фон,
//   поле листа и чернила различаются по альфе и яркости.
// - `diffRatio` — смысл XLSX: 1 минус доля самого частого цвета. У XLSX холст
//   залит целиком (это и есть «фон»), и «чернила» — всё, что от него
//   отличается. Раньше в XLSX-спеках это поле называлось `inkRatio`; имя
//   заменено, чтобы две несовместимые доли не жили под одним словом.
//
// `colors`, `signature` и габариты `pageBox`/`inkBox` считаются в том же
// проходе: второй декод того же кадра стоил бы ещё одного `exportPng`.

import type { Page } from '@playwright/test';

/** Габариты области в пикселях кадра. */
export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Что видно на холсте: размеры, доли площадей и габариты листа и текста. */
export interface FrameStats {
  width: number;
  height: number;
  /** Доля непрозрачных пикселей — столько кадра занял лист. */
  pageRatio: number;
  /** Доля светлых среди непрозрачных — так выглядит белое поле листа. */
  lightRatio: number;
  /** Доля тёмных среди непрозрачных — так выглядит текст (смысл DOCX). */
  inkRatio: number;
  /** Габариты непрозрачной области: лист целиком, даже если он обрезан окном. */
  pageBox: Box | null;
  /** Габариты тёмных пикселей: где именно на листе есть текст. */
  inkBox: Box | null;
  /** Отпечаток кадра: у двух разных картинок он разный. */
  signature: string;
  /** Сколько разных цветов встретилось. */
  colors: number;
  /** Доля пикселей, отличных от самого частого цвета (смысл XLSX). */
  diffRatio: number;
}

/**
 * Разобрать текущий кадр вьюера.
 *
 * Доли DOCX считаются среди непрозрачных пикселей: у прозрачного фона RGB —
 * нули, и в общей статистике он выглядел бы чёрным.
 */
export async function frameStats(page: Page): Promise<FrameStats> {
  return page.evaluate(async () => {
    const viewer = window.docConverter?.viewer;
    if (!viewer) throw new Error('вьюер не поднялся');
    const bytes = await viewer.exportPng();
    // Копия: `Blob` не принимает вид над `SharedArrayBuffer`, а воркер волен
    // отдать любой.
    const bitmap = await createImageBitmap(
      new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
    );
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('нет 2d-контекста');
    ctx.drawImage(bitmap, 0, 0);
    const { data } = ctx.getImageData(0, 0, bitmap.width, bitmap.height);

    const counts = new Map<string, number>();
    let opaque = 0;
    let light = 0;
    let ink = 0;
    let hash = 0x811c9dc5;
    let pageMinX = bitmap.width;
    let pageMinY = bitmap.height;
    let pageMaxX = -1;
    let pageMaxY = -1;
    let inkMinX = bitmap.width;
    let inkMinY = bitmap.height;
    let inkMaxX = -1;
    let inkMaxY = -1;

    for (let i = 0; i < data.length; i += 4) {
      // Цвета и отпечаток считаются по всему кадру: у прозрачного фона свой
      // цвет, и для `diffRatio` он такой же участник, как остальные.
      const key = `${data[i]},${data[i + 1]},${data[i + 2]}`;
      counts.set(key, (counts.get(key) ?? 0) + 1);

      hash = (hash ^ data[i]!) >>> 0;
      hash = Math.imul(hash, 0x01000193) >>> 0;
      hash = (hash ^ data[i + 1]!) >>> 0;
      hash = Math.imul(hash, 0x01000193) >>> 0;
      hash = (hash ^ data[i + 2]!) >>> 0;
      hash = Math.imul(hash, 0x01000193) >>> 0;
      hash = (hash ^ data[i + 3]!) >>> 0;
      hash = Math.imul(hash, 0x01000193) >>> 0;

      if (data[i + 3] === 0) continue;
      opaque += 1;
      const x = (i / 4) % bitmap.width;
      const y = Math.floor(i / 4 / bitmap.width);
      if (x < pageMinX) pageMinX = x;
      if (y < pageMinY) pageMinY = y;
      if (x > pageMaxX) pageMaxX = x;
      if (y > pageMaxY) pageMaxY = y;

      const luma = (data[i]! + data[i + 1]! + data[i + 2]!) / 3;
      if (luma > 200) light += 1;
      if (luma < 160) {
        ink += 1;
        if (x < inkMinX) inkMinX = x;
        if (y < inkMinY) inkMinY = y;
        if (x > inkMaxX) inkMaxX = x;
        if (y > inkMaxY) inkMaxY = y;
      }
    }

    const total = data.length / 4;
    const mostCommon = Math.max(...counts.values());
    const box = (minX: number, minY: number, maxX: number, maxY: number) =>
      maxX < 0 ? null : { x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1 };

    return {
      width: bitmap.width,
      height: bitmap.height,
      pageRatio: opaque / total,
      lightRatio: opaque === 0 ? 0 : light / opaque,
      inkRatio: opaque === 0 ? 0 : ink / opaque,
      pageBox: box(pageMinX, pageMinY, pageMaxX, pageMaxY),
      inkBox: box(inkMinX, inkMinY, inkMaxX, inkMaxY),
      signature: hash.toString(16),
      colors: counts.size,
      diffRatio: 1 - mostCommon / total,
    };
  });
}

/**
 * Доля пикселей кадра, близких к цвету: заливки диаграмм и картинок в глубине
 * областей не сглаживаются, поэтому допуск нужен только на края.
 */
export async function colorShare(
  page: Page,
  rgb: readonly [number, number, number],
  tolerance = 8,
): Promise<number> {
  return page.evaluate(
    async ([r, g, b, tol]) => {
      const viewer = window.docConverter?.viewer;
      if (!viewer) throw new Error('вьюер не поднялся');
      const bytes = await viewer.exportPng();
      const bitmap = await createImageBitmap(
        new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
      );
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d');
      if (!ctx) throw new Error('нет 2d-контекста');
      ctx.drawImage(bitmap, 0, 0);
      const { data } = ctx.getImageData(0, 0, bitmap.width, bitmap.height);

      let hits = 0;
      for (let i = 0; i < data.length; i += 4) {
        if (
          Math.abs(data[i]! - r!) <= tol! &&
          Math.abs(data[i + 1]! - g!) <= tol! &&
          Math.abs(data[i + 2]! - b!) <= tol!
        ) {
          hits += 1;
        }
      }
      return hits / (data.length / 4);
    },
    [rgb[0], rgb[1], rgb[2], tolerance],
  );
}

/** Цвет пикселя холста: проверка, что нарисовалось именно то, что ждём. */
export async function pixelAt(page: Page, x: number, y: number): Promise<[number, number, number]> {
  return page.evaluate(
    async ([px, py]) => {
      const viewer = window.docConverter?.viewer;
      if (!viewer) throw new Error('вьюер не поднялся');
      const bytes = await viewer.exportPng();
      // Копия: `Blob` не принимает вид над `SharedArrayBuffer`, а воркер волен
      // отдать любой.
      const bitmap = await createImageBitmap(
        new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
      );
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d');
      if (!ctx) throw new Error('нет 2d-контекста');
      ctx.drawImage(bitmap, 0, 0);
      const d = ctx.getImageData(px!, py!, 1, 1).data;
      return [d[0]!, d[1]!, d[2]!] as [number, number, number];
    },
    [x, y],
  );
}
