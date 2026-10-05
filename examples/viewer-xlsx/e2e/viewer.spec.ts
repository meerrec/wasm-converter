// Сквозная проверка просмотрщика в настоящем браузере.
//
// Проверяем не «страница открылась», а что кадр действительно нарисован:
// воркер отдаёт PNG, браузер его декодирует, и по пикселям видно, что на
// холсте есть текст и сетка, а не ровная заливка.

import { expect, test, type Page } from '@playwright/test';

/** Что видно на холсте: размеры и сколько пикселей отличаются от фона. */
interface CanvasStats {
  width: number;
  height: number;
  /** Доля пикселей, отличных от самого частого цвета. */
  inkRatio: number;
  /** Сколько разных цветов встретилось. */
  colors: number;
}

declare global {
  interface Window {
    docConverter?: { viewer: { exportPng(): Promise<Uint8Array> } };
  }
}

/** Разбор картинки на стороне браузера: своего декодера PNG у теста нет. */
async function canvasStats(page: Page): Promise<CanvasStats> {
  return page.evaluate(async () => {
    const bytes = await window.docConverter!.viewer.exportPng();
    // Копия: `Blob` не принимает вид над `SharedArrayBuffer`, а воркер волен
    // отдать любой.
    const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(bitmap, 0, 0);
    const { data } = ctx.getImageData(0, 0, bitmap.width, bitmap.height);

    const counts = new Map<string, number>();
    for (let i = 0; i < data.length; i += 4) {
      const key = `${data[i]},${data[i + 1]},${data[i + 2]}`;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    const total = data.length / 4;
    const mostCommon = Math.max(...counts.values());
    return {
      width: bitmap.width,
      height: bitmap.height,
      inkRatio: 1 - mostCommon / total,
      colors: counts.size,
    };
  });
}

/** Цвет пикселя холста: проверка, что нарисовалось именно то, что ждём. */
async function pixelAt(page: Page, x: number, y: number): Promise<[number, number, number]> {
  return page.evaluate(
    async ([px, py]) => {
      const bytes = await window.docConverter!.viewer.exportPng();
      // Копия: `Blob` не принимает вид над `SharedArrayBuffer`, а воркер волен
    // отдать любой.
    const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      const d = ctx.getImageData(px!, py!, 1, 1).data;
      return [d[0]!, d[1]!, d[2]!] as [number, number, number];
    },
    [x, y],
  );
}

/** Открыть пример и дождаться, пока вьюер поднимется. */
async function boot(page: Page): Promise<string[]> {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });

  await page.goto('/');
  await page.waitForFunction(() => window.docConverter !== undefined);
  return errors;
}

test('рисует таблицу: текст, числа и сетка', async ({ page }) => {
  const errors = await boot(page);

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  // Кадр собран: команды дошли до painter'а.
  await expect(page.locator('#cmds')).not.toHaveText('0');

  const stats = await canvasStats(page);
  expect(stats.width).toBeGreaterThan(200);
  expect(stats.height).toBeGreaterThan(200);
  // Сетка и текст: пиксели отличаются от фона, и цветов заметно больше двух.
  expect(stats.inkRatio).toBeGreaterThan(0.02);
  expect(stats.colors).toBeGreaterThan(8);

  expect(errors).toEqual([]);
});

test('переключает листы и прокручивает лист', async ({ page }) => {
  const errors = await boot(page);

  await page.selectOption('#fixture', 'sheets-three.xlsx');
  await expect(page.locator('#status')).toContainText('3 лист', { timeout: 30_000 });
  await expect(page.locator('#sheet option')).toHaveCount(3);

  const first = await canvasStats(page);
  await page.selectOption('#sheet', '2');
  await expect(page.locator('#cmds')).not.toHaveText('0');

  // Разные листы — разные картинки.
  await page.waitForTimeout(200);
  const third = await canvasStats(page);
  expect(third.inkRatio).toBeGreaterThan(0);
  expect(third.inkRatio === first.inkRatio && third.colors === first.colors).toBe(false);

  expect(errors).toEqual([]);
});

test('закреплённые строки остаются на месте при прокрутке', async ({ page }) => {
  // Окно намеренно низкое: лист из десяти строк должен не помещаться целиком,
  // иначе прокручивать нечего. Ширина — с запасом, иначе панель инструментов
  // переносится на несколько строк и съедает всю высоту.
  await page.setViewportSize({ width: 900, height: 240 });
  const errors = await boot(page);

  await page.selectOption('#fixture', 'layout-frozen-rows.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  const before = await canvasStats(page);
  const scrollable = await page.evaluate(() => {
    const scroller = document.querySelector('#viewer > div') as HTMLElement;
    const canScroll = scroller.scrollHeight > scroller.clientHeight;
    scroller.scrollTop = 200;
    return { canScroll, scrollTop: scroller.scrollTop };
  });
  expect(scrollable.canScroll, 'лист должен не помещаться в окно').toBe(true);
  expect(scrollable.scrollTop).toBeGreaterThan(0);
  await expect(page.locator('#frame')).not.toHaveText('0');
  await page.waitForTimeout(300);
  const after = await canvasStats(page);

  // Картинка изменилась, но не пропала: закреплённая строка с заголовком
  // осталась, а под ней поехали данные.
  expect(after.inkRatio).toBeGreaterThan(0.005);
  expect(after.inkRatio).not.toBe(before.inkRatio);

  expect(errors).toEqual([]);
});

test('зовёт ячейку по имени под курсором', async ({ page }) => {
  const errors = await boot(page);

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  const box = await page.locator('#viewer > div').boundingBox();
  expect(box).not.toBeNull();
  // Точка внутри первой ячейки данных: заголовок столбцов 20 px, строк 44 px.
  await page.mouse.move(box!.x + 50, box!.y + 30);
  await expect(page.locator('#cell')).toHaveText('A1');

  expect(errors).toEqual([]);
});

test('без сетки и заголовков кадр проще', async ({ page }) => {
  const errors = await boot(page);

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  // Левый верхний угол занят полосой заголовков — она серого цвета.
  await expect(page.locator('#cmds')).not.toHaveText('0');
  const withHeaders = await pixelAt(page, 2, 2);
  expect(withHeaders).toEqual([245, 245, 245]);

  const withChrome = await canvasStats(page);

  await page.uncheck('#grid');
  await page.uncheck('#headers');
  await page.waitForFunction(
    (before) => document.querySelector('#cmds')?.textContent !== before,
    String(withChrome.colors),
  );
  await page.waitForTimeout(200);

  // Заголовков нет — угол стал фоном листа, и лишних цветов поубавилось.
  const bare = await pixelAt(page, 2, 2);
  expect(bare).toEqual([255, 255, 255]);

  expect(errors).toEqual([]);
});
