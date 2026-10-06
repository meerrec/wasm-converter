// Ленивость PDF-модуля: до первого клика по кнопке экспорта воркер
// просмотрщика не должен тянуть ни JS-обёртку `@doc-converter/wasm-pdf`, ни
// её бинарь, а повторный экспорт обязан обойтись уже собранным инстансом —
// без второго запроса модуля.
//
// Модуль запрашивает сам воркер, поэтому сеть слушаем на уровне страницы:
// Playwright относит запросы dedicated worker'а к странице-владельцу.

import { readFile } from 'node:fs/promises';
import { expect, test, type Download, type Page, type Request } from '@playwright/test';

/** Имена файлов модуля PDF: бинарь и JS-обёртка из `wasm-pack --target web`. */
const PDF_WASM = 'doc_converter_pdf_wasm_bg.wasm';
const PDF_GLUE = 'doc_converter_pdf_wasm.js';

/**
 * Служебная заглушка дев-сервера: `main.ts` берёт адреса модуля и бинаря
 * через `?url`, и Vite отдаёт на такой импорт крохотный модуль со строкой
 * URL. Запрос за ним уходит ещё при загрузке страницы, но байтов модуля не
 * несёт: сам модуль воркер грузит по адресу без `?url`.
 */
function isDevUrlStub(request: Request): boolean {
  return new URL(request.url()).searchParams.has('url');
}

/** Загрузка файла модуля PDF — в отличие от заглушки `?url` выше. */
function isPdfLoad(request: Request, name: string): boolean {
  return request.url().includes(name) && !isDevUrlStub(request);
}

/** URL-ы загрузок файла модуля PDF: сообщение об ошибке покажет, что пришло. */
function pdfLoadUrls(requests: Request[], name: string): string[] {
  return requests.filter((request) => isPdfLoad(request, name)).map((request) => request.url());
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

/** Скачанный файл целиком: это должен быть непустой PDF. */
async function readPdf(download: Download): Promise<Buffer> {
  const path = await download.path();
  expect(path).not.toBeNull();
  const bytes = await readFile(path!);

  expect(bytes.byteLength).toBeGreaterThan(500);
  expect(bytes.subarray(0, 5).toString('latin1')).toBe('%PDF-');
  return bytes;
}

test('грузит PDF-модуль по клику и переиспользует его', async ({ page }) => {
  // Подписка до навигации: в список попадёт и запрос со сборки страницы, если
  // модуль вдруг начнут грузить сразу.
  const requests: Request[] = [];
  page.on('request', (request) => {
    requests.push(request);
  });

  const errors = await boot(page);

  await page.selectOption('#fixture', 'text-cyrillic-wrap.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });
  await expect(page.locator('#export-pdf')).toBeEnabled();

  // Подписка жива: иначе пустой список прошёл бы проверку вхолостую.
  expect(requests.length, 'запросы страницы должны доходить до теста').toBeGreaterThan(0);
  // Книга открыта и нарисована — ни обёртки, ни бинаря в сети ещё не было.
  expect(
    [...pdfLoadUrls(requests, PDF_WASM), ...pdfLoadUrls(requests, PDF_GLUE)],
    'до клика по экспорту PDF-модуль не грузится',
  ).toEqual([]);

  // Оба ожидания ставим до клика: и запрос модуля, и скачивание начинаются сразу.
  const pdfWasmRequest = page.waitForRequest((request) => isPdfLoad(request, PDF_WASM));
  const downloadPromise = page.waitForEvent('download');
  await page.click('#export-pdf');
  const [download] = await Promise.all([downloadPromise, pdfWasmRequest]);
  await readPdf(download);
  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });

  // Второй экспорт: инстанс модуля уже собран, в сеть за ним не идём.
  await expect(page.locator('#export-pdf')).toBeEnabled();
  const secondDownloadPromise = page.waitForEvent('download');
  await page.click('#export-pdf');
  await readPdf(await secondDownloadPromise);

  expect(pdfLoadUrls(requests, PDF_WASM), 'бинарь PDF за весь тест запрошен ровно один раз').toHaveLength(1);
  expect(pdfLoadUrls(requests, PDF_GLUE), 'JS-обёртка PDF за весь тест запрошена ровно один раз').toHaveLength(1);

  // Ошибки собирает boot; console.error из воркера тоже приходит на страницу.
  expect(errors).toEqual([]);
});
