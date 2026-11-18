import type { CommandResult, EditorContext } from './commands.js';
import { numberArg, requiredString, stringArg } from './commands.js';
import {
  DOCUMENT_PART,
  RELATIONSHIP_BASE,
  STYLES_PART,
  addressedRun,
  appendTextRun,
  appendToBody,
  bodyOf,
  createParagraph,
  ensureStyle,
  ensureStyles,
  insertField,
  paragraphProperties,
  positiveNumber,
  replaceText,
  requiredArrayBuffer,
  runProperties,
  setWAttr,
  w,
  a,
} from './docx_shared.js';
import {
  NS,
  children,
  direct,
  element,
  nextId,
  parseXml,
  readXml,
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

const IMAGE_RELATIONSHIP = `${RELATIONSHIP_BASE}/image`;
const CHART_RELATIONSHIP = `${RELATIONSHIP_BASE}/chart`;
const HYPERLINK_RELATIONSHIP = `${RELATIONSHIP_BASE}/hyperlink`;
const COMMENTS_RELATIONSHIP = `${RELATIONSHIP_BASE}/comments`;
const COMMENTS_PART = 'word/comments.xml';
const EMU_PER_PIXEL = 9525;

function declare(document: Document, prefix: string, namespace: string): void {
  const root = document.documentElement;
  if (!root.getAttribute(`xmlns:${prefix}`)) root.setAttribute(`xmlns:${prefix}`, namespace);
}

function integerArgument(context: EditorContext, name: string, minimum: number): number {
  const value = numberArg(context.args, name, Number.NaN);
  if (!Number.isInteger(value) || value < minimum) {
    throw new Error(`параметр «${name}» должен быть целым числом не меньше ${minimum}`);
  }
  return value;
}

export async function insertCover(context: EditorContext): Promise<CommandResult> {
  const text = requiredString(context.args, 'text');
  const document = await readXml(context.zip, DOCUMENT_PART);
  const styles = await ensureStyles(context.zip);
  ensureStyle(styles, 'Normal', 'Normal');
  const titleStyle = ensureStyle(styles, 'Title', 'Title');
  if (!direct(titleStyle, 'rPr')) {
    const properties = w(styles, 'rPr');
    const size = w(styles, 'sz');
    setWAttr(size, 'val', 56);
    properties.appendChild(size);
    titleStyle.appendChild(properties);
  }
  writeXml(context.zip, STYLES_PART, styles);

  const title = createParagraph(document);
  const titleProperties = paragraphProperties(document, title);
  const style = w(document, 'pStyle');
  setWAttr(style, 'val', 'Title');
  titleProperties.appendChild(style);
  const alignment = w(document, 'jc');
  setWAttr(alignment, 'val', 'center');
  titleProperties.appendChild(alignment);
  const spacing = w(document, 'spacing');
  setWAttr(spacing, 'before', 3600);
  titleProperties.appendChild(spacing);
  const titleRun = appendTextRun(document, title, text);
  const runProps = runProperties(document, titleRun);
  runProps.appendChild(w(document, 'b'));
  const titleSize = w(document, 'sz');
  setWAttr(titleSize, 'val', 56);
  runProps.appendChild(titleSize);

  const breakParagraph = createParagraph(document);
  const breakRun = w(document, 'r');
  const pageBreak = w(document, 'br');
  setWAttr(pageBreak, 'type', 'page');
  breakRun.appendChild(pageBreak);
  breakParagraph.appendChild(breakRun);

  const body = bodyOf(document);
  const firstContent = children(body)[0] ?? null;
  body.insertBefore(breakParagraph, firstContent);
  body.insertBefore(title, breakParagraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function insertBreak(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  const paragraph = createParagraph(document);
  const run = w(document, 'r');
  const pageBreak = w(document, 'br');
  setWAttr(pageBreak, 'type', 'page');
  run.appendChild(pageBreak);
  paragraph.appendChild(run);
  appendToBody(document, paragraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function insertTable(context: EditorContext): Promise<CommandResult> {
  const rows = integerArgument(context, 'rows', 1);
  const columns = integerArgument(context, 'cols', 1);
  if (rows * columns > 10000) throw new Error('таблица не может содержать больше 10000 ячеек');
  const document = await readXml(context.zip, DOCUMENT_PART);
  const table = w(document, 'tbl');
  const properties = w(document, 'tblPr');
  const width = w(document, 'tblW');
  setWAttr(width, 'w', 0);
  setWAttr(width, 'type', 'auto');
  properties.appendChild(width);
  const borders = w(document, 'tblBorders');
  for (const side of ['top', 'left', 'bottom', 'right', 'insideH', 'insideV']) {
    const border = w(document, side);
    setWAttr(border, 'val', 'single');
    setWAttr(border, 'sz', 4);
    setWAttr(border, 'space', 0);
    setWAttr(border, 'color', 'auto');
    borders.appendChild(border);
  }
  properties.appendChild(borders);
  table.appendChild(properties);
  const grid = w(document, 'tblGrid');
  const columnWidth = Math.max(1, Math.floor(9000 / columns));
  for (let column = 0; column < columns; column += 1) {
    const gridColumn = w(document, 'gridCol');
    setWAttr(gridColumn, 'w', columnWidth);
    grid.appendChild(gridColumn);
  }
  table.appendChild(grid);
  for (let rowIndex = 0; rowIndex < rows; rowIndex += 1) {
    const row = w(document, 'tr');
    for (let columnIndex = 0; columnIndex < columns; columnIndex += 1) {
      const cell = w(document, 'tc');
      const cellProperties = w(document, 'tcPr');
      const cellWidth = w(document, 'tcW');
      setWAttr(cellWidth, 'w', columnWidth);
      setWAttr(cellWidth, 'type', 'dxa');
      cellProperties.appendChild(cellWidth);
      cell.appendChild(cellProperties);
      cell.appendChild(createParagraph(document));
      row.appendChild(cell);
    }
    table.appendChild(row);
  }
  appendToBody(document, table);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

function imageExtension(mime: string): string {
  const extensions: Record<string, string> = {
    'image/png': 'png',
    'image/jpeg': 'jpeg',
    'image/gif': 'gif',
    'image/bmp': 'bmp',
    'image/tiff': 'tiff',
    'image/svg+xml': 'svg',
    'image/webp': 'webp',
    'image/x-emf': 'emf',
    'image/emf': 'emf',
    'image/x-wmf': 'wmf',
    'image/wmf': 'wmf',
  };
  const extension = extensions[mime.toLowerCase()];
  if (!extension) throw new Error(`неподдерживаемый MIME изображения «${mime}»`);
  return extension;
}

function createInlineImage(
  document: Document,
  relationshipId: string,
  name: string,
  widthEmu: number,
  heightEmu: number,
): Element {
  declare(document, 'wp', NS.wp);
  declare(document, 'a', NS.a);
  declare(document, 'pic', NS.pic);
  declare(document, 'r', NS.r);
  const drawing = w(document, 'drawing');
  const inline = element(document, 'wp', 'inline');
  for (const attribute of ['distT', 'distB', 'distL', 'distR']) setAttr(inline, attribute, 0);
  const extent = element(document, 'wp', 'extent');
  setAttr(extent, 'cx', widthEmu);
  setAttr(extent, 'cy', heightEmu);
  inline.appendChild(extent);
  const effectExtent = element(document, 'wp', 'effectExtent');
  for (const attribute of ['l', 't', 'r', 'b']) setAttr(effectExtent, attribute, 0);
  inline.appendChild(effectExtent);
  const properties = element(document, 'wp', 'docPr');
  setAttr(properties, 'id', nextId([document], 'docPr', 'id'));
  setAttr(properties, 'name', name);
  inline.appendChild(properties);
  const frameProperties = element(document, 'wp', 'cNvGraphicFramePr');
  const locks = a(document, 'graphicFrameLocks');
  setAttr(locks, 'noChangeAspect', 1);
  frameProperties.appendChild(locks);
  inline.appendChild(frameProperties);
  const graphic = a(document, 'graphic');
  const graphicData = a(document, 'graphicData');
  setAttr(graphicData, 'uri', 'http://schemas.openxmlformats.org/drawingml/2006/picture');
  const picture = element(document, 'pic', 'pic');
  const nonVisual = element(document, 'pic', 'nvPicPr');
  const nonVisualProperties = element(document, 'pic', 'cNvPr');
  setAttr(nonVisualProperties, 'id', 0);
  setAttr(nonVisualProperties, 'name', name);
  nonVisual.appendChild(nonVisualProperties);
  nonVisual.appendChild(element(document, 'pic', 'cNvPicPr'));
  picture.appendChild(nonVisual);
  const blipFill = element(document, 'pic', 'blipFill');
  const blip = a(document, 'blip');
  setNsAttr(blip, 'r', 'embed', relationshipId);
  blipFill.appendChild(blip);
  const stretch = a(document, 'stretch');
  stretch.appendChild(a(document, 'fillRect'));
  blipFill.appendChild(stretch);
  picture.appendChild(blipFill);
  const shapeProperties = element(document, 'pic', 'spPr');
  const transform = a(document, 'xfrm');
  const offset = a(document, 'off');
  setAttr(offset, 'x', 0);
  setAttr(offset, 'y', 0);
  transform.appendChild(offset);
  const shapeExtent = a(document, 'ext');
  setAttr(shapeExtent, 'cx', widthEmu);
  setAttr(shapeExtent, 'cy', heightEmu);
  transform.appendChild(shapeExtent);
  shapeProperties.appendChild(transform);
  const geometry = a(document, 'prstGeom');
  setAttr(geometry, 'prst', 'rect');
  geometry.appendChild(a(document, 'avLst'));
  shapeProperties.appendChild(geometry);
  picture.appendChild(shapeProperties);
  graphicData.appendChild(picture);
  graphic.appendChild(graphicData);
  inline.appendChild(graphic);
  drawing.appendChild(inline);
  return drawing;
}

export async function insertPicture(context: EditorContext): Promise<CommandResult> {
  const bytes = requiredArrayBuffer(context.args.bytes);
  const mime = requiredString(context.args, 'mime').toLowerCase();
  const width = positiveNumber(context.args.width, 'width');
  const height = positiveNumber(context.args.height, 'height');
  const name = stringArg(context.args, 'name').trim() || 'Изображение';
  const extension = imageExtension(mime);
  const part = nextPartName(context.zip, 'word/media', 'image', extension);
  context.zip.file(part, bytes);
  await ensureDefaultExtension(context.zip, extension, mime);
  const relationshipId = await ensureRelationship(
    context.zip,
    DOCUMENT_PART,
    IMAGE_RELATIONSHIP,
    relativeTarget(DOCUMENT_PART, part),
  );
  const document = await readXml(context.zip, DOCUMENT_PART);
  const paragraph = createParagraph(document);
  const run = w(document, 'r');
  run.appendChild(createInlineImage(
    document,
    relationshipId,
    name,
    Math.round(width * EMU_PER_PIXEL),
    Math.round(height * EMU_PER_PIXEL),
  ));
  paragraph.appendChild(run);
  appendToBody(document, paragraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true, value: { part } };
}

function chartPoint(document: Document, index: number, value: string): Element {
  const point = element(document, 'c', 'pt');
  setAttr(point, 'idx', index);
  point.appendChild(element(document, 'c', 'v', value));
  return point;
}

function chartAxis(document: Document, category: boolean, axisId: number, crossingId: number): Element {
  const axis = element(document, 'c', category ? 'catAx' : 'valAx');
  const id = element(document, 'c', 'axId');
  setAttr(id, 'val', axisId);
  axis.appendChild(id);
  const scaling = element(document, 'c', 'scaling');
  const orientation = element(document, 'c', 'orientation');
  setAttr(orientation, 'val', 'minMax');
  scaling.appendChild(orientation);
  axis.appendChild(scaling);
  const deleted = element(document, 'c', 'delete');
  setAttr(deleted, 'val', 0);
  axis.appendChild(deleted);
  const position = element(document, 'c', 'axPos');
  setAttr(position, 'val', category ? 'b' : 'l');
  axis.appendChild(position);
  if (!category) {
    const majorGridlines = element(document, 'c', 'majorGridlines');
    axis.appendChild(majorGridlines);
  }
  const numberFormat = element(document, 'c', 'numFmt');
  setAttr(numberFormat, 'formatCode', 'General');
  setAttr(numberFormat, 'sourceLinked', category ? 0 : 1);
  axis.appendChild(numberFormat);
  const majorTick = element(document, 'c', 'majorTickMark');
  setAttr(majorTick, 'val', 'out');
  axis.appendChild(majorTick);
  const minorTick = element(document, 'c', 'minorTickMark');
  setAttr(minorTick, 'val', 'none');
  axis.appendChild(minorTick);
  const labels = element(document, 'c', 'tickLblPos');
  setAttr(labels, 'val', 'nextTo');
  axis.appendChild(labels);
  const crossing = element(document, 'c', 'crossAx');
  setAttr(crossing, 'val', crossingId);
  axis.appendChild(crossing);
  const crosses = element(document, 'c', 'crosses');
  setAttr(crosses, 'val', 'autoZero');
  axis.appendChild(crosses);
  const auto = element(document, 'c', 'auto');
  setAttr(auto, 'val', 1);
  const labelAlignment = element(document, 'c', 'lblAlgn');
  setAttr(labelAlignment, 'val', 'ctr');
  const labelOffset = element(document, 'c', 'lblOffset');
  setAttr(labelOffset, 'val', 100);
  if (category) {
    axis.appendChild(auto);
    axis.appendChild(labelAlignment);
    axis.appendChild(labelOffset);
  }
  return axis;
}

function createChartDocument(title: string): Document {
  const document = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="${NS.c}" xmlns:a="${NS.a}" xmlns:r="${NS.r}"/>`);
  const root = document.documentElement;
  const language = element(document, 'c', 'lang');
  setAttr(language, 'val', 'ru-RU');
  root.appendChild(language);
  const rounded = element(document, 'c', 'roundedCorners');
  setAttr(rounded, 'val', 0);
  root.appendChild(rounded);
  const chart = element(document, 'c', 'chart');
  const titleNode = element(document, 'c', 'title');
  const rich = element(document, 'c', 'rich');
  rich.appendChild(a(document, 'bodyPr'));
  rich.appendChild(a(document, 'lstStyle'));
  const paragraph = a(document, 'p');
  const paragraphProperties = a(document, 'pPr');
  paragraphProperties.appendChild(a(document, 'defRPr'));
  paragraph.appendChild(paragraphProperties);
  const run = a(document, 'r');
  const runProperties = a(document, 'rPr');
  setAttr(runProperties, 'lang', 'ru-RU');
  run.appendChild(runProperties);
  run.appendChild(a(document, 't', title));
  paragraph.appendChild(run);
  rich.appendChild(paragraph);
  titleNode.appendChild(rich);
  const overlay = element(document, 'c', 'overlay');
  setAttr(overlay, 'val', 0);
  titleNode.appendChild(overlay);
  chart.appendChild(titleNode);
  const autoTitle = element(document, 'c', 'autoTitleDeleted');
  setAttr(autoTitle, 'val', 0);
  chart.appendChild(autoTitle);
  const plotArea = element(document, 'c', 'plotArea');
  plotArea.appendChild(element(document, 'c', 'layout'));
  const barChart = element(document, 'c', 'barChart');
  const direction = element(document, 'c', 'barDir');
  setAttr(direction, 'val', 'col');
  barChart.appendChild(direction);
  const grouping = element(document, 'c', 'grouping');
  setAttr(grouping, 'val', 'clustered');
  barChart.appendChild(grouping);
  const varyColors = element(document, 'c', 'varyColors');
  setAttr(varyColors, 'val', 0);
  barChart.appendChild(varyColors);
  const series = element(document, 'c', 'ser');
  const index = element(document, 'c', 'idx');
  setAttr(index, 'val', 0);
  series.appendChild(index);
  const order = element(document, 'c', 'order');
  setAttr(order, 'val', 0);
  series.appendChild(order);
  const seriesText = element(document, 'c', 'tx');
  seriesText.appendChild(element(document, 'c', 'v', title));
  series.appendChild(seriesText);
  const categories = element(document, 'c', 'cat');
  const categoryLiterals = element(document, 'c', 'strLit');
  const categoryCount = element(document, 'c', 'ptCount');
  setAttr(categoryCount, 'val', 3);
  categoryLiterals.appendChild(categoryCount);
  for (let point = 0; point < 3; point += 1) categoryLiterals.appendChild(chartPoint(document, point, `Категория ${point + 1}`));
  categories.appendChild(categoryLiterals);
  series.appendChild(categories);
  const values = element(document, 'c', 'val');
  const valueLiterals = element(document, 'c', 'numLit');
  const format = element(document, 'c', 'formatCode', 'General');
  valueLiterals.appendChild(format);
  const valueCount = element(document, 'c', 'ptCount');
  setAttr(valueCount, 'val', 3);
  valueLiterals.appendChild(valueCount);
  for (let point = 0; point < 3; point += 1) valueLiterals.appendChild(chartPoint(document, point, String(point + 1)));
  values.appendChild(valueLiterals);
  series.appendChild(values);
  barChart.appendChild(series);
  const gapWidth = element(document, 'c', 'gapWidth');
  setAttr(gapWidth, 'val', 150);
  barChart.appendChild(gapWidth);
  for (const id of [111111111, 222222222]) {
    const axisReference = element(document, 'c', 'axId');
    setAttr(axisReference, 'val', id);
    barChart.appendChild(axisReference);
  }
  plotArea.appendChild(barChart);
  plotArea.appendChild(chartAxis(document, true, 111111111, 222222222));
  plotArea.appendChild(chartAxis(document, false, 222222222, 111111111));
  chart.appendChild(plotArea);
  const plotVisible = element(document, 'c', 'plotVisOnly');
  setAttr(plotVisible, 'val', 1);
  chart.appendChild(plotVisible);
  const blanks = element(document, 'c', 'dispBlanksAs');
  setAttr(blanks, 'val', 'gap');
  chart.appendChild(blanks);
  root.appendChild(chart);
  return document;
}

function createChartDrawing(document: Document, relationshipId: string, title: string): Element {
  declare(document, 'wp', NS.wp);
  declare(document, 'a', NS.a);
  declare(document, 'c', NS.c);
  declare(document, 'r', NS.r);
  const drawing = w(document, 'drawing');
  const inline = element(document, 'wp', 'inline');
  const extent = element(document, 'wp', 'extent');
  setAttr(extent, 'cx', 5486400);
  setAttr(extent, 'cy', 3200400);
  inline.appendChild(extent);
  const properties = element(document, 'wp', 'docPr');
  setAttr(properties, 'id', nextId([document], 'docPr', 'id'));
  setAttr(properties, 'name', title);
  inline.appendChild(properties);
  const frameProperties = element(document, 'wp', 'cNvGraphicFramePr');
  frameProperties.appendChild(a(document, 'graphicFrameLocks'));
  inline.appendChild(frameProperties);
  const graphic = a(document, 'graphic');
  const data = a(document, 'graphicData');
  setAttr(data, 'uri', 'http://schemas.openxmlformats.org/drawingml/2006/chart');
  const reference = element(document, 'c', 'chart');
  setNsAttr(reference, 'r', 'id', relationshipId);
  data.appendChild(reference);
  graphic.appendChild(data);
  inline.appendChild(graphic);
  drawing.appendChild(inline);
  return drawing;
}

export async function insertChart(context: EditorContext): Promise<CommandResult> {
  const title = requiredString(context.args, 'title');
  const part = nextPartName(context.zip, 'word/charts', 'chart', 'xml');
  writeXml(context.zip, part, createChartDocument(title));
  await ensureContentType(context.zip, part, 'application/vnd.openxmlformats-officedocument.drawingml.chart+xml');
  const relationshipId = await ensureRelationship(
    context.zip,
    DOCUMENT_PART,
    CHART_RELATIONSHIP,
    relativeTarget(DOCUMENT_PART, part),
  );
  const document = await readXml(context.zip, DOCUMENT_PART);
  const paragraph = createParagraph(document);
  const run = w(document, 'r');
  run.appendChild(createChartDrawing(document, relationshipId, title));
  paragraph.appendChild(run);
  appendToBody(document, paragraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true, value: { part } };
}

export async function insertToc(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  const paragraph = createParagraph(document);
  insertField(paragraph, ' TOC \\o "1-3" \\h \\z \\u ', 'Оглавление (обновите поле в Word)');
  appendToBody(document, paragraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function insertLink(context: EditorContext): Promise<CommandResult> {
  const url = requiredString(context.args, 'url');
  const text = stringArg(context.args, 'text') || url;
  const relationshipId = await ensureRelationship(context.zip, DOCUMENT_PART, HYPERLINK_RELATIONSHIP, url, true);
  const document = await readXml(context.zip, DOCUMENT_PART);
  const styles = await ensureStyles(context.zip);
  const style = ensureStyle(styles, 'Hyperlink', 'Hyperlink', 'character');
  if (!direct(style, 'rPr')) {
    const properties = w(styles, 'rPr');
    const color = w(styles, 'color');
    setWAttr(color, 'val', '0563C1');
    properties.appendChild(color);
    const underline = w(styles, 'u');
    setWAttr(underline, 'val', 'single');
    properties.appendChild(underline);
    style.appendChild(properties);
  }
  writeXml(context.zip, STYLES_PART, styles);
  const paragraph = createParagraph(document);
  const hyperlink = w(document, 'hyperlink');
  setNsAttr(hyperlink, 'r', 'id', relationshipId);
  setWAttr(hyperlink, 'history', '1');
  const run = appendTextRun(document, hyperlink, text);
  const properties = runProperties(document, run);
  const runStyle = w(document, 'rStyle');
  setWAttr(runStyle, 'val', 'Hyperlink');
  properties.insertBefore(runStyle, properties.firstChild);
  hyperlink.appendChild(run);
  paragraph.appendChild(hyperlink);
  appendToBody(document, paragraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function insertCaption(context: EditorContext): Promise<CommandResult> {
  const text = requiredString(context.args, 'text');
  const styles = await ensureStyles(context.zip);
  ensureStyle(styles, 'Normal', 'Normal');
  ensureStyle(styles, 'Caption', 'caption');
  writeXml(context.zip, STYLES_PART, styles);
  const document = await readXml(context.zip, DOCUMENT_PART);
  const paragraph = createParagraph(document);
  const style = w(document, 'pStyle');
  setWAttr(style, 'val', 'Caption');
  paragraphProperties(document, paragraph).appendChild(style);
  appendTextRun(document, paragraph, text);
  appendToBody(document, paragraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function insertCrossRef(context: EditorContext): Promise<CommandResult> {
  const target = requiredString(context.args, 'target');
  const document = await readXml(context.zip, DOCUMENT_PART);
  const paragraph = createParagraph(document);
  insertField(paragraph, ` REF ${target} \\h `, target);
  appendToBody(document, paragraph);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function mailMerge(context: EditorContext): Promise<CommandResult> {
  const records = context.args.records;
  if (!Array.isArray(records) || records.length === 0) {
    throw new Error('параметр «records» должен быть непустым массивом записей');
  }
  const first = records[0];
  if (!first || typeof first !== 'object' || Array.isArray(first)) {
    throw new Error('первая запись mail-merge должна быть объектом');
  }
  const recipients = records.slice(1);
  for (const [index, record] of records.entries()) {
    if (!record || typeof record !== 'object' || Array.isArray(record)) {
      throw new Error(`запись ${index} должна быть объектом`);
    }
    for (const [field, value] of Object.entries(record)) {
      if (typeof value !== 'string') throw new Error(`поле «${field}» записи ${index} должно быть строкой`);
    }
  }
  const document = await readXml(context.zip, DOCUMENT_PART);
  let replacements = 0;
  const entries = Object.entries(first as Record<string, unknown>).sort(([left], [right]) => right.length - left.length);
  for (const [field, rawValue] of entries) {
    if (typeof rawValue !== 'string') throw new Error(`поле «${field}» первой записи должно быть строкой`);
    for (const placeholder of [`{{${field}}}`, `«${field}»`]) {
      replacements += replaceText(document, placeholder, rawValue, true).count;
    }
  }
  if (replacements > 0) writeXml(context.zip, DOCUMENT_PART, document);
  return {
    changed: replacements > 0,
    value: {
      merged: 1,
      replacements,
      remaining: recipients.length,
      recipients,
    },
  };
}

export async function addComment(context: EditorContext): Promise<CommandResult> {
  const text = requiredString(context.args, 'text');
  let comments: Document;
  if (context.zip.file(COMMENTS_PART)) {
    comments = await readXml(context.zip, COMMENTS_PART);
  } else {
    comments = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments xmlns:w="${NS.w}"/>`);
  }
  const id = nextId([comments], 'comment', 'w:id', 0);
  const comment = w(comments, 'comment');
  setWAttr(comment, 'id', id);
  setWAttr(comment, 'author', 'Локальный редактор');
  setWAttr(comment, 'initials', 'ЛР');
  setWAttr(comment, 'date', new Date().toISOString());
  const commentParagraph = createParagraph(comments, text);
  comment.appendChild(commentParagraph);
  comments.documentElement.appendChild(comment);
  writeXml(context.zip, COMMENTS_PART, comments);
  await ensureRelationship(context.zip, DOCUMENT_PART, COMMENTS_RELATIONSHIP, 'comments.xml');
  await ensureContentType(
    context.zip,
    COMMENTS_PART,
    'application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml',
  );

  const document = await readXml(context.zip, DOCUMENT_PART);
  const { run } = addressedRun(document, context.args);
  const parent = run.parentNode;
  if (!parent) throw new Error('run отсоединён от документа');
  const rangeStart = w(document, 'commentRangeStart');
  setWAttr(rangeStart, 'id', id);
  parent.insertBefore(rangeStart, run);
  const rangeEnd = w(document, 'commentRangeEnd');
  setWAttr(rangeEnd, 'id', id);
  parent.insertBefore(rangeEnd, run.nextSibling);
  const referenceRun = w(document, 'r');
  const properties = runProperties(document, referenceRun);
  const style = w(document, 'rStyle');
  setWAttr(style, 'val', 'CommentReference');
  properties.appendChild(style);
  referenceRun.appendChild(properties);
  const reference = w(document, 'commentReference');
  setWAttr(reference, 'id', id);
  referenceRun.appendChild(reference);
  parent.insertBefore(referenceRun, rangeEnd.nextSibling);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true, value: { id } };
}
