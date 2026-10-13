// Стопка страниц в кадре: на стыке двух страниц холст не пустует, а стопка у́же
// области прокрутки стоит по центру.
//
// Оба свойства видны только на кадре целиком, поэтому меряются по PNG
// (`exportPng`), а не по разметке: центрирование — это геометрия того, что
// нарисовано, а не CSS.

import { expect, test, type Page } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameId, openFixture, pageCount, waitForFrame } from '../helpers/docx_frame.js';
import { frameStats, type FrameStats } from '../helpers/png.js';

/**
 * Доли непрозрачных пикселей в верхней и нижней восьмушках кадра.
 *
 * Лист непрозрачен целиком (белая заливка страницы), поэтому доля и говорит,
 * где стопка: полоса, целиком лежащая на листе, покрыта как минимум на ширину
 * листа, а пустая — нулевая.
 */
async function bandCoverage(page: Page): Promise<{ top: number; bottom: number }> {
  return page.evaluate(async () => {
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

    const band = Math.max(1, Math.floor(bitmap.height / 8));
    let top = 0;
    let bottom = 0;
    for (let y = 0; y < bitmap.height; y += 1) {
      for (let x = 0; x < bitmap.width; x += 1) {
        if (data[(y * bitmap.width + x) * 4 + 3] === 0) continue;
        if (y < band) top += 1;
        else if (y >= bitmap.height - band) bottom += 1;
      }
    }
    const perBand = bitmap.width * band;
    return { top: top / perBand, bottom: bottom / perBand };
  });
}

/** Ждать кадр, на котором выполняется условие: раскладка после открытия устаканивается. */
async function waitForStats(
  page: Page,
  ready: (stats: FrameStats) => boolean,
  timeoutMs = 20_000,
): Promise<FrameStats> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const stats = await frameStats(page);
    if (ready(stats) || Date.now() > deadline) return stats;
    await page.waitForTimeout(100);
  }
}

test('на стыке двух страниц холст не пустует', async ({ page }) => {
  const errors = await boot(page, '/docx');

  await openFixture(page, 'tables/table_100_pages.docx');
  expect(await pageCount(page)).toBeGreaterThan(1);

  // Стык первой и второй страниц — в середине окна: полторы сотни пикселей
  // раскладки первой страницы сверху, остальное — вторая.
  const before = await frameId(page);
  await page.evaluate(() => {
    const bridge = window.docConverter;
    if (bridge?.kind !== 'docx') throw new Error('вьюер DOCX не поднялся');
    const first = bridge.viewer.pages[0];
    if (!first) throw new Error('в документе нет страниц');
    bridge.viewer.scrollTo(0, first.height - 150);
  });
  await waitForFrame(page, before);

  const stats = await frameStats(page);
  const bands = await bandCoverage(page);
  expect(stats.pageBox).not.toBeNull();
  // Ширина листа в кадре — потолок для полосы: холст шире листа (лист по центру).
  const sheetShare = (stats.pageBox?.width ?? 0) / stats.width;
  expect(sheetShare).toBeGreaterThan(0.2);

  // Обе полосы накрыты листом: верх — конец первой страницы, низ — вторая.
  // Кадр из одной страницы оставил бы низ пустым (там её содержимое кончилось).
  const shares = `лист занимает ${sheetShare.toFixed(3)} кадра, полосы ${bands.top.toFixed(3)} и ${bands.bottom.toFixed(3)}`;
  expect(bands.top, `верх кадра за стыком: ${shares}`).toBeGreaterThan(sheetShare * 0.8);
  expect(bands.bottom, `низ кадра за стыком: ${shares}`).toBeGreaterThan(sheetShare * 0.8);

  expect(errors).toEqual([]);
});

test('страница у́же окна стоит по центру холста', async ({ page }) => {
  const errors = await boot(page, '/docx');

  await openFixture(page, 'simple/one_paragraph.docx');

  // Дальше ждём кадр, на котором лист уже встал по центру: после открытия высота
  // распорки добавляет вертикальную полосу прокрутки, ширина окна от неё
  // меняется, и поля пересчитываются.
  const centered = (stats: FrameStats): boolean => {
    const box = stats.pageBox;
    if (!box) return false;
    return Math.abs(box.x + box.width / 2 - stats.width / 2) <= 4;
  };
  const stats = await waitForStats(page, centered);
  const box = stats.pageBox;
  expect(box).not.toBeNull();

  // Лист должен быть у́же холста: шире — центрировать нечего, и проверка
  // выполнилась бы сама собой.
  expect(box!.width).toBeLessThan(stats.width * 0.9);

  // Поля слева и справа от листа одинаковые — с точностью до пикселя округления
  // раскладки и прокрутки.
  const left = box!.x;
  const right = stats.width - (box!.x + box!.width);
  expect(
    Math.abs(left - right),
    `лист ${left}…${left + box!.width} в кадре ${stats.width}: поле слева ${left}, справа ${right}`,
  ).toBeLessThanOrEqual(4);

  expect(errors).toEqual([]);
});
