import type { Diagnostic } from '@meerrec/core';

/**
 * Ошибка, которую CLI показывает человеку как есть.
 *
 * `exitCode` — 2 для ошибок в аргументах и путях (пользователь может
 * исправить вызов) и 1 для неудачи самой конвертации.
 *
 * `diagnostic` — машинная диагностика из `@meerrec/core` (`code`, `layer`,
 * `message`); она уходит в `--json` и `formatDiagnostic`, а `message` остаётся
 * явным текстом, чтобы человекочитаемый вывод не менялся. Для ошибок разбора
 * аргументов диагностики может не быть — тогда её соберёт `run`.
 */
export class CliError extends Error {
  readonly exitCode: number;
  readonly diagnostic?: Diagnostic;

  constructor(message: string, exitCode = 2, diagnostic?: Diagnostic) {
    super(message);
    this.name = 'CliError';
    this.exitCode = exitCode;
    // exactOptionalPropertyTypes: отсутствие диагностики — это отсутствие
    // поля, а не `undefined` в нём.
    if (diagnostic !== undefined) this.diagnostic = diagnostic;
  }
}
