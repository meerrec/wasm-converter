import type JSZip from 'jszip';
import type { EditorArgs, EditorFormat } from './types.js';

export interface CommandResult {
  changed?: boolean;
  value?: unknown;
  message?: string;
}

export interface EditorContext {
  zip: JSZip;
  format: EditorFormat;
  args: EditorArgs;
  clipboardFormat: string | null;
  clipboardText: string;
}

export type CommandHandler = (context: EditorContext) => Promise<CommandResult>;

export function stringArg(args: EditorArgs, name: string, fallback = ''): string {
  const value = args[name];
  return typeof value === 'string' ? value : fallback;
}

export function numberArg(args: EditorArgs, name: string, fallback = 0): number {
  const value = args[name];
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

export function booleanArg(args: EditorArgs, name: string, fallback = false): boolean {
  const value = args[name];
  return typeof value === 'boolean' ? value : fallback;
}

export function requiredString(args: EditorArgs, name: string): string {
  const value = stringArg(args, name).trim();
  if (!value) throw new Error(`не задан параметр «${name}»`);
  return value;
}
