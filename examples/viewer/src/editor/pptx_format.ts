import type JSZip from 'jszip';
import type { EditorArgs } from './types.js';
import type { CommandResult } from './commands.js';
import { numberArg, stringArg } from './commands.js';
import { children, direct, element, localName, readXml, setAttr, writeXml } from './xml.js';
import {
  ensureRunProperties,
  firstRunProperties,
  importClone,
  readPresentation,
  requireShape,
  slideIndex,
  slideShapes,
  textArgument,
  textRuns,
} from './pptx_model.js';

export type PptxFormatCommand =
  | 'font-family'
  | 'font-size'
  | 'text-color'
  | 'toggle-bold'
  | 'toggle-italic'
  | 'toggle-underline';

/** Копирует rPr выбранного или первого текстового объекта. */
export async function copyFormat(zip: JSZip, args: EditorArgs): Promise<Element> {
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  let properties: Element | null = null;
  if (args.shape !== undefined) {
    properties = firstRunProperties(requireShape(document, args));
  } else {
    for (const shape of slideShapes(document)) {
      properties = firstRunProperties(shape);
      if (properties) break;
    }
  }
  if (!properties) throw new Error('в выбранном тексте нет rPr для копирования формата');
  return properties;
}

/** Применяет сохранённый rPr к объекту или всем текстовым run'ам слайда. */
export async function pasteFormat(
  zip: JSZip,
  args: EditorArgs,
  copied: Element | null,
): Promise<CommandResult> {
  if (!copied) throw new Error('буфер формата пуст: сначала выполните copy-format');
  return updateRuns(zip, args, (run, document) => {
    const current = direct(run, 'rPr');
    if (current) run.removeChild(current);
    run.insertBefore(importClone(document, copied), run.firstChild);
  });
}

/** Меняет семейство, размер, цвет или начертание текстовых run'ов. */
export async function formatText(
  zip: JSZip,
  args: EditorArgs,
  command: PptxFormatCommand,
): Promise<CommandResult> {
  const toggle = command.startsWith('toggle-');
  const value = toggle ? null : formattingValue(args, command);
  let toggleValue: string | number | null = null;
  return updateRuns(zip, args, (run) => {
    const previous = direct(run, 'rPr');
    if (toggleValue === null && toggle) {
      const enabled = command === 'toggle-bold' ? previous?.getAttribute('b') === '1'
        : command === 'toggle-italic' ? previous?.getAttribute('i') === '1'
          : Boolean(previous?.getAttribute('u') && previous.getAttribute('u') !== 'none');
      toggleValue = enabled ? (command === 'toggle-underline' ? 'none' : 0)
        : (command === 'toggle-underline' ? 'sng' : 1);
    }
    const properties = ensureRunProperties(run);
    if (command === 'font-family') setFontFamily(properties, String(value));
    else if (command === 'font-size') setAttr(properties, 'sz', Number(value));
    else if (command === 'text-color') setTextColor(properties, String(value));
    else if (command === 'toggle-bold' || command === 'toggle-italic') {
      setAttr(properties, command === 'toggle-bold' ? 'b' : 'i', toggleValue ?? 1);
    } else setAttr(properties, 'u', toggleValue ?? 'sng');
  });
}

async function updateRuns(
  zip: JSZip,
  args: EditorArgs,
  update: (run: Element, document: Document) => void,
): Promise<CommandResult> {
  const presentation = await readPresentation(zip);
  const index = slideIndex(args, presentation.slides.length);
  const slide = presentation.slides[index];
  if (!slide) throw new Error('слайд не найден');
  const document = await readXml(zip, slide.path);
  const parent = args.shape === undefined ? document : requireShape(document, args);
  const runs = textRuns(parent);
  if (runs.length === 0) throw new Error('в выбранной области нет текстовых runs');
  for (const run of runs) update(run, document);
  writeXml(zip, slide.path, document);
  return { changed: true, value: { slide: index, runs: runs.length } };
}

function formattingValue(args: EditorArgs, command: PptxFormatCommand): string | number {
  if (command === 'font-family') {
    const value = stringArg(args, 'family', stringArg(args, 'fontFamily', textArgument(args))).trim();
    if (!value) throw new Error('не задано семейство шрифта');
    return value;
  }
  if (command === 'font-size') {
    const raw = args.size ?? args.value ?? args.text;
    const points = typeof raw === 'string' ? Number(raw) : numberArg({ size: raw }, 'size', Number.NaN);
    if (!Number.isFinite(points) || points <= 0 || points > 4000) {
      throw new Error('размер шрифта должен быть числом от 0 до 4000 пунктов');
    }
    return Math.round(points * 100);
  }
  if (command === 'text-color') {
    const value = stringArg(args, 'color', textArgument(args)).trim();
    const match = /^#?([\da-f]{6}|[\da-f]{8})$/i.exec(value);
    if (!match?.[1]) throw new Error('цвет должен быть в формате #RRGGBB');
    return match[1].slice(-6).toUpperCase();
  }
  throw new Error(`команда «${command}» не требует значения`);
}

function setFontFamily(properties: Element, family: string): void {
  for (const name of ['latin', 'ea', 'cs']) {
    const document = properties.ownerDocument;
    if (!document) throw new Error('rPr не принадлежит XML-документу');
    let typeface = direct(properties, name);
    if (!typeface) {
      typeface = element(document, 'a', name);
      const following = name === 'latin' ? ['ea', 'cs', 'sym', 'hlinkClick', 'hlinkMouseOver', 'rtl', 'extLst']
        : name === 'ea' ? ['cs', 'sym', 'hlinkClick', 'hlinkMouseOver', 'rtl', 'extLst']
          : ['sym', 'hlinkClick', 'hlinkMouseOver', 'rtl', 'extLst'];
      const before = children(properties).find((node) => following.includes(localName(node)));
      properties.insertBefore(typeface, before ?? null);
    }
    setAttr(typeface, 'typeface', family);
  }
}

function setTextColor(properties: Element, color: string): void {
  const fills = new Set(['noFill', 'solidFill', 'gradFill', 'blipFill', 'pattFill', 'grpFill']);
  for (const child of children(properties)) {
    if (fills.has(localName(child))) properties.removeChild(child);
  }
  const document = properties.ownerDocument;
  if (!document) throw new Error('rPr не принадлежит XML-документу');
  const fill = element(document, 'a', 'solidFill');
  const rgb = element(document, 'a', 'srgbClr');
  setAttr(rgb, 'val', color);
  fill.appendChild(rgb);
  const before = children(properties).find((node) => localName(node) !== 'ln');
  properties.insertBefore(fill, before ?? null);
}
