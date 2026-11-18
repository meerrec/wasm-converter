import type JSZip from 'jszip';
import type { EditorArgs } from './types.js';
import type { CommandResult } from './commands.js';
import { numberArg, stringArg } from './commands.js';
import {
  NS,
  children,
  direct,
  element,
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
import {
  CONTENT_TYPE,
  REL,
  clearText,
  createTextBox,
  createTransform,
  createXmlDocument,
  ensureShapeTree,
  nextShapeId,
  readPresentation,
  requireShape,
  setShapeText,
  shapeIndex,
  shapeText,
  slideIndex,
  slideShapes,
  slideSize,
  slideText,
  textArgument,
} from './pptx_model.js';

const EMU_PER_PIXEL = 9_525;

/** Читает текст одного или всех слайдов. */
export async function getText(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  if (args.slide !== undefined) {
    const index = slideIndex(args, presentation.slides.length);
    const slide = presentation.slides[index];
    if (!slide) throw new Error('слайд не найден');
    return { value: slideText(await readXml(zip, slide.path)) };
  }
  const texts: string[] = [];
  for (const slide of presentation.slides) texts.push(slideText(await readXml(zip, slide.path)));
  return { value: texts.join('\n\n') };
}

/** Вырезает текст объекта или всех текстовых объектов слайда. */
export async function cutText(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  const shapes = slideShapes(document);
  const selected = shapeIndex(args, shapes.length);
  const targets = selected === null
    ? shapes.filter((shape) => direct(shape, 'txBody') || hasTable(shape))
    : [requireShape(document, args)];
  const value = targets.map(shapeText).filter((text) => text.length > 0).join('\n');
  for (const shape of targets) clearText(shape);
  writeXml(zip, slide.path, document);
  return { changed: true, value };
}

/** Вставляет текст в объект или создаёт новый text box. */
export async function pasteText(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  return putText(zip, args, false);
}

/** Добавляет на слайд новый text box. */
export async function insertText(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  return putText(zip, args, true);
}

/** Вставляет таблицу DrawingML с заданным числом строк и столбцов. */
export async function insertTable(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const rows = numberArg(args, 'rows', 0);
  const cols = numberArg(args, 'cols', 0);
  if (!Number.isInteger(rows) || rows < 1 || !Number.isInteger(cols) || cols < 1) {
    throw new Error('параметры «rows» и «cols» должны быть положительными целыми числами');
  }
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  const size = await slideSize(zip);
  const id = nextShapeId(document);
  const frame = element(document, 'p', 'graphicFrame');
  const nv = element(document, 'p', 'nvGraphicFramePr');
  const properties = element(document, 'p', 'cNvPr');
  setAttr(properties, 'id', id);
  setAttr(properties, 'name', `Table ${id}`);
  const nonVisual = element(document, 'p', 'cNvGraphicFramePr');
  const locks = element(document, 'a', 'graphicFrameLocks');
  setAttr(locks, 'noGrp', 1);
  nonVisual.appendChild(locks);
  nv.appendChild(properties);
  nv.appendChild(nonVisual);
  nv.appendChild(element(document, 'p', 'nvPr'));

  const width = Math.max(914_400, size.cx - 1_828_800);
  const rowHeight = 411_480;
  const height = rows * rowHeight;
  const transform = element(document, 'p', 'xfrm');
  const offset = element(document, 'a', 'off');
  setAttr(offset, 'x', Math.round((size.cx - width) / 2));
  setAttr(offset, 'y', Math.min(1_371_600, Math.max(0, size.cy - height)));
  const extent = element(document, 'a', 'ext');
  setAttr(extent, 'cx', width);
  setAttr(extent, 'cy', height);
  transform.appendChild(offset);
  transform.appendChild(extent);

  const graphic = element(document, 'a', 'graphic');
  const data = element(document, 'a', 'graphicData');
  setAttr(data, 'uri', 'http://schemas.openxmlformats.org/drawingml/2006/table');
  const table = element(document, 'a', 'tbl');
  const tableProperties = element(document, 'a', 'tblPr');
  setAttr(tableProperties, 'firstRow', 1);
  setAttr(tableProperties, 'bandRow', 1);
  const style = element(document, 'a', 'tableStyleId', '{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}');
  tableProperties.appendChild(style);
  const grid = element(document, 'a', 'tblGrid');
  const columnWidth = Math.floor(width / cols);
  for (let column = 0; column < cols; column += 1) {
    const gridColumn = element(document, 'a', 'gridCol');
    setAttr(gridColumn, 'w', column === cols - 1 ? width - columnWidth * (cols - 1) : columnWidth);
    grid.appendChild(gridColumn);
  }
  table.appendChild(tableProperties);
  table.appendChild(grid);
  for (let row = 0; row < rows; row += 1) {
    const tableRow = element(document, 'a', 'tr');
    setAttr(tableRow, 'h', rowHeight);
    for (let column = 0; column < cols; column += 1) {
      tableRow.appendChild(createTableCell(document));
    }
    table.appendChild(tableRow);
  }
  data.appendChild(table);
  graphic.appendChild(data);
  frame.appendChild(nv);
  frame.appendChild(transform);
  frame.appendChild(graphic);
  ensureShapeTree(document).appendChild(frame);
  writeXml(zip, slide.path, document);
  return { changed: true, value: { slide: index, shape: slideShapes(document).length - 1, rows, cols } };
}

/** Добавляет image-part и p:pic с размерами, заданными в пикселях CSS. */
export async function insertPicture(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const bytes = args.bytes;
  if (!(bytes instanceof ArrayBuffer) || bytes.byteLength === 0) {
    throw new Error('параметр «bytes» должен быть непустым ArrayBuffer');
  }
  const mime = stringArg(args, 'mime').split(';', 1)[0]?.trim().toLowerCase() ?? '';
  const extension = imageExtension(mime);
  const width = numberArg(args, 'width', 0);
  const height = numberArg(args, 'height', 0);
  if (!Number.isFinite(width) || width <= 0 || !Number.isFinite(height) || height <= 0) {
    throw new Error('параметры «width» и «height» должны быть положительными числами');
  }
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  const size = await slideSize(zip);
  const path = nextPartName(zip, 'ppt/media', 'image', extension);
  await ensureDefaultExtension(zip, extension, mime);
  zip.file(path, bytes.slice(0));
  const relationshipId = await ensureRelationship(zip, slide.path, REL.image, relativeTarget(slide.path, path));

  const id = nextShapeId(document);
  const picture = element(document, 'p', 'pic');
  const nv = element(document, 'p', 'nvPicPr');
  const properties = element(document, 'p', 'cNvPr');
  setAttr(properties, 'id', id);
  setAttr(properties, 'name', stringArg(args, 'name', `Picture ${id}`));
  const nonVisual = element(document, 'p', 'cNvPicPr');
  const locks = element(document, 'a', 'picLocks');
  setAttr(locks, 'noChangeAspect', 1);
  nonVisual.appendChild(locks);
  nv.appendChild(properties);
  nv.appendChild(nonVisual);
  nv.appendChild(element(document, 'p', 'nvPr'));
  const fill = element(document, 'p', 'blipFill');
  const blip = element(document, 'a', 'blip');
  setNsAttr(blip, 'r', 'embed', relationshipId);
  const stretch = element(document, 'a', 'stretch');
  stretch.appendChild(element(document, 'a', 'fillRect'));
  fill.appendChild(blip);
  fill.appendChild(stretch);
  const shapeProperties = element(document, 'p', 'spPr');
  const cx = Math.round(width * EMU_PER_PIXEL);
  const cy = Math.round(height * EMU_PER_PIXEL);
  shapeProperties.appendChild(createTransform(
    document,
    (size.cx - cx) / 2,
    (size.cy - cy) / 2,
    cx,
    cy,
  ));
  const geometry = element(document, 'a', 'prstGeom');
  setAttr(geometry, 'prst', 'rect');
  geometry.appendChild(element(document, 'a', 'avLst'));
  shapeProperties.appendChild(geometry);
  picture.appendChild(nv);
  picture.appendChild(fill);
  picture.appendChild(shapeProperties);
  ensureShapeTree(document).appendChild(picture);
  writeXml(zip, slide.path, document);
  return { changed: true, value: { slide: index, shape: slideShapes(document).length - 1, path } };
}

/** Создаёт chart-part с кэшированными данными и вставляет graphicFrame. */
export async function insertChart(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const title = textArgument(args, stringArg(args, 'title', 'Chart'));
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  const size = await slideSize(zip);
  const path = nextPartName(zip, 'ppt/charts', 'chart', 'xml');
  const chart = createChartDocument(title);
  writeXml(zip, path, chart);
  await ensureContentType(zip, path, CONTENT_TYPE.chart);
  const relationshipId = await ensureRelationship(zip, slide.path, REL.chart, relativeTarget(slide.path, path));

  const id = nextShapeId(document);
  const frame = element(document, 'p', 'graphicFrame');
  const nv = element(document, 'p', 'nvGraphicFramePr');
  const properties = element(document, 'p', 'cNvPr');
  setAttr(properties, 'id', id);
  setAttr(properties, 'name', `Chart ${id}`);
  nv.appendChild(properties);
  nv.appendChild(element(document, 'p', 'cNvGraphicFramePr'));
  nv.appendChild(element(document, 'p', 'nvPr'));
  const transform = element(document, 'p', 'xfrm');
  const width = Math.max(914_400, size.cx - 2_743_200);
  const height = Math.max(914_400, size.cy - 2_743_200);
  const offset = element(document, 'a', 'off');
  setAttr(offset, 'x', Math.round((size.cx - width) / 2));
  setAttr(offset, 'y', Math.round((size.cy - height) / 2));
  const extent = element(document, 'a', 'ext');
  setAttr(extent, 'cx', width);
  setAttr(extent, 'cy', height);
  transform.appendChild(offset);
  transform.appendChild(extent);
  const graphic = element(document, 'a', 'graphic');
  const data = element(document, 'a', 'graphicData');
  setAttr(data, 'uri', NS.c);
  const reference = element(document, 'c', 'chart');
  setNsAttr(reference, 'r', 'id', relationshipId);
  data.appendChild(reference);
  graphic.appendChild(data);
  frame.appendChild(nv);
  frame.appendChild(transform);
  frame.appendChild(graphic);
  ensureShapeTree(document).appendChild(frame);
  writeXml(zip, slide.path, document);
  return { changed: true, value: { slide: index, shape: slideShapes(document).length - 1, path } };
}

async function putText(zip: JSZip, args: EditorArgs, alwaysNew: boolean): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  const text = textArgument(args);
  let shapePosition: number;
  if (!alwaysNew && args.shape !== undefined) {
    const shape = requireShape(document, args);
    setShapeText(shape, text);
    shapePosition = shapeIndex(args, slideShapes(document).length) ?? 0;
  } else {
    createTextBox(document, text, await slideSize(zip));
    shapePosition = slideShapes(document).length - 1;
  }
  writeXml(zip, slide.path, document);
  return { changed: true, value: { slide: index, shape: shapePosition } };
}

function createTableCell(document: Document): Element {
  const cell = element(document, 'a', 'tc');
  const body = element(document, 'a', 'txBody');
  body.appendChild(element(document, 'a', 'bodyPr'));
  body.appendChild(element(document, 'a', 'lstStyle'));
  body.appendChild(element(document, 'a', 'p'));
  const properties = element(document, 'a', 'tcPr');
  setAttr(properties, 'marL', 91_440);
  setAttr(properties, 'marR', 91_440);
  setAttr(properties, 'marT', 45_720);
  setAttr(properties, 'marB', 45_720);
  setAttr(properties, 'anchor', 'ctr');
  cell.appendChild(body);
  cell.appendChild(properties);
  return cell;
}

function hasTable(shape: Element): boolean {
  return children(shape).some((node) => node.localName === 'graphic')
    || shape.getElementsByTagNameNS(NS.a, 'tbl').length > 0;
}

function imageExtension(mime: string): string {
  const extensions: Record<string, string> = {
    'image/jpeg': 'jpg',
    'image/png': 'png',
    'image/gif': 'gif',
    'image/bmp': 'bmp',
    'image/tiff': 'tiff',
    'image/webp': 'webp',
    'image/svg+xml': 'svg',
    'image/x-emf': 'emf',
    'image/x-wmf': 'wmf',
  };
  const extension = extensions[mime];
  if (!extension) throw new Error(`неподдерживаемый MIME изображения «${mime || 'не задан'}»`);
  return extension;
}

function createChartDocument(title: string): Document {
  const document = createXmlDocument('c', 'chartSpace', `
    <c:chart>
      <c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t></a:t></a:r></a:p></c:rich></c:tx></c:title>
      <c:autoTitleDeleted val="0"/>
      <c:plotArea><c:layout/>
        <c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>
          <c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Series 1</c:v></c:tx>
            <c:cat><c:strLit><c:ptCount val="3"/><c:pt idx="0"><c:v>Category 1</c:v></c:pt><c:pt idx="1"><c:v>Category 2</c:v></c:pt><c:pt idx="2"><c:v>Category 3</c:v></c:pt></c:strLit></c:cat>
            <c:val><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>3</c:v></c:pt><c:pt idx="2"><c:v>2</c:v></c:pt></c:numLit></c:val>
          </c:ser><c:gapWidth val="150"/><c:axId val="111111111"/><c:axId val="222222222"/>
        </c:barChart>
        <c:catAx><c:axId val="111111111"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:majorTickMark val="out"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:crossAx val="222222222"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/></c:catAx>
        <c:valAx><c:axId val="222222222"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="out"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:crossAx val="111111111"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx>
      </c:plotArea><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/>
    </c:chart>`);
  const titleNode = document.getElementsByTagNameNS(NS.a, 't').item(0);
  if (titleNode) titleNode.textContent = title;
  return document;
}
