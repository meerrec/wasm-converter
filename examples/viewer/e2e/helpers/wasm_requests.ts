// Предикаты сетевых запросов за wasm-модулями: ленивость проверяется по тому,
// каких запросов в сети НЕ было.
//
// Модули запрашивает сам воркер, поэтому сеть слушают на уровне страницы:
// Playwright относит запросы dedicated worker'а к странице-владельцу.

import type { Request } from '@playwright/test';

/**
 * Служебная заглушка дев-сервера: `main.ts` берёт адреса модуля и бинаря
 * через `?url`, и Vite отдаёт на такой импорт крохотный модуль со строкой
 * URL. Запрос за ним уходит ещё при загрузке страницы, но байтов модуля не
 * несёт: сам модуль воркер грузит по адресу без `?url`.
 */
export function isDevUrlStub(request: Request): boolean {
  return new URL(request.url()).searchParams.has('url');
}

/** Загрузка файла модуля DOCX — в отличие от заглушки `?url` выше. */
export function isDocxLoad(request: Request, name: string): boolean {
  return request.url().includes(name) && !isDevUrlStub(request);
}

/** Загрузка файла модуля PDF — в отличие от заглушки `?url` выше. */
export function isPdfLoad(request: Request, name: string): boolean {
  return request.url().includes(name) && !isDevUrlStub(request);
}

/** URL-ы загрузок файла модуля DOCX: сообщение об ошибке покажет, что пришло. */
export function docxLoadUrls(requests: Request[], name: string): string[] {
  return requests.filter((request) => isDocxLoad(request, name)).map((request) => request.url());
}

/** URL-ы загрузок файла модуля PDF: сообщение об ошибке покажет, что пришло. */
export function pdfLoadUrls(requests: Request[], name: string): string[] {
  return requests.filter((request) => isPdfLoad(request, name)).map((request) => request.url());
}
