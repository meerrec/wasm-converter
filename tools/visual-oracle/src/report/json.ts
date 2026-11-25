// Сборка машинного отчёта прогона (`report.json`).
//
// Отчёт обязан быть воспроизводимым: одинаковый прогон даёт побайтово
// одинаковый файл, иначе diff отчётов в CI превратился бы в шум. Поэтому
// время генерации приходит извне (`now`), captures сортируются по
// `fixtureId`, затем по `captureId`, а все счётчики заполняются нулями
// заранее — отсутствующий ключ и нулевой ключ должны выглядеть одинаково для
// потребителя.
//
// `byTag` считается по тегам фикстур из corpus index: тег без captures
// попадает в отчёт с нулями, потому что «тег есть, а покрытия нет» — это
// факт о корпусе, а не отсутствие данных.

import type { Corpus } from '../corpus.js';
import type { CaptureResult, Cause, RunReport } from '../types.js';

/** Входные данные сборки отчёта. */
export interface BuildReportInput {
  corpus: Corpus;
  captures: readonly CaptureResult[];
  /** Ревизия движка, попавшая в отчёт (`git rev-parse --short HEAD`). */
  engineRevision: string;
  /** Принятый baseline либо `null`, если baseline ещё не принят. */
  baselineId: string | null;
  /** Часы: подменяются в тестах, чтобы отчёт был детерминирован. */
  now?: () => Date;
}

/**
 * Собрать отчёт прогона.
 *
 * Чистая функция относительно входа: единственный источник недетерминизма —
 * `now`, и он инъектируется.
 */
export function buildReport(input: BuildReportInput): RunReport {
  const captures = [...input.captures].sort(
    (a, b) => a.fixtureId.localeCompare(b.fixtureId) || a.captureId.localeCompare(b.captureId),
  );

  const summary: RunReport['summary'] = {
    total: captures.length,
    match: 0,
    'visual-diff': 0,
    'page-mismatch': 0,
    'missing-reference': 0,
    'capture-error': 0,
    'harness-error': 0,
  };

  const byCause: Record<Cause, number> = {
    parse: 0,
    model: 0,
    layout: 0,
    font: 0,
    render: 0,
    harness: 0,
    reference: 0,
  };

  for (const capture of captures) {
    summary[capture.resultType] += 1;
    byCause[capture.cause] += 1;
  }

  return {
    schemaVersion: '1.0.0',
    generatedAt: (input.now ?? (() => new Date()))().toISOString(),
    engineRevision: input.engineRevision,
    corpusId: input.corpus.corpus.id,
    corpusVersion: input.corpus.corpus.version,
    matrixVersion: input.corpus.corpus.matrixVersion,
    baselineId: input.baselineId,
    summary,
    byCause,
    byTag: buildByTag(input.corpus, captures),
    captures,
  };
}

/**
 * Сериализовать отчёт: два пробела отступа и завершающий перевод строки.
 *
 * Формат фиксирован, чтобы `report.json` читался глазами и не давал
 * ложных diff'ов из-за форматирования.
 */
export function serializeReport(report: RunReport): string {
  return `${JSON.stringify(report, null, 2)}\n`;
}

/** Счётчики по тегам фикстур: сколько captures и сколько из них совпало. */
function buildByTag(
  corpus: Corpus,
  captures: readonly CaptureResult[],
): Record<string, { total: number; match: number }> {
  const tagsByFixture = new Map<string, readonly string[]>();
  const byTag: Record<string, { total: number; match: number }> = {};
  for (const fixture of corpus.fixtures) {
    tagsByFixture.set(fixture.id, fixture.tags);
    for (const tag of fixture.tags) byTag[tag] ??= { total: 0, match: 0 };
  }

  for (const capture of captures) {
    const tags = tagsByFixture.get(capture.fixtureId);
    if (tags === undefined) continue;
    for (const tag of tags) {
      const entry = byTag[tag];
      if (entry === undefined) continue;
      entry.total += 1;
      if (capture.resultType === 'match') entry.match += 1;
    }
  }

  // Ключи отсортированы: порядок вставки зависел бы от порядка фикстур.
  const sorted: Record<string, { total: number; match: number }> = {};
  for (const tag of Object.keys(byTag).sort()) {
    const entry = byTag[tag];
    if (entry !== undefined) sorted[tag] = entry;
  }
  return sorted;
}
