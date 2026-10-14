// Лёгкая проверка маршрута «Euro-Office»: страница-оболочка вокруг iframe.
//
// Хост сайта редактора здесь намеренно не поднимается — его проверяет отдельный
// прогон со сборкой сайта (566 МБ), а этот держит то, что живёт без него: маршрут,
// адрес iframe из `.env.development`, начальный `loading`, хук приёмки и подсказку
// про неподнятый хост. Ошибки консоли не собираются: незагрузившийся
// кросс-доменный iframe порождает их штатно.

import { expect, test, type Page } from '@playwright/test';

/** Адрес редактора: значение `VITE_EUROOFFICE_BASE` из `examples/viewer/.env.development`. */
const EDITOR_URL = 'http://127.0.0.1:5271/editor.html?embed=1';

/**
 * Подсказка про неподнятый хост появляется через 15 с (константа страницы);
 * ждём с запасом, но не настолько, чтобы прогон упёрся в таймаут теста.
 */
const HOST_HINT_TIMEOUT_MS = 25_000;

/**
 * Обрезать запросы к хосту редактора: отсутствие хоста — условие этой спеки, и
 * держать его должен тест, а не машина. У разработчика хост как раз поднят
 * (`node scripts/eurooffice/serve.mjs`, порт 5271) — тогда iframe загрузился бы
 * и прислал `document:ready` задолго до истечения подсказки. Мегабайты редактора
 * при этом не тянутся: запрос обрывается до сети.
 */
async function withoutEditorHost(page: Page): Promise<void> {
  await page.route(`${new URL(EDITOR_URL).origin}/**`, (route) => route.abort());
}

test('открывается по /#/eurooffice и указывает iframe на хост редактора', async ({ page }) => {
  await withoutEditorHost(page);
  await page.goto('/#/eurooffice');

  // Контракт приёмки: id элементов оболочки и хук window.euroofficeViewer.
  await expect(page.locator('#viewer-frame')).toHaveCount(1);
  await expect(page.locator('#viewer-status')).toHaveCount(1);
  await expect(page.locator('#viewer-file')).toHaveCount(1);
  await expect(page.locator('#viewer-open')).toHaveCount(1);
  await expect(page.locator('#viewer-doc-name')).toHaveCount(1);
  await expect(page.locator('#viewer-drop-overlay')).toHaveCount(1);

  const frame = page.locator('#viewer-frame');
  await expect(frame).toHaveAttribute('src', EDITOR_URL);
  await expect(frame).toHaveAttribute('title', 'Редактор Euro-Office');
  await expect(page.getByRole('button', { name: 'Открыть документ' })).toBeVisible();

  // Ждём не только разметку, но и эффект, который публикует хук приёмки.
  const harness = await page.waitForFunction(() => window.euroofficeViewer ?? null);
  expect(await harness.jsonValue()).toEqual({
    state: 'loading',
    openedCount: 0,
    lastFileName: '',
    lastError: null,
  });
  // Начальное состояние статуса — то же, что в хуке: до `document:ready` идёт ожидание.
  await expect(page.locator('#viewer-status')).toHaveAttribute('data-state', 'loading');
});

test('подсказывает про неподнятый хост редактора, оставаясь в loading', async ({ page }) => {
  await withoutEditorHost(page);
  await page.goto('/#/eurooffice');
  const status = page.locator('#viewer-status');
  await expect(status).toHaveAttribute('data-state', 'loading');
  // Хоста нет — `document:ready` не придёт, и без подсказки страница висела бы молча.
  await expect(status).toContainText('serve.mjs', { timeout: HOST_HINT_TIMEOUT_MS });
  // Подсказка меняет только текст: значений `data-state` она не добавляет.
  await expect(status).toHaveAttribute('data-state', 'loading');
});
