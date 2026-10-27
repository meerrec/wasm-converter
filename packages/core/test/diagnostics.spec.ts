import { describe, expect, it } from 'vitest';

import {
  formatDiagnostic,
  toDiagnostic,
  ViewerError,
  workerDiagnostic,
} from '../src/diagnostics.js';
import type { WorkerErrorMsg } from '../src/protocol.js';

/**
 * Единая диагностика: один тип для Rust/воркера/CLI.
 *
 * Проверяем нормализацию: структурный объект, строковый префикс wasm
 * (`[cancelled]`, ADR-0032), код воркера и текст для человека.
 */
describe('toDiagnostic', () => {
  it('принимает структурный объект как есть', () => {
    const diagnostic = {
      code: 'export-failed' as const,
      layer: 'export' as const,
      message: 'pdf writer failed',
      retryable: false,
    };
    expect(toDiagnostic(diagnostic)).toEqual(diagnostic);
  });

  it('разбирает префикс отмены и убирает его из текста', () => {
    expect(toDiagnostic('[cancelled] operation cancelled')).toMatchObject({
      code: 'cancelled',
      layer: 'orchestration',
      message: 'operation cancelled',
      retryable: true,
    });
  });

  it('разбирает префиксы разбора/раскладки/экспорта', () => {
    expect(toDiagnostic('[layout] bad text')).toMatchObject({
      code: 'layout-failed',
      layer: 'layout',
      message: 'bad text',
    });
    expect(toDiagnostic('[export] no pdf')).toMatchObject({
      code: 'export-failed',
      layer: 'export',
    });
  });

  it('без разметки сохраняет текст и берёт fallback', () => {
    expect(
      toDiagnostic(new Error('boom'), { code: 'malformed-package', layer: 'parse', format: 'docx' }),
    ).toEqual({
      code: 'malformed-package',
      layer: 'parse',
      message: 'boom',
      retryable: false,
      format: 'docx',
    });
  });

  it('ViewerError не заворачивается повторно', () => {
    const original = toDiagnostic('[cancelled] stop');
    const error = new ViewerError(original);
    expect(toDiagnostic(error)).toBe(original);
  });
});

describe('workerDiagnostic', () => {
  it('предпочитает присланную диагностику', () => {
    const msg: WorkerErrorMsg = {
      type: 'error',
      message: 'ignored',
      diagnostic: {
        code: 'limit-exceeded',
        layer: 'parse',
        message: 'too many parts',
        retryable: false,
      },
    };
    expect(workerDiagnostic(msg, 'xlsx').message).toBe('too many parts');
  });

  it('собирает диагностику из кода open', () => {
    const msg: WorkerErrorMsg = { type: 'error', message: 'bad zip', code: 'open' };
    expect(workerDiagnostic(msg, 'xlsx')).toMatchObject({
      code: 'malformed-package',
      layer: 'parse',
      format: 'xlsx',
      retryable: false,
    });
  });

  it('собирает диагностику из кода cancelled', () => {
    const msg: WorkerErrorMsg = { type: 'error', message: 'stop', code: 'cancelled' };
    expect(workerDiagnostic(msg)).toMatchObject({
      code: 'cancelled',
      layer: 'orchestration',
      retryable: true,
    });
  });
});

describe('formatDiagnostic', () => {
  it('печатает слой, код, формат и текст', () => {
    expect(
      formatDiagnostic({
        code: 'malformed-package',
        layer: 'parse',
        message: 'bad zip',
        format: 'docx',
        retryable: false,
      }),
    ).toBe('parse/malformed-package: [docx] bad zip');
  });
});
