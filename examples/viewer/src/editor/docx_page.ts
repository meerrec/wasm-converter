import type { CommandResult, EditorContext } from './commands.js';
import { numberArg, requiredString } from './commands.js';
import {
  DOCUMENT_PART,
  RELATIONSHIP_BASE,
  STYLES_PART,
  bodyOf,
  ensureStyle,
  ensureStyles,
  setWAttr,
  w,
  wAttr,
  a,
} from './docx_shared.js';
import {
  NS,
  descendants,
  ensureDirect,
  first,
  normalPath,
  dirName,
  parseXml,
  readXml,
  setAttr,
  writeXml,
} from './xml.js';
import { ensureContentType, ensureRelationship, relativeTarget } from './package.js';

const THEME_RELATIONSHIP = `${RELATIONSHIP_BASE}/theme`;
const THEME_PART = 'word/theme/theme1.xml';
const THEME_CONTENT_TYPE = 'application/vnd.openxmlformats-officedocument.theme+xml';

function sections(document: Document): Element[] {
  const result = descendants(document, 'sectPr');
  if (result.length > 0) return result;
  const section = w(document, 'sectPr');
  bodyOf(document).appendChild(section);
  return [section];
}

function finiteMargin(context: EditorContext, name: string): number {
  const value = numberArg(context.args, name, Number.NaN);
  if (!Number.isFinite(value) || value < 0) {
    throw new Error(`поле «${name}» должно быть неотрицательным числом миллиметров`);
  }
  return value;
}

export async function pageMargins(context: EditorContext): Promise<CommandResult> {
  const values = {
    top: finiteMargin(context, 'top'),
    right: finiteMargin(context, 'right'),
    bottom: finiteMargin(context, 'bottom'),
    left: finiteMargin(context, 'left'),
  };
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const section of sections(document)) {
    const margins = ensureDirect(document, section, 'w', 'pgMar', ['cols', 'docGrid']);
    for (const [side, millimetres] of Object.entries(values)) {
      setWAttr(margins, side, Math.round(millimetres / 25.4 * 1440));
    }
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

function ensurePageSize(document: Document, section: Element): Element {
  return ensureDirect(document, section, 'w', 'pgSz', ['pgMar', 'cols', 'docGrid']);
}

export async function pageOrientation(context: EditorContext): Promise<CommandResult> {
  const orientation = requiredString(context.args, 'value');
  if (orientation !== 'portrait' && orientation !== 'landscape') {
    throw new Error('ориентация должна быть portrait или landscape');
  }
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const section of sections(document)) {
    const size = ensurePageSize(document, section);
    let width = Number(wAttr(size, 'w')) || 11906;
    let height = Number(wAttr(size, 'h')) || 16838;
    if (orientation === 'landscape' && height > width) [width, height] = [height, width];
    if (orientation === 'portrait' && width > height) [width, height] = [height, width];
    setWAttr(size, 'w', width);
    setWAttr(size, 'h', height);
    setWAttr(size, 'orient', orientation);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function pageSize(context: EditorContext): Promise<CommandResult> {
  const value = requiredString(context.args, 'value');
  const sizes: Record<string, [number, number]> = {
    A4: [11906, 16838],
    A3: [16838, 23811],
    Letter: [12240, 15840],
    Legal: [12240, 20160],
  };
  const selected = sizes[value];
  if (!selected) throw new Error('размер страницы должен быть A4, A3, Letter или Legal');
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const section of sections(document)) {
    const size = ensurePageSize(document, section);
    const landscape = wAttr(size, 'orient') === 'landscape';
    setWAttr(size, 'w', landscape ? selected[1] : selected[0]);
    setWAttr(size, 'h', landscape ? selected[0] : selected[1]);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

interface StyleSet {
  normalFont: string;
  normalSize: number;
  headingFont: string;
  headingSizes: number[];
  textColor: string;
  accentColor: string;
  paragraphSpacing: number;
}

function selectedStyleSet(value: string): StyleSet {
  const key = value.toLowerCase().replace(/[\s_-]+/g, '');
  const sets: Record<string, StyleSet> = {
    default: {
      normalFont: 'Calibri', normalSize: 22, headingFont: 'Calibri Light', headingSizes: [32, 28, 24],
      textColor: '000000', accentColor: '2F5496', paragraphSpacing: 160,
    },
    обычный: {
      normalFont: 'Calibri', normalSize: 22, headingFont: 'Calibri Light', headingSizes: [32, 28, 24],
      textColor: '000000', accentColor: '2F5496', paragraphSpacing: 160,
    },
    modern: {
      normalFont: 'Aptos', normalSize: 22, headingFont: 'Aptos Display', headingSizes: [36, 30, 26],
      textColor: '1F1F1F', accentColor: '4472C4', paragraphSpacing: 180,
    },
    современный: {
      normalFont: 'Aptos', normalSize: 22, headingFont: 'Aptos Display', headingSizes: [36, 30, 26],
      textColor: '1F1F1F', accentColor: '4472C4', paragraphSpacing: 180,
    },
    classic: {
      normalFont: 'Times New Roman', normalSize: 24, headingFont: 'Times New Roman', headingSizes: [32, 28, 24],
      textColor: '000000', accentColor: '000000', paragraphSpacing: 120,
    },
    классический: {
      normalFont: 'Times New Roman', normalSize: 24, headingFont: 'Times New Roman', headingSizes: [32, 28, 24],
      textColor: '000000', accentColor: '000000', paragraphSpacing: 120,
    },
    minimal: {
      normalFont: 'Arial', normalSize: 21, headingFont: 'Arial', headingSizes: [30, 26, 22],
      textColor: '222222', accentColor: '666666', paragraphSpacing: 140,
    },
    минимальный: {
      normalFont: 'Arial', normalSize: 21, headingFont: 'Arial', headingSizes: [30, 26, 22],
      textColor: '222222', accentColor: '666666', paragraphSpacing: 140,
    },
  };
  const result = sets[key];
  if (!result) throw new Error('набор стилей должен быть Default, Modern, Classic или Minimal');
  return result;
}

function patchFonts(document: Document, properties: Element, font: string): void {
  const fonts = ensureDirect(document, properties, 'w', 'rFonts');
  for (const name of ['ascii', 'hAnsi', 'eastAsia', 'cs']) setWAttr(fonts, name, font);
}

function patchSizeAndColor(document: Document, properties: Element, size: number, color: string): void {
  setWAttr(ensureDirect(document, properties, 'w', 'sz'), 'val', size);
  setWAttr(ensureDirect(document, properties, 'w', 'szCs'), 'val', size);
  setWAttr(ensureDirect(document, properties, 'w', 'color'), 'val', color);
}

export async function styleSet(context: EditorContext): Promise<CommandResult> {
  const preset = selectedStyleSet(requiredString(context.args, 'value'));
  const styles = await ensureStyles(context.zip);
  const defaults = ensureDirect(styles, styles.documentElement, 'w', 'docDefaults');
  const runDefault = ensureDirect(styles, defaults, 'w', 'rPrDefault');
  const defaultRunProperties = ensureDirect(styles, runDefault, 'w', 'rPr');
  patchFonts(styles, defaultRunProperties, preset.normalFont);
  patchSizeAndColor(styles, defaultRunProperties, preset.normalSize, preset.textColor);
  const paragraphDefault = ensureDirect(styles, defaults, 'w', 'pPrDefault');
  const defaultParagraphProperties = ensureDirect(styles, paragraphDefault, 'w', 'pPr');
  const defaultSpacing = ensureDirect(styles, defaultParagraphProperties, 'w', 'spacing');
  setWAttr(defaultSpacing, 'after', preset.paragraphSpacing);
  setWAttr(defaultSpacing, 'line', 240);
  setWAttr(defaultSpacing, 'lineRule', 'auto');

  const normal = ensureStyle(styles, 'Normal', 'Normal');
  const normalRunProperties = ensureDirect(styles, normal, 'w', 'rPr');
  patchFonts(styles, normalRunProperties, preset.normalFont);
  patchSizeAndColor(styles, normalRunProperties, preset.normalSize, preset.textColor);
  const normalParagraphProperties = ensureDirect(styles, normal, 'w', 'pPr');
  setWAttr(ensureDirect(styles, normalParagraphProperties, 'w', 'spacing'), 'after', preset.paragraphSpacing);

  for (let index = 0; index < 3; index += 1) {
    const id = `Heading${index + 1}`;
    const heading = ensureStyle(styles, id, `heading ${index + 1}`);
    const properties = ensureDirect(styles, heading, 'w', 'rPr');
    patchFonts(styles, properties, preset.headingFont);
    patchSizeAndColor(styles, properties, preset.headingSizes[index] ?? 24, preset.accentColor);
    ensureDirect(styles, properties, 'w', 'b');
    const paragraphProperties = ensureDirect(styles, heading, 'w', 'pPr');
    ensureDirect(styles, paragraphProperties, 'w', 'keepNext');
    ensureDirect(styles, paragraphProperties, 'w', 'keepLines');
    const spacing = ensureDirect(styles, paragraphProperties, 'w', 'spacing');
    setWAttr(spacing, 'before', 240 - index * 40);
    setWAttr(spacing, 'after', 120);
    setWAttr(ensureDirect(styles, paragraphProperties, 'w', 'outlineLvl'), 'val', index);
  }
  writeXml(context.zip, STYLES_PART, styles);
  return { changed: true };
}

const THEME_PALETTES: Record<string, string[]> = {
  office: ['4472C4', 'ED7D31', 'A5A5A5', 'FFC000', '5B9BD5', '70AD47'],
  dark: ['8FAADC', 'F4B183', 'A9D18E', 'FFD966', '9DC3E6', 'C9C9C9'],
  blue: ['2E75B6', '5B9BD5', '1F4E79', '9DC3E6', '4472C4', '70AD47'],
  green: ['548235', '70AD47', 'A9D18E', '2F6B4F', '92D050', 'B4C6E7'],
  grayscale: ['404040', '737373', '969696', 'BFBFBF', 'D9D9D9', '595959'],
};

function themePalette(value: string): { name: string; colors: string[] } {
  const key = value.toLowerCase().replace(/[\s_-]+/g, '');
  const aliases: Record<string, string> = {
    office: 'office', обычная: 'office', default: 'office',
    dark: 'dark', тёмная: 'dark', темная: 'dark',
    blue: 'blue', синяя: 'blue',
    green: 'green', зелёная: 'green', зеленая: 'green',
    grayscale: 'grayscale', greyscale: 'grayscale', серая: 'grayscale',
  };
  const name = aliases[key] ?? 'office';
  return { name, colors: THEME_PALETTES[name] ?? THEME_PALETTES.office as string[] };
}

function addColor(parent: Element, name: string, value: string): void {
  const document = parent.ownerDocument;
  if (!document) throw new Error('у темы нет документа-владельца');
  const container = a(document, name);
  const color = a(document, 'srgbClr');
  setAttr(color, 'val', value);
  container.appendChild(color);
  parent.appendChild(container);
}

function addSolidFill(document: Document, parent: Element): void {
  const fill = a(document, 'solidFill');
  fill.appendChild(a(document, 'schemeClr'));
  const scheme = first(fill, 'schemeClr');
  if (scheme) setAttr(scheme, 'val', 'phClr');
  parent.appendChild(fill);
}

function addFormatScheme(document: Document, parent: Element): void {
  const format = a(document, 'fmtScheme');
  setAttr(format, 'name', 'Office');
  const fills = a(document, 'fillStyleLst');
  for (let index = 0; index < 3; index += 1) addSolidFill(document, fills);
  format.appendChild(fills);
  const lines = a(document, 'lnStyleLst');
  for (let index = 0; index < 3; index += 1) {
    const line = a(document, 'ln');
    setAttr(line, 'w', 6350 + index * 6350);
    setAttr(line, 'cap', 'flat');
    setAttr(line, 'cmpd', 'sng');
    setAttr(line, 'algn', 'ctr');
    addSolidFill(document, line);
    const dash = a(document, 'prstDash');
    setAttr(dash, 'val', 'solid');
    line.appendChild(dash);
    line.appendChild(a(document, 'round'));
    const head = a(document, 'headEnd');
    setAttr(head, 'type', 'none');
    setAttr(head, 'w', 'med');
    setAttr(head, 'len', 'med');
    line.appendChild(head);
    const tail = a(document, 'tailEnd');
    setAttr(tail, 'type', 'none');
    setAttr(tail, 'w', 'med');
    setAttr(tail, 'len', 'med');
    line.appendChild(tail);
    lines.appendChild(line);
  }
  format.appendChild(lines);
  const effects = a(document, 'effectStyleLst');
  for (let index = 0; index < 3; index += 1) {
    const style = a(document, 'effectStyle');
    style.appendChild(a(document, 'effectLst'));
    effects.appendChild(style);
  }
  format.appendChild(effects);
  const backgroundFills = a(document, 'bgFillStyleLst');
  for (let index = 0; index < 3; index += 1) addSolidFill(document, backgroundFills);
  format.appendChild(backgroundFills);
  parent.appendChild(format);
}

function createTheme(value: string, colors: string[]): Document {
  const document = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="${NS.a}"/>`);
  const root = document.documentElement;
  setAttr(root, 'name', value);
  const themeElements = a(document, 'themeElements');
  const colorScheme = a(document, 'clrScheme');
  setAttr(colorScheme, 'name', value);
  const systemColors: [string, string, string][] = [
    ['dk1', 'windowText', '000000'],
    ['lt1', 'window', 'FFFFFF'],
  ];
  for (const [name, system, lastColor] of systemColors) {
    const container = a(document, name);
    const color = a(document, 'sysClr');
    setAttr(color, 'val', system);
    setAttr(color, 'lastClr', lastColor);
    container.appendChild(color);
    colorScheme.appendChild(container);
  }
  addColor(colorScheme, 'dk2', '44546A');
  addColor(colorScheme, 'lt2', 'E7E6E6');
  for (let index = 0; index < 6; index += 1) addColor(colorScheme, `accent${index + 1}`, colors[index] ?? '4472C4');
  addColor(colorScheme, 'hlink', '0563C1');
  addColor(colorScheme, 'folHlink', '954F72');
  themeElements.appendChild(colorScheme);
  const fontScheme = a(document, 'fontScheme');
  setAttr(fontScheme, 'name', value);
  const schemeFonts: [string, string][] = [['majorFont', 'Calibri Light'], ['minorFont', 'Calibri']];
  for (const [name, typeface] of schemeFonts) {
    const font = a(document, name);
    const latin = a(document, 'latin');
    setAttr(latin, 'typeface', typeface);
    font.appendChild(latin);
    font.appendChild(a(document, 'ea'));
    font.appendChild(a(document, 'cs'));
    fontScheme.appendChild(font);
  }
  themeElements.appendChild(fontScheme);
  addFormatScheme(document, themeElements);
  root.appendChild(themeElements);
  return document;
}

async function relatedThemePart(context: EditorContext): Promise<string> {
  try {
    const relationships = await readXml(context.zip, 'word/_rels/document.xml.rels');
    const relationship = descendants(relationships, 'Relationship').find(
      (node) => node.getAttribute('Type') === THEME_RELATIONSHIP,
    );
    const target = relationship?.getAttribute('Target');
    if (target) return normalPath(dirName(DOCUMENT_PART), target);
  } catch {
    // Отсутствующий .rels создаётся общим ensureRelationship.
  }
  return THEME_PART;
}

export async function theme(context: EditorContext): Promise<CommandResult> {
  const value = requiredString(context.args, 'value');
  const palette = themePalette(value);
  const part = await relatedThemePart(context);
  let document: Document;
  if (context.zip.file(part)) {
    document = await readXml(context.zip, part);
    setAttr(document.documentElement, 'name', value);
    const scheme = first(document, 'clrScheme');
    if (scheme) {
      setAttr(scheme, 'name', value);
      for (let index = 0; index < 6; index += 1) {
        const accent = first(scheme, `accent${index + 1}`);
        const color = accent ? first(accent, 'srgbClr') : null;
        if (color) setAttr(color, 'val', palette.colors[index] ?? '4472C4');
      }
    }
  } else {
    document = createTheme(value, palette.colors);
  }
  writeXml(context.zip, part, document);
  await ensureRelationship(context.zip, DOCUMENT_PART, THEME_RELATIONSHIP, relativeTarget(DOCUMENT_PART, part));
  await ensureContentType(context.zip, part, THEME_CONTENT_TYPE);
  return { changed: true };
}
