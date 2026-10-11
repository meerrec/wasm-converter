import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { expandInputs, globToRegExp, hasMagic } from '../src/expand.js';

describe('globToRegExp', () => {
  it('* не переходит через слэш', () => {
    const re = globToRegExp('*.xlsx');
    expect(re.test('a.xlsx')).toBe(true);
    expect(re.test('sub/a.xlsx')).toBe(false);
  });

  it('**/ покрывает и ноль каталогов', () => {
    const re = globToRegExp('**/*.xlsx');
    expect(re.test('a.xlsx')).toBe(true);
    expect(re.test('a/b/c.xlsx')).toBe(true);
    expect(re.test('a/b/c.docx')).toBe(false);
  });

  it('? — один символ, [...] — класс', () => {
    expect(globToRegExp('a?.docx').test('ab.docx')).toBe(true);
    expect(globToRegExp('a?.docx').test('abc.docx')).toBe(false);
    expect(globToRegExp('[ab]*.docx').test('b1.docx')).toBe(true);
    expect(globToRegExp('[!ab]*.docx').test('b1.docx')).toBe(false);
  });

  it('точки и скобки экранируются как литералы', () => {
    expect(globToRegExp('v1.2/*.docx').test('v1.2/x.docx')).toBe(true);
    expect(globToRegExp('v1.2/*.docx').test('v1x2/x.docx')).toBe(false);
  });

  it('магия распознаётся', () => {
    expect(hasMagic('in/**/*.xlsx')).toBe(true);
    expect(hasMagic('in/book.xlsx')).toBe(false);
  });
});

describe('expandInputs', () => {
  let root: string;

  beforeAll(async () => {
    root = await mkdtemp(join(tmpdir(), 'doc-converter-expand-'));
    await mkdir(join(root, 'in', 'nested'), { recursive: true });
    await writeFile(join(root, 'in', 'a.docx'), 'x');
    await writeFile(join(root, 'in', 'b.xlsx'), 'x');
    await writeFile(join(root, 'in', 'readme.txt'), 'x');
    await writeFile(join(root, 'in', 'nested', 'c.docx'), 'x');
    await mkdir(join(root, 'in', '.hidden'), { recursive: true });
    await writeFile(join(root, 'in', '.hidden', 'd.docx'), 'x');
  });

  afterAll(async () => {
    const { rm } = await import('node:fs/promises');
    await rm(root, { recursive: true, force: true });
  });

  it('обходит каталог рекурсивно, отбрасывая чужие расширения и скрытые каталоги', async () => {
    const files = await expandInputs([join(root, 'in')]);
    expect(files).toEqual([join(root, 'in', 'a.docx'), join(root, 'in', 'b.xlsx'), join(root, 'in', 'nested', 'c.docx')]);
  });

  it('разворачивает glob от статичного каталога', async () => {
    const files = await expandInputs([join(root, 'in', '*.docx')]);
    expect(files).toEqual([join(root, 'in', 'a.docx')]);

    const recursive = await expandInputs([join(root, 'in', '**/*.docx')]);
    expect(recursive).toEqual([join(root, 'in', 'a.docx'), join(root, 'in', 'nested', 'c.docx')]);
  });

  it('принимает и файл, и несколько шаблонов без дублей', async () => {
    const files = await expandInputs([join(root, 'in', 'a.docx'), join(root, 'in', '*.docx')]);
    expect(files).toEqual([join(root, 'in', 'a.docx')]);
  });

  it('ругается на несуществующий путь и на пустую выборку', async () => {
    await expect(expandInputs([join(root, 'nope.docx')])).rejects.toThrow(/input not found/);
    await expect(expandInputs([join(root, 'in', '*.pptx')])).rejects.toThrow(/no \.docx\/\.xlsx files matched/);
  });
});
