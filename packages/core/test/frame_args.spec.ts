import { describe, expect, it } from 'vitest';

import type {
  DocxOpenedMsg,
  OutMsg,
  PageInfo,
  RenderConfig,
  RenderRequest,
  Viewport,
} from '../src/protocol.js';
import { frameTarget, toOptions, toViewport } from '../src/worker/frame_args.js';

const viewport: Viewport = { x: 10, y: 20, w: 800, h: 600, scale: 2 };
const config: RenderConfig = { showGrid: true, showHeaders: false, theme: 'dark', dpr: 1 };

/** Кадр XLSX: лист и настройки показа. */
const xlsxReq = (sheet?: number): RenderRequest =>
  sheet === undefined ? { viewport, config } : { sheet, viewport, config };

/** Кадр DOCX: зазор между страницами; настроек показа у него нет. */
const docxReq = (gap?: number): RenderRequest =>
  gap === undefined ? { viewport, config } : { gap, viewport, config };

describe('toViewport', () => {
  it('перекладывает окно в единицы Rust', () => {
    expect(toViewport(viewport)).toEqual({
      scrollX: 10,
      scrollY: 20,
      width: 800,
      height: 600,
      scale: 2,
    });
  });
});

describe('toOptions', () => {
  it('оформление превращает во флаг тёмной темы', () => {
    expect(toOptions(config)).toEqual({ showGrid: true, showHeaders: false, dark: true });
    expect(toOptions({ ...config, theme: 'light' })).toMatchObject({ dark: false });
  });
});

describe('frameTarget', () => {
  it('для XLSX берёт лист запроса', () => {
    expect(frameTarget('xlsx', xlsxReq(3))).toEqual({
      kind: 'xlsx',
      sheet: 3,
      viewport: toViewport(viewport),
      options: toOptions(config),
    });
  });

  it('для XLSX без листа оставляет прежний лист по умолчанию', () => {
    expect(frameTarget('xlsx', xlsxReq())).toMatchObject({ kind: 'xlsx', sheet: 0 });
  });

  it('для DOCX берёт зазор стопки и не тащит настройки показа', () => {
    const target = frameTarget('docx', docxReq(2));

    expect(target).toEqual({ kind: 'docx', gap: 2, viewport: toViewport(viewport) });
    // Настроек показа у кадра стопки нет — их не должно быть и в аргументах.
    expect('options' in target).toBe(false);
  });

  it('для DOCX без зазора отказывает, а не рисует стопку встык', () => {
    expect(() => frameTarget('docx', docxReq())).toThrow(/gap/);
  });

  it('пока ничего не открыто, кадр не собирается', () => {
    expect(() => frameTarget(null, xlsxReq(0))).toThrow(/no document/);
    expect(() => frameTarget(null, docxReq(0))).toThrow(/no document/);
  });
});

describe('ответ на открытие DOCX', () => {
  const pages: PageInfo[] = [
    { width: 595, height: 842 },
    { width: 595, height: 842 },
  ];
  const opened: OutMsg = { type: 'docx-opened', id: 1, pages } satisfies DocxOpenedMsg;

  it('отдельного тега и страниц хватает, чтобы отличить его от книги', () => {
    expect(opened).toEqual({ type: 'docx-opened', id: 1, pages });
    expect('sheets' in opened).toBe(false);
  });
});
