import { DOMParser, XMLSerializer } from '@xmldom/xmldom';
import type JSZip from 'jszip';

export const NS = {
  w: 'http://schemas.openxmlformats.org/wordprocessingml/2006/main',
  r: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships',
  a: 'http://schemas.openxmlformats.org/drawingml/2006/main',
  p: 'http://schemas.openxmlformats.org/presentationml/2006/main',
  x: 'http://schemas.openxmlformats.org/spreadsheetml/2006/main',
  rel: 'http://schemas.openxmlformats.org/package/2006/relationships',
  ct: 'http://schemas.openxmlformats.org/package/2006/content-types',
  xdr: 'http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing',
  c: 'http://schemas.openxmlformats.org/drawingml/2006/chart',
  wp: 'http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing',
  pic: 'http://schemas.openxmlformats.org/drawingml/2006/picture',
} as const;

export function parseXml(source: string): Document {
  const document = new DOMParser().parseFromString(source, 'application/xml');
  const error = document.getElementsByTagName('parsererror').item(0);
  if (error) throw new Error(`некорректный XML: ${error.textContent ?? 'ошибка разбора'}`);
  return document;
}

export function serializeXml(document: Document): string {
  return new XMLSerializer().serializeToString(document);
}

export function localName(node: Node): string {
  return (node as Element).localName ?? node.nodeName.replace(/^.*:/, '');
}

/** Добавить несколько узлов сразу: DOM-метода `append` у xmldom нет. */
export function append(parent: Node, ...nodes: Node[]): void {
  for (const node of nodes) parent.appendChild(node);
}

export function children(parent: Node, name?: string): Element[] {
  const result: Element[] = [];
  for (let node = parent.firstChild; node; node = node.nextSibling) {
    if (node.nodeType === 1 && (!name || localName(node) === name)) result.push(node as Element);
  }
  return result;
}

export function descendants(parent: Node, name: string): Element[] {
  const result: Element[] = [];
  const visit = (node: Node): void => {
    for (let child = node.firstChild; child; child = child.nextSibling) {
      if (child.nodeType !== 1) continue;
      if (localName(child) === name) result.push(child as Element);
      visit(child);
    }
  };
  visit(parent);
  return result;
}

export function first(parent: Node, name: string): Element | null {
  return descendants(parent, name)[0] ?? null;
}

export function direct(parent: Node, name: string): Element | null {
  return children(parent, name)[0] ?? null;
}

export function qname(document: Document, prefix: keyof typeof NS, name: string): string {
  const root = document.documentElement;
  // Корень в пространстве имён по умолчанию (`<worksheet xmlns="…">`) не
  // объявляет префикс: `lookupPrefix` вернёт null, и `x:row` уехал бы в XML
  // без объявления `xmlns:x` — xmldom такое не разбирает. Имя без префикса
  // наследует пространство имён корня.
  if (root.namespaceURI === NS[prefix] && (root.prefix === null || root.prefix === '')) {
    return name;
  }
  const declared = root.lookupPrefix?.(NS[prefix]);
  return `${declared ?? prefix}:${name}`;
}

export function element(
  document: Document,
  prefix: keyof typeof NS,
  name: string,
  text?: string,
): Element {
  const node = document.createElementNS(NS[prefix], qname(document, prefix, name));
  if (text !== undefined) node.textContent = text;
  return node;
}

export function ensureDirect(
  document: Document,
  parent: Element,
  prefix: keyof typeof NS,
  name: string,
  before?: string[],
): Element {
  const existing = direct(parent, name);
  if (existing) return existing;
  const node = element(document, prefix, name);
  if (before) {
    const next = children(parent).find((child) => before.includes(localName(child)));
    if (next) {
      parent.insertBefore(node, next);
      return node;
    }
  }
  parent.appendChild(node);
  return node;
}

export function setAttr(node: Element, name: string, value: string | number): void {
  node.setAttribute(name, String(value));
}

export function setNsAttr(
  node: Element,
  prefix: keyof typeof NS,
  name: string,
  value: string | number,
): void {
  node.setAttributeNS(NS[prefix], `${prefix}:${name}`, String(value));
}

export function removeChildren(parent: Element, name: string): void {
  for (const child of children(parent, name)) parent.removeChild(child);
}

export function replaceChildren(parent: Element, nodes: Node[]): void {
  while (parent.firstChild) parent.removeChild(parent.firstChild);
  for (const node of nodes) parent.appendChild(node);
}

export function textOf(parent: Node, name = 't'): string {
  return descendants(parent, name).map((node) => node.textContent ?? '').join('');
}

export async function readXml(zip: JSZip, path: string): Promise<Document> {
  const file = zip.file(path);
  if (!file) throw new Error(`в пакете нет ${path}`);
  return parseXml(await file.async('string'));
}

export function writeXml(zip: JSZip, path: string, document: Document): void {
  zip.file(path, serializeXml(document));
}

export function normalPath(base: string, target: string): string {
  if (target.startsWith('/')) return target.replace(/^\/+/, '');
  const parts = `${base}/${target}`.split('/');
  const result: string[] = [];
  for (const part of parts) {
    if (!part || part === '.') continue;
    if (part === '..') result.pop();
    else result.push(part);
  }
  return result.join('/');
}

export function dirName(path: string): string {
  return path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
}

export function baseName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

export function relsPath(part: string): string {
  return `${dirName(part)}/_rels/${baseName(part)}.rels`;
}

export function nextId(documents: Document[], local: string, attribute: string, start = 1): number {
  let max = start - 1;
  for (const document of documents) {
    for (const node of descendants(document, local)) {
      const value = Number(node.getAttribute(attribute));
      if (Number.isFinite(value)) max = Math.max(max, value);
    }
  }
  return max + 1;
}

export function clone<T extends Node>(node: T): T {
  return node.cloneNode(true) as T;
}
