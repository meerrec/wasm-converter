// Замеры по кадру холста: своего декодера PNG в Node нет, поэтому картинку
// разбирает сама страница через `ImageBitmap`, а тест получает числа.
//
// У DOCX холст залит не весь: белый лист лежит на прозрачном фоне, и «самый
// частый цвет» тут ничего не значит — у прозрачных пикселей каналы тоже нули.
// Поэтому фон, поле листа и чернила различаются по альфе и яркости.

import type { DocxViewerHandle } from '@doc-converter/core';
import type { Page } from '@playwright/test';

declare global {
  interface Window {
    docConverter?: { viewer: DocxViewerHandle };
  }
}

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
  /** Доля тёмных среди непрозрачных — так выглядит текст. */
  inkRatio: number;
  /** Габариты непрозрачной области: лист целиком, даже если он обрезан окном. */
  pageBox: Box | null;
  /** Габариты тёмных пикселей: где именно на листе есть текст. */
  inkBox: Box | null;
  /** Отпечаток кадра: у двух разных картинок он разный. */
  signature: string;
}

/**
 * Разобрать текущий кадр вьюера.
 *
 * Доли считаются среди непрозрачных пикселей: у прозрачного фона RGB — нули, и
 * в общей статистике он выглядел бы чёрным.
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
      // Отпечаток считается по всему кадру: у прозрачного фона свой цвет.
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
    };
  });
}

/** Номер текущего кадра из строки состояния: по нему видно, что painter отработал. */
export async function frameId(page: Page): Promise<number> {
  return page.evaluate(() => Number(document.querySelector('#frame')?.textContent ?? '0'));
}

/** Дождаться кадра с номером больше `from`: столько painter'у нужно на перерисовку. */
export async function waitForFrame(page: Page, from: number): Promise<number> {
  await page.waitForFunction(
    (previous) => Number(document.querySelector('#frame')?.textContent ?? '0') > previous,
    from,
    { timeout: 30_000 },
  );
  return frameId(page);
}

/** Открыть пример и дождаться, пока вьюер поднимется. */
export async function boot(page: Page): Promise<string[]> {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });

  await page.goto('/');
  await page.waitForFunction(() => window.docConverter !== undefined);
  return errors;
}

/**
 * Открыть фикстуру и дождаться первого нарисованного кадра.
 *
 * Ждать одну строку состояния мало: `open` асинхронен, а до его ответа холст
 * ещё пуст. Поэтому сначала дожидаемся раскладки (число страниц), потом —
 * картинки на холсте.
 */
export async function openFixture(page: Page, name: string): Promise<FrameStats> {
  await page.selectOption('#fixture', name);
  await page.waitForFunction(
    () => (window.docConverter?.viewer.pageCount ?? 0) > 0,
    undefined,
    { timeout: 30_000 },
  );

  const deadline = Date.now() + 30_000;
  for (;;) {
    const stats = await frameStats(page);
    if (stats.inkBox) return stats;
    if (Date.now() > deadline) throw new Error(`страница ${name} осталась пустой`);
    await page.waitForTimeout(100);
  }
}

/** Число страниц в открытом документе. */
export async function pageCount(page: Page): Promise<number> {
  return page.evaluate(() => window.docConverter?.viewer.pageCount ?? 0);
}
