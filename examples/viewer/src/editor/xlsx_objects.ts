import type JSZip from 'jszip';
import {
  NS,
  append,
  children,
  descendants,
  direct,
  dirName,
  element,
  ensureDirect,
  normalPath,
  parseXml,
  readXml,
  relsPath,
  setAttr,
  setNsAttr,
  writeXml,
} from './xml.js';
import {
  ensureContentType,
  ensureDefaultExtension,
  ensureRelationship,
  nextPartName,
  relativeTarget,
} from './package.js';
import type { CellAddress, CellRange, XlsxSheet, XlsxWorkbook } from './xlsx_model.js';
import {
  appendBlankSheet,
  cellName,
  cellText,
  findCell,
  quoteSheetName,
  rangeName,
  saveSheet,
  setCellNumber,
  setCellText,
} from './xlsx_model.js';

const DRAWING_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing';
const IMAGE_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/image';
const CHART_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart';
const TABLE_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/table';
const COMMENTS_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments';
const VML_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing';
const DRAWING_CONTENT_TYPE = 'application/vnd.openxmlformats-officedocument.drawing+xml';
const CHART_CONTENT_TYPE = 'application/vnd.openxmlformats-officedocument.drawingml.chart+xml';
const TABLE_CONTENT_TYPE = 'application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml';
const COMMENTS_CONTENT_TYPE = 'application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml';
const VML_NS = 'urn:schemas-microsoft-com:vml';
const OFFICE_NS = 'urn:schemas-microsoft-com:office:office';
const EXCEL_NS = 'urn:schemas-microsoft-com:office:excel';

function relationshipTarget(zip: JSZip, source: string, suffix: string): Promise<string | null> {
  return readXml(zip, relsPath(source)).then((document) => {
    const relationship = descendants(document, 'Relationship').find(
      (node) => node.getAttribute('Type')?.endsWith(suffix),
    );
    const target = relationship?.getAttribute('Target');
    return target ? normalPath(dirName(source), target) : null;
  }).catch(() => null);
}

async function createDrawing(zip: JSZip, sheet: XlsxSheet): Promise<{ document: Document; path: string }> {
  const existingPath = await relationshipTarget(zip, sheet.path, '/drawing');
  if (existingPath && zip.file(existingPath)) {
    return { document: await readXml(zip, existingPath), path: existingPath };
  }
  const path = nextPartName(zip, 'xl/drawings', 'drawing', 'xml');
  const document = parseXml(
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="${NS.xdr}" xmlns:a="${NS.a}" xmlns:r="${NS.r}"/>`,
  );
  writeXml(zip, path, document);
  await ensureContentType(zip, path, DRAWING_CONTENT_TYPE);
  const id = await ensureRelationship(zip, sheet.path, DRAWING_REL, relativeTarget(sheet.path, path));
  const drawing = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'drawing', [
    'legacyDrawing', 'legacyDrawingHF', 'picture', 'oleObjects', 'controls', 'tableParts', 'extLst',
  ]);
  setNsAttr(drawing, 'r', 'id', id);
  return { document, path };
}

function appendAnchor(document: Document): Element {
  const anchor = element(document, 'xdr', 'oneCellAnchor');
  const from = element(document, 'xdr', 'from');
  for (const name of ['col', 'colOff', 'row', 'rowOff']) from.appendChild(element(document, 'xdr', name, '0'));
  anchor.appendChild(from);
  document.documentElement.appendChild(anchor);
  return anchor;
}

function setAnchorPosition(document: Document, anchor: Element, address: CellAddress, width: number, height: number): void {
  const from = direct(anchor, 'from');
  if (!from) throw new Error('в drawing нет начальной позиции');
  const values: Record<string, number> = { col: address.col, colOff: 0, row: address.row, rowOff: 0 };
  for (const child of children(from)) child.textContent = String(values[child.localName] ?? 0);
  let extent = direct(anchor, 'ext');
  if (!extent) {
    extent = element(document, 'xdr', 'ext');
    anchor.insertBefore(extent, from.nextSibling);
  }
  setAttr(extent, 'cx', Math.max(1, Math.round(width * 9525)));
  setAttr(extent, 'cy', Math.max(1, Math.round(height * 9525)));
}

function extensionForMime(mime: string): string {
  const normalized = mime.toLowerCase().split(';')[0]?.trim();
  const extensions: Record<string, string> = {
    'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/bmp': 'bmp',
    'image/tiff': 'tiff', 'image/svg+xml': 'svg', 'image/webp': 'webp',
  };
  const extension = normalized ? extensions[normalized] : undefined;
  if (!extension) throw new Error(`неподдерживаемый MIME изображения «${mime}»`);
  return extension;
}

export async function insertPicture(
  zip: JSZip,
  sheet: XlsxSheet,
  address: CellAddress,
  bytes: ArrayBuffer,
  mime: string,
  width: number,
  height: number,
  name: string,
): Promise<string> {
  if (!(bytes instanceof ArrayBuffer) || bytes.byteLength === 0) throw new Error('bytes должен содержать непустой ArrayBuffer');
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    throw new Error('width и height должны быть положительными числами');
  }
  const extension = extensionForMime(mime);
  const imagePath = nextPartName(zip, 'xl/media', 'image', extension);
  zip.file(imagePath, bytes);
  await ensureDefaultExtension(zip, extension, mime.split(';')[0]?.trim() || `image/${extension}`);
  const drawing = await createDrawing(zip, sheet);
  const imageId = await ensureRelationship(zip, drawing.path, IMAGE_REL, relativeTarget(drawing.path, imagePath));
  const anchor = appendAnchor(drawing.document);
  setAnchorPosition(drawing.document, anchor, address, width, height);
  const picture = element(drawing.document, 'xdr', 'pic');
  const nonVisual = element(drawing.document, 'xdr', 'nvPicPr');
  const properties = element(drawing.document, 'xdr', 'cNvPr');
  const id = descendants(drawing.document, 'cNvPr').reduce((maximum, node) => Math.max(maximum, Number(node.getAttribute('id')) || 0), 0) + 1;
  setAttr(properties, 'id', id);
  setAttr(properties, 'name', name.trim() || `Изображение ${id}`);
  append(nonVisual, properties, element(drawing.document, 'xdr', 'cNvPicPr'));
  const fill = element(drawing.document, 'xdr', 'blipFill');
  const blip = element(drawing.document, 'a', 'blip');
  setNsAttr(blip, 'r', 'embed', imageId);
  const stretch = element(drawing.document, 'a', 'stretch');
  stretch.appendChild(element(drawing.document, 'a', 'fillRect'));
  append(fill, blip, stretch);
  const shape = element(drawing.document, 'xdr', 'spPr');
  const transform = element(drawing.document, 'a', 'xfrm');
  const offset = element(drawing.document, 'a', 'off');
  setAttr(offset, 'x', 0);
  setAttr(offset, 'y', 0);
  const size = element(drawing.document, 'a', 'ext');
  setAttr(size, 'cx', Math.round(width * 9525));
  setAttr(size, 'cy', Math.round(height * 9525));
  append(transform, offset, size);
  const geometry = element(drawing.document, 'a', 'prstGeom');
  setAttr(geometry, 'prst', 'rect');
  geometry.appendChild(element(drawing.document, 'a', 'avLst'));
  append(shape, transform, geometry);
  append(picture, nonVisual, fill, shape);
  append(anchor, picture, element(drawing.document, 'xdr', 'clientData'));
  writeXml(zip, drawing.path, drawing.document);
  saveSheet(zip, sheet);
  return imagePath;
}

function chartText(document: Document, parent: Element, text: string): void {
  const rich = element(document, 'c', 'rich');
  const body = element(document, 'a', 'bodyPr');
  const list = element(document, 'a', 'lstStyle');
  const paragraph = element(document, 'a', 'p');
  const properties = element(document, 'a', 'pPr');
  properties.appendChild(element(document, 'a', 'defRPr'));
  const run = element(document, 'a', 'r');
  append(run, element(document, 'a', 'rPr'), element(document, 'a', 't', text));
  append(paragraph, properties, run);
  append(rich, body, list, paragraph);
  parent.appendChild(rich);
}

function absoluteRange(sheet: XlsxSheet, range: CellRange): string {
  return `${quoteSheetName(sheet.name)}!${rangeName(range, true)}`;
}

function chartCache(
  document: Document,
  parent: Element,
  kind: 'str' | 'num',
  reference: string,
  values: string[],
): void {
  const ref = element(document, 'c', kind === 'str' ? 'strRef' : 'numRef');
  ref.appendChild(element(document, 'c', 'f', reference));
  const cache = element(document, 'c', kind === 'str' ? 'strCache' : 'numCache');
  if (kind === 'num') cache.appendChild(element(document, 'c', 'formatCode', 'General'));
  const count = element(document, 'c', 'ptCount');
  setAttr(count, 'val', values.length);
  cache.appendChild(count);
  values.forEach((value, index) => {
    const point = element(document, 'c', 'pt');
    setAttr(point, 'idx', index);
    point.appendChild(element(document, 'c', 'v', value));
    cache.appendChild(point);
  });
  ref.appendChild(cache);
  parent.appendChild(ref);
}

function chartAxis(document: Document, id: number, crossId: number, position: string, category: boolean): Element {
  const axis = element(document, 'c', category ? 'catAx' : 'valAx');
  const axId = element(document, 'c', 'axId');
  setAttr(axId, 'val', id);
  const scaling = element(document, 'c', 'scaling');
  scaling.appendChild(element(document, 'c', 'orientation'));
  scaling.firstElementChild?.setAttribute('val', 'minMax');
  const deletion = element(document, 'c', 'delete');
  setAttr(deletion, 'val', 0);
  const axPos = element(document, 'c', 'axPos');
  setAttr(axPos, 'val', position);
  const major = element(document, 'c', 'majorTickMark');
  setAttr(major, 'val', 'out');
  const minor = element(document, 'c', 'minorTickMark');
  setAttr(minor, 'val', 'none');
  const labels = element(document, 'c', 'tickLblPos');
  setAttr(labels, 'val', 'nextTo');
  const cross = element(document, 'c', 'crossAx');
  setAttr(cross, 'val', crossId);
  const crosses = element(document, 'c', 'crosses');
  setAttr(crosses, 'val', 'autoZero');
  append(axis, axId, scaling, deletion, axPos, major, minor, labels, cross, crosses);
  if (category) {
    const auto = element(document, 'c', 'auto');
    setAttr(auto, 'val', 1);
    const align = element(document, 'c', 'lblAlgn');
    setAttr(align, 'val', 'ctr');
    const offset = element(document, 'c', 'lblOffset');
    setAttr(offset, 'val', 100);
    const multi = element(document, 'c', 'noMultiLvlLbl');
    setAttr(multi, 'val', 0);
    append(axis, auto, align, offset, multi);
  }
  return axis;
}

export async function insertChart(
  zip: JSZip,
  workbook: XlsxWorkbook,
  sheet: XlsxSheet,
  range: CellRange,
  title: string,
): Promise<string> {
  if (range.start.row === range.end.row) throw new Error('для диаграммы нужны хотя бы две строки');
  const valueCol = range.end.col > range.start.col ? range.start.col + 1 : range.start.col;
  const categories: string[] = [];
  const values: string[] = [];
  for (let row = range.start.row + 1; row <= range.end.row; row += 1) {
    const category = range.end.col > range.start.col
      ? cellText(workbook, findCell(sheet, { row, col: range.start.col }))
      : String(row - range.start.row);
    const value = cellText(workbook, findCell(sheet, { row, col: valueCol }));
    categories.push(category);
    values.push(value || '0');
  }
  const path = nextPartName(zip, 'xl/charts', 'chart', 'xml');
  const document = parseXml(
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="${NS.c}" xmlns:a="${NS.a}" xmlns:r="${NS.r}"/>`,
  );
  const language = element(document, 'c', 'lang');
  setAttr(language, 'val', 'ru-RU');
  const rounded = element(document, 'c', 'roundedCorners');
  setAttr(rounded, 'val', 0);
  const chart = element(document, 'c', 'chart');
  const titleNode = element(document, 'c', 'title');
  const titleText = element(document, 'c', 'tx');
  chartText(document, titleText, title.trim() || 'Диаграмма');
  append(titleNode, titleText, element(document, 'c', 'layout'));
  const overlay = element(document, 'c', 'overlay');
  setAttr(overlay, 'val', 0);
  titleNode.appendChild(overlay);
  const autoTitle = element(document, 'c', 'autoTitleDeleted');
  setAttr(autoTitle, 'val', 0);
  const plotArea = element(document, 'c', 'plotArea');
  plotArea.appendChild(element(document, 'c', 'layout'));
  const bar = element(document, 'c', 'barChart');
  const direction = element(document, 'c', 'barDir');
  setAttr(direction, 'val', 'col');
  const grouping = element(document, 'c', 'grouping');
  setAttr(grouping, 'val', 'clustered');
  const vary = element(document, 'c', 'varyColors');
  setAttr(vary, 'val', 0);
  const series = element(document, 'c', 'ser');
  const index = element(document, 'c', 'idx');
  setAttr(index, 'val', 0);
  const order = element(document, 'c', 'order');
  setAttr(order, 'val', 0);
  const seriesTitle = element(document, 'c', 'tx');
  seriesTitle.appendChild(element(document, 'c', 'v', title.trim() || 'Ряд 1'));
  append(series, index, order, seriesTitle);
  const category = element(document, 'c', 'cat');
  const categoryRange = {
    start: { row: range.start.row + 1, col: range.start.col },
    end: { row: range.end.row, col: range.start.col },
  };
  chartCache(document, category, 'str', absoluteRange(sheet, categoryRange), categories);
  const value = element(document, 'c', 'val');
  const valueRange = {
    start: { row: range.start.row + 1, col: valueCol },
    end: { row: range.end.row, col: valueCol },
  };
  chartCache(document, value, 'num', absoluteRange(sheet, valueRange), values);
  append(series, category, value);
  const gap = element(document, 'c', 'gapWidth');
  setAttr(gap, 'val', 150);
  const firstAxis = element(document, 'c', 'axId');
  setAttr(firstAxis, 'val', 100000001);
  const secondAxis = element(document, 'c', 'axId');
  setAttr(secondAxis, 'val', 100000002);
  append(bar, direction, grouping, vary, series, gap, firstAxis, secondAxis);
  append(plotArea, bar, chartAxis(document, 100000001, 100000002, 'b', true), chartAxis(document, 100000002, 100000001, 'l', false));
  const visible = element(document, 'c', 'plotVisOnly');
  setAttr(visible, 'val', 1);
  const blanks = element(document, 'c', 'dispBlanksAs');
  setAttr(blanks, 'val', 'gap');
  append(chart, titleNode, autoTitle, plotArea, visible, blanks);
  append(document.documentElement, language, rounded, chart);
  writeXml(zip, path, document);
  await ensureContentType(zip, path, CHART_CONTENT_TYPE);

  const drawing = await createDrawing(zip, sheet);
  const chartId = await ensureRelationship(zip, drawing.path, CHART_REL, relativeTarget(drawing.path, path));
  const anchor = appendAnchor(drawing.document);
  setAnchorPosition(drawing.document, anchor, { row: range.end.row + 2, col: range.start.col }, 480, 288);
  const frame = element(drawing.document, 'xdr', 'graphicFrame');
  const nonVisual = element(drawing.document, 'xdr', 'nvGraphicFramePr');
  const properties = element(drawing.document, 'xdr', 'cNvPr');
  const id = descendants(drawing.document, 'cNvPr').reduce((maximum, node) => Math.max(maximum, Number(node.getAttribute('id')) || 0), 0) + 1;
  setAttr(properties, 'id', id);
  setAttr(properties, 'name', title.trim() || `Диаграмма ${id}`);
  const frameProperties = element(drawing.document, 'xdr', 'cNvGraphicFramePr');
  const locks = element(drawing.document, 'a', 'graphicFrameLocks');
  setAttr(locks, 'noChangeAspect', 1);
  frameProperties.appendChild(locks);
  append(nonVisual, properties, frameProperties);
  const transform = element(drawing.document, 'xdr', 'xfrm');
  const offset = element(drawing.document, 'a', 'off');
  setAttr(offset, 'x', 0);
  setAttr(offset, 'y', 0);
  const size = element(drawing.document, 'a', 'ext');
  setAttr(size, 'cx', 0);
  setAttr(size, 'cy', 0);
  append(transform, offset, size);
  const graphic = element(drawing.document, 'a', 'graphic');
  const data = element(drawing.document, 'a', 'graphicData');
  setAttr(data, 'uri', NS.c);
  const reference = element(drawing.document, 'c', 'chart');
  setNsAttr(reference, 'r', 'id', chartId);
  data.appendChild(reference);
  graphic.appendChild(data);
  append(frame, nonVisual, transform, graphic);
  append(anchor, frame, element(drawing.document, 'xdr', 'clientData'));
  writeXml(zip, drawing.path, drawing.document);
  saveSheet(zip, sheet);
  return path;
}

async function nextTableId(zip: JSZip): Promise<number> {
  let maximum = 0;
  for (const path of Object.keys(zip.files)) {
    if (!/^xl\/tables\/[^/]+\.xml$/.test(path)) continue;
    const file = zip.file(path);
    if (!file) continue;
    const document = parseXml(await file.async('string'));
    maximum = Math.max(maximum, Number(document.documentElement.getAttribute('id')) || 0);
  }
  return maximum + 1;
}

export async function createTable(
  zip: JSZip,
  workbook: XlsxWorkbook,
  sheet: XlsxSheet,
  range: CellRange,
  name: string,
): Promise<string> {
  const id = await nextTableId(zip);
  const baseName = (name.trim() || `Таблица${id}`).replace(/\s/g, '_').replace(/[^\p{L}\p{N}_.]/gu, '_');
  if (!baseName || /^\d/.test(baseName)) throw new Error('некорректное имя таблицы');
  const path = nextPartName(zip, 'xl/tables', 'table', 'xml');
  const document = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="${NS.x}"/>`);
  setAttr(document.documentElement, 'id', id);
  setAttr(document.documentElement, 'name', baseName);
  setAttr(document.documentElement, 'displayName', baseName);
  setAttr(document.documentElement, 'ref', rangeName(range));
  setAttr(document.documentElement, 'totalsRowShown', 0);
  const filter = element(document, 'x', 'autoFilter');
  setAttr(filter, 'ref', rangeName(range));
  const columns = element(document, 'x', 'tableColumns');
  const usedNames = new Set<string>();
  for (let col = range.start.col; col <= range.end.col; col += 1) {
    const header = cellText(workbook, findCell(sheet, { row: range.start.row, col })).trim() || `Столбец${col - range.start.col + 1}`;
    let unique = header;
    let suffix = 2;
    while (usedNames.has(unique.toLowerCase())) unique = `${header}${suffix++}`;
    usedNames.add(unique.toLowerCase());
    const column = element(document, 'x', 'tableColumn');
    setAttr(column, 'id', col - range.start.col + 1);
    setAttr(column, 'name', unique);
    columns.appendChild(column);
  }
  setAttr(columns, 'count', range.end.col - range.start.col + 1);
  const style = element(document, 'x', 'tableStyleInfo');
  setAttr(style, 'name', 'TableStyleMedium2');
  setAttr(style, 'showFirstColumn', 0);
  setAttr(style, 'showLastColumn', 0);
  setAttr(style, 'showRowStripes', 1);
  setAttr(style, 'showColumnStripes', 0);
  append(document.documentElement, filter, columns, style);
  writeXml(zip, path, document);
  await ensureContentType(zip, path, TABLE_CONTENT_TYPE);
  const relationship = await ensureRelationship(zip, sheet.path, TABLE_REL, relativeTarget(sheet.path, path));
  const tableParts = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'tableParts', ['extLst']);
  const part = element(sheet.document, 'x', 'tablePart');
  setNsAttr(part, 'r', 'id', relationship);
  tableParts.appendChild(part);
  setAttr(tableParts, 'count', children(tableParts, 'tablePart').length);
  saveSheet(zip, sheet);
  return path;
}

function numeric(value: string): number | null {
  if (!value.trim()) return null;
  const parsed = Number(value.replace(/\s/g, '').replace(',', '.'));
  return Number.isFinite(parsed) ? parsed : null;
}

export async function createPivot(
  zip: JSZip,
  workbook: XlsxWorkbook,
  source: XlsxSheet,
  range: CellRange,
  name: string,
): Promise<XlsxSheet> {
  const headers: string[] = [];
  for (let col = range.start.col; col <= range.end.col; col += 1) {
    headers.push(cellText(workbook, findCell(source, { row: range.start.row, col })).trim() || `Столбец ${col - range.start.col + 1}`);
  }
  interface Aggregate { count: number; values: number[]; numeric: boolean[] }
  const groups = new Map<string, Aggregate>();
  for (let row = range.start.row + 1; row <= range.end.row; row += 1) {
    const key = cellText(workbook, findCell(source, { row, col: range.start.col })).trim() || '(пусто)';
    const aggregate = groups.get(key) ?? { count: 0, values: headers.map(() => 0), numeric: headers.map(() => true) };
    aggregate.count += 1;
    for (let offset = 1; offset < headers.length; offset += 1) {
      const text = cellText(workbook, findCell(source, { row, col: range.start.col + offset }));
      const value = numeric(text);
      if (value === null) aggregate.numeric[offset] = false;
      else aggregate.values[offset] = (aggregate.values[offset] ?? 0) + value;
    }
    groups.set(key, aggregate);
  }
  if (groups.size === 0) throw new Error('диапазон не содержит данных для сводного листа');
  const sheetName = `Сводная ${name.trim() || source.name}`;
  const result = await appendBlankSheet(zip, workbook, sheetName);
  setCellText(result, { row: 0, col: 0 }, headers[0] ?? 'Группа');
  for (let offset = 1; offset < headers.length; offset += 1) {
    const numericColumn = [...groups.values()].every((group) => group.numeric[offset]);
    setCellText(result, { row: 0, col: offset }, `${numericColumn ? 'Сумма' : 'Количество'}: ${headers[offset]}`);
  }
  let row = 1;
  for (const [key, aggregate] of groups) {
    setCellText(result, { row, col: 0 }, key);
    for (let offset = 1; offset < headers.length; offset += 1) {
      const numericColumn = [...groups.values()].every((group) => group.numeric[offset]);
      setCellNumber(result, { row, col: offset }, numericColumn ? aggregate.values[offset] ?? 0 : aggregate.count);
    }
    row += 1;
  }
  setCellText(result, { row, col: 0 }, 'Общий итог');
  for (let offset = 1; offset < headers.length; offset += 1) {
    const numericColumn = [...groups.values()].every((group) => group.numeric[offset]);
    const total = numericColumn
      ? [...groups.values()].reduce((sum, group) => sum + (group.values[offset] ?? 0), 0)
      : [...groups.values()].reduce((sum, group) => sum + group.count, 0);
    setCellNumber(result, { row, col: offset }, total);
  }
  saveSheet(zip, result);
  return result;
}

function commentText(document: Document, text: string): Element {
  const node = element(document, 'x', 'text');
  const run = element(document, 'x', 'r');
  const properties = element(document, 'x', 'rPr');
  const size = element(document, 'x', 'sz');
  setAttr(size, 'val', 9);
  const family = element(document, 'x', 'family');
  setAttr(family, 'val', 2);
  append(properties, size, family);
  append(run, properties, element(document, 'x', 't', text));
  node.appendChild(run);
  return node;
}

async function createVml(zip: JSZip, sheet: XlsxSheet, address: CellAddress): Promise<void> {
  const existing = direct(sheet.document.documentElement, 'legacyDrawing');
  if (existing) return;
  const path = nextPartName(zip, 'xl/drawings', 'vmlDrawing', 'vml');
  const document = parseXml(`<?xml version="1.0" encoding="UTF-8"?><xml xmlns:v="${VML_NS}" xmlns:o="${OFFICE_NS}" xmlns:x="${EXCEL_NS}"/>`);
  const layout = document.createElementNS(OFFICE_NS, 'o:shapelayout');
  const idmap = document.createElementNS(OFFICE_NS, 'o:idmap');
  idmap.setAttribute('v:ext', 'edit');
  idmap.setAttribute('data', '1');
  layout.appendChild(idmap);
  const shapeType = document.createElementNS(VML_NS, 'v:shapetype');
  shapeType.setAttribute('id', '_x0000_t202');
  shapeType.setAttribute('coordsize', '21600,21600');
  shapeType.setAttribute('o:spt', '202');
  shapeType.setAttribute('path', 'm,l,21600r21600,l21600,xe');
  const stroke = document.createElementNS(VML_NS, 'v:stroke');
  stroke.setAttribute('joinstyle', 'miter');
  const formulas = document.createElementNS(VML_NS, 'v:formulas');
  for (const [f, value] of [['f', 'if lineDrawn pixelLineWidth 0'], ['f', 'sum @0 1 0'], ['f', 'sum 0 0 @1'], ['f', 'prod @2 1 2'], ['f', 'prod @3 21600 pixelWidth'], ['f', 'prod @3 21600 pixelHeight'], ['f', 'sum @0 0 1'], ['f', 'prod @6 1 2'], ['f', 'prod @7 21600 pixelWidth'], ['f', 'sum @8 21600 0'], ['f', 'prod @7 21600 pixelHeight'], ['f', 'sum @10 21600 0']]) {
    const formula = document.createElementNS(VML_NS, 'v:f');
    formula.setAttribute('eqn', value ?? '');
    formulas.appendChild(formula);
  }
  const pathNode = document.createElementNS(VML_NS, 'v:path');
  pathNode.setAttribute('o:extrusionok', 'f');
  pathNode.setAttribute('gradientshapeok', 't');
  pathNode.setAttribute('o:connecttype', 'rect');
  append(shapeType, stroke, formulas, pathNode);
  const shape = document.createElementNS(VML_NS, 'v:shape');
  shape.setAttribute('id', `_x0000_s${1025 + address.row * 16 + address.col}`);
  shape.setAttribute('type', '#_x0000_t202');
  shape.setAttribute('style', 'position:absolute;margin-left:59.25pt;margin-top:1.5pt;width:144pt;height:79pt;z-index:1;visibility:hidden');
  shape.setAttribute('fillcolor', '#ffffe1');
  shape.setAttribute('o:insetmode', 'auto');
  const fill = document.createElementNS(VML_NS, 'v:fill');
  fill.setAttribute('color2', '#ffffe1');
  const shadow = document.createElementNS(VML_NS, 'v:shadow');
  shadow.setAttribute('on', 't');
  shadow.setAttribute('color', 'black');
  shadow.setAttribute('obscured', 't');
  const textbox = document.createElementNS(VML_NS, 'v:textbox');
  textbox.setAttribute('style', 'mso-direction-alt:auto');
  const client = document.createElementNS(EXCEL_NS, 'x:ClientData');
  client.setAttribute('ObjectType', 'Note');
  append(client, 
    document.createElementNS(EXCEL_NS, 'x:MoveWithCells'),
    document.createElementNS(EXCEL_NS, 'x:SizeWithCells'),
  );
  const anchor = document.createElementNS(EXCEL_NS, 'x:Anchor');
  anchor.textContent = `${address.col + 1}, 15, ${address.row}, 2, ${address.col + 3}, 15, ${address.row + 3}, 2`;
  const autoFill = document.createElementNS(EXCEL_NS, 'x:AutoFill');
  autoFill.textContent = 'False';
  const row = document.createElementNS(EXCEL_NS, 'x:Row');
  row.textContent = String(address.row);
  const column = document.createElementNS(EXCEL_NS, 'x:Column');
  column.textContent = String(address.col);
  append(client, anchor, autoFill, row, column);
  append(shape, fill, shadow, textbox, client);
  append(document.documentElement, layout, shapeType, shape);
  writeXml(zip, path, document);
  await ensureDefaultExtension(zip, 'vml', 'application/vnd.openxmlformats-officedocument.vmlDrawing');
  const relationship = await ensureRelationship(zip, sheet.path, VML_REL, relativeTarget(sheet.path, path));
  const legacy = ensureDirect(sheet.document, sheet.document.documentElement, 'x', 'legacyDrawing', [
    'legacyDrawingHF', 'picture', 'oleObjects', 'controls', 'tableParts', 'extLst',
  ]);
  setNsAttr(legacy, 'r', 'id', relationship);
}

export async function addComment(zip: JSZip, sheet: XlsxSheet, address: CellAddress, text: string): Promise<void> {
  if (!text.trim()) throw new Error('текст комментария не должен быть пустым');
  let path = await relationshipTarget(zip, sheet.path, '/comments');
  let document: Document;
  if (path && zip.file(path)) {
    document = await readXml(zip, path);
  } else {
    path = nextPartName(zip, 'xl', 'comments', 'xml');
    document = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="${NS.x}"><authors><author>Редактор</author></authors><commentList/></comments>`);
    writeXml(zip, path, document);
    await ensureContentType(zip, path, COMMENTS_CONTENT_TYPE);
    await ensureRelationship(zip, sheet.path, COMMENTS_REL, relativeTarget(sheet.path, path));
  }
  const authors = ensureDirect(document, document.documentElement, 'x', 'authors', ['commentList']);
  if (children(authors, 'author').length === 0) authors.appendChild(element(document, 'x', 'author', 'Редактор'));
  const list = ensureDirect(document, document.documentElement, 'x', 'commentList');
  const reference = cellName(address);
  let comment = children(list, 'comment').find((node) => node.getAttribute('ref') === reference);
  if (!comment) {
    comment = element(document, 'x', 'comment');
    setAttr(comment, 'ref', reference);
    setAttr(comment, 'authorId', 0);
    list.appendChild(comment);
  }
  while (comment.firstChild) comment.removeChild(comment.firstChild);
  comment.appendChild(commentText(document, text));
  writeXml(zip, path, document);
  await createVml(zip, sheet, address);
  saveSheet(zip, sheet);
}

