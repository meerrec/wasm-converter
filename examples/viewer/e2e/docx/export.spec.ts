// Сквозная проверка пользовательского экспорта: кнопки панели отдают файл на
// скачивание, а имя и вид файла выводятся из подписи открытого документа.
//
// Текстовые форматы собираются в Rust и приезжают строкой, поэтому у них кроме
// имени проверяется содержимое: это единственная точка снаружи, где видно, что
// конвертер действительно что-то сконвертировал.

import { expect, test, type Page, type Request } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { downloadPdf, downloadText } from '../helpers/download.js';
import { isPdfLoad, pdfLoadUrls } from '../helpers/wasm_requests.js';

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
  const errors = await boot(page, '/docx');

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
  const errors = await boot(page, '/docx');
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

test('грузит PDF-модуль по клику и переиспользует его', async ({ page }) => {
  // Подписка до навигации: в список попадёт и запрос со сборки страницы, если
  // модуль вдруг начнут грузить сразу.
  const requests: Request[] = [];
  page.on('request', (request) => {
    requests.push(request);
  });

  const errors = await boot(page, '/docx');
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

  // По два запроса на файл — норма объединённого примера: первый делает воркер,
  // когда грузит модуль, второй — main, когда после успешного экспорта кладёт
  // байты в кэш браузера (`warmPdfWasmCache` в `lib/viewer_urls.ts`). Повторный
  // экспорт не добавляет ни одного: модуль уже собран, а его адреса отозваны.
  expect(pdfLoadUrls(requests, PDF_WASM), 'бинарь PDF за весь тест запрошен ровно два раза').toHaveLength(
    2,
  );
  expect(pdfLoadUrls(requests, PDF_GLUE), 'JS-обёртка PDF за весь тест запрошена ровно два раза').toHaveLength(
    2,
  );

  // Ошибки собирает boot; console.error из воркера тоже приходит на страницу.
  expect(errors).toEqual([]);
});
