import type { CellAddress, CellRange, SheetSelection, XlsxSelection } from '../protocol.js';

export const MAX_ROW = 1_048_575;
export const MAX_COL = 16_383;

export type SelectionMode = 'normal' | 'extend' | 'add';

/** Внутреннее состояние автомата; публичный снимок не раскрывает drag-якоря. */
export interface SelectionState extends SheetSelection {
  sheetIndex: number;
  anchor: CellAddress;
  cursor: CellAddress;
  mode: SelectionMode;
}

const coordinate = (value: number, maximum: number): number =>
  Number.isFinite(value) ? Math.min(Math.max(Math.trunc(value), 0), maximum) : 0;

export const address = (row: number, col: number): CellAddress => ({
  row: coordinate(row, MAX_ROW),
  col: coordinate(col, MAX_COL),
});

export const sameAddress = (left: CellAddress, right: CellAddress): boolean =>
  left.row === right.row && left.col === right.col;

export const normalizeRange = (range: CellRange): CellRange => ({
  start: address(Math.min(range.start.row, range.end.row), Math.min(range.start.col, range.end.col)),
  end: address(Math.max(range.start.row, range.end.row), Math.max(range.start.col, range.end.col)),
});

export const rangeBetween = (anchor: CellAddress, cursor: CellAddress): CellRange =>
  normalizeRange({ start: anchor, end: cursor });

export const singleCell = (cell: CellAddress): CellRange => ({ start: cell, end: cell });
export const fullRows = (first: number, last: number): CellRange =>
  normalizeRange({ start: address(first, 0), end: address(last, MAX_COL) });
export const fullColumns = (first: number, last: number): CellRange =>
  normalizeRange({ start: address(0, first), end: address(MAX_ROW, last) });
export const fullSheet = (): CellRange => ({ start: address(0, 0), end: address(MAX_ROW, MAX_COL) });

export const createSelection = (sheetIndex: number, active: CellAddress = address(0, 0)): SelectionState => ({
  sheetIndex,
  active,
  anchor: active,
  cursor: active,
  ranges: [singleCell(active)],
  mode: 'normal',
});

export const publicSelection = (state: SelectionState): XlsxSelection => ({
  sheetIndex: state.sheetIndex,
  active: { ...state.active },
  ranges: state.ranges.map((range) => ({
    start: { ...range.start },
    end: { ...range.end },
  })),
});

export const sheetSelection = (state: SelectionState): SheetSelection => ({
  active: state.active,
  ranges: state.ranges,
});

export const primaryRange = (state: SelectionState): CellRange =>
  state.ranges[state.ranges.length - 1] ?? singleCell(state.active);

export const selectionContains = (range: CellRange, cell: CellAddress): boolean =>
  cell.row >= range.start.row && cell.row <= range.end.row
  && cell.col >= range.start.col && cell.col <= range.end.col;

export const selectionEquals = (left: SelectionState | null, right: SelectionState | null): boolean => {
  if (left === right) return true;
  if (!left || !right || !sameAddress(left.active, right.active) || left.ranges.length !== right.ranges.length) {
    return false;
  }
  return left.ranges.every((range, index) => {
    const other = right.ranges[index];
    return other !== undefined
      && sameAddress(range.start, other.start)
      && sameAddress(range.end, other.end);
  });
};

/** A1, AA10, XFD1048576 и $-форма. */
export function parseCell(reference: string): CellAddress {
  const match = /^\$?([A-Za-z]+)\$?([1-9]\d*)$/.exec(reference.trim());
  if (!match?.[1] || !match[2]) throw new Error(`некорректная ссылка «${reference}»`);
  let col = 0;
  for (const letter of match[1].toUpperCase()) col = col * 26 + letter.charCodeAt(0) - 64;
  const row = Number(match[2]) - 1;
  if (row > MAX_ROW || col - 1 > MAX_COL) throw new Error(`ссылка вне листа «${reference}»`);
  return address(row, col - 1);
}

/** A1:B2, A:A, 1:3 или одиночная ячейка. */
export function parseRange(reference: string): CellRange {
  const parts = reference.trim().split(':');
  if (parts.length > 2 || !parts[0]) throw new Error(`некорректный диапазон «${reference}»`);
  const first = parts[0];
  const second = parts[1] ?? first;
  if (/^[A-Za-z]+$/.test(first) && /^[A-Za-z]+$/.test(second)) {
    const col = (value: string): number => parseCell(`${value}1`).col;
    return fullColumns(col(first), col(second));
  }
  if (/^\d+$/.test(first) && /^\d+$/.test(second)) {
    const row = (value: string): number => {
      const parsed = Number(value);
      if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > MAX_ROW + 1) {
        throw new Error(`строка вне листа «${value}»`);
      }
      return parsed - 1;
    };
    return fullRows(row(first), row(second));
  }
  return rangeBetween(parseCell(first), parseCell(second));
}

export function columnName(col: number): string {
  let value = Math.min(Math.max(Math.trunc(col), 0), MAX_COL) + 1;
  let result = '';
  while (value > 0) {
    const remainder = (value - 1) % 26;
    result = String.fromCharCode(65 + remainder) + result;
    value = Math.floor((value - 1) / 26);
  }
  return result;
}

export const cellName = (cell: CellAddress): string => `${columnName(cell.col)}${cell.row + 1}`;

export function rangeName(range: CellRange): string {
  const normalized = normalizeRange(range);
  if (normalized.start.row === 0 && normalized.end.row === MAX_ROW) {
    const first = columnName(normalized.start.col);
    const last = columnName(normalized.end.col);
    return first === last ? first : `${first}:${last}`;
  }
  if (normalized.start.col === 0 && normalized.end.col === MAX_COL) {
    const first = String(normalized.start.row + 1);
    const last = String(normalized.end.row + 1);
    return first === last ? first : `${first}:${last}`;
  }
  const first = cellName(normalized.start);
  const last = cellName(normalized.end);
  return first === last ? first : `${first}:${last}`;
}

export function selectionName(state: SelectionState): string {
  const primary = rangeName(primaryRange(state));
  return state.ranges.length > 1 ? `${primary}; ещё ${state.ranges.length - 1}` : primary;
}

/** Перемещение активной ячейки внутри последнего диапазона. */
export function moveWithinSelection(state: SelectionState, backward: boolean, columnMajor: boolean): CellAddress {
  const range = primaryRange(state);
  const rows = range.end.row - range.start.row + 1;
  const cols = range.end.col - range.start.col + 1;
  const rowOffset = Math.min(Math.max(state.active.row - range.start.row, 0), rows - 1);
  const colOffset = Math.min(Math.max(state.active.col - range.start.col, 0), cols - 1);
  const length = rows * cols;
  const index = columnMajor ? colOffset * rows + rowOffset : rowOffset * cols + colOffset;
  const next = (index + (backward ? -1 : 1) + length) % length;
  const row = columnMajor ? next % rows : Math.floor(next / cols);
  const col = columnMajor ? Math.floor(next / rows) : next % cols;
  return address(range.start.row + row, range.start.col + col);
}
