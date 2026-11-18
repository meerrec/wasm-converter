import type JSZip from 'jszip';
import {
  NS,
  baseName,
  descendants,
  dirName,
  element,
  normalPath,
  parseXml,
  readXml,
  relsPath,
  serializeXml,
  setAttr,
  setNsAttr,
  writeXml,
} from './xml.js';

export async function ensureRelationship(
  zip: JSZip,
  sourcePart: string,
  type: string,
  target: string,
  external = false,
): Promise<string> {
  const path = relsPath(sourcePart);
  let document: Document;
  try {
    document = await readXml(zip, path);
  } catch {
    document = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="${NS.rel}"/>`);
  }
  const relationships = descendants(document, 'Relationship');
  const existing = relationships.find(
    (node) => node.getAttribute('Type') === type && node.getAttribute('Target') === target,
  );
  if (existing) return existing.getAttribute('Id') ?? '';
  let index = relationships.length + 1;
  while (relationships.some((node) => node.getAttribute('Id') === `rId${index}`)) index += 1;
  const id = `rId${index}`;
  const relationship = document.createElementNS(NS.rel, 'Relationship');
  setAttr(relationship, 'Id', id);
  setAttr(relationship, 'Type', type);
  setAttr(relationship, 'Target', target);
  if (external) setAttr(relationship, 'TargetMode', 'External');
  document.documentElement.appendChild(relationship);
  writeXml(zip, path, document);
  return id;
}

export async function relationshipTarget(
  zip: JSZip,
  sourcePart: string,
  id: string,
): Promise<string | null> {
  try {
    const document = await readXml(zip, relsPath(sourcePart));
    const relationship = descendants(document, 'Relationship').find(
      (node) => node.getAttribute('Id') === id,
    );
    const target = relationship?.getAttribute('Target');
    return target ? normalPath(dirName(sourcePart), target) : null;
  } catch {
    return null;
  }
}

export async function ensureContentType(
  zip: JSZip,
  part: string,
  contentType: string,
): Promise<void> {
  const path = '[Content_Types].xml';
  const document = await readXml(zip, path);
  const existing = descendants(document, 'Override').find(
    (node) => node.getAttribute('PartName') === `/${part}`,
  );
  if (existing) {
    existing.setAttribute('ContentType', contentType);
  } else {
    const override = document.createElementNS(NS.ct, 'Override');
    setAttr(override, 'PartName', `/${part}`);
    setAttr(override, 'ContentType', contentType);
    document.documentElement.appendChild(override);
  }
  writeXml(zip, path, document);
}

export async function ensureDefaultExtension(
  zip: JSZip,
  extension: string,
  contentType: string,
): Promise<void> {
  const path = '[Content_Types].xml';
  const document = await readXml(zip, path);
  const normalized = extension.replace(/^\./, '').toLowerCase();
  const existing = descendants(document, 'Default').find(
    (node) => node.getAttribute('Extension')?.toLowerCase() === normalized,
  );
  if (!existing) {
    const node = document.createElementNS(NS.ct, 'Default');
    setAttr(node, 'Extension', normalized);
    setAttr(node, 'ContentType', contentType);
    document.documentElement.appendChild(node);
    writeXml(zip, path, document);
  }
}

export function nextPartName(zip: JSZip, directory: string, stem: string, extension: string): string {
  let index = 1;
  while (zip.file(`${directory}/${stem}${index}.${extension}`)) index += 1;
  return `${directory}/${stem}${index}.${extension}`;
}

export function relativeTarget(sourcePart: string, targetPart: string): string {
  const source = dirName(sourcePart).split('/').filter(Boolean);
  const target = targetPart.split('/').filter(Boolean);
  let common = 0;
  while (source[common] && source[common] === target[common]) common += 1;
  const up = source.slice(common).map(() => '..');
  return [...up, ...target.slice(common)].join('/');
}

export function partName(path: string): string {
  return baseName(path);
}
