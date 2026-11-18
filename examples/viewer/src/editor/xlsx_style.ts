import { XMLSerializer } from '@xmldom/xmldom';
import type JSZip from 'jszip';
import {
  NS,
  append,
  children,
  clone,
  direct,
  element,
  ensureDirect,
  parseXml,
  readXml,
  setAttr,
  writeXml,
} from './xml.js';
import { ensureContentType, ensureRelationship } from './package.js';
import type { CellRange, XlsxSheet } from './xlsx_model.js';
import { ensureCell, findCell } from './xlsx_model.js';

const STYLES_REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles';
const STYLES_CONTENT_TYPE = 'application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml';
const MAX_XLSX_ROW = 1_048_576;
const MAX_XLSX_COLUMN = 16_384;

let copiedStyleId: number | null = null;

const MINIMAL_STYLES = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="${NS.x}">
<numFmts count="0"/>
<fonts count="1"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts>
<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>`;

const ORDER: Record<string, string[]> = {
  numFmts: ['fonts', 'fills', 'borders', 'cellStyleXfs', 'cellXfs', 'cellStyles', 'dxfs', 'tableStyles', 'extLst'],
  fonts: ['fills', 'borders', 'cellStyleXfs', 'cellXfs', 'cellStyles', 'dxfs', 'tableStyles', 'extLst'],
  fills: ['borders', 'cellStyleXfs', 'cellXfs', 'cellStyles', 'dxfs', 'tableStyles', 'extLst'],
  borders: ['cellStyleXfs', 'cellXfs', 'cellStyles', 'dxfs', 'tableStyles', 'extLst'],
  cellStyleXfs: ['cellXfs', 'cellStyles', 'dxfs', 'tableStyles', 'extLst'],
  cellXfs: ['cellStyles', 'dxfs', 'tableStyles', 'extLst'],
  cellStyles: ['dxfs', 'tableStyles', 'extLst'],
};

class Styles {
  private constructor(
    private readonly zip: JSZip,
    readonly document: Document,
    readonly path: string,
  ) {}

  static async load(zip: JSZip): Promise<Styles> {
    const path = 'xl/styles.xml';
    if (zip.file(path)) return new Styles(zip, await readXml(zip, path), path);
    const document = parseXml(MINIMAL_STYLES);
    writeXml(zip, path, document);
    await ensureContentType(zip, path, STYLES_CONTENT_TYPE);
    await ensureRelationship(zip, 'xl/workbook.xml', STYLES_REL, 'styles.xml');
    return new Styles(zip, document, path);
  }

  save(): void {
    writeXml(this.zip, this.path, this.document);
  }

  collection(name: string): Element {
    return ensureDirect(this.document, this.document.documentElement, 'x', name, ORDER[name]);
  }

  item(collection: string, index: number): Element {
    const items = children(this.collection(collection), collection === 'cellStyleXfs' || collection === 'cellXfs' ? 'xf' : collection.slice(0, -1));
    const item = items[index] ?? items[0];
    if (!item) throw new Error(`в styles.xml нет ${collection}`);
    return item;
  }

  add(collection: string, source: Element): number {
    const parent = this.collection(collection);
    const singular = collection === 'cellStyleXfs' || collection === 'cellXfs' ? 'xf' : collection.slice(0, -1);
    const items = children(parent, singular);
    const serializer = new XMLSerializer();
    const canonical = serializer.serializeToString(source);
    const existing = items.findIndex((item) => serializer.serializeToString(item) === canonical);
    if (existing >= 0) return existing;
    parent.appendChild(source);
    setAttr(parent, 'count', items.length + 1);
    return items.length;
  }

  cloneXf(styleId: number): Element {
    return clone(this.item('cellXfs', styleId));
  }

  styleId(cell: Element): number {
    const value = Number(cell.getAttribute('s') ?? 0);
    return Number.isInteger(value) && value >= 0 ? value : 0;
  }

  updateCell(sheet: XlsxSheet, row: number, col: number, update: (xf: Element, styleId: number) => void): void {
    if (row >= MAX_XLSX_ROW || col >= MAX_XLSX_COLUMN) throw new Error('адрес за пределами листа XLSX');
    const cell = ensureCell(sheet, { row, col });
    const styleId = this.styleId(cell);
    const xf = this.cloneXf(styleId);
    update(xf, styleId);
    const nextId = this.add('cellXfs', xf);
    setAttr(cell, 's', nextId);
  }

  fontId(xf: Element): number {
    return Number(xf.getAttribute('fontId') ?? 0) || 0;
  }

  updateFont(xf: Element, update: (font: Element) => void): void {
    const font = clone(this.item('fonts', this.fontId(xf)));
    update(font);
    setAttr(xf, 'fontId', this.add('fonts', font));
    setAttr(xf, 'applyFont', 1);
  }

  numFmtCode(xf: Element): string {
    const id = Number(xf.getAttribute('numFmtId') ?? 0) || 0;
    const custom = children(this.collection('numFmts'), 'numFmt').find((item) => Number(item.getAttribute('numFmtId')) === id);
    if (custom) return custom.getAttribute('formatCode') ?? 'General';
    const builtin: Record<number, string> = {
      0: 'General', 1: '0', 2: '0.00', 3: '#,##0', 4: '#,##0.00', 9: '0%', 10: '0.00%',
      11: '0.00E+00', 12: '# ?/?', 13: '# ??/??', 14: 'mm-dd-yy', 15: 'd-mmm-yy',
      16: 'd-mmm', 17: 'mmm-yy', 18: 'h:mm AM/PM', 19: 'h:mm:ss AM/PM', 20: 'h:mm',
      21: 'h:mm:ss', 22: 'm/d/yy h:mm', 37: '#,##0 ;(#,##0)', 38: '#,##0 ;[Red](#,##0)',
      39: '#,##0.00;(#,##0.00)', 40: '#,##0.00;[Red](#,##0.00)',
      45: 'mm:ss', 46: '[h]:mm:ss', 47: 'mmss.0', 48: '##0.0E+0', 49: '@',
    };
    return builtin[id] ?? 'General';
  }

  setNumberFormat(xf: Element, code: string): void {
    const formats = children(this.collection('numFmts'), 'numFmt');
    const existing = formats.find((item) => item.getAttribute('formatCode') === code);
    let id = Number(existing?.getAttribute('numFmtId'));
    if (!existing) {
      const builtin = [...BUILTIN_FORMATS].find(([, format]) => format === code);
      if (builtin) {
        id = builtin[0];
      } else {
        id = Math.max(163, ...formats.map((item) => Number(item.getAttribute('numFmtId')) || 0)) + 1;
        const format = element(this.document, 'x', 'numFmt');
        setAttr(format, 'numFmtId', id);
        setAttr(format, 'formatCode', code);
        this.add('numFmts', format);
      }
    }
    setAttr(xf, 'numFmtId', id);
    setAttr(xf, 'applyNumberFormat', 1);
  }

  namedStyleId(name: 'Normal' | 'Heading1'): number {
    const styles = this.collection('cellStyles');
    const expectedNames = name === 'Normal' ? ['normal', 'обычный'] : ['heading1', 'heading 1', 'заголовок 1'];
    let style = children(styles, 'cellStyle').find((item) => expectedNames.includes((item.getAttribute('name') ?? '').toLowerCase()));
    if (style) return Number(style.getAttribute('xfId') ?? 0) || 0;
    const styleXfs = this.collection('cellStyleXfs');
    if (name === 'Normal' && children(styleXfs, 'xf').length > 0) return 0;
    const xf = element(this.document, 'x', 'xf');
    setAttr(xf, 'numFmtId', 0);
    setAttr(xf, 'fontId', 0);
    setAttr(xf, 'fillId', 0);
    setAttr(xf, 'borderId', 0);
    if (name === 'Heading1') {
      this.updateFont(xf, (font) => {
        setFontChild(this.document, font, 'b');
        setFontChild(this.document, font, 'sz', '15');
        const color = setFontChild(this.document, font, 'color');
        setAttr(color, 'rgb', 'FF1F4E78');
      });
      setAttr(xf, 'applyFont', 1);
    }
    const xfId = this.add('cellStyleXfs', xf);
    style = element(this.document, 'x', 'cellStyle');
    setAttr(style, 'name', name === 'Heading1' ? 'Heading 1' : 'Normal');
    setAttr(style, 'xfId', xfId);
    if (name === 'Heading1') setAttr(style, 'builtinId', 1);
    styles.appendChild(style);
    setAttr(styles, 'count', children(styles, 'cellStyle').length);
    return xfId;
  }
}

const BUILTIN_FORMATS: ReadonlyMap<number, string> = new Map([
  [1, '0'], [2, '0.00'], [3, '#,##0'], [4, '#,##0.00'], [9, '0%'], [10, '0.00%'],
]);

function setFontChild(document: Document, font: Element, name: string, value?: string): Element {
  let node = direct(font, name);
  if (!node) {
    node = element(document, 'x', name);
    font.appendChild(node);
  }
  if (value !== undefined) setAttr(node, 'val', value);
  return node;
}

function removeFontChild(font: Element, name: string): void {
  for (const node of children(font, name)) font.removeChild(node);
}

function normalizedColor(value: string): string {
  const color = value.trim().replace(/^#/, '');
  if (/^[\dA-Fa-f]{6}$/.test(color)) return `FF${color.toUpperCase()}`;
  if (/^[\dA-Fa-f]{8}$/.test(color)) return color.toUpperCase();
  throw new Error('цвет должен быть в формате RRGGBB или AARRGGBB');
}

function setAlignment(document: Document, xf: Element): Element {
  let alignment = direct(xf, 'alignment');
  if (!alignment) {
    alignment = element(document, 'x', 'alignment');
    xf.appendChild(alignment);
  }
  setAttr(xf, 'applyAlignment', 1);
  return alignment;
}

function decimals(code: string, delta: number): string {
  if (code === 'General') return delta > 0 ? '0.0' : '0';
  return code.split(';').map((section) => {
    const match = /([#0])(\.([#0]+))/.exec(section);
    if (match?.index !== undefined) {
      const current = match[3]?.length ?? 0;
      const next = Math.max(0, current + delta);
      const replacement = next === 0 ? match[1] ?? '0' : `${match[1]}.${'0'.repeat(next)}`;
      return `${section.slice(0, match.index)}${replacement}${section.slice(match.index + match[0].length)}`;
    }
    if (delta < 0) return section;
    const placeholder = /[#0]/.exec(section);
    if (!placeholder || placeholder.index === undefined) return section;
    const at = placeholder.index;
    return `${section.slice(0, at + 1)}.${'0'.repeat(delta)}${section.slice(at + 1)}`;
  }).join(';');
}

function updateBorder(document: Document, border: Element, value: string): void {
  const normalized = value.trim().toLowerCase();
  const edges = ['left', 'right', 'top', 'bottom', 'insideH', 'insideV'];
  for (const edge of edges) {
    let node = direct(border, edge);
    if (!node) {
      node = element(document, 'x', edge);
      border.appendChild(node);
    }
    node.removeAttribute('style');
  }
  if (!normalized || normalized === 'none' || normalized === 'нет') return;
  const selected = new Set<string>();
  if (['all', 'все'].includes(normalized)) {
    for (const edge of ['left', 'right', 'top', 'bottom', 'insideH', 'insideV']) selected.add(edge);
  } else if (['outline', 'outer', 'внешние'].includes(normalized)) {
    for (const edge of ['left', 'right', 'top', 'bottom']) selected.add(edge);
  } else if (['inside', 'внутренние'].includes(normalized)) {
    selected.add('insideH');
    selected.add('insideV');
  } else {
    const aliases: Record<string, string> = { horizontal: 'insideH', vertical: 'insideV' };
    for (const part of normalized.split(/[\s,+]+/).filter(Boolean)) selected.add(aliases[part] ?? part);
  }
  for (const edge of selected) {
    if (!edges.includes(edge)) throw new Error(`неподдерживаемая граница «${edge}»`);
    let node = direct(border, edge);
    if (!node) {
      node = element(document, 'x', edge);
      border.appendChild(node);
    }
    setAttr(node, 'style', 'thin');
    let color = direct(node, 'color');
    if (!color) {
      color = element(document, 'x', 'color');
      node.appendChild(color);
    }
    setAttr(color, 'auto', 1);
  }
}

export function copyFormat(sheet: XlsxSheet, row: number, col: number): number {
  const cell = findCell(sheet, { row, col });
  copiedStyleId = Number(cell?.getAttribute('s') ?? 0) || 0;
  return copiedStyleId;
}

export async function pasteFormat(zip: JSZip, sheet: XlsxSheet, range: CellRange): Promise<boolean> {
  if (copiedStyleId === null) throw new Error('сначала скопируйте формат ячейки');
  const styles = await Styles.load(zip);
  let changed = false;
  forEachAddress(range, (row, col) => {
    const cell = ensureCell(sheet, { row, col });
    if (styles.styleId(cell) === copiedStyleId) return;
    setAttr(cell, 's', copiedStyleId ?? 0);
    changed = true;
  });
  if (changed) styles.save();
  return changed;
}

function forEachAddress(range: CellRange, callback: (row: number, col: number) => void): void {
  for (let row = range.start.row; row <= range.end.row; row += 1) {
    for (let col = range.start.col; col <= range.end.col; col += 1) callback(row, col);
  }
}

export async function applyStyleCommand(
  zip: JSZip,
  sheet: XlsxSheet,
  range: CellRange,
  command: string,
  value: string,
  delta: number,
  customCode: string,
): Promise<boolean> {
  const styles = await Styles.load(zip);
  let toggle = true;
  if (command.startsWith('toggle-')) {
    const first = ensureCell(sheet, range.start);
    const xf = styles.cloneXf(styles.styleId(first));
    const font = styles.item('fonts', styles.fontId(xf));
    const child = command === 'toggle-bold' ? 'b'
      : command === 'toggle-italic' ? 'i'
        : command === 'toggle-underline' ? 'u' : 'strike';
    toggle = !direct(font, child);
  }
  if (command === 'wrap-text') {
    const first = ensureCell(sheet, range.start);
    const xf = styles.cloneXf(styles.styleId(first));
    toggle = direct(xf, 'alignment')?.getAttribute('wrapText') !== '1';
  }
  forEachAddress(range, (row, col) => {
    styles.updateCell(sheet, row, col, (xf) => {
      if (command === 'font-family' || command === 'font-size' || command === 'text-color' || command.startsWith('toggle-')) {
        if (command === 'font-family' && !value.trim()) throw new Error('не задано семейство шрифта');
        if (command === 'font-size' && (!Number.isFinite(Number(value)) || Number(value) <= 0)) throw new Error('размер шрифта должен быть положительным числом');
        styles.updateFont(xf, (font) => {
          if (command === 'font-family') setFontChild(styles.document, font, 'name', value.trim());
          if (command === 'font-size') setFontChild(styles.document, font, 'sz', String(Number(value)));
          if (command === 'text-color') setAttr(setFontChild(styles.document, font, 'color'), 'rgb', normalizedColor(value));
          if (command.startsWith('toggle-')) {
            const child = command === 'toggle-bold' ? 'b'
              : command === 'toggle-italic' ? 'i'
                : command === 'toggle-underline' ? 'u' : 'strike';
            if (toggle) setFontChild(styles.document, font, child);
            else removeFontChild(font, child);
          }
        });
      } else if (command === 'highlight') {
        if (!value.trim() || value.trim().toLowerCase() === 'none') {
          setAttr(xf, 'fillId', 0);
        } else {
          const fill = element(styles.document, 'x', 'fill');
          const pattern = element(styles.document, 'x', 'patternFill');
          setAttr(pattern, 'patternType', 'solid');
          const foreground = element(styles.document, 'x', 'fgColor');
          setAttr(foreground, 'rgb', normalizedColor(value));
          const background = element(styles.document, 'x', 'bgColor');
          setAttr(background, 'indexed', 64);
          append(pattern, foreground, background);
          fill.appendChild(pattern);
          setAttr(xf, 'fillId', styles.add('fills', fill));
        }
        setAttr(xf, 'applyFill', 1);
      } else if (command === 'borders') {
        const borderId = Number(xf.getAttribute('borderId') ?? 0) || 0;
        const border = clone(styles.item('borders', borderId));
        updateBorder(styles.document, border, value);
        setAttr(xf, 'borderId', styles.add('borders', border));
        setAttr(xf, 'applyBorder', 1);
      } else if (command === 'align') {
        if (!['left', 'center', 'right', 'justify'].includes(value)) throw new Error('align должен быть left, center, right или justify');
        setAttr(setAlignment(styles.document, xf), 'horizontal', value);
      } else if (command === 'wrap-text') {
        setAttr(setAlignment(styles.document, xf), 'wrapText', toggle ? 1 : 0);
      } else if (command === 'indent') {
        const alignment = setAlignment(styles.document, xf);
        const current = Number(alignment.getAttribute('indent') ?? 0) || 0;
        setAttr(alignment, 'indent', Math.max(0, Math.min(250, current + delta)));
      } else if (command === 'number-format') {
        let code = value;
        if (value === 'currency') code = '$#,##0.00';
        if (value === 'percent') code = '0.00%';
        if (value === 'comma') code = '#,##0.00';
        if (value === 'custom') code = customCode.trim();
        if (value === 'decimal-increase' || value === 'decimal-decrease') {
          code = decimals(styles.numFmtCode(xf), value === 'decimal-increase' ? 1 : -1);
        }
        if (!code) throw new Error('для custom задайте code с форматом числа');
        styles.setNumberFormat(xf, code);
      } else if (command === 'style') {
        if (value !== 'Normal' && value !== 'Heading1') throw new Error('style должен быть Normal или Heading1');
        setAttr(xf, 'xfId', styles.namedStyleId(value));
        setAttr(xf, 'applyStyle', 1);
      } else {
        throw new Error(`неподдерживаемая команда форматирования «${command}»`);
      }
    });
  });
  styles.save();
  return true;
}

