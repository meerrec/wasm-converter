import { describe, expect, it } from 'vitest';

import { CliError } from '../src/errors.js';
import { assertSupported, extensionOf, formatFromPath, inputKind, parseFormat, SUPPORTED, withExtension } from '../src/formats.js';

describe('formats', () => {
  it('разбирает расширения без учёта регистра', () => {
    expect(inputKind('/tmp/Report.DOCX')).toBe('docx');
    expect(inputKind('book.xlsx')).toBe('xlsx');
    expect(inputKind('book.xls')).toBeNull();
    expect(inputKind('doc.zip')).toBeNull();
    expect(inputKind('noextension')).toBeNull();
  });

  it('разбирает новые расширения в верный InputKind', () => {
    expect(inputKind('deck.pptx')).toBe('pptx');
    expect(inputKind('letter.odt')).toBe('odt');
    expect(inputKind('sheet.ods')).toBe('ods');
    expect(inputKind('slides.odp')).toBe('odp');
    expect(inputKind('book.xlsb')).toBe('xlsb');
    // `.docm` остаётся вне скоупа: макросы CLI не читает.
    expect(inputKind('book.docm')).toBeNull();
  });

  it('матрица SUPPORTED: документы и презентации — четыре цели, таблицы — PDF', () => {
    expect(SUPPORTED.docx).toEqual(['pdf', 'md', 'html', 'txt']);
    expect(SUPPORTED.odt).toEqual(['pdf', 'md', 'html', 'txt']);
    expect(SUPPORTED.pptx).toEqual(['pdf', 'md', 'html', 'txt']);
    expect(SUPPORTED.odp).toEqual(['pdf', 'md', 'html', 'txt']);
    expect(SUPPORTED.xlsx).toEqual(['pdf']);
    expect(SUPPORTED.ods).toEqual(['pdf']);
    expect(SUPPORTED.xlsb).toEqual(['pdf']);
  });

  it('понимает синонимы выходных расширений', () => {
    expect(formatFromPath('out.markdown')).toBe('md');
    expect(formatFromPath('out.HTM')).toBe('html');
    expect(formatFromPath('out.text')).toBe('txt');
    expect(formatFromPath('out.pdf')).toBe('pdf');
    expect(formatFromPath('out.docx')).toBeNull();
  });

  it('расширение — часть имени файла, а не каталога', () => {
    expect(extensionOf('/tmp/doc.x/docx')).toBe('');
    expect(extensionOf('.hidden')).toBe('');
  });

  it('меняет расширение, не трогая каталоги с точкой', () => {
    expect(withExtension('/tmp/v1.2/report.docx', 'pdf')).toBe('/tmp/v1.2/report.pdf');
    expect(withExtension('report', 'md')).toBe('report.md');
  });

  it('нормализует --format', () => {
    expect(parseFormat('PDF')).toBe('pdf');
    expect(parseFormat('.md')).toBe('md');
    expect(() => parseFormat('doc')).toThrow(CliError);
  });

  it('запрещает xlsx → текст', () => {
    expect(() => assertSupported('xlsx', 'md')).toThrow(/xlsx → md is not supported/);
    expect(() => assertSupported('docx', 'txt')).not.toThrow();
    expect(() => assertSupported('xlsx', 'pdf')).not.toThrow();
  });
});
