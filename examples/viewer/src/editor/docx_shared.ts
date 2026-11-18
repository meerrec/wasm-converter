import type JSZip from 'jszip';
import type { EditorArgs } from './types.js';
import {
  NS,
  children,
  clone,
  descendants,
  direct,
  element,
  ensureDirect,
  first,
  parseXml,
  readXml,
  setNsAttr,
} from './xml.js';
import { ensureContentType, ensureRelationship } from './package.js';

export const DOCUMENT_PART = 'word/document.xml';
export const STYLES_PART = 'word/styles.xml';
export const RELATIONSHIP_BASE = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships';
export const XML_NAMESPACE = 'http://www.w3.org/XML/1998/namespace';

export interface TextReplacement {
  count: number;
  paragraph: number;
  run: number;
}

export function w(document: Document, name: string, text?: string): Element {
  return element(document, 'w', name, text);
}

export function a(document: Document, name: string, text?: string): Element {
  return element(document, 'a', name, text);
}

export function wAttr(node: Element, name: string): string | null {
  return node.getAttributeNS(NS.w, name) ?? node.getAttribute(`w:${name}`) ?? node.getAttribute(name);
}

export function setWAttr(node: Element, name: string, value: string | number): void {
  setNsAttr(node, 'w', name, value);
}

export function removeWAttr(node: Element, name: string): void {
  node.removeAttributeNS(NS.w, name);
  node.removeAttribute(`w:${name}`);
  node.removeAttribute(name);
}

export function setPreserveSpace(node: Element): void {
  node.setAttributeNS(XML_NAMESPACE, 'xml:space', 'preserve');
}

export function runProperties(document: Document, run: Element): Element {
  return ensureDirect(document, run, 'w', 'rPr');
}

export function paragraphProperties(document: Document, paragraph: Element): Element {
  return ensureDirect(document, paragraph, 'w', 'pPr');
}

export function documentParagraphs(document: Document): Element[] {
  return descendants(document, 'p');
}

export function paragraphRuns(paragraph: Element): Element[] {
  return descendants(paragraph, 'r');
}

function nonNegativeIndex(args: EditorArgs, name: string): number | null {
  if (args[name] === undefined) return null;
  const value = args[name];
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0) {
    throw new Error(`параметр «${name}» должен быть неотрицательным целым числом`);
  }
  return value;
}

export function addressedParagraph(document: Document, args: EditorArgs): Element {
  const index = nonNegativeIndex(args, 'paragraph');
  if (index === null) throw new Error('не задан параметр «paragraph»');
  const paragraph = documentParagraphs(document)[index];
  if (!paragraph) throw new Error(`абзац ${index} не найден`);
  return paragraph;
}

export function addressedRun(document: Document, args: EditorArgs): { paragraph: Element; run: Element } {
  const paragraph = addressedParagraph(document, args);
  const index = nonNegativeIndex(args, 'run');
  if (index === null) throw new Error('не задан параметр «run»');
  const run = paragraphRuns(paragraph)[index];
  if (!run) throw new Error(`run ${index} не найден`);
  return { paragraph, run };
}

export function selectedParagraphs(document: Document, args: EditorArgs): Element[] {
  const index = nonNegativeIndex(args, 'paragraph');
  const result = index === null ? documentParagraphs(document) : [addressedParagraph(document, args)];
  if (result.length === 0) throw new Error('в документе нет абзацев');
  return result;
}

export function selectedRuns(document: Document, args: EditorArgs): Element[] {
  const paragraphIndex = nonNegativeIndex(args, 'paragraph');
  const runIndex = nonNegativeIndex(args, 'run');
  if ((paragraphIndex === null) !== (runIndex === null)) {
    throw new Error('для адресной правки нужны оба параметра: «paragraph» и «run»');
  }
  if (paragraphIndex !== null && runIndex !== null) return [addressedRun(document, args).run];
  const result = selectedParagraphs(document, args).flatMap(paragraphRuns);
  if (result.length === 0) throw new Error('в выбранной области нет run’ов');
  return result;
}

export function runText(run: Element): string {
  let result = '';
  const visit = (node: Node): void => {
    for (let child = node.firstChild; child; child = child.nextSibling) {
      if (child.nodeType !== 1) continue;
      const elementNode = child as Element;
      const name = elementNode.localName ?? elementNode.nodeName.replace(/^.*:/, '');
      if (name === 't' || name === 'delText' || name === 'instrText') result += child.textContent ?? '';
      else if (name === 'tab') result += '\t';
      else if (name === 'br' || name === 'cr') result += '\n';
      else if (name === 'noBreakHyphen') result += '‑';
      else visit(child);
    }
  };
  visit(run);
  return result;
}

export function paragraphText(paragraph: Element): string {
  return paragraphRuns(paragraph).map(runText).join('');
}

export function setRunText(document: Document, run: Element, text: string): void {
  const properties = direct(run, 'rPr');
  for (const child of children(run)) {
    if (child !== properties) run.removeChild(child);
  }
  const textNode = w(document, 't', text);
  if (/^\s|\s$/.test(text) || text.includes('  ')) setPreserveSpace(textNode);
  run.appendChild(textNode);
}

export function appendTextRun(document: Document, paragraph: Element, text: string): Element {
  const run = w(document, 'r');
  paragraph.appendChild(run);
  setRunText(document, run, text);
  return run;
}

export function bodyOf(document: Document): Element {
  const body = first(document, 'body');
  if (!body) throw new Error('в word/document.xml нет w:body');
  return body;
}

export function appendToBody(document: Document, node: Element): void {
  const body = bodyOf(document);
  const section = direct(body, 'sectPr');
  if (section) body.insertBefore(node, section);
  else body.appendChild(node);
}

export function createParagraph(document: Document, text?: string): Element {
  const paragraph = w(document, 'p');
  if (text !== undefined) appendTextRun(document, paragraph, text);
  return paragraph;
}

export async function ensureStyles(zip: JSZip): Promise<Document> {
  let document: Document;
  if (zip.file(STYLES_PART)) {
    document = await readXml(zip, STYLES_PART);
  } else {
    document = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="${NS.w}"/>`);
    await ensureRelationship(zip, DOCUMENT_PART, `${RELATIONSHIP_BASE}/styles`, 'styles.xml');
    await ensureContentType(
      zip,
      STYLES_PART,
      'application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml',
    );
  }
  return document;
}

export function ensureStyle(
  document: Document,
  styleId: string,
  name: string,
  type: 'paragraph' | 'character' = 'paragraph',
): Element {
  const existing = descendants(document, 'style').find((style) => wAttr(style, 'styleId') === styleId);
  if (existing) return existing;
  const style = w(document, 'style');
  setWAttr(style, 'type', type);
  setWAttr(style, 'styleId', styleId);
  if (styleId === 'Normal') setWAttr(style, 'default', '1');
  const nameNode = w(document, 'name');
  setWAttr(nameNode, 'val', name);
  style.appendChild(nameNode);
  if (styleId !== 'Normal') {
    const basedOn = w(document, 'basedOn');
    setWAttr(basedOn, 'val', 'Normal');
    style.appendChild(basedOn);
    style.appendChild(w(document, 'qFormat'));
  }
  document.documentElement.appendChild(style);
  return style;
}

export function replaceStyleProperties(document: Document, style: Element, name: 'rPr' | 'pPr'): Element {
  const old = direct(style, name);
  if (old) style.removeChild(old);
  const properties = w(document, name);
  style.appendChild(properties);
  return properties;
}

export function importElement(document: Document, node: Element): Element {
  return document.importNode ? (document.importNode(node, true) as Element) : clone(node);
}

export function replaceTextInParagraph(
  document: Document,
  paragraph: Element,
  search: string,
  replacement: string,
  all: boolean,
  startAt = 0,
): TextReplacement {
  if (!search) throw new Error('строка поиска не должна быть пустой');
  let count = 0;
  let firstRun = -1;
  let from = startAt;
  for (;;) {
    const runs = paragraphRuns(paragraph);
    const texts = runs.map(runText);
    const joined = texts.join('');
    const match = joined.indexOf(search, from);
    if (match < 0) break;
    const end = match + search.length;
    let offset = 0;
    let startRun = -1;
    let startOffset = 0;
    let endRun = -1;
    let endOffset = 0;
    for (let index = 0; index < runs.length; index += 1) {
      const text = texts[index] ?? '';
      const next = offset + text.length;
      if (startRun < 0 && match >= offset && match <= next && (text.length > 0 || match === offset)) {
        startRun = index;
        startOffset = match - offset;
      }
      if (endRun < 0 && end >= offset && end <= next && (end < next || next === end)) {
        endRun = index;
        endOffset = end - offset;
        break;
      }
      offset = next;
    }
    if (startRun < 0 || endRun < 0) break;
    if (firstRun < 0) firstRun = startRun;
    if (startRun === endRun) {
      const text = texts[startRun] ?? '';
      setRunText(document, runs[startRun] as Element, text.slice(0, startOffset) + replacement + text.slice(endOffset));
    } else {
      const firstText = texts[startRun] ?? '';
      const lastText = texts[endRun] ?? '';
      setRunText(document, runs[startRun] as Element, firstText.slice(0, startOffset) + replacement);
      for (let index = startRun + 1; index < endRun; index += 1) setRunText(document, runs[index] as Element, '');
      setRunText(document, runs[endRun] as Element, lastText.slice(endOffset));
    }
    count += 1;
    if (!all) break;
    from = match + replacement.length;
  }
  return { count, paragraph: -1, run: firstRun };
}

export function replaceText(
  document: Document,
  search: string,
  replacement: string,
  all: boolean,
): TextReplacement {
  let count = 0;
  let firstParagraph = -1;
  let firstRun = -1;
  const paragraphs = documentParagraphs(document);
  for (let index = 0; index < paragraphs.length; index += 1) {
    const result = replaceTextInParagraph(document, paragraphs[index] as Element, search, replacement, all);
    if (result.count > 0 && firstParagraph < 0) {
      firstParagraph = index;
      firstRun = result.run;
    }
    count += result.count;
    if (count > 0 && !all) break;
  }
  return { count, paragraph: firstParagraph, run: firstRun };
}

export function positiveNumber(value: unknown, name: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
    throw new Error(`параметр «${name}» должен быть положительным числом`);
  }
  return value;
}

export function requiredArrayBuffer(value: unknown, name = 'bytes'): ArrayBuffer {
  if (!(value instanceof ArrayBuffer)) throw new Error(`параметр «${name}» должен быть ArrayBuffer`);
  return value;
}

export function insertField(paragraph: Element, instruction: string, displayText: string): void {
  const document = paragraph.ownerDocument;
  if (!document) throw new Error('у абзаца нет документа-владельца');
  const beginRun = w(document, 'r');
  const begin = w(document, 'fldChar');
  setWAttr(begin, 'fldCharType', 'begin');
  setWAttr(begin, 'dirty', 'true');
  beginRun.appendChild(begin);
  paragraph.appendChild(beginRun);
  const instructionRun = w(document, 'r');
  const instructionNode = w(document, 'instrText', instruction);
  setPreserveSpace(instructionNode);
  instructionRun.appendChild(instructionNode);
  paragraph.appendChild(instructionRun);
  const separatorRun = w(document, 'r');
  const separator = w(document, 'fldChar');
  setWAttr(separator, 'fldCharType', 'separate');
  separatorRun.appendChild(separator);
  paragraph.appendChild(separatorRun);
  appendTextRun(document, paragraph, displayText);
  const endRun = w(document, 'r');
  const end = w(document, 'fldChar');
  setWAttr(end, 'fldCharType', 'end');
  endRun.appendChild(end);
  paragraph.appendChild(endRun);
}

export function boolValue(node: Element | null, fallback = false): boolean {
  if (!node) return fallback;
  const value = wAttr(node, 'val') ?? wAttr(node, 'value');
  return value === null || !['0', 'false', 'off', 'none'].includes(value.toLowerCase());
}

export function setToggle(properties: Element, document: Document, name: string, enabled: boolean): void {
  const node = ensureDirect(document, properties, 'w', name);
  setWAttr(node, 'val', enabled ? '1' : '0');
}
