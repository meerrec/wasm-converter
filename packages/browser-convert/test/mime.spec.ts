/**
 * Тесты карты расширений в MIME-типы: известные форматы, регистр, дефолт.
 */
import { describe, expect, it } from 'vitest';

import { documentMimeType } from '../src/mime.js';

describe('documentMimeType', () => {
  it('возвращает MIME для известных расширений', () => {
    expect(documentMimeType('docx')).toBe(
      'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    );
    expect(documentMimeType('doc')).toBe('application/msword');
    expect(documentMimeType('odt')).toBe('application/vnd.oasis.opendocument.text');
    expect(documentMimeType('rtf')).toBe('application/rtf');
    expect(documentMimeType('txt')).toBe('text/plain');
    expect(documentMimeType('xlsx')).toBe(
      'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    );
    expect(documentMimeType('xls')).toBe('application/vnd.ms-excel');
    expect(documentMimeType('ods')).toBe('application/vnd.oasis.opendocument.spreadsheet');
    expect(documentMimeType('csv')).toBe('text/csv');
    expect(documentMimeType('pptx')).toBe(
      'application/vnd.openxmlformats-officedocument.presentationml.presentation',
    );
    expect(documentMimeType('ppt')).toBe('application/vnd.ms-powerpoint');
    expect(documentMimeType('odp')).toBe('application/vnd.oasis.opendocument.presentation');
    expect(documentMimeType('pdf')).toBe('application/pdf');
    expect(documentMimeType('png')).toBe('image/png');
    expect(documentMimeType('jpg')).toBe('image/jpeg');
    expect(documentMimeType('jpeg')).toBe('image/jpeg');
    expect(documentMimeType('html')).toBe('text/html');
  });

  it('не чувствителен к регистру', () => {
    expect(documentMimeType('DOCX')).toBe(
      'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    );
    expect(documentMimeType('Pdf')).toBe('application/pdf');
    expect(documentMimeType('JPEG')).toBe('image/jpeg');
  });

  it('снимает ведущую точку и пробелы по краям', () => {
    expect(documentMimeType('.docx')).toBe(
      'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    );
    expect(documentMimeType(' pdf ')).toBe('application/pdf');
  });

  it('неизвестное и пустое расширение → octet-stream', () => {
    expect(documentMimeType('bin')).toBe('application/octet-stream');
    expect(documentMimeType('')).toBe('application/octet-stream');
  });
});
