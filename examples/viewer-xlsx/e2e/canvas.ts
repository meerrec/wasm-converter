// Замеры по кадру холста: своего декодера PNG в Node нет, поэтому картинку
// разбирает сама страница через `ImageBitmap`, а тест получает числа.

import type { Page } from '@playwright/test';

/** API вьюера, вывешенное примером на `window` (см. `src/main.ts`). */
interface ViewerBridge {
  exportPng(): Promise<Uint8Array>;
}

/** Что видно на холсте: размеры, «чернила» и число цветов. */
export interface FrameStats {
  width: number;
  height: number;
  /** Доля пикселей, отличных от самого частого цвета. */
  inkRatio: number;
  /** Сколько разных цветов встретилось. */
  colors: number;
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

/** Разобрать кадр: размеры, «чернила» и цвета. */
export async function frameStats(page: Page): Promise<FrameStats> {
  return page.evaluate(async () => {
    const bridge = (window as unknown as { docConverter?: { viewer?: ViewerBridge } }).docConverter
      ?.viewer;
    if (!bridge) throw new Error('вьюер не поднялся');
    const bytes = await bridge.exportPng();
    // Копия: `Blob` не принимает вид над `SharedArrayBuffer`, а воркер волен
    // отдать любой.
    const bitmap = await createImageBitmap(
      new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
    );
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(bitmap, 0, 0);
    const { data } = ctx.getImageData(0, 0, bitmap.width, bitmap.height);

    const counts = new Map<string, number>();
    for (let i = 0; i < data.length; i += 4) {
      const key = `${data[i]},${data[i + 1]},${data[i + 2]}`;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    const mostCommon = Math.max(...counts.values());
    return {
      width: bitmap.width,
      height: bitmap.height,
      inkRatio: 1 - mostCommon / (data.length / 4),
      colors: counts.size,
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
      const bridge = (window as unknown as { docConverter?: { viewer?: ViewerBridge } }).docConverter
        ?.viewer;
      if (!bridge) throw new Error('вьюер не поднялся');
      const bytes = await bridge.exportPng();
      const bitmap = await createImageBitmap(
        new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
      );
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d')!;
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
