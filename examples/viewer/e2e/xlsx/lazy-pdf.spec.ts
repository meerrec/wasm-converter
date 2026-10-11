// Ленивость PDF-модуля: до первого клика по кнопке экспорта воркер
// просмотрщика не должен тянуть ни JS-обёртку `@doc-converter/wasm-pdf`, ни
// её бинарь, а повторный экспорт обязан обойтись уже собранным инстансом —
// без второго запроса модуля.
//
// Модуль запрашивает сам воркер, поэтому сеть слушаем на уровне страницы:
// Playwright относит запросы dedicated worker'а к странице-владельцу.

import { readFile } from 'node:fs/promises';
import { expect, test, type Download, type Request } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { isPdfLoad, pdfLoadUrls } from '../helpers/wasm_requests.js';

/** Имена файлов модуля PDF: бинарь и JS-обёртка из `wasm-pack --target web`. */
const PDF_WASM = 'doc_converter_pdf_wasm_bg.wasm';
const PDF_GLUE = 'doc_converter_pdf_wasm.js';

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

  const errors = await boot(page, '/xlsx');

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

  // По два запроса на файл — норма объединённого примера: первый делает воркер,
  // когда грузит модуль, второй — main, когда после успешного экспорта кладёт
  // байты в кэш браузера (`warmPdfWasmCache` в `lib/viewer_urls.ts`). Повторный
  // экспорт не добавляет ни одного: модуль уже собран, а его адреса отозваны.
  expect(
    pdfLoadUrls(requests, PDF_WASM),
    'бинарь PDF за весь тест запрошен ровно два раза: загрузка воркером и прогрев кэша',
  ).toHaveLength(2);
  expect(
    pdfLoadUrls(requests, PDF_GLUE),
    'JS-обёртка PDF за весь тест запрошена ровно два раза: загрузка воркером и прогрев кэша',
  ).toHaveLength(2);

  // Ошибки собирает boot; console.error из воркера тоже приходит на страницу.
  expect(errors).toEqual([]);
});
