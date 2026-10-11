// Сквозная проверка пользовательского экспорта: кнопки панели отдают файл на
// скачивание, а имя и вид файла выводятся из подписи открытого документа.
//
// Текстовые форматы собираются в Rust и приезжают строкой, поэтому у них кроме
// имени проверяется содержимое: это единственная точка снаружи, где видно, что
// конвертер действительно что-то сконвертировал.

import { expect, test, type Page, type Request } from '@playwright/test';
import { boot } from './canvas.js';
import { downloadPdf, downloadText } from './download.js';

/** Фикстура с одним абзацем: из её подписи выводятся имена скачанных файлов. */
const FIXTURE = 'simple/one_paragraph.docx';

/** Открыть фикстуру и дождаться, пока вьюер отдаст страницы и оживут кнопки. */
async function openOneParagraph(page: Page): Promise<void> {
  await page.selectOption('#fixture', FIXTURE);
  await expect(page.locator('#status')).toContainText('one_paragraph.docx: 1 стр.', {
    timeout: 30_000,
  });
  for (const id of ['#export-pdf', '#export-md', '#export-html', '#export-txt']) {
    await expect(page.locator(id)).toBeEnabled();
  }
}

test('экспортирует открытый документ в PDF', async ({ page }) => {
  const errors = await boot(page);

  // Кнопки оживают только вместе с документом: экспортировать пока нечего.
  await expect(page.locator('#export-pdf')).toBeDisabled();

  await openOneParagraph(page);

  const file = await downloadPdf(page, '#export-pdf');
  // Имя — из подписи документа: каталог фикстуры срезан, расширение заменено.
  expect(file.name).toBe('one_paragraph.pdf');

  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });
  // После экспорта кнопка снова рабочая: файл можно выгрузить ещё раз.
  await expect(page.locator('#export-pdf')).toBeEnabled();

  expect(errors).toEqual([]);
});

test('экспортирует документ в Markdown, HTML и текст', async ({ page }) => {
  const errors = await boot(page);
  await openOneParagraph(page);

  const markdown = await downloadText(page, '#export-md');
  expect(markdown.name).toBe('one_paragraph.md');
  // Строка абзаца — с разметки и начинается: экспорт отдал содержимое документа.
  expect(markdown.text.startsWith('Hello, World!')).toBe(true);
  await expect(page.locator('#status')).toContainText('Markdown готов');

  const html = await downloadText(page, '#export-html');
  expect(html.name).toBe('one_paragraph.html');
  // HTML — страница целиком, а не фрагмент разметки.
  expect(html.text.startsWith('<!DOCTYPE html>')).toBe(true);
  await expect(page.locator('#status')).toContainText('HTML готов');

  const text = await downloadText(page, '#export-txt');
  expect(text.name).toBe('one_paragraph.txt');
  // Абзац — ровно одна строка с завершающим переводом: ни разметки, ни лишнего.
  expect(text.text).toBe('Hello, World!\n');
  await expect(page.locator('#status')).toContainText('Текст готов');

  await expect(page.locator('#export-txt')).toBeEnabled();

  expect(errors).toEqual([]);
});

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

test('грузит PDF-модуль по клику и переиспользует его', async ({ page }) => {
  // Подписка до навигации: в список попадёт и запрос со сборки страницы, если
  // модуль вдруг начнут грузить сразу.
  const requests: Request[] = [];
  page.on('request', (request) => {
    requests.push(request);
  });

  const errors = await boot(page);
  await openOneParagraph(page);

  // Подписка жива: иначе пустой список прошёл бы проверку вхолостую.
  expect(requests.length, 'запросы страницы должны доходить до теста').toBeGreaterThan(0);
  // Документ открыт — ни обёртки, ни бинаря PDF в сети ещё не было.
  expect(
    [...pdfLoadUrls(requests, PDF_WASM), ...pdfLoadUrls(requests, PDF_GLUE)],
    'до клика по экспорту PDF-модуль не грузится',
  ).toEqual([]);

  // Оба ожидания ставим до клика: и запрос модуля, и скачивание начинаются сразу.
  const pdfWasmRequest = page.waitForRequest((request) => isPdfLoad(request, PDF_WASM));
  await downloadPdf(page, '#export-pdf');
  await pdfWasmRequest;
  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });

  // Второй экспорт: инстанс модуля уже собран, в сеть за ним не идём.
  await expect(page.locator('#export-pdf')).toBeEnabled();
  await downloadPdf(page, '#export-pdf');

  expect(pdfLoadUrls(requests, PDF_WASM), 'бинарь PDF за весь тест запрошен ровно один раз').toHaveLength(
    1,
  );
  expect(pdfLoadUrls(requests, PDF_GLUE), 'JS-обёртка PDF за весь тест запрошена ровно один раз').toHaveLength(
    1,
  );

  // Ошибки собирает boot; console.error из воркера тоже приходит на страницу.
  expect(errors).toEqual([]);
});
