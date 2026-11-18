import {
  NS,
  children,
  descendants,
  direct,
  element,
  ensureDirect,
  removeChildren,
  setAttr,
  setNsAttr,
  writeXml,
} from './xml.js';
import { ensureRelationship } from './package.js';
import { booleanArg, numberArg, requiredString, stringArg } from './commands.js';
import type { CommandResult, EditorContext } from './commands.js';
import type { CellRange, XlsxSheet, XlsxWorkbook } from './xlsx_model.js';
import {
  appendBlankSheet,
  cellFormula,
  cellName,
  cellSnapshot,
  cellText,
  clearCellContents,
  findCell,
  formulaReferences,
  loadWorkbook,
  parseCell,
  parseRange,
  quoteSheetName,
  rangeName,
  rangesIntersect,
  saveSheet,
  saveWorkbook,
  selectSheet,
  selection,
  setCellText,
  sheetData,
  shiftCells,
  sortRange,
  numericCellValue,
} from './xlsx_model.js';
import { applyStyleCommand, copyFormat, pasteFormat } from './xlsx_style.js';
import { evaluateFormula } from './xlsx_formula.js';
import { addComment, createPivot, createTable, insertChart, insertPicture } from './xlsx_objects.js';

const HYPERLINK_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink';
const STYLE_COMMANDS = new Set([
  'font-family', 'font-size', 'text-color', 'highlight', 'toggle-bold', 'toggle-italic',
  'toggle-underline', 'toggle-strikethrough', 'borders', 'align', 'wrap-text', 'indent',
  'number-format', 'style',
]);

function createBlankDocument(context: EditorContext): void {
  const { zip } = context;
  for (const path of Object.keys(zip.files)) zip.remove(path);
  zip.file('[Content_Types].xml', `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="${NS.ct}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>`);
  zip.file('_rels/.rels', `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="${NS.rel}"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>`);
  zip.file('xl/workbook.xml', `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="${NS.x}" xmlns:r="${NS.r}"><sheets><sheet name="Лист1" sheetId="1" r:id="rId1"/></sheets><calcPr calcId="0"/></workbook>`);
  zip.file('xl/_rels/workbook.xml.rels', `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="${NS.rel}"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>`);
  zip.file('xl/worksheets/sheet1.xml', `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="${NS.x}" xmlns:r="${NS.r}"><dimension ref="A1"/><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><sheetData/></worksheet>`);
}

async function newDocument(context: EditorContext): Promise<void> {
  let workbook: XlsxWorkbook;
  try {
    workbook = await loadWorkbook(context.zip);
  } catch {
    createBlankDocument(context);
    return;
  }
  const first = workbook.sheets[0];
  if (!first) throw new Error('в книге нет листов');
  const sheets = direct(workbook.document.documentElement, 'sheets');
  if (!sheets) throw new Error('в книге нет списка листов');
  for (const [index, sheet] of children(sheets, 'sheet').entries()) {
    if (index > 0) sheets.removeChild(sheet);
  }
  const data = sheetData(first);
  removeChildren(data, 'row');
  for (const name of [
    'autoFilter', 'sortState', 'mergeCells', 'conditionalFormatting', 'dataValidations', 'hyperlinks',
    'drawing', 'legacyDrawing', 'picture', 'oleObjects', 'controls', 'tableParts',
  ]) {
    removeChildren(first.document.documentElement, name);
  }
  for (const view of descendants(first.document.documentElement, 'sheetView')) removeChildren(view, 'pane');
  removeChildren(workbook.document.documentElement, 'definedNames');
  saveSheet(context.zip, first);
  saveWorkbook(context.zip, workbook);
}

function commandValue(context: EditorContext): string {
  return stringArg(context.args, 'value', stringArg(context.args, 'text'));
}

function fullRecalculation(workbook: XlsxWorkbook): void {
  const properties = ensureDirect(workbook.document, workbook.document.documentElement, 'x', 'calcPr', ['oleSize', 'customWorkbookViews', 'pivotCaches', 'smartTagPr', 'smartTagTypes', 'webPublishing', 'fileRecoveryPr', 'webPublishObjects', 'extLst']);
  setAttr(properties, 'fullCalcOnLoad', 1);
  setAttr(properties, 'forceFullCalc', 1);
  setAttr(properties, 'calcMode', 'auto');
}

function setFormula(context: EditorContext, workbook: XlsxWorkbook, sheet: XlsxSheet, formula: string): void {
  const address = selection(context.args).start;
  setCellText(sheet, address, formula.startsWith('=') ? formula : `=${formula}`);
  fullRecalculation(workbook);
  saveSheet(context.zip, sheet);
  saveWorkbook(context.zip, workbook);
}

function inferredSumRange(workbook: XlsxWorkbook, sheet: XlsxSheet, range: CellRange): CellRange | null {
  const hasNumber = (row: number, col: number): boolean => {
    if (row < 0 || col < 0) return false;
    const text = cellText(workbook, findCell(sheet, { row, col })).trim();
    return Boolean(text) && Number.isFinite(Number(text.replace(/\s/g, '').replace(',', '.')));
  };
  const target = range.start;
  if (hasNumber(target.row - 1, target.col)) {
    let start = target.row - 1;
    while (hasNumber(start - 1, target.col)) start -= 1;
    return { start: { row: start, col: target.col }, end: { row: target.row - 1, col: target.col } };
  }
  if (hasNumber(target.row, target.col - 1)) {
    let start = target.col - 1;
    while (hasNumber(target.row, start - 1)) start -= 1;
    return { start: { row: target.row, col: start }, end: { row: target.row, col: target.col - 1 } };
  }
  return null;
}

function parseDelimited(text: string, delimiter: string): string[][] {
  const separator = delimiter === '\\t' || delimiter.toLowerCase() === 'tab' ? '\t' : delimiter || ',';
  if (separator.length !== 1) throw new Error('delimiter должен быть одним символом');
  const rows: string[][] = [];
  let row: string[] = [];
  let value = '';
  let quoted = false;
  for (let index = 0; index <= text.length; index += 1) {
    const char = index === text.length ? '\n' : text[index] ?? '';
    if (quoted) {
      if (char === '"' && text[index + 1] === '"') {
        value += '"';
        index += 1;
      } else if (char === '"') quoted = false;
      else value += char;
      continue;
    }
    if (char === '"' && value.length === 0) quoted = true;
    else if (char === separator) {
      row.push(value);
      value = '';
    } else if (char === '\n' || char === '\r') {
      if (char === '\r' && text[index + 1] === '\n') index += 1;
      row.push(value);
      if (row.some((cell) => cell.length > 0) || index < text.length) rows.push(row);
      row = [];
      value = '';
    } else value += char;
  }
  if (quoted) throw new Error('незакрытая кавычка в импортируемом тексте');
  return rows;
}

async function importText(context: EditorContext, workbook: XlsxWorkbook): Promise<XlsxSheet> {
  const text = stringArg(context.args, 'text');
  const rows = parseDelimited(text, stringArg(context.args, 'delimiter', ','));
  const sheet = await appendBlankSheet(context.zip, workbook, 'Импорт');
  rows.forEach((row, rowIndex) => {
    row.forEach((value, col) => setCellText(sheet, { row: rowIndex, col }, value));
  });
  saveSheet(context.zip, sheet);
  return sheet;
}

function mergeCells(context: EditorContext, sheet: XlsxSheet): boolean {
  const range = selection(context.args);
  const merges = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'mergeCells', [
    'phoneticPr', 'conditionalFormatting', 'dataValidations', 'hyperlinks', 'printOptions', 'pageMargins',
    'pageSetup', 'headerFooter', 'drawing', 'legacyDrawing', 'tableParts', 'extLst',
  ]);
  const reference = rangeName(range);
  for (const merge of children(merges, 'mergeCell')) {
    const existing = merge.getAttribute('ref') ?? '';
    if (existing === reference) return false;
    if (rangesIntersect(parseRange(existing), range)) throw new Error(`диапазон пересекается с объединением ${existing}`);
  }
  const node = element(sheet.document, 'x', 'mergeCell');
  setAttr(node, 'ref', reference);
  merges.appendChild(node);
  setAttr(merges, 'count', children(merges, 'mergeCell').length);
  saveSheet(context.zip, sheet);
  return true;
}

function setFilter(context: EditorContext, sheet: XlsxSheet): boolean {
  const reference = rangeName(selection(context.args));
  let filter = direct(sheet.document.documentElement, 'autoFilter');
  if (filter?.getAttribute('ref') === reference) return false;
  if (!filter) {
    filter = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'autoFilter', [
      'sortState', 'dataConsolidate', 'customSheetViews', 'mergeCells', 'phoneticPr',
      'conditionalFormatting', 'dataValidations', 'hyperlinks', 'printOptions', 'pageMargins',
      'pageSetup', 'headerFooter', 'drawing', 'tableParts', 'extLst',
    ]);
  }
  setAttr(filter, 'ref', reference);
  saveSheet(context.zip, sheet);
  return true;
}

function clearCells(context: EditorContext, sheet: XlsxSheet): boolean {
  const range = selection(context.args);
  let changed = false;
  for (let row = range.start.row; row <= range.end.row; row += 1) {
    for (let col = range.start.col; col <= range.end.col; col += 1) {
      const cell = findCell(sheet, { row, col });
      if (!cell) continue;
      changed = clearCellContents(cell) || changed;
      if (!cell.hasAttribute('s') && children(cell).length === 0) {
        cell.parentNode?.removeChild(cell);
        changed = true;
      }
    }
  }
  if (changed) saveSheet(context.zip, sheet);
  return changed;
}


function pageMargins(context: EditorContext, sheet: XlsxSheet): boolean {
  const margins = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'pageMargins', [
    'pageSetup', 'headerFooter', 'rowBreaks', 'colBreaks', 'drawing', 'legacyDrawing', 'tableParts', 'extLst',
  ]);
  let changed = false;
  for (const name of ['top', 'right', 'bottom', 'left', 'header', 'footer']) {
    if (typeof context.args[name] !== 'number') continue;
    const value = numberArg(context.args, name, 0);
    if (value < 0) throw new Error('поля страницы не могут быть отрицательными');
    if (margins.getAttribute(name) !== String(value)) changed = true;
    setAttr(margins, name, value);
  }
  if (changed) saveSheet(context.zip, sheet);
  return changed;
}

function pageSetup(context: EditorContext, sheet: XlsxSheet, kind: 'orientation' | 'size'): boolean {
  const value = requiredString(context.args, 'value');
  const setup = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'pageSetup', [
    'headerFooter', 'rowBreaks', 'colBreaks', 'drawing', 'legacyDrawing', 'tableParts', 'extLst',
  ]);
  if (kind === 'orientation') {
    if (!['portrait', 'landscape'].includes(value)) throw new Error('orientation должен быть portrait или landscape');
    if (setup.getAttribute('orientation') === value) return false;
    setAttr(setup, 'orientation', value);
  } else {
    const sizes: Record<string, number> = { letter: 1, legal: 5, a3: 8, a4: 9, a5: 11, b4: 12, b5: 13 };
    const paperSize = /^\d+$/.test(value) ? Number(value) : sizes[value.toLowerCase()];
    if (!paperSize || paperSize < 1 || paperSize > 118) throw new Error(`неподдерживаемый размер бумаги «${value}»`);
    if (setup.getAttribute('paperSize') === String(paperSize)) return false;
    setAttr(setup, 'paperSize', paperSize);
  }
  saveSheet(context.zip, sheet);
  return true;
}

function printArea(context: EditorContext, workbook: XlsxWorkbook, sheet: XlsxSheet): boolean {
  const range = selection(context.args);
  const reference = `${quoteSheetName(sheet.name)}!${rangeName(range, true)}`;
  const names = ensureDirect(workbook.document, workbook.document.documentElement, 'x', 'definedNames', [
    'calcPr', 'oleSize', 'customWorkbookViews', 'pivotCaches', 'smartTagPr', 'smartTagTypes',
    'webPublishing', 'fileRecoveryPr', 'webPublishObjects', 'extLst',
  ]);
  let existing = children(names, 'definedName').find(
    (node) => node.getAttribute('name') === '_xlnm.Print_Area'
      && Number(node.getAttribute('localSheetId') ?? -1) === sheet.index,
  );
  if (existing?.textContent === reference) return false;
  if (!existing) {
    existing = element(workbook.document, 'x', 'definedName');
    setAttr(existing, 'name', '_xlnm.Print_Area');
    setAttr(existing, 'localSheetId', sheet.index);
    names.appendChild(existing);
  }
  existing.textContent = reference;
  saveWorkbook(context.zip, workbook);
  return true;
}

function freeze(context: EditorContext, sheet: XlsxSheet): boolean {
  const row = numberArg(context.args, 'row', 0);
  const col = numberArg(context.args, 'col', 0);
  if (!Number.isInteger(row) || !Number.isInteger(col) || row < 0 || col < 0) throw new Error('row и col должны быть неотрицательными целыми');
  const existingViews = direct(sheet.document.documentElement, 'sheetViews');
  const existingView = existingViews ? direct(existingViews, 'sheetView') : null;
  if (row === 0 && col === 0 && !existingView?.hasChildNodes()) return false;
  if (row === 0 && col === 0 && !direct(existingView ?? sheet.document.documentElement, 'pane')) {
    const hasPane = existingView ? children(existingView, 'pane').length > 0 : false;
    if (!hasPane) return false;
  }
  const views = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'sheetViews', [
    'sheetFormatPr', 'cols', 'sheetData', 'sheetCalcPr', 'sheetProtection', 'autoFilter', 'mergeCells',
    'hyperlinks', 'pageMargins', 'pageSetup', 'drawing', 'tableParts', 'extLst',
  ]);
  let view = direct(views, 'sheetView');
  if (!view) {
    view = element(sheet.document, 'x', 'sheetView');
    setAttr(view, 'workbookViewId', 0);
    views.appendChild(view);
  }
  for (const pane of children(view, 'pane')) view.removeChild(pane);
  if (row === 0 && col === 0) {
    saveSheet(context.zip, sheet);
    return true;
  }
  const pane = element(sheet.document, 'x', 'pane');
  if (col > 0) setAttr(pane, 'xSplit', col);
  if (row > 0) setAttr(pane, 'ySplit', row);
  setAttr(pane, 'topLeftCell', cellName({ row, col }));
  setAttr(pane, 'activePane', row === 0 ? 'topRight' : col === 0 ? 'bottomLeft' : 'bottomRight');
  setAttr(pane, 'state', 'frozen');
  view.insertBefore(pane, view.firstChild);
  saveSheet(context.zip, sheet);
  return true;
}

export async function executeXlsx(context: EditorContext, command: string): Promise<CommandResult> {
  if (command === 'new-document') {
    await newDocument(context);
    return { changed: true };
  }
  const workbook = await loadWorkbook(context.zip);
  const sheet = selectSheet(workbook, context.args);
  const range = selection(context.args);

  if (command === 'get-cell') {
    return { value: cellSnapshot(workbook, sheet, range.start) };
  }
  if (command === 'set-cell') {
    setCellText(sheet, range.start, commandValue(context));
    if (commandValue(context).startsWith('=')) {
      fullRecalculation(workbook);
      saveWorkbook(context.zip, workbook);
    }
    saveSheet(context.zip, sheet);
    return { changed: true };
  }
  if (command === 'copy-format') {
    const id = copyFormat(sheet, range.start.row, range.start.col);
    context.clipboardFormat = 'xlsx-style-id';
    context.clipboardText = String(id);
    return { value: { styleId: id } };
  }
  if (command === 'paste-format') {
    const changed = await pasteFormat(context.zip, sheet, range);
    if (changed) saveSheet(context.zip, sheet);
    return { changed };
  }
  if (STYLE_COMMANDS.has(command)) {
    const changed = await applyStyleCommand(
      context.zip,
      sheet,
      range,
      command,
      commandValue(context),
      numberArg(context.args, 'delta', 0),
      stringArg(context.args, 'code'),
    );
    if (changed) saveSheet(context.zip, sheet);
    return { changed };
  }
  if (command === 'merge') return { changed: mergeCells(context, sheet) };
  if (command === 'insert-cell' || command === 'delete-cell') {
    const shift = requiredString(context.args, 'shift');
    const changed = shiftCells(sheet, range.start, shift);
    if (changed) {
      fullRecalculation(workbook);
      saveSheet(context.zip, sheet);
      saveWorkbook(context.zip, workbook);
    }
    return { changed };
  }
  if (command === 'autosum' || command === 'insert-function') {
    const inferred = inferredSumRange(workbook, sheet, range);
    let formula = command === 'autosum' ? 'SUM' : requiredString(context.args, 'value');
    if (!formula.includes('(')) {
      if (!inferred) throw new Error('рядом с ячейкой нет чисел для автосуммы');
      formula = `${formula.replace(/^=/, '')}(${rangeName(inferred)})`;
    }
    setFormula(context, workbook, sheet, formula);
    return { changed: true, value: { formula: formula.startsWith('=') ? formula : `=${formula}` } };
  }
  if (command === 'financial-function') {
    const name = requiredString(context.args, 'value').toUpperCase();
    if (!['PMT', 'FV', 'PV'].includes(name)) throw new Error('financial-function поддерживает PMT, FV и PV');
    const defaults: Record<string, string> = {
      PMT: '0.05/12,60,10000', FV: '0.05/12,60,-1000', PV: '0.05/12,60,-1000',
    };
    const args = stringArg(context.args, 'args', defaults[name] ?? '');
    setFormula(context, workbook, sheet, `${name}(${args})`);
    return { changed: true };
  }
  if (command === 'precedents') {
    const cell = findCell(sheet, range.start);
    return { value: cell ? formulaReferences(cellFormula(cell)) : [] };
  }
  if (command === 'evaluate-formula') {
    const value = evaluateFormula(
      requiredString(context.args, 'value'),
      (reference) => numericCellValue(workbook, sheet, parseCell(reference)),
      (reference) => {
        const referenced = parseRange(reference);
        const values: number[] = [];
        for (let row = referenced.start.row; row <= referenced.end.row; row += 1) {
          for (let col = referenced.start.col; col <= referenced.end.col; col += 1) {
            values.push(numericCellValue(workbook, sheet, { row, col }));
          }
        }
        return values;
      },
    );
    return { value };
  }
  if (command === 'sort') {
    const changed = sortRange(workbook, sheet, range, numberArg(context.args, 'column', 0), booleanArg(context.args, 'descending', false));
    if (changed) saveSheet(context.zip, sheet);
    return { changed };
  }
  if (command === 'filter') return { changed: setFilter(context, sheet) };
  if (command === 'clear') return { changed: clearCells(context, sheet) };
  if (command === 'import-text') {
    const imported = await importText(context, workbook);
    return { changed: true, value: { sheet: imported.name } };
  }
  if (command === 'create-table') {
    const part = await createTable(context.zip, workbook, sheet, range, requiredString(context.args, 'name'));
    return { changed: true, value: { part } };
  }
  if (command === 'create-pivot') {
    const pivot = await createPivot(context.zip, workbook, sheet, range, stringArg(context.args, 'name'));
    return {
      changed: true,
      value: { sheet: pivot.name, kind: 'summary' },
      message: 'Создан настоящий сводный лист с агрегатами; pivotCache не создавался.',
    };
  }
  if (command === 'insert-picture') {
    const bytes = context.args.bytes;
    if (!(bytes instanceof ArrayBuffer)) throw new Error('bytes должен быть ArrayBuffer');
    const part = await insertPicture(
      context.zip,
      sheet,
      range.start,
      bytes,
      requiredString(context.args, 'mime'),
      numberArg(context.args, 'width', 320),
      numberArg(context.args, 'height', 240),
      stringArg(context.args, 'name'),
    );
    return { changed: true, value: { part } };
  }
  if (command === 'insert-chart') {
    const part = await insertChart(context.zip, workbook, sheet, range, stringArg(context.args, 'title'));
    return { changed: true, value: { part } };
  }
  if (command === 'insert-link') {
    const url = requiredString(context.args, 'url');
    const address = range.start;
    setCellText(sheet, address, stringArg(context.args, 'text', url));
    const id = await ensureRelationship(context.zip, sheet.path, HYPERLINK_REL, url, true);
    const links = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'hyperlinks', [
      'printOptions', 'pageMargins', 'pageSetup', 'headerFooter', 'rowBreaks', 'colBreaks',
      'drawing', 'legacyDrawing', 'tableParts', 'extLst',
    ]);
    for (const link of children(links, 'hyperlink')) {
      if (link.getAttribute('ref') === cellName(address)) links.removeChild(link);
    }
    const link = element(sheet.document, 'x', 'hyperlink');
    setAttr(link, 'ref', cellName(address));
    setNsAttr(link, 'r', 'id', id);
    links.appendChild(link);
    saveSheet(context.zip, sheet);
    return { changed: true };
  }
  if (command === 'page-margins') return { changed: pageMargins(context, sheet) };
  if (command === 'page-orientation') return { changed: pageSetup(context, sheet, 'orientation') };
  if (command === 'page-size') return { changed: pageSetup(context, sheet, 'size') };
  if (command === 'print-area') return { changed: printArea(context, workbook, sheet) };
  if (command === 'freeze') return { changed: freeze(context, sheet) };
  if (command === 'add-comment') {
    await addComment(context.zip, sheet, range.start, stringArg(context.args, 'text'));
    return { changed: true };
  }
  throw new Error(`неизвестная команда XLSX «${command}»`);
}

