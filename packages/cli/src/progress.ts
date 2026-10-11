/**
 * Прогресс `batch` в stderr.
 *
 * Свой минимальный индикатор вместо `cli-progress`/`ora`: stdout остаётся
 * чистым для скриптов, а от прогресса нужны только счётчик, полоса и
 * последняя неудача. В терминале строка перерисовывается через `\r`, в
 * пайпе — построчный лог: там `\r` превратился бы в мусор.
 */

export interface ProgressEvent {
  ok: boolean;
  /** Имя файла без каталогов. */
  name: string;
  /** Что записать после имени: путь выхода и размер или текст ошибки. */
  detail: string;
}

/** Размер в удобных единицах: `4.0 KiB`. */
export function formatBytes(bytes: number): string {
  const units = ['B', 'KiB', 'MiB', 'GiB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

export class ProgressReporter {
  readonly #total: number;
  readonly #stream: NodeJS.WriteStream;
  readonly #quiet: boolean;
  readonly #tty: boolean;
  #done = 0;
  #failed = 0;
  #wrote = false;

  constructor(total: number, options: { stream?: NodeJS.WriteStream; quiet?: boolean } = {}) {
    this.#total = total;
    this.#stream = options.stream ?? process.stderr;
    this.#quiet = options.quiet ?? false;
    this.#tty = this.#stream.isTTY === true;
  }

  /** Сколько файлов не удалось. */
  get failed(): number {
    return this.#failed;
  }

  report(event: ProgressEvent): void {
    this.#done += 1;
    if (!event.ok) this.#failed += 1;
    if (this.#quiet) return;

    if (this.#tty) {
      this.#stream.write(`\r${this.#bar()}  ${event.name}${' '.repeat(20)}\r`);
    } else {
      const mark = event.ok ? 'ok  ' : 'FAIL';
      this.#stream.write(`${mark} ${this.#done}/${this.#total} ${event.name}: ${event.detail}\n`);
    }
  }

  /** Закрыть строку прогресса; в терминале — перевести курсор. */
  finish(): void {
    if (this.#quiet || !this.#tty || !this.#wrote) return;
    this.#stream.write(`\r${this.#bar()}${' '.repeat(20)}\n`);
  }

  #bar(): string {
    const width = 24;
    const filled = this.#total === 0 ? width : Math.round((this.#done / this.#total) * width);
    const percent = this.#total === 0 ? 100 : Math.round((this.#done / this.#total) * 100);
    const failures = this.#failed === 0 ? '' : ` failed: ${this.#failed}`;
    this.#wrote = true;
    return `[${'#'.repeat(filled)}${'-'.repeat(width - filled)}] ${this.#done}/${this.#total} ${percent}%${failures}`;
  }
}
