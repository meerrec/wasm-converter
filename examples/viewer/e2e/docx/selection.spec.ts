// Выделение текста в DOM-слое: drag через страницы, TSV таблицы и Ctrl+A.
//
// Слой виртуализирован, поэтому проверяем именно то, что видит пользователь:
// видимые построчные spans, нативное выделение и кастомное копирование.

import { expect, test } from '@playwright/test';

import { boot } from '../helpers/boot.js';
import { openFixture } from '../helpers/docx_frame.js';
import '../helpers/bridge.js';

/**
 * Позвать обработчик копирования слоя с явным `ClipboardEvent`-подобным
 * событием: системный буфер в headless-браузерах недоступен, а сериализация —
 * наша логика, её и проверяем. Firefox не заполняет `clipboardData` в
 * конструкторе, поэтому свойство ставится явно.
 */
async function copyFromLayer(page: import('@playwright/test').Page): Promise<string> {
  return page.evaluate(() => {
    const layer = document.querySelector('.dc-text-layer');
    if (!layer) throw new Error('слоя нет');
    const data = new DataTransfer();
    const event = new Event('copy', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: data });
    layer.dispatchEvent(event);
    return data.getData('text/plain');
  });
}

test.describe('DOCX selection', () => {
  test('drag через три страницы даёт текст в порядке чтения', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    await openFixture(page, 'a11y/a11y_audit.docx');
    await page.evaluate(() => {
      const bridge = window.docConverter;
      if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
      bridge.viewer.setZoom(0.2);
    });
    await page.waitForTimeout(200);

    const first = page.locator('.dc-text-page[data-page="0"] .dc-text-line').first();
    const middle = page.locator('.dc-text-page[data-page="1"] .dc-text-line').first();
    const last = page.locator('.dc-text-page[data-page="2"] .dc-text-line').last();
    await expect(first).toBeVisible();
    await expect(middle).toBeVisible();
    await expect(last).toBeVisible();
    const from = await first.boundingBox();
    const to = await last.boundingBox();
    if (!from || !to) throw new Error('нет геометрии spans');

    await page.mouse.move(from.x + 1, from.y + from.height / 2);
    await page.mouse.down();
    for (let step = 1; step <= 8; step += 1) {
      const ratio = step / 8;
      await page.mouse.move(
        from.x + 1 + (to.x + to.width - from.x - 1) * ratio,
        from.y + (to.y + to.height - from.y) * ratio,
      );
    }
    await page.mouse.up();

    const text = await page.evaluate(() => window.getSelection()?.toString() ?? '');
    expect(text.length).toBeGreaterThan(10);
    // Выделение начинается с первого символа первой страницы и доходит до
    // третьей; последняя строка может оборваться на середине глифа — это
    // нормальное поведение drag, поэтому сверяем префиксы.
    expect(text).toContain(((await first.textContent()) ?? '').slice(0, 10));
    expect(text).toContain(((await middle.textContent()) ?? '').slice(0, 10));
    expect(text).toContain(((await last.textContent()) ?? '').slice(0, 10));
  });

  test('таблица копируется как TSV', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    await openFixture(page, 'a11y/a11y_audit.docx');

    const tablePage = await page.evaluate(async () => {
      const bridge = window.docConverter;
      if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
      const full = await bridge.viewer.getFullTextLayer();
      const table = full.runs.find((run) => run.table != null);
      if (!table) throw new Error('в документе нет таблиц');
      return table.page;
    });
    await page.evaluate((pageNumber) => {
      const bridge = window.docConverter;
      if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
      bridge.viewer.showPage(pageNumber);
    }, tablePage);
    await page.waitForTimeout(300);

    const keys = await page.evaluate(async (pageNumber) => {
      const bridge = window.docConverter;
      if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
      const layer = await bridge.viewer.getTextLayer(pageNumber);
      const table = layer.runs.filter(
        (run) => run.page === pageNumber && run.table != null,
      );
      return [...new Set(table.map((run) => `${run.page}:${run.lineIndex}`))];
    }, tablePage);
    expect(keys.length).toBeGreaterThanOrEqual(4);
    await expect(page.locator(`[data-line="${keys[0]}"]`)).toHaveCount(1);
    await expect(page.locator(`[data-line="${keys[keys.length - 1]}"]`)).toHaveCount(1);

    await page.evaluate(({ start, end }) => {
      const first = document.querySelector(`[data-line="${start}"]`);
      const last = document.querySelector(`[data-line="${end}"]`);
      if (!first?.firstChild || !last?.firstChild) throw new Error('спаны таблицы не смонтированы');
      const range = document.createRange();
      range.setStart(first.firstChild, 0);
      range.setEnd(last.firstChild, (last.textContent ?? '').length);
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
    }, { start: keys[0], end: keys[keys.length - 1] });

    await page.waitForTimeout(100);
    const copied = await copyFromLayer(page);
    expect(copied).toContain('\t');
    expect(copied).toContain('\n');
    const rows = copied.split('\n');
    expect(rows.length).toBeGreaterThan(1);
    expect(rows[0]?.split('\t').length).toBeGreaterThanOrEqual(2);
  });

  test('Ctrl+A копирует весь документ, а не окно страниц', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    await openFixture(page, 'a11y/a11y_audit.docx');
    // Полный текст строится после открытия из полного TextLayer.
    await expect(page.locator('.dc-text-semantic [role="paragraph"]').first()).not.toBeEmpty();

    await page.locator('.dc-text-layer').focus();
    await page.keyboard.press('Control+A');
    const copied = await copyFromLayer(page);
    expect(copied.length).toBeGreaterThan(500);
    expect(copied).toContain('20');
  });
});
