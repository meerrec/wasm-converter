import type { CommandResult, EditorContext } from './commands.js';
import {
  DOCUMENT_PART,
  RELATIONSHIP_BASE,
  STYLES_PART,
  addressedRun,
  boolValue,
  documentParagraphs,
  ensureStyle,
  ensureStyles,
  importElement,
  paragraphProperties,
  paragraphRuns,
  paragraphText,
  positiveNumber,
  replaceText,
  runProperties,
  runText,
  selectedParagraphs,
  selectedRuns,
  setRunText,
  setToggle,
  setWAttr,
  w,
  wAttr,
} from './docx_shared.js';
import {
  children,
  clone,
  descendants,
  direct,
  ensureDirect,
  parseXml,
  readXml,
  writeXml,
} from './xml.js';
import { ensureContentType, ensureRelationship } from './package.js';
import { numberArg, requiredString, stringArg } from './commands.js';

const NUMBERING_PART = 'word/numbering.xml';
const HIGHLIGHTS = new Set([
  'black', 'blue', 'cyan', 'green', 'magenta', 'red', 'yellow', 'white',
  'darkBlue', 'darkCyan', 'darkGreen', 'darkMagenta', 'darkRed', 'darkYellow', 'darkGray', 'lightGray',
]);

let formatClipboard: { properties: Element | null } | null = null;

export async function getText(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  return { value: documentParagraphs(document).map(paragraphText).join('\n') };
}

export async function getRun(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  const { run } = addressedRun(document, context.args);
  const properties = direct(run, 'rPr');
  const fonts = properties ? direct(properties, 'rFonts') : null;
  const size = properties ? direct(properties, 'sz') : null;
  const color = properties ? direct(properties, 'color') : null;
  const highlight = properties ? direct(properties, 'highlight') : null;
  const vertAlign = properties ? direct(properties, 'vertAlign') : null;
  return {
    value: {
      text: runText(run),
      fontFamily: fonts ? wAttr(fonts, 'ascii') : null,
      fontSize: size && wAttr(size, 'val') ? Number(wAttr(size, 'val')) / 2 : null,
      color: color ? wAttr(color, 'val') : null,
      highlight: highlight ? wAttr(highlight, 'val') : null,
      bold: boolValue(properties ? direct(properties, 'b') : null),
      italic: boolValue(properties ? direct(properties, 'i') : null),
      underline: boolValue(properties ? direct(properties, 'u') : null),
      strikethrough: boolValue(properties ? direct(properties, 'strike') : null),
      subscript: wAttr(vertAlign ?? (properties ?? run), 'val') === 'subscript' && vertAlign !== null,
      superscript: wAttr(vertAlign ?? (properties ?? run), 'val') === 'superscript' && vertAlign !== null,
    },
  };
}

export async function setRun(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  const { run } = addressedRun(document, context.args);
  setRunText(document, run, stringArg(context.args, 'text'));
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function copyFormat(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  const { run } = addressedRun(document, context.args);
  const properties = direct(run, 'rPr');
  formatClipboard = { properties: properties ? clone(properties) : null };
  return { changed: false, message: 'Формат run скопирован' };
}

export async function pasteFormat(context: EditorContext): Promise<CommandResult> {
  if (!formatClipboard) throw new Error('буфер формата пуст: сначала выполните copy-format');
  const document = await readXml(context.zip, DOCUMENT_PART);
  const { run } = addressedRun(document, context.args);
  const old = direct(run, 'rPr');
  if (old) run.removeChild(old);
  if (formatClipboard.properties) run.insertBefore(importElement(document, formatClipboard.properties), run.firstChild);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function fontFamily(context: EditorContext): Promise<CommandResult> {
  const value = requiredString(context.args, 'value');
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const run of selectedRuns(document, context.args)) {
    const properties = runProperties(document, run);
    const fonts = ensureDirect(document, properties, 'w', 'rFonts');
    for (const name of ['ascii', 'hAnsi', 'eastAsia', 'cs']) setWAttr(fonts, name, value);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function fontSize(context: EditorContext): Promise<CommandResult> {
  const value = positiveNumber(context.args.value, 'value');
  const halfPoints = Math.round(value * 2);
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const run of selectedRuns(document, context.args)) {
    const properties = runProperties(document, run);
    setWAttr(ensureDirect(document, properties, 'w', 'sz'), 'val', halfPoints);
    setWAttr(ensureDirect(document, properties, 'w', 'szCs'), 'val', halfPoints);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function textColor(context: EditorContext): Promise<CommandResult> {
  const value = requiredString(context.args, 'value').toUpperCase();
  if (!/^[0-9A-F]{6}$/.test(value)) throw new Error('цвет должен быть в формате RRGGBB');
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const run of selectedRuns(document, context.args)) {
    const color = ensureDirect(document, runProperties(document, run), 'w', 'color');
    setWAttr(color, 'val', value);
    for (const name of ['themeColor', 'themeTint', 'themeShade']) color.removeAttribute(`w:${name}`);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function highlight(context: EditorContext): Promise<CommandResult> {
  const value = requiredString(context.args, 'value');
  if (value !== 'none' && !HIGHLIGHTS.has(value)) {
    throw new Error(`неподдерживаемый цвет выделения «${value}»; используйте название цвета Word или none`);
  }
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const run of selectedRuns(document, context.args)) {
    const properties = runProperties(document, run);
    const old = direct(properties, 'highlight');
    if (value === 'none') {
      if (old) properties.removeChild(old);
    } else {
      setWAttr(ensureDirect(document, properties, 'w', 'highlight'), 'val', value);
    }
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

type ToggleName = 'b' | 'i' | 'u' | 'strike' | 'subscript' | 'superscript';

export async function toggleRunProperty(context: EditorContext, toggle: ToggleName): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  const runs = selectedRuns(document, context.args);
  const isEnabled = (run: Element): boolean => {
    const properties = direct(run, 'rPr');
    if (!properties) return false;
    if (toggle === 'subscript' || toggle === 'superscript') {
      return wAttr(direct(properties, 'vertAlign') ?? properties, 'val') === toggle && direct(properties, 'vertAlign') !== null;
    }
    return boolValue(direct(properties, toggle));
  };
  const enabled = !runs.every(isEnabled);
  for (const run of runs) {
    const properties = runProperties(document, run);
    if (toggle === 'subscript' || toggle === 'superscript') {
      const old = direct(properties, 'vertAlign');
      if (old) properties.removeChild(old);
      if (enabled) {
        const vertical = w(document, 'vertAlign');
        setWAttr(vertical, 'val', toggle);
        properties.appendChild(vertical);
      }
    } else {
      setToggle(properties, document, toggle, enabled);
    }
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

function nextNumericId(nodes: Element[], attribute: string): number {
  let maximum = 0;
  for (const node of nodes) maximum = Math.max(maximum, Number(wAttr(node, attribute)) || 0);
  return maximum + 1;
}

async function ensureNumbering(context: EditorContext, kind: 'bullets' | 'numbering' | 'multilevel-list'): Promise<number> {
  let document: Document;
  if (context.zip.file(NUMBERING_PART)) {
    document = await readXml(context.zip, NUMBERING_PART);
  } else {
    document = parseXml(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>`);
  }
  const abstractId = nextNumericId(descendants(document, 'abstractNum'), 'abstractNumId');
  const numberId = nextNumericId(descendants(document, 'num'), 'numId');
  const abstract = w(document, 'abstractNum');
  setWAttr(abstract, 'abstractNumId', abstractId);
  const multiLevel = w(document, 'multiLevelType');
  setWAttr(multiLevel, 'val', kind === 'multilevel-list' ? 'multilevel' : 'singleLevel');
  abstract.appendChild(multiLevel);
  const levelCount = kind === 'multilevel-list' ? 9 : 1;
  for (let levelIndex = 0; levelIndex < levelCount; levelIndex += 1) {
    const level = w(document, 'lvl');
    setWAttr(level, 'ilvl', levelIndex);
    const start = w(document, 'start');
    setWAttr(start, 'val', 1);
    level.appendChild(start);
    const format = w(document, 'numFmt');
    setWAttr(format, 'val', kind === 'bullets' ? 'bullet' : 'decimal');
    level.appendChild(format);
    const text = w(document, 'lvlText');
    const placeholders = Array.from({ length: levelIndex + 1 }, (_, index) => `%${index + 1}`).join('.');
    setWAttr(text, 'val', kind === 'bullets' ? '•' : kind === 'numbering' ? '%1.' : `${placeholders}.`);
    level.appendChild(text);
    const justification = w(document, 'lvlJc');
    setWAttr(justification, 'val', 'left');
    level.appendChild(justification);
    const properties = w(document, 'pPr');
    const indent = w(document, 'ind');
    setWAttr(indent, 'left', 720 * (levelIndex + 1));
    setWAttr(indent, 'hanging', 360);
    properties.appendChild(indent);
    level.appendChild(properties);
    if (kind === 'bullets') {
      const runProps = w(document, 'rPr');
      const fonts = w(document, 'rFonts');
      setWAttr(fonts, 'ascii', 'Symbol');
      setWAttr(fonts, 'hAnsi', 'Symbol');
      runProps.appendChild(fonts);
      level.appendChild(runProps);
    }
    abstract.appendChild(level);
  }
  const nums = children(document.documentElement, 'num');
  if (nums[0]) document.documentElement.insertBefore(abstract, nums[0]);
  else document.documentElement.appendChild(abstract);
  const number = w(document, 'num');
  setWAttr(number, 'numId', numberId);
  const abstractReference = w(document, 'abstractNumId');
  setWAttr(abstractReference, 'val', abstractId);
  number.appendChild(abstractReference);
  document.documentElement.appendChild(number);
  writeXml(context.zip, NUMBERING_PART, document);
  await ensureRelationship(context.zip, DOCUMENT_PART, `${RELATIONSHIP_BASE}/numbering`, 'numbering.xml');
  await ensureContentType(
    context.zip,
    NUMBERING_PART,
    'application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml',
  );
  return numberId;
}

export async function applyList(context: EditorContext, kind: 'bullets' | 'numbering' | 'multilevel-list'): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  const paragraphs = selectedParagraphs(document, context.args);
  const numberId = await ensureNumbering(context, kind);
  for (const paragraph of paragraphs) {
    const properties = paragraphProperties(document, paragraph);
    const old = direct(properties, 'numPr');
    if (old) properties.removeChild(old);
    const numberProperties = w(document, 'numPr');
    const level = w(document, 'ilvl');
    setWAttr(level, 'val', 0);
    const id = w(document, 'numId');
    setWAttr(id, 'val', numberId);
    numberProperties.appendChild(level);
    numberProperties.appendChild(id);
    const next = children(properties).find((node) => ['spacing', 'ind', 'jc', 'rPr', 'sectPr'].includes(node.localName));
    if (next) properties.insertBefore(numberProperties, next);
    else properties.appendChild(numberProperties);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function lineSpacing(context: EditorContext): Promise<CommandResult> {
  const value = positiveNumber(context.args.value, 'value');
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const paragraph of selectedParagraphs(document, context.args)) {
    const spacing = ensureDirect(document, paragraphProperties(document, paragraph), 'w', 'spacing');
    setWAttr(spacing, 'line', Math.round(value * 240));
    setWAttr(spacing, 'lineRule', 'auto');
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function align(context: EditorContext): Promise<CommandResult> {
  const value = requiredString(context.args, 'value');
  const values = new Set(['left', 'center', 'right', 'justify']);
  if (!values.has(value)) throw new Error('выравнивание должно быть left, center, right или justify');
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const paragraph of selectedParagraphs(document, context.args)) {
    setWAttr(ensureDirect(document, paragraphProperties(document, paragraph), 'w', 'jc'), 'val', value);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function indent(context: EditorContext): Promise<CommandResult> {
  const side = requiredString(context.args, 'value');
  if (side !== 'left' && side !== 'right') throw new Error('сторона отступа должна быть left или right');
  const delta = numberArg(context.args, 'delta', Number.NaN);
  if (!Number.isFinite(delta)) throw new Error('параметр «delta» должен быть числом');
  const document = await readXml(context.zip, DOCUMENT_PART);
  for (const paragraph of selectedParagraphs(document, context.args)) {
    const indentation = ensureDirect(document, paragraphProperties(document, paragraph), 'w', 'ind');
    const current = Number(wAttr(indentation, side)) || 0;
    setWAttr(indentation, side, Math.max(0, Math.round(current + delta * 20)));
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function paragraphStyle(context: EditorContext): Promise<CommandResult> {
  const value = requiredString(context.args, 'value');
  if (value !== 'Normal' && value !== 'Heading1') throw new Error('стиль должен быть Normal или Heading1');
  const document = await readXml(context.zip, DOCUMENT_PART);
  const styles = await ensureStyles(context.zip);
  ensureStyle(styles, 'Normal', 'Normal');
  ensureStyle(styles, 'Heading1', 'heading 1');
  writeXml(context.zip, STYLES_PART, styles);
  for (const paragraph of selectedParagraphs(document, context.args)) {
    setWAttr(ensureDirect(document, paragraphProperties(document, paragraph), 'w', 'pStyle'), 'val', value);
  }
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

export async function findText(context: EditorContext): Promise<CommandResult> {
  const search = requiredString(context.args, 'text');
  const document = await readXml(context.zip, DOCUMENT_PART);
  let count = 0;
  let firstParagraph = -1;
  let firstRun = -1;
  const paragraphs = documentParagraphs(document);
  for (let paragraphIndex = 0; paragraphIndex < paragraphs.length; paragraphIndex += 1) {
    const runs = paragraphRuns(paragraphs[paragraphIndex] as Element);
    const joined = runs.map(runText).join('');
    let from = 0;
    for (;;) {
      const match = joined.indexOf(search, from);
      if (match < 0) break;
      if (firstParagraph < 0) {
        firstParagraph = paragraphIndex;
        let offset = 0;
        for (let runIndex = 0; runIndex < runs.length; runIndex += 1) {
          const text = runText(runs[runIndex] as Element);
          if (match >= offset && match <= offset + text.length) {
            firstRun = runIndex;
            break;
          }
          offset += text.length;
        }
      }
      count += 1;
      from = match + search.length;
    }
  }
  return { value: { count, paragraph: firstParagraph, run: firstRun } };
}

export async function replace(context: EditorContext): Promise<CommandResult> {
  const search = requiredString(context.args, 'find');
  const replacement = stringArg(context.args, 'replacement');
  const all = context.args.all === true;
  const document = await readXml(context.zip, DOCUMENT_PART);
  const result = replaceText(document, search, replacement, all);
  if (result.count > 0) writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: result.count > 0, value: result };
}

export function clearFormatClipboard(): void {
  formatClipboard = null;
}
