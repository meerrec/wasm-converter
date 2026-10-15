// Сквозная проверка страницы «Конвертер» — витрины `@meerrec/browser-convert`.
//
// Кнопка «Конвертировать» здесь не нажимается сознательно: рантайм x2t — это
// Git LFS (около 39 МБ), в CI-прогоне e2e артефакты LFS не выкачиваются
// (`copy-x2t` пропускает их с предупреждением), и тянуть их на каждый прогон
// незачем. Сама конвертация покрыта отдельно — харнессом `experiments/x2t-spike`
// и ручным прогоном координатора, — а спека держит то, что живёт без x2t:
// маппинг целей по семейству, медиа-экстракцию и препроцессоры.

import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';

/** Корень фикстур: от каталога спеки до корня монорепозитория четыре уровня. */
const FIXTURES = path.resolve(import.meta.dirname, '../../../../test-fixtures');

/** DOCX с одной картинкой в пакете: `word/media/tight.png`. */
const DOCX_WITH_MEDIA = path.join(FIXTURES, 'docx/images/anchor_wrap_tight.docx');

/** DOCX без ruby — препроцессор обязан вернуть исходные байты. */
const DOCX_PLAIN = path.join(FIXTURES, 'docx/simple/one_paragraph.docx');

/** Обычная книга без медиа: и «без изменений» у препроцессора, и пустая галерея. */
const XLSX_PLAIN = path.join(FIXTURES, 'xlsx/content-single-cell.xlsx');

/**
 * Открыть страницу «Конвертер»: как `boot` у вьюеров, но своего моста
 * `window.docConverter` у неё нет, поэтому вход — просто маршрут по хэшу,
 * а ошибки консоли собираются те же.
 */
async function openConvert(page: Page): Promise<string[]> {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.goto('/#/convert');
  await expect(page.locator('h1')).toHaveText('Конвертер');
  return errors;
}

test('открывается по /#/convert: заголовок, ссылки и пустое состояние', async ({ page }) => {
  const errors = await openConvert(page);

  // Переключателя страниц у примера нет, ссылки на соседей — единственная навигация.
  await expect(page.getByRole('link', { name: 'Слово' })).toHaveAttribute('href', '#/docx');
  await expect(page.getByRole('link', { name: 'ДваИксель' })).toHaveAttribute('href', '#/xlsx');

  // Без файла конвертировать нечего: select пуст, кнопки заблокированы.
  await expect(page.locator('#convert-target')).toBeDisabled();
  await expect(page.locator('#convert-target option')).toHaveText(['—']);
  await expect(page.locator('#convert-run')).toBeDisabled();
  await expect(page.locator('#media-show')).toBeDisabled();
  // Препроцессор выбирается типом файла: до выбора кнопок нет вовсе.
  await expect(page.locator('#preprocess-docx')).toHaveCount(0);

  expect(errors).toEqual([]);
});

test('строит цели по семейству файла', async ({ page }) => {
  const errors = await openConvert(page);

  await page.setInputFiles('#convert-file', DOCX_WITH_MEDIA);
  // Word: все форматы семейства, кроме исходного, и первая цель выбрана сразу.
  await expect(page.locator('#convert-target option')).toHaveText(['odt', 'rtf', 'txt']);
  await expect(page.locator('#convert-target')).toHaveValue('odt');
  await expect(page.locator('#convert-run')).toBeEnabled();
  await expect(page.locator('main .conv-note').first()).toContainText('семейство Word');

  await page.setInputFiles('#convert-file', XLSX_PLAIN);
  // Excel: свой набор, и прежние цели не остаются.
  await expect(page.locator('#convert-target option')).toHaveText(['ods', 'csv']);
  await expect(page.locator('#convert-target')).toHaveValue('ods');
  await expect(page.locator('main .conv-note').first()).toContainText('семейство Excel');

  expect(errors).toEqual([]);
});

test('извлекает медиа из docx', async ({ page }) => {
  const errors = await openConvert(page);

  await page.setInputFiles('#convert-file', DOCX_WITH_MEDIA);
  // До извлечения нет ни галереи, ни сообщения о пустоте: секция молчит.
  await expect(page.locator('#media-gallery')).toHaveCount(0);
  await expect(page.locator('#media-empty')).toHaveCount(0);

  await page.click('#media-show');
  // Картинка одна: путь внутри пакета становится alt и именем, mime — <img>.
  const items = page.locator('#media-gallery > li');
  await expect(items).toHaveCount(1);
  const image = items.locator('img');
  await expect(image).toHaveAttribute('alt', 'media/tight.png');
  await expect(image).toHaveAttribute('src', /^blob:/);
  await expect(image).toBeVisible();
  await expect(page.locator('#media-empty')).toHaveCount(0);

  expect(errors).toEqual([]);
});

test('препроцессоры на чистых файлах не меняют байты', async ({ page }) => {
  const errors = await openConvert(page);

  // XLSX: кнопка одна, и переносы чинить нечего.
  await page.setInputFiles('#convert-file', XLSX_PLAIN);
  await expect(page.locator('#preprocess-xlsx')).toBeVisible();
  await expect(page.locator('#preprocess-docx')).toHaveCount(0);
  await page.click('#preprocess-xlsx');
  await expect(page.locator('#preprocess-status')).toHaveText('без изменений');
  await expect(page.locator('#preprocess-download')).toBeVisible();

  // DOCX: смена файла сбрасывает прошлый результат, ruby в документе нет.
  await page.setInputFiles('#convert-file', DOCX_PLAIN);
  await expect(page.locator('#preprocess-docx')).toBeVisible();
  await page.click('#preprocess-docx');
  await expect(page.locator('#preprocess-status')).toHaveText('без изменений');

  expect(errors).toEqual([]);
});

test('сообщает, что медиа в файле нет', async ({ page }) => {
  const errors = await openConvert(page);

  await page.setInputFiles('#convert-file', XLSX_PLAIN);
  await page.click('#media-show');
  await expect(page.locator('#media-empty')).toBeVisible();
  await expect(page.locator('#media-gallery')).toHaveCount(0);

  expect(errors).toEqual([]);
});
