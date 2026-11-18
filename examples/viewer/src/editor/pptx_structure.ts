import type JSZip from 'jszip';
import type { EditorArgs } from './types.js';
import type { CommandResult } from './commands.js';
import {
  NS,
  baseName,
  children,
  descendants,
  direct,
  element,
  first,
  parseXml,
  readXml,
  relsPath,
  serializeXml,
  setAttr,
  writeXml,
} from './xml.js';
import {
  ensureContentType,
  ensureRelationship,
  nextPartName,
  relativeTarget,
} from './package.js';
import {
  CONTENT_TYPE,
  REL,
  clearText,
  createSlideDocument,
  ensureShapeTree,
  insertSlideReference,
  nextSlideId,
  readPresentation,
  relationshipIdOf,
  slideIndex,
  slideShapes,
  writePresentation,
} from './pptx_model.js';
import type { PptxPresentation, PptxSlide } from './pptx_model.js';

export interface PptxLayout {
  path: string;
  name: string;
}

/** Оставляет в пакете один пустой слайд и удаляет старые slide-part'ы. */
export async function newDocument(zip: JSZip): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const layouts = await readLayouts(zip, presentation);
  const layout = layouts[0];
  if (!layout) throw new Error('в презентации нет slide layout для нового документа');
  const path = 'ppt/slides/slide1.xml';

  for (const file of zip.file(/^ppt\/slides\//)) zip.remove(file.name);
  removeSlideContentTypes(await readXml(zip, '[Content_Types].xml'), zip);
  for (const relationship of descendants(presentation.relationships, 'Relationship')) {
    if (relationship.getAttribute('Type') === REL.slide) relationship.parentNode?.removeChild(relationship);
  }
  const list = direct(presentation.document.documentElement, 'sldIdLst');
  if (list) {
    for (const reference of children(list, 'sldId')) list.removeChild(reference);
  }
  presentation.slides = [];
  writePresentation(zip, presentation);

  writeXml(zip, path, createSlideDocument());
  await ensureRelationship(zip, path, REL.slideLayout, relativeTarget(path, layout.path));
  await ensureContentType(zip, path, CONTENT_TYPE.slide);
  const relationshipId = await ensureRelationship(zip, pathForPresentation(), REL.slide, 'slides/slide1.xml');
  const slide = { id: nextSlideId(presentation), relationshipId, path };
  insertSlideReference(presentation, slide, 0);
  writeXml(zip, 'ppt/presentation.xml', presentation.document);
  return { changed: true, value: { slide: 0, path } };
}

/** Создаёт новый пустой слайд на выбранном layout. */
export async function createSlide(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const layout = await selectLayout(zip, presentation, args.layout ?? 0);
  const path = nextPartName(zip, 'ppt/slides', 'slide', 'xml');
  const index = presentation.slides.length;
  const slide: PptxSlide = {
    id: nextSlideId(presentation),
    relationshipId: '',
    path,
  };
  writeXml(zip, path, createSlideDocument());
  await ensureRelationship(zip, path, REL.slideLayout, relativeTarget(path, layout.path));
  await ensureContentType(zip, path, CONTENT_TYPE.slide);
  slide.relationshipId = await ensureRelationship(
    zip,
    pathForPresentation(),
    REL.slide,
    relativeTarget(pathForPresentation(), path),
  );
  insertSlideReference(presentation, slide, index);
  writeXml(zip, 'ppt/presentation.xml', presentation.document);
  return { changed: true, value: { slide: index, path, layout: layout.name } };
}

/** Меняет существующий slide layout у слайда. */
export async function changeSlideLayout(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  if (args.layout === undefined) throw new Error('не задан параметр «layout»');
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const layout = await selectLayout(zip, presentation, args.layout);
  const path = relsPath(slide.path);
  let relationships: Document;
  try {
    relationships = await readXml(zip, path);
  } catch {
    relationships = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="${NS.rel}"/>`);
  }
  for (const relationship of descendants(relationships, 'Relationship')) {
    if (relationship.getAttribute('Type') === REL.slideLayout) {
      relationship.parentNode?.removeChild(relationship);
    }
  }
  writeXml(zip, path, relationships);
  await ensureRelationship(zip, slide.path, REL.slideLayout, relativeTarget(slide.path, layout.path));
  return { changed: true, value: { slide: index, layout: layout.name, path: layout.path } };
}

/** Удаляет не-placeholder объекты и очищает текст placeholder'ов. */
export async function resetSlide(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  const tree = ensureShapeTree(document);
  for (const shape of slideShapes(document)) {
    if (first(shape, 'ph')) {
      if (first(shape, 'txBody') || first(shape, 'tbl')) clearText(shape);
    } else {
      tree.removeChild(shape);
    }
  }
  writeXml(zip, slide.path, document);
  return { changed: true, value: { slide: index } };
}

/** Удаляет слайд, его rels, presentation relationship и content-type override. */
export async function deleteSlide(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const list = direct(presentation.document.documentElement, 'sldIdLst');
  const reference = children(list ?? presentation.document.documentElement, 'sldId')[index];
  if (reference?.parentNode) reference.parentNode.removeChild(reference);
  for (const relationship of descendants(presentation.relationships, 'Relationship')) {
    if (relationship.getAttribute('Id') === slide.relationshipId) {
      relationship.parentNode?.removeChild(relationship);
    }
  }
  presentation.slides.splice(index, 1);
  writePresentation(zip, presentation);
  zip.remove(slide.path);
  zip.remove(relsPath(slide.path));
  await removeContentType(zip, slide.path);
  return { changed: true, value: { slide: index, path: slide.path } };
}

/** Дублирует слайд и его связи, кроме notesSlide с обратной связью на источник. */
export async function duplicateSlide(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const source = presentation.slides[index];
  if (!source) throw new Error('слайд не найден');
  const sourceDocument = await readXml(zip, source.path);
  const path = nextPartName(zip, 'ppt/slides', 'slide', 'xml');
  writeXml(zip, path, parseXml(serializeXml(sourceDocument)));

  const sourceRels = zip.file(relsPath(source.path));
  if (sourceRels) {
    const relationships = parseXml(await sourceRels.async('string'));
    for (const relationship of descendants(relationships, 'Relationship')) {
      if (relationship.getAttribute('Type') === REL.notesSlide) {
        relationship.parentNode?.removeChild(relationship);
      }
    }
    writeXml(zip, relsPath(path), relationships);
  }
  await ensureContentType(zip, path, CONTENT_TYPE.slide);
  const relationshipId = await ensureRelationship(
    zip,
    pathForPresentation(),
    REL.slide,
    relativeTarget(pathForPresentation(), path),
  );
  const copy = { id: nextSlideId(presentation), relationshipId, path };
  insertSlideReference(presentation, copy, index + 1);
  writeXml(zip, 'ppt/presentation.xml', presentation.document);
  return { changed: true, value: { slide: index + 1, source: index, path } };
}

/** Переставляет sldId по полной 0-based перестановке слайдов. */
export async function sortSlides(zip: JSZip, args: EditorArgs): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const order = args.order;
  if (!Array.isArray(order) || order.length !== presentation.slides.length) {
    throw new Error('параметр «order» должен содержать индексы всех слайдов');
  }
  const normalized = order.map((value) => typeof value === 'number' ? value : Number.NaN);
  const unique = new Set(normalized);
  if (normalized.some((value) => !Number.isInteger(value) || value < 0 || value >= order.length)
    || unique.size !== order.length) {
    throw new Error('параметр «order» должен быть перестановкой 0-based индексов слайдов');
  }
  const list = direct(presentation.document.documentElement, 'sldIdLst');
  if (!list) throw new Error('в presentation.xml нет sldIdLst');
  const references = children(list, 'sldId');
  for (const reference of references) list.removeChild(reference);
  const sortedSlides: PptxSlide[] = [];
  for (const sourceIndex of normalized) {
    const reference = references[sourceIndex];
    const slide = presentation.slides[sourceIndex];
    if (!reference || !slide) throw new Error('список слайдов повреждён');
    list.appendChild(reference);
    sortedSlides.push(slide);
  }
  presentation.slides = sortedSlides;
  writePresentation(zip, presentation);
  return { changed: true, value: { order: normalized } };
}

/** Возвращает layout'ы в порядке master → sldLayoutIdLst. */
export async function readLayouts(
  zip: JSZip,
  existingPresentation?: PptxPresentation,
): Promise<PptxLayout[]> {
  const presentation = existingPresentation ?? await readPresentation(zip);
  const paths: string[] = [];
  const masters = descendants(presentation.document.documentElement, 'sldMasterId');
  for (const masterReference of masters) {
    const relationshipId = relationshipIdOf(masterReference);
    const masterRelationship = descendants(presentation.relationships, 'Relationship').find(
      (node) => node.getAttribute('Id') === relationshipId && node.getAttribute('Type') === REL.slideMaster,
    );
    const target = masterRelationship?.getAttribute('Target');
    if (!target) continue;
    const masterPath = target.startsWith('/') ? target.slice(1) : `ppt/${target}`;
    const normalizedMasterPath = normalize(masterPath);
    const master = await readXml(zip, normalizedMasterPath);
    const masterRels = await readXml(zip, relsPath(normalizedMasterPath));
    for (const layoutReference of descendants(master.documentElement, 'sldLayoutId')) {
      const id = relationshipIdOf(layoutReference);
      const relationship = descendants(masterRels, 'Relationship').find(
        (node) => node.getAttribute('Id') === id && node.getAttribute('Type') === REL.slideLayout,
      );
      const layoutTarget = relationship?.getAttribute('Target');
      if (layoutTarget) paths.push(normalize(`${directory(normalizedMasterPath)}/${layoutTarget}`));
    }
  }
  if (paths.length === 0) {
    for (const file of zip.file(/^ppt\/slideLayouts\/slideLayout[^/]*\.xml$/)) paths.push(file.name);
    paths.sort(naturalCompare);
  }
  const uniquePaths = [...new Set(paths)];
  const layouts: PptxLayout[] = [];
  for (const path of uniquePaths) {
    const document = await readXml(zip, path);
    const common = direct(document.documentElement, 'cSld');
    layouts.push({
      path,
      name: common?.getAttribute('name')
        || document.documentElement.getAttribute('type')
        || baseName(path).replace(/\.xml$/i, ''),
    });
  }
  return layouts;
}

async function selectLayout(
  zip: JSZip,
  presentation: PptxPresentation,
  value: unknown,
): Promise<PptxLayout> {
  const layouts = await readLayouts(zip, presentation);
  if (layouts.length === 0) throw new Error('в презентации нет доступных slide layout');
  if (typeof value === 'number' && Number.isInteger(value)) {
    const layout = layouts[value];
    if (layout) return layout;
  } else if (typeof value === 'string' && value.trim()) {
    const name = value.trim();
    const layout = layouts.find((candidate) => candidate.name.localeCompare(name, undefined, { sensitivity: 'accent' }) === 0
      || candidate.path === name
      || baseName(candidate.path).replace(/\.xml$/i, '') === name);
    if (layout) return layout;
    if (/^\d+$/.test(name)) {
      const byIndex = layouts[Number(name)];
      if (byIndex) return byIndex;
    }
  }
  throw new Error(`slide layout «${String(value)}» не найден (доступно: ${layouts.length})`);
}

async function removeContentType(zip: JSZip, part: string): Promise<void> {
  const document = await readXml(zip, '[Content_Types].xml');
  for (const override of descendants(document, 'Override')) {
    if (override.getAttribute('PartName') === `/${part}`) override.parentNode?.removeChild(override);
  }
  writeXml(zip, '[Content_Types].xml', document);
}

function removeSlideContentTypes(document: Document, zip: JSZip): void {
  for (const override of descendants(document, 'Override')) {
    const part = override.getAttribute('PartName') ?? '';
    if (/^\/ppt\/slides\/[^/]+\.xml$/i.test(part)) override.parentNode?.removeChild(override);
  }
  writeXml(zip, '[Content_Types].xml', document);
}

function pathForPresentation(): string {
  return 'ppt/presentation.xml';
}

function directory(path: string): string {
  return path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
}

function normalize(path: string): string {
  const result: string[] = [];
  for (const part of path.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') result.pop();
    else result.push(part);
  }
  return result.join('/');
}

function naturalCompare(left: string, right: string): number {
  return left.localeCompare(right, undefined, { numeric: true });
}
