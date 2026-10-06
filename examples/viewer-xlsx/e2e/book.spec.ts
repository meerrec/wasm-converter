// Экспорт всей книги: кнопка «Экспорт книги» кладёт в один PDF все листы
// `sheets-three.xlsx` и добавляет закладки (`/Outlines`, `bookmarks: true`
// по умолчанию) — по одной на лист.
//
// Одиночная выгрузка скачивается рядом и служит точкой сравнения: проверка
// «страниц больше, чем у листа» осмысленна только относительно неё.

import { expect, test } from '@playwright/test';
import { boot } from './canvas.js';
import { downloadPdf } from './download.js';
import { probePdf } from './pdf-probe.js';

test('экспортирует всю книгу с закладками', async ({ page }) => {
  const errors = await boot(page);

  await page.selectOption('#fixture', 'sheets-three.xlsx');
  await expect(page.locator('#status')).toContainText('3 лист', { timeout: 30_000 });

  const sheet = await downloadPdf(page, '#export-pdf');
  expect(sheet.name).toBe('sheets-three.pdf');
  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });

  const book = await downloadPdf(page, '#export-book-pdf');
  // Подпись «(все листы)» отличает книгу от одиночного листа ещё в имени файла.
  expect(book.name).toBe('sheets-three (все листы).pdf');
  expect(book.name).not.toBe(sheet.name);

  const sheetProbe = probePdf(sheet.bytes);
  const bookProbe = probePdf(book.bytes);
  // В `sheets-three.xlsx` три листа, каждый умещается на страницу. Числа
  // конкретные: один лист — одна страница, книга — три, и заодно книга
  // длиннее одиночной выгрузки.
  expect(sheetProbe.pageCount, 'страниц в выгрузке листа').toBe(1);
  expect(bookProbe.pageCount, 'страниц в выгрузке книги').toBe(3);
  expect(bookProbe.pageCount).toBeGreaterThan(sheetProbe.pageCount);
  // Закладки книги printpdf пишет деревом `/Outlines` в каталог документа.
  expect(bookProbe.text).toContain('/Outlines');

  expect(errors).toEqual([]);
});
