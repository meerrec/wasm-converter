/**
 * Тесты класса X2TConverter: init/loadX2t, разбор имени, санитизация под MEMFS,
 * сквозная передача результата x2t (коды не фильтруются). Рантайм x2t замокан целиком.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@meerrec/x2t', () => ({
  loadX2t: vi.fn(),
  convert: vi.fn(),
  X2tError: class X2tError extends Error {},
}));

import { convert as convertWithX2t, loadX2t } from '@meerrec/x2t';
import type { X2tModule } from '@meerrec/x2t';

import { X2TConverter } from '../src/converter.js';

const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';

/** Модуль x2t в тестах — заглушка: рантайм сюда не заходит. */
const fakeModule = {} as unknown as X2tModule;

beforeEach(() => {
  vi.mocked(loadX2t).mockReset();
  vi.mocked(loadX2t).mockResolvedValue(fakeModule);
  vi.mocked(convertWithX2t).mockReset();
});

describe('X2TConverter.init', () => {
  it('передаёт loadX2t все заданные опции', async () => {
    const onPrint = (line: string): void => {
      void line;
    };
    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js', wasmUrl: '/x2t/x2t.wasm', onPrint });
    expect(loadX2t).toHaveBeenCalledWith({
      scriptUrl: '/x2t/x2t.js',
      wasmUrl: '/x2t/x2t.wasm',
      onPrint,
    });
  });

  it('без wasmUrl/onPrint не передаёт эти поля вовсе', async () => {
    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });
    const options = vi.mocked(loadX2t).mock.calls[0]![0]!;
    expect(options).toEqual({ scriptUrl: '/x2t/x2t.js' });
    expect('wasmUrl' in options).toBe(false);
    expect('onPrint' in options).toBe(false);
  });

  it('повторный init бросает', async () => {
    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });
    await expect(converter.init({ scriptUrl: '/x2t/x2t.js' })).rejects.toThrow('already called');
  });
});

describe('X2TConverter.convert', () => {
  it('без init бросает понятную ошибку', async () => {
    const converter = new X2TConverter();
    await expect(converter.convert(new Uint8Array([1]), 'odt')).rejects.toThrow('requires init');
  });

  it('конвертирует File в ODT: from/to/fileName и полный результат', async () => {
    const output = new Uint8Array([1, 2, 3]);
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: output, code: 0, stderr: '' });

    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });
    const file = new File(['тело документа'], 'отчёт.docx', { type: DOCX_MIME });

    const result = await converter.convert(file, 'ODT');

    expect(convertWithX2t).toHaveBeenCalledTimes(1);
    const [moduleArg, inputArg, optionsArg] = vi.mocked(convertWithX2t).mock.calls[0]!;
    expect(moduleArg).toBe(fakeModule);
    // Кириллица в имени сохраняется — MEMFS её переживает.
    expect(optionsArg).toEqual({ from: 'docx', to: 'odt', fileName: 'отчёт' });
    expect(inputArg).toEqual(new Uint8Array(await file.arrayBuffer()));

    expect(result).toEqual({
      bytes: output,
      code: 0,
      stderr: '',
      fileName: 'отчёт.odt',
      mimeType: 'application/vnd.oasis.opendocument.text',
    });
  });

  it('санитизирует имя из options.fileName: слэши в подчёркивания, пробелы и кириллица остаются', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: new Uint8Array(0), code: 0, stderr: '' });

    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });

    const result = await converter.convert(new Blob(['x']), 'odt', {
      fileName: 'папка/мой файл<1>.docx',
    });

    const optionsArg = vi.mocked(convertWithX2t).mock.calls[0]![2]!;
    expect(optionsArg.fileName).toBe('папка_мой файл_1_');
    expect(result.fileName).toBe('папка_мой файл_1_.odt');
  });

  it('обрезает слишком длинное имя', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: new Uint8Array(0), code: 0, stderr: '' });

    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });

    const result = await converter.convert(new Blob(['x']), 'odt', {
      fileName: `${'a'.repeat(150)}.docx`,
    });

    expect(result.fileName).toBe(`${'a'.repeat(100)}.odt`);
  });

  it('пустое после санитизации имя → document', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: new Uint8Array(0), code: 0, stderr: '' });

    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });

    const result = await converter.convert(new Blob(['x']), 'docx', { fileName: '  .docx' });

    expect(result.fileName).toBe('document.docx');
  });

  it('Blob без имени → document.<расширение>', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: new Uint8Array(0), code: 0, stderr: '' });

    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });

    const result = await converter.convert(new Blob(['x'], { type: 'text/plain' }), 'odt');

    const optionsArg = vi.mocked(convertWithX2t).mock.calls[0]![2]!;
    expect(optionsArg).toEqual({ from: 'odt', to: 'odt', fileName: 'document' });
    expect(result.fileName).toBe('document.odt');
  });

  it('принимает Uint8Array как вход', async () => {
    const output = new Uint8Array([9]);
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: output, code: 0, stderr: '' });

    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });
    const input = new Uint8Array([4, 5, 6]);

    const result = await converter.convert(input, 'txt');

    const [, inputArg, optionsArg] = vi.mocked(convertWithX2t).mock.calls[0]!;
    expect(inputArg).toBe(input);
    expect(optionsArg).toEqual({ from: 'txt', to: 'txt', fileName: 'document' });
    expect(result.mimeType).toBe('text/plain');
  });

  it('имя без расширения → ошибка', async () => {
    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });
    await expect(converter.convert(new File(['x'], 'report'), 'odt')).rejects.toThrow(
      'no usable extension',
    );
  });

  it('код 80 не фильтруется', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({
      bytes: new Uint8Array(0),
      code: 80,
      stderr: 'renderer not available',
    });

    const converter = new X2TConverter();
    await converter.init({ scriptUrl: '/x2t/x2t.js' });

    const result = await converter.convert(new Blob(['x']), 'pdf');

    expect(result.code).toBe(80);
    expect(result.stderr).toBe('renderer not available');
    expect(result.mimeType).toBe('application/pdf');
  });
});
