// Обёртка axe-core для Playwright: один прогон на документ после загрузки.
//
// axe-core — единственная новая devDependency Спринта 2: без неё гейт
// «0 violations» проверялся бы глазами. Скрипт кладём в страницу из пакета,
// а не с CDN: CI работает без сети.

import { createRequire } from 'node:module';
import type { Page } from '@playwright/test';

const require = createRequire(import.meta.url);
/** Путь к собранному axe.min.js внутри devDependency. */
export const AXE_SCRIPT = require.resolve('axe-core/axe.min.js');

/** Минимум полей результата, который нужен тесту для диагностики. */
export interface AxeViolation {
  id: string;
  impact: string | null;
  description: string;
  nodes: { target: string[]; failureSummary?: string }[];
}

interface AxeResults {
  violations: AxeViolation[];
}

/**
 * Прогнать axe по текущей странице. Скрипт добавляется один раз; повторный
 * вызов переиспользует уже загруженный `window.axe`.
 */
export async function runAxe(page: Page, selector = 'body'): Promise<AxeResults> {
  const loaded = await page.evaluate(() => 'axe' in globalThis);
  if (!loaded) await page.addScriptTag({ path: AXE_SCRIPT });
  return page.evaluate(
    async (context) => {
      const axe = (
        globalThis as unknown as {
          axe: {
            run: (
              context: string,
              options: unknown,
            ) => Promise<{ violations: AxeViolation[] }>;
          };
        }
      ).axe;
      return axe.run(context, {
        runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] },
      });
    },
    selector,
  );
}
