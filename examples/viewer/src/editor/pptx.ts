import type { CommandResult, EditorContext } from './commands.js';
import {
  cutText,
  getText,
  insertChart,
  insertPicture,
  insertTable,
  insertText,
  pasteText,
} from './pptx_content.js';
import { copyFormat, formatText, pasteFormat } from './pptx_format.js';
import type { PptxFormatCommand } from './pptx_format.js';
import {
  changeSlideLayout,
  createSlide,
  deleteSlide,
  duplicateSlide,
  newDocument,
  resetSlide,
  sortSlides,
} from './pptx_structure.js';

let copiedFormat: Element | null = null;

const FORMAT_COMMANDS = new Set<PptxFormatCommand>([
  'font-family',
  'font-size',
  'text-color',
  'toggle-bold',
  'toggle-italic',
  'toggle-underline',
]);

/** Выполняет адресную PPTX-команду над исходным ZIP-пакетом. */
export async function executePptx(context: EditorContext, command: string): Promise<CommandResult> {
  if (context.format !== 'pptx') throw new Error(`команда PPTX вызвана для формата ${context.format}`);
  const { zip, args } = context;
  if (command === 'new-document') {
    copiedFormat = null;
    return newDocument(zip);
  }
  if (command === 'get-text') return getText(zip, args);
  if (command === 'cut-text') {
    const result = await cutText(zip, args);
    context.clipboardFormat = 'pptx-text';
    context.clipboardText = typeof result.value === 'string' ? result.value : '';
    return result;
  }
  if (command === 'paste-text') {
    const withClipboard = args.text === undefined && args.value === undefined
      ? { ...args, text: context.clipboardText }
      : args;
    return pasteText(zip, withClipboard);
  }
  if (command === 'insert-text') return insertText(zip, args);
  if (command === 'create-slide') return createSlide(zip, args);
  if (command === 'slide-layout') return changeSlideLayout(zip, args);
  if (command === 'reset-slide') return resetSlide(zip, args);
  if (command === 'insert-table') return insertTable(zip, args);
  if (command === 'insert-picture') return insertPicture(zip, args);
  if (command === 'insert-chart') return insertChart(zip, args);
  if (command === 'copy-format') {
    copiedFormat = await copyFormat(zip, args);
    return { changed: false, message: 'формат скопирован' };
  }
  if (command === 'paste-format') return pasteFormat(zip, args, copiedFormat);
  if (isFormatCommand(command)) return formatText(zip, args, command);
  if (command === 'delete-slide') return deleteSlide(zip, args);
  if (command === 'duplicate-slide') return duplicateSlide(zip, args);
  if (command === 'sort-slides') return sortSlides(zip, args);
  throw new Error(`неизвестная команда PPTX «${command}»`);
}

function isFormatCommand(command: string): command is PptxFormatCommand {
  return FORMAT_COMMANDS.has(command as PptxFormatCommand);
}
