import { describe, expect, it } from 'vitest';

import type { Diagnostic } from '@meerrec/core';

import { CliError } from '../src/errors.js';

const diagnostic: Diagnostic = {
  code: 'unsupported-format',
  layer: 'orchestration',
  message: 'xlsx → md is not supported',
  retryable: false,
};

describe('CliError', () => {
  it('по умолчанию — код 2 и без диагностики', () => {
    const error = new CliError('bad arguments');
    expect(error).toBeInstanceOf(Error);
    expect(error.name).toBe('CliError');
    expect(error.message).toBe('bad arguments');
    expect(error.exitCode).toBe(2);
    expect(error.diagnostic).toBeUndefined();
  });

  it('хранит явный код возврата и диагностику', () => {
    const error = new CliError('xlsx → md is not supported', 1, diagnostic);
    expect(error.exitCode).toBe(1);
    expect(error.diagnostic).toEqual(diagnostic);
    // Текст ошибки остаётся явным и не подменяется диагностикой.
    expect(error.message).toBe('xlsx → md is not supported');
  });
});
