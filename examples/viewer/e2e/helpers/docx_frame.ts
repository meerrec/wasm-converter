// Работа с кадром DOCX-страницы: строка состояния, выбор фикстуры и ожидание
// первой нарисованной страницы.

import type { Page } from '@playwright/test';
import { frameStats, type FrameStats } from './png.js';

/** Номер текущего кадра из строки состояния: по нему видно, что painter отработал. */
export async function frameId(page: Page): Promise<number> {
  return page.evaluate(() => Number(document.querySelector('#frame')?.textContent ?? '0'));
}

/** Дождаться кадра с номером больше `from`: столько painter'у нужно на перерисовку. */
export async function waitForFrame(page: Page, from: number): Promise<number> {
  await page.waitForFunction(
    (previous) => Number(document.querySelector('#frame')?.textContent ?? '0') > previous,
    from,
    { timeout: 30_000 },
  );
  return frameId(page);
}

/**
 * Открыть фикстуру и дождаться первого нарисованного кадра.
 *
 * Ждать одну строку состояния мало: `open` асинхронен, а до его ответа холст
 * ещё пуст. Поэтому сначала дожидаемся раскладки (число страниц), потом —
 * картинки на холсте.
 */
export async function openFixture(page: Page, name: string): Promise<FrameStats> {
  await page.selectOption('#fixture', name);
  await page.waitForFunction(
    () => {
      const bridge = window.docConverter;
      return bridge?.kind === 'docx' && bridge.viewer.pageCount > 0;
    },
    undefined,
    { timeout: 30_000 },
  );

  const deadline = Date.now() + 30_000;
  for (;;) {
    const stats = await frameStats(page);
    if (stats.inkBox) return stats;
    if (Date.now() > deadline) throw new Error(`страница ${name} осталась пустой`);
    await page.waitForTimeout(100);
  }
}

/** Число страниц в открытом документе. */
export async function pageCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const bridge = window.docConverter;
    return bridge?.kind === 'docx' ? bridge.viewer.pageCount : 0;
  });
}
