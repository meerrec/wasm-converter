import type JSZip from 'jszip';
import type { EditorArgs } from './types.js';
import { numberArg } from './commands.js';
import { normalPath, parseXml, readXml } from './xml.js';
import {
  NS,
  children,
  clone,
  descendants,
  direct,
  element,
  ensureDirect,
  first,
  localName,
  setAttr,
  setNsAttr,
  writeXml,
} from './xml.js';

export const REL = {
  slide: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide',
  slideLayout: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout',
  slideMaster: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster',
  image: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/image',
  chart: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart',
  notesSlide: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide',
} as const;

export const CONTENT_TYPE = {
  slide: 'application/vnd.openxmlformats-officedocument.presentationml.slide+xml',
  chart: 'application/vnd.openxmlformats-officedocument.drawingml.chart+xml',
} as const;

const SHAPE_NAMES = new Set(['sp', 'pic', 'graphicFrame', 'cxnSp', 'grpSp', 'contentPart']);

export interface PptxSlide {
  id: number;
  relationshipId: string;
  path: string;
}

export interface PptxPresentation {
  document: Document;
  relationships: Document;
  slides: PptxSlide[];
}

/** Возвращает строковый аргумент text/value с учётом значений команд ленты. */
export function textArgument(args: EditorArgs, fallback = ''): string {
  const text = args.text;
  if (typeof text === 'string') return text;
  const value = args.value;
  return typeof value === 'string' ? value : fallback;
}

/** Проверяет и возвращает 0-based индекс слайда. */
export function slideIndex(args: EditorArgs, count: number): number {
  const index = numberArg(args, 'slide', 0);
  if (!Number.isInteger(index) || index < 0 || index >= count) {
    throw new Error(`слайд ${index} не существует (всего слайдов: ${count})`);
  }
  return index;
}

/** Проверяет 0-based индекс объекта в дереве фигур слайда. */
export function shapeIndex(args: EditorArgs, count: number): number | null {
  if (args.shape === undefined) return null;
  const index = numberArg(args, 'shape', -1);
  if (!Number.isInteger(index) || index < 0 || index >= count) {
    throw new Error(`объект ${index} не существует (всего объектов: ${count})`);
  }
  return index;
}

/** Читает presentation.xml, его связи и упорядоченный список слайдов. */
export async function readPresentation(zip: JSZip): Promise<PptxPresentation> {
  const document = await readXml(zip, 'ppt/presentation.xml');
  const relationships = await readXml(zip, 'ppt/_rels/presentation.xml.rels');
  const byId = new Map<string, Element>();
  for (const relationship of descendants(relationships, 'Relationship')) {
    const id = relationship.getAttribute('Id');
    if (id) byId.set(id, relationship);
  }
  const slides: PptxSlide[] = [];
  const list = direct(document.documentElement, 'sldIdLst');
  for (const node of children(list ?? document.documentElement, 'sldId')) {
    const relationshipId = relationshipIdOf(node);
    const relationship = byId.get(relationshipId);
    const target = relationship?.getAttribute('Target');
    if (!relationshipId || !target || relationship?.getAttribute('Type') !== REL.slide) {
      throw new Error(`связь слайда «${relationshipId || 'без id'}» повреждена`);
    }
    slides.push({
      id: Number(node.getAttribute('id')),
      relationshipId,
      path: normalPath('ppt', target),
    });
  }
  return { document, relationships, slides };
}

/** Возвращает связанный с PresentationML элемент r:id. */
export function relationshipIdOf(node: Element): string {
  return node.getAttributeNS(NS.r, 'id') ?? node.getAttribute('r:id') ?? '';
}

/** Добавляет слайд в sldIdLst в указанную позицию. */
export function insertSlideReference(
  presentation: PptxPresentation,
  slide: PptxSlide,
  index: number,
): void {
  const list = ensureDirect(presentation.document, presentation.document.documentElement, 'p', 'sldIdLst', [
    'sldSz',
    'notesSz',
  ]);
  const node = element(presentation.document, 'p', 'sldId');
  setAttr(node, 'id', slide.id);
  setNsAttr(node, 'r', 'id', slide.relationshipId);
  const references = children(list, 'sldId');
  const before = references[index];
  if (before) list.insertBefore(node, before);
  else list.appendChild(node);
  presentation.slides.splice(index, 0, slide);
}

/** Возвращает следующий свободный идентификатор p:sldId. */
export function nextSlideId(presentation: PptxPresentation): number {
  let maximum = 255;
  for (const slide of presentation.slides) {
    if (Number.isFinite(slide.id)) maximum = Math.max(maximum, slide.id);
  }
  return maximum + 1;
}

/** Сохраняет presentation.xml и его rels. */
export function writePresentation(zip: JSZip, presentation: PptxPresentation): void {
  writeXml(zip, 'ppt/presentation.xml', presentation.document);
  writeXml(zip, 'ppt/_rels/presentation.xml.rels', presentation.relationships);
}

/** Создаёт валидный пустой слайд с обязательным групповым деревом. */
export function createSlideDocument(): Document {
  const document = createXmlDocument(
    'p',
    'sld',
    `<p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>`,
  );
  return document;
}

/** Создаёт XML-документ с стандартными namespace PPTX. */
export function createXmlDocument(
  prefix: 'p' | 'c',
  rootName: string,
  contents = '',
): Document {
  const source = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><${prefix}:${rootName} xmlns:p="${NS.p}" xmlns:a="${NS.a}" xmlns:r="${NS.r}" xmlns:c="${NS.c}">${contents}</${prefix}:${rootName}>`;
  return parseXml(source);
}

/** Возвращает дерево фигур слайда, добавляя обязательные групповые узлы. */
export function ensureShapeTree(document: Document): Element {
  const cSld = ensureDirect(document, document.documentElement, 'p', 'cSld', ['clrMapOvr', 'transition', 'timing']);
  const tree = ensureDirect(document, cSld, 'p', 'spTree', ['controls', 'extLst']);
  if (!direct(tree, 'nvGrpSpPr')) {
    const nv = element(document, 'p', 'nvGrpSpPr');
    const properties = element(document, 'p', 'cNvPr');
    setAttr(properties, 'id', 1);
    setAttr(properties, 'name', '');
    nv.appendChild(properties);
    nv.appendChild(element(document, 'p', 'cNvGrpSpPr'));
    nv.appendChild(element(document, 'p', 'nvPr'));
    tree.insertBefore(nv, tree.firstChild);
  }
  if (!direct(tree, 'grpSpPr')) {
    const group = element(document, 'p', 'grpSpPr');
    const transform = element(document, 'a', 'xfrm');
    for (const name of ['off', 'ext', 'chOff', 'chExt']) {
      const node = element(document, 'a', name);
      setAttr(node, name === 'off' || name === 'chOff' ? 'x' : 'cx', 0);
      setAttr(node, name === 'off' || name === 'chOff' ? 'y' : 'cy', 0);
      transform.appendChild(node);
    }
    group.appendChild(transform);
    const nv = direct(tree, 'nvGrpSpPr');
    tree.insertBefore(group, nv?.nextSibling ?? null);
  }
  return tree;
}

/** Возвращает адресуемые 0-based индексом объекты верхнего уровня слайда. */
export function slideShapes(document: Document): Element[] {
  return children(ensureShapeTree(document)).filter((node) => SHAPE_NAMES.has(localName(node)));
}

/** Возвращает объект по индексу или выбрасывает понятную ошибку. */
export function requireShape(document: Document, args: EditorArgs): Element {
  const shapes = slideShapes(document);
  const index = shapeIndex(args, shapes.length);
  if (index === null) throw new Error('не задан параметр «shape»');
  const shape = shapes[index];
  if (!shape) throw new Error('объект слайда не найден');
  return shape;
}

/** Возвращает следующий свободный идентификатор cNvPr на слайде. */
export function nextShapeId(document: Document): number {
  let maximum = 1;
  for (const node of descendants(document, 'cNvPr')) {
    const id = Number(node.getAttribute('id'));
    if (Number.isFinite(id)) maximum = Math.max(maximum, id);
  }
  return maximum + 1;
}

/** Возвращает размер слайда в EMU. */
export async function slideSize(zip: JSZip): Promise<{ cx: number; cy: number }> {
  const document = await readXml(zip, 'ppt/presentation.xml');
  const size = direct(document.documentElement, 'sldSz');
  return {
    cx: positiveNumber(size?.getAttribute('cx'), 12_192_000),
    cy: positiveNumber(size?.getAttribute('cy'), 6_858_000),
  };
}

/** Создаёт преобразование положения и размера фигуры в EMU. */
export function createTransform(
  document: Document,
  x: number,
  y: number,
  cx: number,
  cy: number,
): Element {
  const transform = element(document, 'a', 'xfrm');
  const offset = element(document, 'a', 'off');
  setAttr(offset, 'x', Math.round(x));
  setAttr(offset, 'y', Math.round(y));
  const extent = element(document, 'a', 'ext');
  setAttr(extent, 'cx', Math.round(cx));
  setAttr(extent, 'cy', Math.round(cy));
  transform.appendChild(offset);
  transform.appendChild(extent);
  return transform;
}

/** Создаёт текстовый блок с заданным текстом. */
export function createTextBox(document: Document, text: string, size: { cx: number; cy: number }): Element {
  const id = nextShapeId(document);
  const shape = element(document, 'p', 'sp');
  const nv = element(document, 'p', 'nvSpPr');
  const properties = element(document, 'p', 'cNvPr');
  setAttr(properties, 'id', id);
  setAttr(properties, 'name', `TextBox ${id}`);
  const nonVisual = element(document, 'p', 'cNvSpPr');
  setAttr(nonVisual, 'txBox', 1);
  nv.appendChild(properties);
  nv.appendChild(nonVisual);
  nv.appendChild(element(document, 'p', 'nvPr'));
  const shapeProperties = element(document, 'p', 'spPr');
  const width = Math.min(5_486_400, Math.max(914_400, size.cx - 1_828_800));
  const height = 914_400;
  shapeProperties.appendChild(createTransform(
    document,
    Math.max(0, (size.cx - width) / 2),
    Math.min(Math.max(0, size.cy - height), 1_371_600),
    width,
    height,
  ));
  const geometry = element(document, 'a', 'prstGeom');
  setAttr(geometry, 'prst', 'rect');
  geometry.appendChild(element(document, 'a', 'avLst'));
  shapeProperties.appendChild(geometry);
  const body = element(document, 'p', 'txBody');
  const bodyProperties = element(document, 'a', 'bodyPr');
  setAttr(bodyProperties, 'wrap', 'square');
  body.appendChild(bodyProperties);
  body.appendChild(element(document, 'a', 'lstStyle'));
  appendTextParagraphs(document, body, text, null, null);
  shape.appendChild(nv);
  shape.appendChild(shapeProperties);
  shape.appendChild(body);
  ensureShapeTree(document).appendChild(shape);
  return shape;
}

/** Заменяет текст существующей фигуры, сохраняя свойства первого абзаца и run. */
export function setShapeText(shape: Element, text: string): void {
  const body = direct(shape, 'txBody');
  if (!body) throw new Error('выбранный объект не содержит редактируемый текст');
  const document = shape.ownerDocument;
  if (!document) throw new Error('объект не принадлежит XML-документу');
  const oldParagraph = direct(body, 'p');
  const paragraphProperties = oldParagraph ? direct(oldParagraph, 'pPr') : null;
  const oldRun = descendants(body, 'r')[0];
  const runProperties = oldRun ? direct(oldRun, 'rPr') : null;
  for (const paragraph of children(body, 'p')) body.removeChild(paragraph);
  appendTextParagraphs(document, body, text, paragraphProperties, runProperties);
}

/** Очищает текст фигуры или всех ячеек таблицы. */
export function clearText(shape: Element): void {
  if (direct(shape, 'txBody')) {
    setShapeText(shape, '');
    return;
  }
  const paragraphs = descendants(shape, 'p');
  if (paragraphs.length === 0) throw new Error('выбранный объект не содержит редактируемый текст');
  for (const paragraph of paragraphs) {
    for (const child of children(paragraph)) {
      if (localName(child) !== 'pPr') paragraph.removeChild(child);
    }
  }
}

/** Возвращает текст фигуры с переводами строк между абзацами. */
export function shapeText(shape: Element): string {
  return descendants(shape, 'p')
    .map((paragraph) => descendants(paragraph, 't').map((node) => node.textContent ?? '').join(''))
    .join('\n');
}

/** Возвращает текст слайда с переводами строк между абзацами. */
export function slideText(document: Document): string {
  const tree = ensureShapeTree(document);
  return slideShapes(document)
    .map(shapeText)
    .filter((text) => text.length > 0)
    .join('\n') || descendants(tree, 't').map((node) => node.textContent ?? '').join('');
}

/** Возвращает все текстовые run'ы фигуры или слайда. */
export function textRuns(parent: Node): Element[] {
  return descendants(parent, 'r').filter((run) => run.namespaceURI === NS.a || direct(run, 't'));
}

/** Возвращает или создаёт rPr в schema-позиции перед a:t. */
export function ensureRunProperties(run: Element): Element {
  const existing = direct(run, 'rPr');
  if (existing) return existing;
  const document = run.ownerDocument;
  if (!document) throw new Error('run не принадлежит XML-документу');
  const properties = element(document, 'a', 'rPr');
  run.insertBefore(properties, run.firstChild);
  return properties;
}

/** Находит rPr первого текстового run'а фигуры. */
export function firstRunProperties(shape: Element): Element | null {
  const run = textRuns(shape)[0];
  return run ? direct(run, 'rPr') : null;
}

/** Клонирует узел в другой XML-документ. */
export function importClone<T extends Node>(document: Document, node: T): T {
  return document.importNode(node, true) as T;
}

/** Возвращает первый потомок по локальному имени. */
export function descendant(parent: Node, name: string): Element | null {
  return first(parent, name);
}

function appendTextParagraphs(
  document: Document,
  body: Element,
  text: string,
  paragraphProperties: Element | null,
  runProperties: Element | null,
): void {
  for (const line of text.replace(/\r\n?/g, '\n').split('\n')) {
    const paragraph = element(document, 'a', 'p');
    if (paragraphProperties) paragraph.appendChild(clone(paragraphProperties));
    const run = element(document, 'a', 'r');
    if (runProperties) run.appendChild(clone(runProperties));
    const textNode = element(document, 'a', 't', line);
    if (/^\s|\s$/.test(line)) textNode.setAttribute('xml:space', 'preserve');
    run.appendChild(textNode);
    paragraph.appendChild(run);
    body.appendChild(paragraph);
  }
}

function positiveNumber(value: string | null | undefined, fallback: number): number {
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? number : fallback;
}
