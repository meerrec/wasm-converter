// Полный круг Embed API без сайта редактора: iframe подменяется лёгкой
// страницей-заглушкой, отвечающей на команды так же, как отвечал бы редактор.
// Хост сайта (566 МБ) для этого не нужен совсем, а проверяется то, что лёгкой
// проверке маршрута недоступно: origin в опции клиента, очередь файла до
// `document:ready`, `readonly: true` в команде открытия и рост счётчика
// `openedCount` на `document:opened`.
//
// Ответы заглушки читаются на стороне страницы, а не из её рамки: у iframe,
// отданного подменой запроса, Firefox не отдаёт ни `url()`, ни контекст.

import { expect, test, type Page } from '@playwright/test';

/** Origin, на который указывает страница в дев-режиме (`.env.development`). */
const EDITOR_ORIGIN = 'http://127.0.0.1:5271';

/** Небольшие байты: страница только передаёт их редактору, разбор не наш. */
const DOCX_BYTES = 2048;

/** Копилка сырых сообщений от iframe — её и читает тест. */
interface RepliesWindow {
  __editorReplies?: unknown[];
}

/**
 * Заглушка редактора: `ready` через задержку, а на `document:open-buffer` —
 * `document:opened` с тем же id. Что именно пришло в команде (`readonly`,
 * размер байтов), заглушка возвращает в ответе: так это видно странице и тесту.
 */
function fakeEditor(readyDelayMs: number): string {
  return `<!doctype html>
<html lang="ru"><head><meta charset="utf-8"><title>Заглушка редактора</title></head>
<body><script>
  window.addEventListener('message', (event) => {
    const data = event.data;
    if (typeof data !== 'object' || data === null || typeof data.type !== 'string') return;
    if (data.type !== 'document:open-buffer') return;
    const payload = data.payload ?? {};
    window.parent.postMessage(
      {
        type: 'document:opened',
        id: data.id,
        payload: {
          fileName: payload.fileName,
          readonly: payload.readonly,
          byteLength: payload.buffer instanceof ArrayBuffer ? payload.buffer.byteLength : null,
        },
      },
      '*',
    );
  });
  setTimeout(() => window.parent.postMessage({ type: 'document:ready', payload: {} }, '*'), ${readyDelayMs});
</script></body></html>`;
}

/**
 * Заголовки ответа заглушки. Оба обязательны, и это не прихоть подмены: страница
 * демо кросс-доменно изолирована (COOP `same-origin` + COEP `require-corp`), а
 * такую страницу браузер встраивает кросс-доменный iframe, только если в ответе
 * самого iframe есть `Cross-Origin-Resource-Policy: cross-origin` **и** COEP
 * `require-corp` — одного CORP мало, вложение падает с `ERR_BLOCKED_BY_RESPONSE`.
 * Тот же набор обязан отдавать хост сайта редактора в дев-режиме.
 */
const FAKE_EDITOR_HEADERS = {
  'Content-Type': 'text/html; charset=utf-8',
  'Cross-Origin-Resource-Policy': 'cross-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

/** Отдать заглушку вместо редактора и копить её ответы на самой странице. */
async function serveFakeEditor(page: Page, readyDelayMs: number): Promise<void> {
  await page.addInitScript(() => {
    const holder = window as unknown as RepliesWindow;
    holder.__editorReplies = [];
    window.addEventListener('message', (event) => {
      holder.__editorReplies?.push(event.data);
    });
  });
  await page.route(`${EDITOR_ORIGIN}/**`, (route) =>
    route.fulfill({ headers: FAKE_EDITOR_HEADERS, body: fakeEditor(readyDelayMs) }),
  );
}

/** Ответы, дошедшие от заглушки. */
async function replies(page: Page): Promise<readonly unknown[]> {
  return page.evaluate(() => (window as unknown as RepliesWindow).__editorReplies ?? []);
}

test('открывает документ байтами в режиме только просмотра', async ({ page }) => {
  await serveFakeEditor(page, 0);
  await page.goto('/#/eurooffice');

  // Событие от чужого origin'а клиент отсеивает, поэтому `ready` доходит только
  // потому, что страница отдала клиенту origin из `.env.development`.
  await expect(page.locator('#viewer-status')).toHaveAttribute('data-state', 'ready');

  await page.locator('#viewer-file').setInputFiles({
    name: 'draft.docx',
    mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    buffer: Buffer.alloc(DOCX_BYTES),
  });

  const status = page.locator('#viewer-status');
  await expect(status).toHaveAttribute('data-state', 'opened');
  await expect(status).toHaveText('«draft.docx» — только просмотр');
  await expect(page.locator('#viewer-doc-name')).toHaveText('draft.docx');

  // Файл ушёл байтами и только на просмотр — в этом весь контракт страницы.
  expect(await replies(page)).toContainEqual({
    type: 'document:opened',
    id: expect.any(String),
    payload: { fileName: 'draft.docx', readonly: true, byteLength: DOCX_BYTES },
  });
  expect(await page.evaluate(() => window.euroofficeViewer?.openedCount)).toBe(1);
});

test('файл, выбранный до готовности редактора, ждёт её в очереди', async ({ page }) => {
  await serveFakeEditor(page, 800);
  await page.goto('/#/eurooffice');

  const status = page.locator('#viewer-status');
  await expect(status).toHaveAttribute('data-state', 'loading');
  await page.locator('#viewer-file').setInputFiles({
    name: 'queued.docx',
    mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    buffer: Buffer.alloc(64),
  });
  // До `ready` файл не потерян и не открыт: он в очереди, о чём и говорит статус.
  await expect(status).toHaveText(
    'редактор ещё поднимается — «queued.docx» откроется сразу после готовности',
  );
  expect(await replies(page)).toEqual([]);

  // Готовность редактора открывает файл из очереди без второго выбора.
  await expect(status).toHaveAttribute('data-state', 'opened');
  await expect(status).toHaveText('«queued.docx» — только просмотр');
  expect(await page.evaluate(() => window.euroofficeViewer?.openedCount)).toBe(1);
});

test('ошибку редактора показывает с расшифровкой кода', async ({ page }) => {
  await serveFakeEditor(page, 0);
  // Заглушка отвечает ошибкой на любую команду: страница должна показать её текст.
  await page.route(`${EDITOR_ORIGIN}/**`, (route) =>
    route.fulfill({
      headers: FAKE_EDITOR_HEADERS,
      body: `<!doctype html><html lang="ru"><body><script>
        window.parent.postMessage({ type: 'document:ready', payload: {} }, '*');
        window.addEventListener('message', () => window.parent.postMessage(
          { type: 'document:error', payload: { message: 'файл не разобран', code: -82 } }, '*',
        ));
      </script></body></html>`,
    }),
  );
  await page.goto('/#/eurooffice');

  const status = page.locator('#viewer-status');
  await expect(status).toHaveAttribute('data-state', 'ready');
  await page.locator('#viewer-file').setInputFiles({
    name: 'broken.docx',
    mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    buffer: Buffer.alloc(16),
  });

  await expect(status).toHaveAttribute('data-state', 'error');
  // Код превращает в текст `describeEditorError` из пакета, а не страница.
  await expect(status).toContainText('код -82: не удалось сконвертировать');
  expect(await page.evaluate(() => window.euroofficeViewer?.lastError)).toContain('код -82');
});
