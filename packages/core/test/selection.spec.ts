import { describe, expect, it } from 'vitest';

import { createLogicalAxis, createLogicalScroll, MAX_DOM_SCROLL_CSS_PX } from '../src/render/logical_scroll.js';
import {
  address,
  createSelection,
  fullColumns,
  fullRows,
  fullSheet,
  MAX_COL,
  MAX_ROW,
  moveWithinSelection,
  parseCell,
  parseRange,
  rangeName,
} from '../src/render/selection.js';

describe('A1-диапазоны', () => {
  it('читает границы полного листа и отвергает выход за них', () => {
    expect(parseCell('XFD1048576')).toEqual({ row: MAX_ROW, col: MAX_COL });
    expect(parseCell('$B$12')).toEqual({ row: 11, col: 1 });
    expect(() => parseCell('XFE1')).toThrow('вне листа');
    expect(() => parseCell('A1048577')).toThrow('вне листа');
    expect(() => parseRange('1:1048577')).toThrow('вне листа');
  });

  it('представляет полные строки, столбцы и лист без списка ячеек', () => {
    expect(parseRange('2:4')).toEqual(fullRows(1, 3));
    expect(parseRange('B:D')).toEqual(fullColumns(1, 3));
    expect(fullSheet().end).toEqual({ row: MAX_ROW, col: MAX_COL });
    expect(rangeName(fullColumns(0, 1))).toBe('A:B');
    expect(rangeName(fullRows(0, 1))).toBe('1:2');
  });

  it('обходит диапазон Tab по строкам и Enter по столбцам', () => {
    const state = {
      ...createSelection(0, address(1, 1)),
      ranges: [{ start: address(1, 1), end: address(2, 2) }],
    };
    expect(moveWithinSelection(state, false, false)).toEqual(address(1, 2));
    expect(moveWithinSelection(state, true, false)).toEqual(address(2, 2));
    expect(moveWithinSelection(state, false, true)).toEqual(address(2, 1));
    expect(moveWithinSelection(state, true, true)).toEqual(address(2, 2));
  });
});

describe('логическая прокрутка', () => {
  it('не меняет малые документы', () => {
    const axis = createLogicalAxis(800, 600);
    expect(axis.logicalMax).toBe(200);
    expect(axis.domMax).toBe(200);
    expect(axis.toDom(100)).toBe(100);
    expect(axis.toLogical(100)).toBe(100);
  });

  it('достигает обоих концов полного листа и ограничивает DOM', () => {
    const axis = createLogicalAxis(20_971_520 * 4, 600);
    expect(axis.domMax).toBe(MAX_DOM_SCROLL_CSS_PX);
    expect(axis.toDom(0)).toBe(0);
    expect(axis.toDom(axis.logicalMax)).toBe(axis.domMax);
    expect(axis.toLogical(axis.domMax)).toBe(axis.logicalMax);
    const middle = axis.logicalMax * 0.63;
    expect(axis.toLogical(axis.toDom(middle))).toBeCloseTo(middle, 3);
  });

  it('scrollTo возвращает DOM-координаты и сохраняет логическую позицию', () => {
    const scroll = createLogicalScroll();
    scroll.configure(1_048_576, 20_971_520, 1, 800, 600);
    const [left, top] = scroll.scrollTo(400_000, 10_000_000);
    scroll.syncFromDom(left, top);
    expect(scroll.x).toBeCloseTo(400_000, 3);
    expect(scroll.y).toBeCloseTo(10_000_000, 3);
    expect(scroll.width).toBeLessThanOrEqual(MAX_DOM_SCROLL_CSS_PX + 800);
    expect(scroll.height).toBeLessThanOrEqual(MAX_DOM_SCROLL_CSS_PX + 600);
  });
});
