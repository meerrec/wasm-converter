// Подъём страницы примера в браузере: общий вход для всех сквозных спеков.

import type { Page } from '@playwright/test';
import { waitForViewer } from './bridge.js';

/** Открыть пример и дождаться, пока вьюер поднимется. */
export async function boot(page: Page, route: '/docx' | '/xlsx' | '/pptx'): Promise<string[]> {
  // Ошибки собираются с самого начала: падение страницы должен показать текст
  // ошибки, а не таймаут ожидания моста.
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });

  await page.goto(`/#${route}`);
  await page.waitForFunction(() => window.docConverter !== undefined);
  // Мост появляется и у чужого вида, поэтому вид проверяем отдельно — тем
  // ожиданием, которое знает, какого вида вьюер тут нужен.
  await waitForViewer(page, route === '/docx' ? 'docx' : route === '/pptx' ? 'pptx' : 'xlsx');
  return errors;
}
