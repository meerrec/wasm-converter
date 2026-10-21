// Мост к активному вьюеру — единственное объявление `window.docConverter` для
// сквозных тестов.
//
// Страница кладёт мост сама (см. `src/lib/bridge.ts`), и её декларация в
// `src` остаётся только для сборки приложения: спеки и хелперы видят тип
// отсюда, а `declare global` написано ровно один раз — на весь набор.

import type { DocxViewerHandle, PptxViewerHandle, XlsxViewerHandle } from '@meerrec/core';
import type { Page } from '@playwright/test';

/** Вид вьюера: какое приложение поднято маршрутом. */
export type ViewerKind = 'docx' | 'xlsx' | 'pptx';

declare global {
  interface Window {
    docConverter?:
      | { kind: 'docx'; viewer: DocxViewerHandle }
      | { kind: 'xlsx'; viewer: XlsxViewerHandle }
      | { kind: 'pptx'; viewer: PptxViewerHandle };
  }
}

/**
 * Дождаться, что страница подняла вьюер нужного вида.
 *
 * Вид проверяется, а не просто наличие моста: под чужим видом хелперы молча
 * уходят в таймаут, и разбираться пришлось бы по дампу трассы, а не по
 * сообщению. Поэтому после ожидания виден ещё и фактический мост.
 */
export async function waitForViewer(page: Page, kind: ViewerKind): Promise<void> {
  try {
    await page.waitForFunction((expected) => window.docConverter?.kind === expected, kind);
  } catch {
    const actual = await page.evaluate(() => window.docConverter?.kind ?? null);
    throw new Error(
      `вьюер «${kind}» не поднялся: window.docConverter — ${
        actual === null ? 'моста нет' : `вид «${actual}»`
      }`,
    );
  }
}
