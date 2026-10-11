/**
 * Разворачивание входов `batch`: файлы, каталоги и glob-шаблоны.
 *
 * Своего glob-движка в зависимостях нет намеренно: нужны `*`, `**`, `?` и
 * `[...]`, а не полноценный micromatch, и лишняя зависимость в CLI, который
 * запускают на голом `node`, дороже полусотни строк здесь.
 */

import { readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';

import { CliError } from './errors.js';
import { inputKind } from './formats.js';

/** Есть ли в шаблоне спецсимволы glob. */
export function hasMagic(pattern: string): boolean {
  return /[*?[]/.test(pattern);
}

/**
 * Перевести glob в регулярное выражение по правилам оболочки:
 * `*` — всё, кроме `/`; `**` — вместе с `/`; `?` — один символ;
 * `[...]` — класс символов.
 */
export function globToRegExp(glob: string): RegExp {
  let source = '';
  for (let i = 0; i < glob.length; i += 1) {
    const char = glob[i]!;
    if (char === '*') {
      if (glob[i + 1] === '*') {
        i += 1;
        if (glob[i + 1] === '/') {
          // `**/` покрывает и ноль каталогов: `**/*.xlsx` матчит `a.xlsx`.
          i += 1;
          source += '(?:[^/]+/)*';
        } else {
          source += '.*';
        }
      } else {
        source += '[^/]*';
      }
    } else if (char === '?') {
      source += '[^/]';
    } else if (char === '[') {
      const closing = glob.indexOf(']', i + 2);
      if (closing === -1) {
        source += '\\[';
      } else {
        const body = glob.slice(i + 1, closing);
        const negated = body.startsWith('!') || body.startsWith('^');
        const chars = negated ? body.slice(1) : body;
        source += `[${negated ? '^' : ''}${chars.replace(/[\\^\]]/g, '\\$&')}]`;
        i = closing;
      }
    } else {
      source += char.replace(/[.+^${}()|\\\]]/g, '\\$&');
    }
  }
  return new RegExp(`^${source}$`);
}

/**
 * Развернуть входы в список существующих `.docx`/`.xlsx`.
 *
 * Каталоги обходятся рекурсивно, скрытые каталоги пропускаются. Каждый шаблон
 * обязан дать хотя бы один файл — иначе это опечатка, и о ней нужно сказать
 * до запуска воркеров. Файлы с чужими расширениями молча отбрасываются:
 * `batch ./in` не должен падать из-за `readme.txt` рядом с книгами.
 */
export async function expandInputs(patterns: readonly string[]): Promise<string[]> {
  const found = new Set<string>();
  for (const pattern of patterns) {
    const matches = await expandOne(pattern);
    if (matches.length === 0) {
      throw new CliError(`no .docx/.xlsx files matched '${pattern}'`);
    }
    for (const match of matches) found.add(match);
  }
  return [...found].sort();
}

async function expandOne(pattern: string): Promise<string[]> {
  const info = await statOrNull(pattern);
  if (info?.isDirectory() === true) return walk(pattern, () => true);
  if (info?.isFile() === true) return inputKind(pattern) === null ? [] : [pattern];
  if (info !== null) return [];

  if (!hasMagic(pattern)) {
    throw new CliError(`input not found: ${pattern}`);
  }
  const root = staticPrefix(pattern);
  if (root === null) {
    // Шаблон начинается с магии — обходить пришлось бы всё дерево от корня.
    throw new CliError(`glob '${pattern}' must start with a literal directory, e.g. './in/**/*.xlsx'`);
  }
  const relative = globToRegExp(pattern.slice(root.length));
  return walk(root, (path) => relative.test(path));
}

/** Каталог до первого сегмента со спецсимволами — точка отсчёта обхода. */
function staticPrefix(pattern: string): string | null {
  const segments = pattern.split('/');
  const staticSegments: string[] = [];
  for (const segment of segments) {
    if (hasMagic(segment)) break;
    staticSegments.push(segment);
  }
  if (staticSegments.length === 0) return null;
  // Последний уцелевший сегмент — имя файла, а не каталог.
  staticSegments.pop();
  const root = staticSegments.join('/');
  return root === '' ? (pattern.startsWith('/') ? '/' : './') : `${root}/`;
}

/** Рекурсивный обход: `match` получает путь относительно `root`. */
async function walk(root: string, match: (relativePath: string) => boolean): Promise<string[]> {
  const found: string[] = [];

  const visit = async (dir: string, prefix: string): Promise<void> => {
    const entries = await readdir(dir, { withFileTypes: true });
    for (const entry of entries) {
      const relativePath = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        // Скрытые каталоги (`.git`, `.turbo`) — не источник документов.
        if (entry.name.startsWith('.')) continue;
        await visit(path, relativePath);
      } else if (entry.isFile() && inputKind(entry.name) !== null && match(relativePath)) {
        found.push(path);
      }
    }
  };

  await visit(root, '');
  return found.sort();
}

async function statOrNull(path: string): Promise<Awaited<ReturnType<typeof stat>> | null> {
  try {
    return await stat(path);
  } catch {
    return null;
  }
}
