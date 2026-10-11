// Ленивость DOCX-модуля: до открытия документа воркер просмотрщика не должен
// тянуть ни JS-обёртку `@doc-converter/wasm-docx`, ни её бинарь, а второе
// открытие обязано обойтись уже собранным инстансом — без второго запроса
// модуля.
//
// Модуль запрашивает сам воркер, поэтому сеть слушаем на уровне страницы:
// Playwright относит запросы dedicated worker'а к странице-владельцу.

import { expect, test, type Request } from '@playwright/test';
import { boot } from './canvas.js';

/** Имена файлов модуля DOCX: бинарь и JS-обёртка из `wasm-pack --target web`. */
const DOCX_WASM = 'doc_converter_docx_wasm_bg.wasm';
const DOCX_GLUE = 'doc_converter_docx_wasm.js';

/**
 * Служебная заглушка дев-сервера: `main.ts` берёт адреса модуля и бинаря
 * через `?url`, и Vite отдаёт на такой импорт крохотный модуль со строкой
 * URL. Запрос за ним уходит ещё при загрузке страницы, но байтов модуля не
 * несёт: сам модуль воркер грузит по адресу без `?url`.
 */
function isDevUrlStub(request: Request): boolean {
  return new URL(request.url()).searchParams.has('url');
}

/** Загрузка файла модуля DOCX — в отличие от заглушки `?url` выше. */
function isDocxLoad(request: Request, name: string): boolean {
  return request.url().includes(name) && !isDevUrlStub(request);
}

/** URL-ы загрузок файла модуля DOCX: сообщение об ошибке покажет, что пришло. */
function docxLoadUrls(requests: Request[], name: string): string[] {
  return requests.filter((request) => isDocxLoad(request, name)).map((request) => request.url());
}

test('грузит DOCX-модуль при открытии документа и переиспользует его', async ({ page }) => {
  // Подписка до навигации: в список попадёт и запрос со сборки страницы, если
  // модуль вдруг начнут грузить сразу.
  const requests: Request[] = [];
  page.on('request', (request) => {
    requests.push(request);
  });

  const errors = await boot(page);
  await expect(page.locator('#status')).toHaveText('файл не открыт');

  // Подписка жива: иначе пустой список прошёл бы проверку вхолостую.
  expect(requests.length, 'запросы страницы должны доходить до теста').toBeGreaterThan(0);
  // Вьюер поднят, документ ещё не открыт — ни обёртки, ни бинаря в сети не было.
  expect(
    [...docxLoadUrls(requests, DOCX_WASM), ...docxLoadUrls(requests, DOCX_GLUE)],
    'до открытия документа DOCX-модуль не грузится',
  ).toEqual([]);

  // Оба ожидания ставим до выбора фикстуры: и запрос модуля, и разбор начинаются сразу.
  const docxGlueRequest = page.waitForRequest((request) => isDocxLoad(request, DOCX_GLUE));
  const docxWasmRequest = page.waitForRequest((request) => isDocxLoad(request, DOCX_WASM));
  await page.selectOption('#fixture', 'simple/one_paragraph.docx');
  await Promise.all([docxGlueRequest, docxWasmRequest]);
  await expect(page.locator('#status')).toContainText('one_paragraph.docx: 1 стр.', {
    timeout: 30_000,
  });

  // Второй документ: инстанс модуля уже собран, в сеть за ним не идём. Ждём
  // строку состояния именно этого документа: «загружаю …» без числа страниц.
  await page.selectOption('#fixture', 'simple/multiple_paragraphs_0.docx');
  await expect(page.locator('#status')).toContainText(/multiple_paragraphs_0\.docx: [1-9]\d* стр\./, {
    timeout: 30_000,
  });

  expect(docxLoadUrls(requests, DOCX_WASM), 'бинарь DOCX за весь тест запрошен ровно один раз').toHaveLength(1);
  expect(docxLoadUrls(requests, DOCX_GLUE), 'JS-обёртка DOCX за весь тест запрошена ровно один раз').toHaveLength(1);

  // Ошибки собирает boot; console.error из воркера тоже приходит на страницу.
  expect(errors).toEqual([]);
});
