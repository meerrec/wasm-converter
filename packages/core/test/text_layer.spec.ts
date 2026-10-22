// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';

import type { PageInfo, TextLayer, TextRun } from '../src/protocol.js';
import { createTextLayer } from '../src/render/text_layer.js';

/** Минимальный run: геометрия фиксирована, важны идентификаторы и роли. */
function run(overrides: Partial<TextRun>): TextRun {
  return {
    id: 'p1:pg0:l0:f0',
    bbox: { x: 0, y: 0, width: 50, height: 10 },
    lineRect: { x: 0, y: 0, width: 100, height: 10 },
    text: 'текст',
    style: {
      fontFamily: 'Carlito',
      sizePx: 12,
      bold: false,
      italic: false,
      underline: false,
      color: null,
    },
    page: 0,
    lineIndex: 0,
    fragmentIndex: 0,
    readingOrder: 0,
    direction: 'ltr',
    bidiLevel: 0,
    textAlign: 'left',
    nodeId: 1,
    blockKind: 'paragraph',
    headingLevel: null,
    table: null,
    ...overrides,
  };
}

function layer(runs: TextRun[]): TextLayer {
  const pageMap = [];
  for (const page of new Set(runs.map((item) => item.page))) {
    const pageRuns = runs.filter((item) => item.page === page);
    pageMap.push({ page, start: 0, count: pageRuns.length });
  }
  return { runs, pageMap };
}

const PAGES: PageInfo[] = Array.from({ length: 10 }, () => ({ width: 600, height: 100 }));
const TOPS = PAGES.map((_, index) => index * 100);

describe('createTextLayer', () => {
  it('виртуализирует страницы окна ±1 и не монтирует дальние', async () => {
    const requested: number[] = [];
    const controller = createTextLayer({
      fetchPage: async (page) => {
        requested.push(page);
        return layer([run({ page, lineIndex: page, nodeId: 10 + page })]);
      },
      fetchAll: async () => layer([]),
    });
    controller.configure(PAGES, TOPS);
    controller.updateWindow(450, 100, 1);
    await new Promise((resolve) => setTimeout(resolve, 0));

    // Окно: страницы 3..6 (pageAt(450)=4, pageAt(550)=5, ±1).
    expect(requested).toEqual([3, 4, 5, 6]);
    expect(controller.element.querySelectorAll('.dc-text-line').length).toBe(4);
    expect(controller.element.querySelector('[data-page="0"]')).toBeNull();
  });

  it('строит семантическое дерево с заголовком и таблицей', async () => {
    const controller = createTextLayer({
      fetchPage: async (page) => layer([run({ page })]),
      fetchAll: async () =>
        layer([
          run({
            id: 'h',
            nodeId: 1,
            blockKind: 'heading',
            headingLevel: 1,
            text: 'Заголовок',
          }),
          run({
            id: 'p',
            nodeId: 2,
            readingOrder: 1,
            text: 'Абзац',
          }),
          run({
            id: 't0',
            nodeId: 3,
            readingOrder: 2,
            text: 'Заголовок таблицы',
            table: {
              tableId: 7,
              cellId: 70,
              row: 0,
              col: 0,
              colSpan: 1,
              rowSpan: 1,
              isHeader: true,
              isRepeatedHeader: false,
              isVmergeContinuation: false,
            },
          }),
          run({
            id: 't1',
            nodeId: 3,
            readingOrder: 3,
            text: 'Ячейка',
            table: {
              tableId: 7,
              cellId: 71,
              row: 1,
              col: 1,
              colSpan: 1,
              rowSpan: 1,
              isHeader: false,
              isRepeatedHeader: false,
              isVmergeContinuation: false,
            },
          }),
        ]),
    });
    controller.configure(PAGES, TOPS);
    await new Promise((resolve) => setTimeout(resolve, 0));

    const semantic = controller.element.querySelector('.dc-text-semantic');
    expect(semantic?.querySelector('[role="heading"]')?.getAttribute('aria-level')).toBe('1');
    expect(semantic?.querySelector('[role="paragraph"]')?.textContent).toBe('Абзац');
    expect(semantic?.querySelector('[role="table"]')).not.toBeNull();
    expect(semantic?.querySelector('[role="columnheader"]')?.getAttribute('aria-colindex')).toBe(
      '1',
    );
    expect(semantic?.querySelector('[role="cell"]')?.getAttribute('aria-colindex')).toBe('2');
    expect(semantic?.querySelector('[role="table"]')?.getAttribute('aria-hidden')).toBeNull();

    const visual = controller.element.querySelector('.dc-text-visual');
    expect(visual?.getAttribute('aria-hidden')).toBe('true');
  });
});
