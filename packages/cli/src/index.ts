/**
 * Программный вход `@meerrec/cli`: то же, что делает bin, но из кода —
 * без разбора `process.argv` и без завершения процесса.
 */

export { parseArgs, USAGE, defaultWorkers, type BatchCommand, type Command, type ConvertCommand } from './args.js';
export { convertFile, type ConvertJob, type ConvertResult } from './convert.js';
export { CliError } from './errors.js';
export { expandInputs, globToRegExp } from './expand.js';
export {
  assertSupported,
  extensionOf,
  formatFromPath,
  inputKind,
  OUTPUT_EXTENSION,
  parseFormat,
  SUPPORTED,
  withExtension,
  type InputKind,
  type OutputFormat,
} from './formats.js';
export { WorkerPool, type WorkerFactory, type WorkerLike } from './pool.js';
export { formatBytes, ProgressReporter } from './progress.js';
export { run } from './run.js';
