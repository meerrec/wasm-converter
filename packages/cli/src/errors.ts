/**
 * Ошибка, которую CLI показывает человеку как есть.
 *
 * `exitCode` — 2 для ошибок в аргументах и путях (пользователь может
 * исправить вызов) и 1 для неудачи самой конвертации.
 */
export class CliError extends Error {
  readonly exitCode: number;

  constructor(message: string, exitCode = 2) {
    super(message);
    this.name = 'CliError';
    this.exitCode = exitCode;
  }
}
