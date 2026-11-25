// Классификация результата capture: тип результата и причина расхождения.
//
// Порядок правил жёсткий — первое сработавшее выигрывает, потому что каждое
// следующее опирается на данные, которых у предыдущего может не быть:
//
// 1. Нет manifest или он не прошёл валидацию → `missing-reference` /
//    `reference`. Сравнивать не с чем, и это не дефект движка.
// 2. Сбой захвата → `capture-error` / `harness`, кроме кодов сломанного
//    контура (`missing-tool`, `browser-failed`, `fixture-not-listed`,
//    `rasterize-failed`) — они дают `harness-error` / `harness`. Таблица
//    отображения берётся из `capture/errors.ts`: один источник правды.
// 3. Число страниц или листов не совпало → `page-mismatch` / `layout`.
// 4. Размеры изображений не совпали → `page-mismatch` / `layout`.
// 5. Ноль расходящихся пикселей вне масок → `match` / `render` (нейтральная
//    причина: совпадение ничего не говорит о том, кто отрисовал кадр).
// 6. Иначе `visual-diff` с предварительной причиной по правилам ниже.
//
// Предварительная причина `visual-diff` детерминирована и переопределяема
// оператором через `overrides` (ключ — `fixtureId` или `fixtureId/captureId`):
//
// - весь bounding box расхождений в нижних 15 % кадра (верхние 85 % совпали)
//   → `layout` (съехала пагинация или подвал);
// - регионы покрывают ≥ 60 % площади → `layout` (сдвиг целых блоков);
// - ≥ 20 мелких регионов (каждый < 0.5 % площади) с объединением < 25 % →
//   `font` (рассыпанные глифы — типичный признак подмены шрифта);
// - иначе → `render`.
//
// Переопределение побеждает правило, `causeSource` становится `override`, а
// `causeEvidence` сохраняет и правило, и решение оператора: отчёт должен
// показывать, что именно было перекрыто.

import { CAPTURE_ERROR_CODES, resultTypeForCode, type CaptureErrorCode } from '../capture/errors.js';
import type { Cause, Rect, ResultType } from '../types.js';
import type { CompareOutcome } from './compare.js';

/** Состояние эталона для фикстуры. */
export type ReferenceState = 'present' | 'missing' | 'invalid';

/** Сбой захвата в том виде, в каком его отдаёт capture-слой. */
export interface CaptureFailure {
  code: string;
  message: string;
}

/** Структурное расхождение: число страниц DOCX или листов/страниц XLSX. */
export interface StructureMismatch {
  reference: number;
  actual: number;
  /** Что именно считалось: `pages`, `sheets`, `print-pages`. */
  unit: string;
}

/** Всё, что нужно классификатору. */
export interface ClassifyInput {
  fixtureId: string;
  captureId: string;
  reference: ReferenceState;
  /** Сбой захвата либо `null`, если захват удался. */
  error: CaptureFailure | null;
  /** Структурное расхождение либо `null`, если счётчики сошлись. */
  structure: StructureMismatch | null;
  /** Результат сравнения либо `null`, если сравнивать было нечего. */
  comparison: CompareOutcome | null;
  /** Переопределения причины: `fixtureId` или `fixtureId/captureId`. */
  overrides?: Readonly<Record<string, Cause>>;
}

/** Итог классификации — ровно те поля `CaptureResult`, которые она решает. */
export interface Classification {
  resultType: ResultType;
  cause: Cause;
  causeSource: 'rule' | 'override';
  causeEvidence: string;
}

/**
 * Классифицировать capture.
 *
 * Чистая функция: одинаковый вход даёт одинаковый выход, порядок правил
 * зафиксирован в шапке модуля.
 */
export function classify(input: ClassifyInput): Classification {
  const rule = applyRules(input);
  const override = findOverride(input);
  if (override === undefined) return rule;
  return {
    resultType: rule.resultType,
    cause: override,
    causeSource: 'override',
    causeEvidence: `${rule.causeEvidence}; override=${override}`,
  };
}

/** Жёсткие правила в порядке приоритета. */
function applyRules(input: ClassifyInput): Classification {
  if (input.reference !== 'present') {
    return {
      resultType: 'missing-reference',
      cause: 'reference',
      causeSource: 'rule',
      causeEvidence:
        input.reference === 'missing'
          ? 'no reference manifest for this fixture'
          : 'reference manifest failed validation',
    };
  }

  if (input.error !== null) {
    return {
      resultType: isCaptureErrorCode(input.error.code)
        ? resultTypeForCode(input.error.code)
        : 'capture-error',
      cause: 'harness',
      causeSource: 'rule',
      causeEvidence: `capture error ${input.error.code}: ${input.error.message}`,
    };
  }

  if (input.structure !== null) {
    return {
      resultType: 'page-mismatch',
      cause: 'layout',
      causeSource: 'rule',
      causeEvidence: `${input.structure.unit} ${input.structure.reference} vs ${input.structure.actual}`,
    };
  }

  const comparison = input.comparison;
  if (comparison === null) {
    // Захват удался, эталон есть, но сравнивать нечего: это дыра в контуре,
    // а не совпадение. Молчаливое `match` скрыло бы её от сертификации.
    return {
      resultType: 'harness-error',
      cause: 'harness',
      causeSource: 'rule',
      causeEvidence: 'capture produced no comparable image',
    };
  }

  if (comparison.kind === 'page-mismatch') {
    return {
      resultType: 'page-mismatch',
      cause: 'layout',
      causeSource: 'rule',
      causeEvidence: `dimensions ${comparison.reference.width}x${comparison.reference.height} vs ${comparison.actual.width}x${comparison.actual.height}`,
    };
  }

  const { metrics, regions, diffBounds, width, height } = comparison;
  if (metrics.diffPixels === 0) {
    return {
      resultType: 'match',
      cause: 'render',
      causeSource: 'rule',
      causeEvidence: `0 diff pixels outside masks; ssim=${metrics.ssim === null ? 'n/a' : metrics.ssim.toFixed(4)}`,
    };
  }

  const cause = provisionalCause(regions, diffBounds, width, height);
  return {
    resultType: 'visual-diff',
    cause,
    causeSource: 'rule',
    causeEvidence: `rule=${cause}; ${regions.length} region(s), ${(metrics.diffRatio * 100).toFixed(2)}% of pixels`,
  };
}

/**
 * Предварительная причина для `visual-diff`.
 *
 * Правила перечислены в шапке модуля; здесь только их реализация. Порядок
 * важен: сдвиг целых блоков (`layout`) проверяется раньше рассыпанных
 * глифов (`font`), иначе крупный сдвиг с множеством мелких регионов был бы
 * ошибочно назван шрифтовым.
 */
function provisionalCause(
  regions: readonly Rect[],
  diffBounds: Rect | null,
  width: number,
  height: number,
): Cause {
  const area = width * height;
  if (area === 0) return 'render';

  // Верхние 85 % кадра совпали, всё расхождение — в нижней полосе. Считается
  // по точному bounding box, а не по квантованным регионам: регион сетки
  // 8×8 мог бы задеть полосу краем и соврать в обе стороны.
  if (diffBounds !== null && diffBounds.y >= height * 0.85) return 'layout';

  let covered = 0;
  for (const rect of regions) covered += rect.width * rect.height;
  if (covered / area >= 0.6) return 'layout';

  const smallLimit = area * 0.005;
  const small = regions.filter((rect) => rect.width * rect.height < smallLimit);
  if (small.length >= 20) {
    let smallCovered = 0;
    for (const rect of small) smallCovered += rect.width * rect.height;
    if (smallCovered / area < 0.25) return 'font';
  }

  return 'render';
}

/** Сузить строку до известного кода сбоя: чужие коды не должны попасть в таблицу. */
function isCaptureErrorCode(code: string): code is CaptureErrorCode {
  return (CAPTURE_ERROR_CODES as readonly string[]).includes(code);
}

/** Найти переопределение: сначала по паре, затем по фикстуре. */
function findOverride(input: ClassifyInput): Cause | undefined {
  const overrides = input.overrides;
  if (overrides === undefined) return undefined;
  return overrides[`${input.fixtureId}/${input.captureId}`] ?? overrides[input.fixtureId];
}
