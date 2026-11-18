/// <reference lib="webworker" />

import JSZip from 'jszip';
import type { EditorContext, CommandResult } from './commands.js';
import { executeDocx } from './docx.js';
import { executePptx } from './pptx.js';
import { executeXlsx } from './xlsx.js';
import type { EditorArgs, EditorFormat, EditorRequest, EditorResponse } from './types.js';

const HISTORY_LIMIT = 20;

let zip: JSZip | null = null;
let format: EditorFormat | null = null;
let name = 'document';
let current: ArrayBuffer | null = null;
let undoStack: ArrayBuffer[] = [];
let redoStack: ArrayBuffer[] = [];
let clipboardFormat: string | null = null;
let clipboardText = '';

async function bytes(): Promise<ArrayBuffer> {
  if (!zip) throw new Error('документ не открыт');
  return zip.generateAsync({
    type: 'arraybuffer',
    compression: 'DEFLATE',
    compressionOptions: { level: 6 },
  });
}

function flags(changed: boolean, data?: ArrayBuffer) {
  return {
    changed,
    canUndo: undoStack.length > 0,
    canRedo: redoStack.length > 0,
    ...(data ? { bytes: data } : {}),
  };
}

async function open(nextFormat: EditorFormat, source: ArrayBuffer, nextName: string): Promise<EditorResponse> {
  const opened = await JSZip.loadAsync(source.slice(0));
  zip = opened;
  format = nextFormat;
  name = nextName;
  current = source.slice(0);
  undoStack = [];
  redoStack = [];
  clipboardFormat = null;
  clipboardText = '';
  return { type: 'result', id: 0, ...flags(false) };
}

async function run(command: string, args: EditorArgs): Promise<EditorResponse> {
  if (!zip || !format) throw new Error('документ не открыт');
  const context: EditorContext = { zip, format, args, clipboardFormat, clipboardText };
  let result: CommandResult;
  if (format === 'docx') result = await executeDocx(context, command);
  else if (format === 'xlsx') result = await executeXlsx(context, command);
  else result = await executePptx(context, command);
  clipboardFormat = context.clipboardFormat;
  clipboardText = context.clipboardText;
  if (result.changed !== true) {
    return {
      type: 'result',
      id: 0,
      ...flags(false),
      value: result.value,
      ...(result.message !== undefined ? { message: result.message } : {}),
    };
  }
  const next = await bytes();
  if (current) {
    undoStack.push(current);
    if (undoStack.length > HISTORY_LIMIT) undoStack.shift();
  }
  redoStack = [];
  current = next;
  return {
    type: 'result',
    id: 0,
    ...flags(true, next),
    value: result.value,
    ...(result.message !== undefined ? { message: result.message } : {}),
  };
}

async function restore(target: 'undo' | 'redo'): Promise<EditorResponse> {
  if (!zip) throw new Error('документ не открыт');
  const from = target === 'undo' ? undoStack : redoStack;
  const to = target === 'undo' ? redoStack : undoStack;
  const next = from.pop();
  if (!next) return { type: 'result', id: 0, ...flags(false), message: 'история пуста' };
  if (current) to.push(current);
  current = next;
  zip = await JSZip.loadAsync(next.slice(0));
  return { type: 'result', id: 0, ...flags(true, next.slice(0)) };
}

self.onmessage = async (event: MessageEvent<EditorRequest>): Promise<void> => {
  const message = event.data;
  if (message.type === 'close') {
    zip = null;
    current = null;
    undoStack = [];
    redoStack = [];
    self.close();
    return;
  }
  let response: EditorResponse;
  try {
    if (message.type === 'open') {
      response = await open(message.format, message.bytes, message.name);
    } else if (message.type === 'create') {
      response = await open(message.format, message.bytes, message.name);
    } else if (message.type === 'execute') {
      response = await run(message.command, message.args);
    } else if (message.type === 'save') {
      response = { type: 'result', id: 0, ...flags(false, await bytes()), value: { name } };
    } else {
      response = await restore(message.type);
    }
  } catch (error) {
    response = {
      type: 'error',
      id: message.id,
      message: error instanceof Error ? error.message : String(error),
    };
  }
  const responseWithId = { ...response, id: message.id } as EditorResponse;
  if (responseWithId.type === 'result' && responseWithId.bytes) {
    self.postMessage(responseWithId, [responseWithId.bytes]);
  } else {
    self.postMessage(responseWithId);
  }
};
