import type JSZip from 'jszip';
import {
  NS,
  children,
  clone,
  descendants,
  direct,
  element,
  ensureDirect,
  normalPath,
  readXml,
  removeChildren,
  setAttr,
  setNsAttr,
  textOf,
  writeXml,
} from './xml.js';
import {
  ensureContentType,
  ensureRelationship,
  nextPartName,
  relativeTarget,
} from './package.js';
import type { EditorArgs } from './types.js';

export interface CellAddress {
  row: number;
  col: number;
}

export interface CellRange {
  start: CellAddress;
  end: CellAddress;
}

export interface XlsxSheet {
  document: Document;
  path: string;
  name: string;
  sheetId: number;
  relationshipId: string;
  index: number;
}

export interface XlsxWorkbook {
  document: Document;
  path: string;
  sheets: XlsxSheet[];
  sharedStrings: string[];
}

export interface CellSnapshot {
  text: string;
  formula: string;
}

const WORKBOOK_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument';
const WORKSHEET_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet';
const SHARED_STRINGS_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings';
const XML_SPACE = 'http://www.w3.org/XML/1998/namespace';
const MAX_SELECTION_CELLS = 100_000;

export function columnName(column: number): string {
  if (!Number.isInteger(column) || column < 0) throw new Error('некорректный столбец');
  let value = column + 1;
  let result = '';
  while (value > 0) {
    const remainder = (value - 1) % 26;
    result = String.fromCharCode(65 + remainder) + result;
    value = Math.floor((value - 1) / 26);
  }
  return result;
}

export function columnIndex(name: string): number {
  let result = 0;
  for (const letter of name.toUpperCase()) {
    if (letter < 'A' || letter > 'Z') throw new Error(`некорректный столбец «${name}»`);
    result = result * 26 + letter.charCodeAt(0) - 64;
  }
  return result - 1;
}

export function cellName(address: CellAddress, absolute = false): string {
  const marker = absolute ? '$' : '';
  return `${marker}${columnName(address.col)}${marker}${address.row + 1}`;
}

export function parseCell(reference: string): CellAddress {
  const match = /^\$?([A-Za-z]+)\$?([1-9]\d*)$/.exec(reference.trim());
  if (!match?.[1] || !match[2]) throw new Error(`некорректная ссылка «${reference}»`);
  return { row: Number(match[2]) - 1, col: columnIndex(match[1]) };
}

export function rangeName(range: CellRange, absolute = false): string {
  const start = cellName(range.start, absolute);
  const end = cellName(range.end, absolute);
  return start === end ? start : `${start}:${end}`;
}

export function parseRange(reference: string): CellRange {
  const withoutSheet = reference.trim().replace(/^.*!/, '');
  const [first, second, ...extra] = withoutSheet.split(':');
  if (!first || extra.length > 0) throw new Error(`некорректный диапазон «${reference}»`);
  const start = parseCell(first);
  const end = second ? parseCell(second) : start;
  const range = {
    start: { row: Math.min(start.row, end.row), col: Math.min(start.col, end.col) },
    end: { row: Math.max(start.row, end.row), col: Math.max(start.col, end.col) },
  };
  const count = (range.end.row - range.start.row + 1) * (range.end.col - range.start.col + 1);
  if (count > MAX_SELECTION_CELLS) throw new Error('диапазон слишком велик для одной команды');
  return range;
}

export function selection(args: EditorArgs): CellRange {
  const reference = typeof args.range === 'string' ? args.range.trim() : '';
  if (reference) return parseRange(reference);
  const row = typeof args.row === 'number' && Number.isInteger(args.row) ? args.row : 0;
  const col = typeof args.col === 'number' && Number.isInteger(args.col) ? args.col : 0;
  if (row < 0 || col < 0) throw new Error('row и col должны быть неотрицательными');
  return { start: { row, col }, end: { row, col } };
}

export function rangesIntersect(left: CellRange, right: CellRange): boolean {
  return left.start.row <= right.end.row
    && left.end.row >= right.start.row
    && left.start.col <= right.end.col
    && left.end.col >= right.start.col;
}

function relationshipId(node: Element): string {
  return node.getAttributeNS(NS.r, 'id') ?? node.getAttribute('r:id') ?? '';
}

async function packageWorkbookPath(zip: JSZip): Promise<string> {
  try {
    const relationships = await readXml(zip, '_rels/.rels');
    const relationship = descendants(relationships, 'Relationship').find(
      (node) => node.getAttribute('Type') === WORKBOOK_REL || node.getAttribute('Type')?.endsWith('/officeDocument'),
    );
    const target = relationship?.getAttribute('Target');
    if (target) return normalPath('', target);
  } catch {
    // Нестандартный пакет без корневых rels всё ещё может содержать обычный workbook.
  }
  if (zip.file('xl/workbook.xml')) return 'xl/workbook.xml';
  throw new Error('в пакете нет книги XLSX');
}

async function sharedStrings(zip: JSZip, workbookPath: string): Promise<string[]> {
  let path = 'xl/sharedStrings.xml';
  try {
    const relationships = await readXml(zip, `${workbookPath.slice(0, workbookPath.lastIndexOf('/'))}/_rels/${workbookPath.slice(workbookPath.lastIndexOf('/') + 1)}.rels`);
    const relationship = descendants(relationships, 'Relationship').find(
      (node) => node.getAttribute('Type') === SHARED_STRINGS_REL || node.getAttribute('Type')?.endsWith('/sharedStrings'),
    );
    if (relationship?.getAttribute('Target')) {
      path = normalPath(workbookPath.slice(0, workbookPath.lastIndexOf('/')), relationship.getAttribute('Target') ?? '');
    }
  } catch {
    // sharedStrings необязателен.
  }
  if (!zip.file(path)) return [];
  const document = await readXml(zip, path);
  return children(document.documentElement, 'si').map((node) => textOf(node));
}

export async function loadWorkbook(zip: JSZip): Promise<XlsxWorkbook> {
  const path = await packageWorkbookPath(zip);
  const document = await readXml(zip, path);
  let relationships: Document | null = null;
  const relsPath = `${path.slice(0, path.lastIndexOf('/'))}/_rels/${path.slice(path.lastIndexOf('/') + 1)}.rels`;
  try {
    relationships = await readXml(zip, relsPath);
  } catch {
    throw new Error('в пакете нет связей книги XLSX');
  }
  const sheetsNode = direct(document.documentElement, 'sheets');
  if (!sheetsNode) throw new Error('в книге нет списка листов');
  const sheets: XlsxSheet[] = [];
  for (const [index, node] of children(sheetsNode, 'sheet').entries()) {
    const id = relationshipId(node);
    const relationship = descendants(relationships, 'Relationship').find((item) => item.getAttribute('Id') === id);
    const target = relationship?.getAttribute('Target');
    if (!target) throw new Error(`для листа «${node.getAttribute('name') ?? id}» нет worksheet part`);
    const sheetPath = normalPath(path.slice(0, path.lastIndexOf('/')), target);
    sheets.push({
      document: await readXml(zip, sheetPath),
      path: sheetPath,
      name: node.getAttribute('name') ?? `Лист ${index + 1}`,
      sheetId: Number(node.getAttribute('sheetId')) || index + 1,
      relationshipId: id,
      index,
    });
  }
  return { document, path, sheets, sharedStrings: await sharedStrings(zip, path) };
}

export function selectSheet(workbook: XlsxWorkbook, args: EditorArgs): XlsxSheet {
  const requested = args.sheet;
  let sheet: XlsxSheet | undefined;
  if (typeof requested === 'number' && Number.isInteger(requested)) sheet = workbook.sheets[requested];
  if (typeof requested === 'string') {
    sheet = workbook.sheets.find((item) => item.name === requested);
    if (!sheet && /^\d+$/.test(requested)) sheet = workbook.sheets[Number(requested)];
  }
  if (!requested) sheet = workbook.sheets[0];
  if (!sheet) throw new Error('лист не найден');
  return sheet;
}

export function saveSheet(zip: JSZip, sheet: XlsxSheet): void {
  updateDimension(sheet.document);
  writeXml(zip, sheet.path, sheet.document);
}

export function saveWorkbook(zip: JSZip, workbook: XlsxWorkbook): void {
  writeXml(zip, workbook.path, workbook.document);
}

export function sheetData(sheet: XlsxSheet): Element {
  return ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'sheetData', [
    'sheetCalcPr', 'sheetProtection', 'protectedRanges', 'scenarios', 'autoFilter', 'sortState',
    'dataConsolidate', 'customSheetViews', 'mergeCells', 'phoneticPr', 'conditionalFormatting',
    'dataValidations', 'hyperlinks', 'printOptions', 'pageMargins', 'pageSetup', 'headerFooter',
    'rowBreaks', 'colBreaks', 'customProperties', 'cellWatches', 'ignoredErrors', 'smartTags',
    'drawing', 'legacyDrawing', 'picture', 'oleObjects', 'controls', 'tableParts', 'extLst',
  ]);
}

export function rowCells(row: Element): Element[] {
  return children(row, 'c');
}

export function findRow(sheet: XlsxSheet, index: number): Element | null {
  return children(sheetData(sheet), 'row').find((row) => Number(row.getAttribute('r')) === index + 1) ?? null;
}

export function ensureRow(sheet: XlsxSheet, index: number): Element {
  const existing = findRow(sheet, index);
  if (existing) return existing;
  const data = sheetData(sheet);
  const row = element(sheet.document, 'x', 'row');
  setAttr(row, 'r', index + 1);
  const next = children(data, 'row').find((item) => Number(item.getAttribute('r')) > index + 1);
  data.insertBefore(row, next ?? null);
  return row;
}

export function cellAddress(cell: Element): CellAddress {
  return parseCell(cell.getAttribute('r') ?? '');
}

export function findCell(sheet: XlsxSheet, address: CellAddress): Element | null {
  const row = findRow(sheet, address.row);
  return rowCells(row ?? sheet.document.createElement('unused')).find(
    (cell) => cell.getAttribute('r') === cellName(address),
  ) ?? null;
}

export function ensureCell(sheet: XlsxSheet, address: CellAddress): Element {
  const existing = findCell(sheet, address);
  if (existing) return existing;
  const row = ensureRow(sheet, address.row);
  const cell = element(sheet.document, 'x', 'c');
  setAttr(cell, 'r', cellName(address));
  const next = rowCells(row).find((item) => cellAddress(item).col > address.col);
  row.insertBefore(cell, next ?? null);
  return cell;
}

export function cellFormula(cell: Element): string {
  const formula = direct(cell, 'f')?.textContent ?? '';
  return formula ? `=${formula}` : '';
}

export function cellText(workbook: XlsxWorkbook, cell: Element | null): string {
  if (!cell) return '';
  const type = cell.getAttribute('t') ?? 'n';
  if (type === 'inlineStr') return textOf(cell);
  const value = direct(cell, 'v')?.textContent ?? '';
  if (type === 's') return workbook.sharedStrings[Number(value)] ?? '';
  if (type === 'b') return value === '1' ? 'TRUE' : 'FALSE';
  return value;
}

export function cellSnapshot(workbook: XlsxWorkbook, sheet: XlsxSheet, address: CellAddress): CellSnapshot {
  const cell = findCell(sheet, address);
  return { text: cellText(workbook, cell), formula: cell ? cellFormula(cell) : '' };
}

export function numericCellValue(workbook: XlsxWorkbook, sheet: XlsxSheet, address: CellAddress): number {
  const cell = findCell(sheet, address);
  if (!cell) return 0;
  const text = cellText(workbook, cell).trim();
  if (!text) return 0;
  const value = Number(text.replace(/\s/g, '').replace(',', '.'));
  if (!Number.isFinite(value)) throw new Error(`ячейка ${cellName(address)} не содержит число`);
  return value;
}

export function clearCellContents(cell: Element): boolean {
  const hadContent = Boolean(direct(cell, 'f') || direct(cell, 'v') || direct(cell, 'is') || cell.hasAttribute('t'));
  removeChildren(cell, 'f');
  removeChildren(cell, 'v');
  removeChildren(cell, 'is');
  cell.removeAttribute('t');
  return hadContent;
}

export function setCellText(sheet: XlsxSheet, address: CellAddress, text: string): Element {
  const cell = ensureCell(sheet, address);
  clearCellContents(cell);
  if (text.startsWith('=')) {
    const formula = element(sheet.document, 'x', 'f', text.slice(1));
    cell.appendChild(formula);
    return cell;
  }
  setAttr(cell, 't', 'inlineStr');
  const inline = element(sheet.document, 'x', 'is');
  const value = element(sheet.document, 'x', 't', text);
  if (/^\s|\s$/.test(text)) value.setAttributeNS(XML_SPACE, 'xml:space', 'preserve');
  inline.appendChild(value);
  cell.appendChild(inline);
  return cell;
}

export function setCellNumber(sheet: XlsxSheet, address: CellAddress, value: number): Element {
  const cell = ensureCell(sheet, address);
  clearCellContents(cell);
  cell.appendChild(element(sheet.document, 'x', 'v', String(value)));
  return cell;
}

export function updateDimension(document: Document): void {
  const data = direct(document.documentElement, 'sheetData');
  const cells = data ? descendants(data, 'c') : [];
  const addresses = cells.map(cellAddress).filter((address) => address.row >= 0 && address.col >= 0);
  let dimension = direct(document.documentElement, 'dimension');
  if (addresses.length === 0) {
    if (dimension) setAttr(dimension, 'ref', 'A1');
    return;
  }
  if (!dimension) {
    dimension = element(document, 'x', 'dimension');
    document.documentElement.insertBefore(dimension, document.documentElement.firstChild);
  }
  const range = {
    start: {
      row: Math.min(...addresses.map((address) => address.row)),
      col: Math.min(...addresses.map((address) => address.col)),
    },
    end: {
      row: Math.max(...addresses.map((address) => address.row)),
      col: Math.max(...addresses.map((address) => address.col)),
    },
  };
  setAttr(dimension, 'ref', rangeName(range));
}

export function quoteSheetName(name: string): string {
  return `'${name.replace(/'/g, "''")}'`;
}

export async function appendBlankSheet(
  zip: JSZip,
  workbook: XlsxWorkbook,
  requestedName: string,
): Promise<XlsxSheet> {
  const base = requestedName.trim().replace(/[\\/*?:[\]]/g, '_').slice(0, 31) || 'Лист';
  let name = base;
  let suffix = 2;
  while (workbook.sheets.some((sheet) => sheet.name.toLowerCase() === name.toLowerCase())) {
    name = `${base.slice(0, 31 - String(suffix).length - 1)} ${suffix}`;
    suffix += 1;
  }
  const path = nextPartName(zip, 'xl/worksheets', 'sheet', 'xml');
  const document = await import('./xml.js').then(({ parseXml }) => parseXml(
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="${NS.x}" xmlns:r="${NS.r}"><dimension ref="A1"/><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><sheetData/></worksheet>`,
  ));
  writeXml(zip, path, document);
  await ensureContentType(zip, path, 'application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml');
  const id = await ensureRelationship(zip, workbook.path, WORKSHEET_REL, relativeTarget(workbook.path, path));
  const sheets = direct(workbook.document.documentElement, 'sheets');
  if (!sheets) throw new Error('в книге нет списка листов');
  const sheetId = Math.max(0, ...workbook.sheets.map((sheet) => sheet.sheetId)) + 1;
  const node = element(workbook.document, 'x', 'sheet');
  setAttr(node, 'name', name);
  setAttr(node, 'sheetId', sheetId);
  setNsAttr(node, 'r', 'id', id);
  sheets.appendChild(node);
  const sheet = { document, path, name, sheetId, relationshipId: id, index: workbook.sheets.length };
  workbook.sheets.push(sheet);
  saveWorkbook(zip, workbook);
  return sheet;
}

export function formulaReferences(formula: string): string[] {
  const result = new Set<string>();
  const pattern = /(?:'[^']+'|[A-Za-z_][\w.]*)?!?\$?([A-Za-z]{1,3})\$?([1-9]\d*)(?::\$?([A-Za-z]{1,3})\$?([1-9]\d*))?/g;
  for (const match of formula.replace(/"[^"]*"/g, '""').matchAll(pattern)) {
    const start = parseCell(`${match[1]}${match[2]}`);
    const end = match[3] && match[4] ? parseCell(`${match[3]}${match[4]}`) : start;
    const rowCount = Math.abs(end.row - start.row) + 1;
    const colCount = Math.abs(end.col - start.col) + 1;
    if (rowCount * colCount > 10_000) {
      result.add(rangeName({ start, end }));
      continue;
    }
    for (let row = Math.min(start.row, end.row); row <= Math.max(start.row, end.row); row += 1) {
      for (let col = Math.min(start.col, end.col); col <= Math.max(start.col, end.col); col += 1) {
        result.add(cellName({ row, col }));
      }
    }
  }
  return [...result];
}

function shiftedReference(reference: string, origin: CellAddress, mode: string): string {
  return reference.replace(/(\$?)([A-Za-z]{1,3})(\$?)([1-9]\d*)/g, (whole, absoluteCol: string, letters: string, absoluteRow: string, digits: string) => {
    let row = Number(digits) - 1;
    let col = columnIndex(letters);
    if (mode === 'right' && col >= origin.col) col += 1;
    if (mode === 'left' && col > origin.col) col -= 1;
    if (mode === 'down' && row >= origin.row) row += 1;
    if (mode === 'up' && row > origin.row) row -= 1;
    return `${absoluteCol}${columnName(col)}${absoluteRow}${row + 1}`;
  });
}

function shiftFormula(formula: string, origin: CellAddress, mode: string): string {
  return formula.split(/("[^"]*")/g).map((part, index) => (
    index % 2 === 1 ? part : shiftedReference(part, origin, mode)
  )).join('');
}

export function shiftCells(sheet: XlsxSheet, address: CellAddress, mode: string): boolean {
  const data = sheetData(sheet);
  const horizontal = mode === 'right' || mode === 'left';
  const deleting = mode === 'left' || mode === 'up';
  if (!['right', 'left', 'down', 'up'].includes(mode)) throw new Error('shift должен быть right, left, down или up');
  let changed = false;
  const rows = children(data, 'row');
  if (deleting) {
    const target = findCell(sheet, address);
    if (target) {
      target.parentNode?.removeChild(target);
      changed = true;
    }
  } else {
    ensureCell(sheet, address);
    changed = true;
  }
  const moving: Array<{ cell: Element; from: CellAddress; to: CellAddress }> = [];
  for (const row of rows) {
    for (const cell of rowCells(row)) {
      const current = cellAddress(cell);
      const inMovedArea = horizontal
        ? current.row === address.row && (deleting ? current.col > address.col : current.col > address.col)
        : current.col === address.col && (deleting ? current.row > address.row : current.row > address.row);
      if (!inMovedArea) continue;
      const to = {
        row: current.row + (horizontal ? 0 : deleting ? -1 : 1),
        col: current.col + (horizontal ? deleting ? -1 : 1 : 0),
      };
      moving.push({ cell, from: current, to });
    }
  }
  const ordered = deleting ? moving : [...moving].reverse();
  for (const item of ordered) {
    item.cell.setAttribute('r', cellName(item.to));
    const row = ensureRow(sheet, item.to.row);
    if (item.cell.parentNode !== row) row.appendChild(item.cell);
    changed = true;
  }
  for (const row of children(data, 'row')) {
    const cells = rowCells(row);
    if (cells.length === 0) {
      data.removeChild(row);
      continue;
    }
    cells.sort((left, right) => cellAddress(left).col - cellAddress(right).col);
    for (const cell of cells) row.appendChild(cell);
    const first = cells[0];
    if (first) row.setAttribute('r', String(cellAddress(first).row + 1));
  }
  const sortedRows = children(data, 'row').sort((left, right) => Number(left.getAttribute('r')) - Number(right.getAttribute('r')));
  for (const row of sortedRows) data.appendChild(row);
  for (const formula of descendants(sheet.document.documentElement, 'f')) {
    formula.textContent = shiftFormula(formula.textContent ?? '', address, mode);
  }
  return changed;
}

export function sortRange(
  workbook: XlsxWorkbook,
  sheet: XlsxSheet,
  range: CellRange,
  column: number,
  descending: boolean,
): boolean {
  if (!Number.isInteger(column) || column < 0 || column >= range.end.col - range.start.col + 1) {
    throw new Error('column должен быть нулевым индексом внутри диапазона');
  }
  interface SortRow {
    row: Element;
    cells: Element[];
    key: string;
    index: number;
  }
  const rows: SortRow[] = [];
  for (let rowIndex = range.start.row; rowIndex <= range.end.row; rowIndex += 1) {
    const row = ensureRow(sheet, rowIndex);
    const cells: Element[] = [];
    for (let col = range.start.col; col <= range.end.col; col += 1) {
      const existing = findCell(sheet, { row: rowIndex, col });
      cells.push(existing ? clone(existing) : element(sheet.document, 'x', 'c'));
    }
    const keyCell = findCell(sheet, { row: rowIndex, col: range.start.col + column });
    rows.push({ row, cells, key: cellText(workbook, keyCell), index: rows.length });
  }
  const sorted = [...rows].sort((left, right) => {
    if (!left.key && !right.key) return left.index - right.index;
    if (!left.key) return 1;
    if (!right.key) return -1;
    const leftNumber = Number(left.key.replace(/\s/g, '').replace(',', '.'));
    const rightNumber = Number(right.key.replace(/\s/g, '').replace(',', '.'));
    const comparison = Number.isFinite(leftNumber) && Number.isFinite(rightNumber)
      ? leftNumber - rightNumber
      : left.key.localeCompare(right.key, undefined, { numeric: true, sensitivity: 'base' });
    return (descending ? -comparison : comparison) || left.index - right.index;
  });
  const changed = sorted.some((item, index) => item !== rows[index]);
  if (!changed) return false;
  for (const [index, item] of sorted.entries()) {
    const rowIndex = range.start.row + index;
    const row = rows[index]?.row;
    if (!row) continue;
    for (const cell of rowCells(row)) {
      const address = cellAddress(cell);
      if (address.col >= range.start.col && address.col <= range.end.col) row.removeChild(cell);
    }
    for (const [offset, source] of item.cells.entries()) {
      const col = range.start.col + offset;
      const cell = source;
      cell.setAttribute('r', cellName({ row: rowIndex, col }));
      if (!direct(cell, 'f') && !direct(cell, 'v') && !direct(cell, 'is') && !cell.hasAttribute('s')) continue;
      row.appendChild(cell);
    }
    const cells = rowCells(row).sort((left, right) => cellAddress(left).col - cellAddress(right).col);
    for (const cell of cells) row.appendChild(cell);
  }
  return true;
}
