// Работа с произвольным JSON на границе процесса: единственный канонический
// type guard и каноническая сериализация. Оба нужны и загрузчику corpus
// index, и валидатору manifest, и алгоритму `baselineId`, поэтому живут в
// одном месте — расхождение между копиями сломало бы хеш baseline.

/**
 * Проверка «это JSON-объект»: не `null`, не массив.
 *
 * Поля остаются `unknown` — дальше их проверяет вызывающий код по месту.
 */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Канонический JSON: ключи объектов отсортированы, пробелов нет.
 *
 * Канонизация нужна, чтобы `baselineId` не зависел от порядка ключей в
 * исходном файле. Массивы сохраняют порядок — он часть данных.
 */
export function canonicalJson(value: unknown): string {
  return JSON.stringify(canonicalize(value));
}

function canonicalize(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (isRecord(value)) {
    const sorted: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) {
      sorted[key] = canonicalize(value[key]);
    }
    return sorted;
  }
  return value;
}
