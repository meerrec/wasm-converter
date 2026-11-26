// Прогон оракула: corpus index → захват собственного вьюера → сравнение с
// эталоном → `report.json` и `report.html`.
//
// Модуль — единственное место, где сходятся все три слоя, и он же держит два
// правила, без которых прогон бесполезен:
//
// 1. Сбой одной фикстуры не прерывает прогон. Захват возвращает размеченный
//    результат, а не бросает исключение; исключение здесь означает поломку
//    контура, и оно всё равно не должно съедать уже снятые кадры.
// 2. Сессия захвата создаётся лениво. Корпус без единого эталона (типичное
//    состояние до приёмки baseline) обязан отработать за миллисекунды, не
//    поднимая ни dev-сервер, ни Chromium: иначе `run` на пустом корпусе стоил
//    бы дороже, чем `validate`, и в CI его пришлось бы выключать.
//
// Код выхода считается из отчёта чистой функцией `exitCodeFor` — ровно по
// таблице `docs/strict-viewer/baseline-policy.md` §6.

import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { isAbsolute, join, relative } from 'node:path';

import { engineRevision, type CapturedImage } from './capture/capture.js';
import type { CaptureErrorCode } from './capture/errors.js';
import { createCaptureSession, type CaptureSession } from './capture/index.js';
import { filterFixtures, loadCorpus, type CorpusFixture } from './corpus.js';
import { classify, type StructureMismatch } from './diff/classify.js';
import { compareImages } from './diff/compare.js';
import { decodePng, encodePng, type RgbaImage } from './diff/png.js';
import { validateManifestFile, type Manifest, type ManifestCapture } from './manifest.js';
import { REPO_ROOT, resolveFromRepo } from './paths.js';
import { renderHtml, type HtmlFixtureInfo } from './report/html.js';
import { buildReport, serializeReport } from './report/json.js';
import type {
  CaptureResult,
  Cause,
  ResultType,
  RunReport,
  RuntimeMetadata,
  ViewMode,
} from './types.js';

/** Причины расхождения, которые принимает `--override`. */
export const CAUSES: readonly Cause[] = [
  'parse',
  'model',
  'layout',
  'font',
  'render',
  'harness',
  'reference',
];

/** Параметры прогона. */
export interface RunOptions {
  /** Corpus index: список фикстур, режимов и пробелов матрицы. */
  corpusPath: string;
  /**
   * Корень эталонов. По умолчанию — `corpus.storage.rootUri` относительно
   * корня репозитория: эталоны живут там, где их объявил корпус.
   */
  referencesRoot?: string;
  /** Каталог отчёта и артефактов захвата. Создаётся при необходимости. */
  outDir: string;
  /** Фильтры `tag=<tag>` / `id=<glob>`; пусто — весь корпус. */
  filters?: readonly string[];
  /** Переопределения причины: `fixtureId` или `fixtureId/captureId`. */
  overrides?: Readonly<Record<string, Cause>>;
  /** Порт dev-сервера. По умолчанию — исторический порт просмотрщика. */
  port?: number;
  /** Запускать Chromium с окном — для отладки. */
  headless?: boolean;
  /** Часы: подменяются в тестах, чтобы отчёт был детерминирован. */
  now?: () => Date;
  /**
   * Готовая сессия захвата вместо настоящей — только для тестов.
   *
   * Позволяет проверить сборку строк отчёта без Chromium и dev-сервера.
   */
  captureSession?: CaptureSession;
}

/** Выборка пуста: фильтры не совпали ни с одной фикстурой. */
export class EmptySelectionError extends Error {
  readonly filters: readonly string[];

  constructor(filters: readonly string[]) {
    super(`no fixture matches the filters: ${filters.join(' ')}`);
    this.name = 'EmptySelectionError';
    this.filters = filters;
  }
}

/** Итог прогона. */
export interface RunSummary {
  report: RunReport;
  exitCode: 0 | 1 | 2 | 3;
  reportJsonPath: string;
  reportHtmlPath: string;
}

/**
 * Код выхода по отчёту (baseline-policy §6).
 *
 * Порядок проверок — от худшего к лучшему: сломанный контур (3) перекрывает
 * неполный захват (2), а тот — измеренное расхождение (1). Иначе прогон с
 * одним `harness-error` и одним `visual-diff` выглядел бы как обычная
 * регрессия рендера, хотя сертифицировать его нельзя вовсе.
 */
export function exitCodeFor(report: RunReport): 0 | 1 | 2 | 3 {
  const summary = report.summary;
  if (summary['harness-error'] > 0) return 3;
  if (summary['missing-reference'] > 0 || summary['capture-error'] > 0) return 2;
  if (summary['visual-diff'] > 0 || summary['page-mismatch'] > 0) return 1;
  return 0;
}

/**
 * Счётчики, которые черновой прогон (`baselineId: null`) обязан обнулить.
 *
 * `missing-reference` здесь нет намеренно: черновой корпус доснимается по
 * фикстуре, и отсутствие эталона — ожидаемое состояние, а не регрессия.
 */
const DRAFT_GATE_COUNTERS: readonly ResultType[] = [
  'capture-error',
  'harness-error',
  'visual-diff',
  'page-mismatch',
];

/**
 * Вердикт чернового прогона: `null` — CI может быть зелёным, иначе причина отказа.
 *
 * `exitCodeFor` кода 2 не хватает: он считает `missing-reference` раньше
 * визуальных расхождений, поэтому прогон «83 пары без эталона + 1 visual-diff»
 * отдаёт 2, и проверка одного лишь кода выхода пропустила бы регрессию рендера.
 * Здесь судьба решается по счётчикам, а не по их приоритету. Принятый baseline
 * (`baselineId` задан) судится кодом выхода целиком, без этой функции.
 *
 * @param report — report.json прогона
 * @param exitCode — код выхода того же прогона (`exitCodeFor`)
 * @returns текст причины отказа для `::error::` либо `null`, если прогон приемлем
 */
export function draftGateFailure(report: RunReport, exitCode: number): string | null {
  const summary: Record<string, unknown> = report.summary;
  const counters = DRAFT_GATE_COUNTERS.map((name) => {
    const value = summary[name];
    return `${name}=${typeof value === 'number' ? value : `?(${String(value)})`}`;
  }).join(' ');
  const missing = summary['missing-reference'];
  const detail = `${counters} missing-reference=${String(missing)} exitCode=${exitCode}`;

  if (exitCode !== 0 && exitCode !== 2) {
    // Принятого baseline здесь быть не может: код 1 при черновом корпусе значит,
    // что запись `baselineId` разошлась с тем, что видит прогон. Код 3 —
    // сломанный контур. Оба не сертифицируют движок, какой бы ни была выборка.
    return `draft run exited with code ${exitCode}, expected 0 or 2: ${detail}`;
  }
  const failed: string[] = [];
  for (const name of DRAFT_GATE_COUNTERS) {
    const value = summary[name];
    if (typeof value !== 'number') {
      // Отчёт без счётчика — не «ноль», а нечем судить: молча пропустить
      // значит принять непроверенный прогон.
      failed.push(`${name} is not a number (${String(value)})`);
    } else if (value !== 0) {
      failed.push(`${name}=${value}`);
    }
  }
  if (failed.length > 0) {
    return `draft run failed beyond missing-reference: ${failed.join(', ')}; ${detail}`;
  }
  return null;
}

/**
 * Разобрать `--override <fixtureId|fixtureId/captureId>=<cause>`.
 *
 * @throws {Error} если запись не содержит `=` или причина неизвестна:
 * опечатка в причине иначе молча стала бы «правилом» отчёта.
 */
export function parseOverrides(entries: readonly string[]): Record<string, Cause> {
  const overrides: Record<string, Cause> = {};
  for (const entry of entries) {
    const separator = entry.indexOf('=');
    if (separator <= 0) {
      throw new Error(
        `invalid override "${entry}": expected <fixtureId|fixtureId/captureId>=<cause>`,
      );
    }
    const key = entry.slice(0, separator);
    const cause = entry.slice(separator + 1);
    if (!(CAUSES as readonly string[]).includes(cause)) {
      throw new Error(
        `invalid override "${entry}": unknown cause "${cause}"; expected one of ${CAUSES.join(', ')}`,
      );
    }
    overrides[key] = cause as Cause;
  }
  return overrides;
}

/** Состояние прогона, общее для всех фикстур. */
interface RunContext {
  referencesRoot: string;
  fixturesRoot: string;
  outDir: string;
  overrides: Readonly<Record<string, Cause>>;
  /** Ленивая сессия: `null` — контур уже сломан, повторять бессмысленно. */
  session: () => Promise<CaptureSession | null>;
  /** Причина, по которой сессия не поднялась, либо `null`. */
  sessionFailure: () => { code: CaptureErrorCode; message: string } | null;
  /**
   * Подмена сессии захвата — только для тестов.
   *
   * Прогон с настоящим Chromium проверяется сквозным сценарием; здесь нужен
   * способ проверить, что доказательство стабильности берётся у самого
   * артефакта, а не у фикстуры целиком.
   */
  captureSession?: CaptureSession;
}

/** Итог одного режима одной фикстуры. */
interface ModeResult {
  results: CaptureResult[];
  /** Путь manifest, если он существует, — для ссылки в HTML-отчёте. */
  manifestPath: string | null;
}

/**
 * Прогнать корпус.
 *
 * @throws {CorpusError} если corpus index не читается или не проходит
 * валидацию: без корпуса отчёт описывал бы неизвестно что, поэтому прогон
 * падает до создания каталога вывода, а CLI печатает ошибки и выходит с 3.
 */
export async function runOracle(options: RunOptions): Promise<RunSummary> {
  const corpusPath = resolveFromRepo(options.corpusPath);
  const corpus = await loadCorpus(corpusPath);
  const referencesRoot = resolveFromRepo(
    options.referencesRoot ?? corpus.corpus.storage.rootUri,
  );
  const fixturesRoot = resolveFromRepo('test-fixtures');
  const outDir = resolveFromRepo(options.outDir);

  const fixtures =
    options.filters !== undefined && options.filters.length > 0
      ? filterFixtures(corpus.fixtures, options.filters)
      : corpus.fixtures;
  if (fixtures.length === 0) {
    // Пустая выборка дала бы отчёт «total 0» и код 0: опечатка в фильтре
    // выглядела бы как зелёная сертификация.
    throw new EmptySelectionError(options.filters ?? []);
  }
  await mkdir(outDir, { recursive: true });

  let session: CaptureSession | null = null;
  let failure: { code: CaptureErrorCode; message: string } | null = null;

  const ensureSession = async (): Promise<CaptureSession | null> => {
    if (session !== null) return session;
    if (failure !== null) return null;
    if (ctx.captureSession !== undefined) {
      session = ctx.captureSession;
      return session;
    }
    try {
      session = await createCaptureSession({
        repoRoot: REPO_ROOT,
        outDir,
        ...(options.port === undefined ? {} : { port: options.port }),
        ...(options.headless === undefined ? {} : { headless: options.headless }),
      });
      return session;
    } catch (error) {
      // Сервер или браузер не поднялись: это сломанный контур, и каждая
      // оставшаяся фикстура получит `harness-error`, а не «нет эталона».
      failure = { code: 'browser-failed', message: errorText(error) };
      return null;
    }
  };

  const ctx: RunContext = {
    referencesRoot,
    fixturesRoot,
    outDir,
    overrides: options.overrides ?? {},
    session: ensureSession,
    sessionFailure: () => failure,
    ...(options.captureSession === undefined ? {} : { captureSession: options.captureSession }),
  };

  const results: CaptureResult[] = [];
  const fixtureInfo: HtmlFixtureInfo[] = [];

  try {
    for (const fixture of fixtures) {
      let manifestPath: string | null = null;
      for (const mode of fixture.modes) {
        const modeResult = await runMode(ctx, fixture, mode);
        results.push(...modeResult.results);
        manifestPath ??= modeResult.manifestPath;
      }
      fixtureInfo.push({
        fixtureId: fixture.id,
        title: fixture.title,
        sourcePath: fixture.source.path,
        manifestPath:
          manifestPath === null ? null : artifactUri(outDir, manifestPath),
      });
    }
  } finally {
    // Сессия закрывается всегда: брошенный Chromium держал бы порт и не дал
    // бы следующему прогону поднять сервер.
    if (session !== null) await session.close();
  }

  const report = buildReport({
    corpus,
    captures: results,
    engineRevision: await engineRevisionOrUnknown(),
    baselineId: corpus.corpus.baselineId,
    ...(options.now === undefined ? {} : { now: options.now }),
  });

  const reportJsonPath = join(outDir, 'report.json');
  const reportHtmlPath = join(outDir, 'report.html');
  await writeFile(reportJsonPath, serializeReport(report), 'utf8');
  await writeFile(
    reportHtmlPath,
    renderHtml(report, { artifactsDir: '.', fixtures: fixtureInfo }),
    'utf8',
  );

  return { report, exitCode: exitCodeFor(report), reportJsonPath, reportHtmlPath };
}

/** Один режим одной фикстуры: эталон → захват → сравнение. */
async function runMode(
  ctx: RunContext,
  fixture: CorpusFixture,
  mode: ViewMode,
): Promise<ModeResult> {
  const manifestPath = join(ctx.referencesRoot, fixture.format, fixture.id, mode, 'manifest.json');

  if (!(await fileExists(manifestPath))) {
    return {
      results: [referenceRow(fixture, mode, `no reference manifest at ${manifestPath}`)],
      manifestPath: null,
    };
  }

  const validation = await validateManifestFile(manifestPath, {
    fixturesRoot: ctx.fixturesRoot,
    storageRoot: ctx.referencesRoot,
  });
  if (!validation.ok) {
    // Manifest есть, но не проходит схему или семантику: это повреждённый
    // артефакт контура (baseline-policy §6, код 3), а не отсутствие эталона.
    // Смешать их значило бы выдать сломанный baseline за «ещё не снятый».
    return {
      results: [
        harnessRow(
          fixture,
          mode,
          'manifest-invalid',
          `reference manifest is invalid: ${summarizeErrors(validation.errors)}`,
        ),
      ],
      manifestPath,
    };
  }

  const manifest = validation.manifest;
  const session = await ctx.session();
  if (session === null) {
    const failure = ctx.sessionFailure();
    return {
      results: [
        failureRow(fixture, mode, failure?.code ?? 'browser-failed', failure?.message ?? 'capture session is unavailable'),
      ],
      manifestPath,
    };
  }

  let outcome: Awaited<ReturnType<CaptureSession['capture']>>;
  try {
    outcome = await session.capture(fixture.id, manifest);
  } catch (error) {
    // Захват обещает не бросать, но запись артефакта на диск может упасть
    // мимо его границы. Одна такая фикстура не должна съедать остальные.
    return {
      results: [harnessRow(fixture, mode, 'capture-crashed', errorText(error))],
      manifestPath,
    };
  }
  if (!outcome.ok) {
    return {
      results: [failureRow(fixture, mode, outcome.code, outcome.message)],
      manifestPath,
    };
  }

  const structure = structureMismatch(manifest, outcome.structure);
  const results: CaptureResult[] = [];
  for (const capture of manifest.captures) {
    results.push(
      await compareCapture(ctx, fixture, mode, capture, outcome.captures, outcome.runtime, structure),
    );
  }
  return { results, manifestPath };
}

/** Сравнить один capture manifest с соответствующим кадром захвата. */
async function compareCapture(
  ctx: RunContext,
  fixture: CorpusFixture,
  mode: ViewMode,
  capture: ManifestCapture,
  captured: readonly CapturedImage[],
  runtime: RuntimeMetadata,
  structure: StructureMismatch | null,
): Promise<CaptureResult> {
  const actual = captured.find((image) => image.captureId === capture.id);
  if (actual === undefined) {
    return failureRow(
      fixture,
      mode,
      'export-failed',
      `capture "${capture.id}" produced no image`,
      capture.id,
    );
  }

  const referencePath = join(ctx.referencesRoot, capture.image.uri);
  let referenceImage: RgbaImage;
  try {
    referenceImage = decodePng(new Uint8Array(await readFile(referencePath)));
  } catch (error) {
    // Эталон прошёл schema/semantic validation, но не декодируется: это
    // повреждённый артефакт, а не дефект движка (baseline-policy §6, код 3).
    return harnessRow(
      fixture,
      mode,
      'reference-decode',
      `reference artifact is unreadable: ${errorText(error)}`,
      capture.id,
    );
  }

  let actualImage: RgbaImage;
  try {
    actualImage = decodePng(new Uint8Array(await readFile(actual.path)));
  } catch (error) {
    return harnessRow(
      fixture,
      mode,
      'actual-decode',
      `captured artifact is unreadable: ${errorText(error)}`,
      capture.id,
    );
  }

  // Структурная проверка идёт до пикселей: при разном числе страниц
  // сравнивать нечего, и «высокий SSIM» не должен это маскировать.
  const comparison = structure === null
    ? compareImages(referenceImage, actualImage, (capture.masks ?? []).map((mask) => mask.rect))
    : null;

  let diffImage: string | null = null;
  if (comparison !== null && comparison.kind === 'compared') {
    const diffPath = join(ctx.outDir, fixture.id, mode, 'diff', `${capture.id}.png`);
    await mkdir(join(ctx.outDir, fixture.id, mode, 'diff'), { recursive: true });
    await writeFile(diffPath, encodePng(comparison.overlay));
    diffImage = artifactUri(ctx.outDir, diffPath);
  }

  const classification = classify({
    fixtureId: fixture.id,
    captureId: capture.id,
    reference: 'present',
    error: null,
    structure,
    comparison,
    overrides: ctx.overrides,
  });

  return {
    fixtureId: fixture.id,
    format: fixture.format,
    mode,
    captureId: capture.id,
    resultType: classification.resultType,
    cause: classification.cause,
    causeSource: classification.causeSource,
    causeEvidence: classification.causeEvidence,
    reference: {
      uri: artifactUri(ctx.outDir, referencePath),
      sha256: capture.image.sha256,
      width: capture.image.width,
      height: capture.image.height,
    },
    actual: {
      uri: artifactUri(ctx.outDir, actual.path),
      sha256: actual.sha256,
      width: actual.width,
      height: actual.height,
    },
    diffImage,
    metrics: comparison !== null && comparison.kind === 'compared' ? comparison.metrics : null,
    regions: comparison !== null && comparison.kind === 'compared' ? comparison.regions : [],
    // Доказательство стабильности — от самого артефакта: `runtime` фикстуры
    // хранит значения последнего capture, и на строке первой страницы они
    // выглядели бы как хеши второй.
    runtime: {
      ...runtime,
      stableHashes: actual.stableHashes,
      timeToStableMs: actual.timeToStableMs,
    },
    error: null,
  };
}

/**
 * Структурное расхождение: число страниц эталона против числа страниц движка.
 *
 * `null` означает «сверять нечего»: у XLSX Normal view нет постраничной
 * структуры, а совпавшие счётчики не создают расхождения.
 */
function structureMismatch(
  manifest: Manifest,
  structure: { unit: 'pages' | 'print-pages'; count: number } | null,
): StructureMismatch | null {
  if (structure === null) return null;
  const reference = manifest.captures.length;
  if (reference === structure.count) return null;
  return { reference, actual: structure.count, unit: structure.unit };
}

/** Строка отчёта для отсутствующего или невалидного эталона. */
function referenceRow(fixture: CorpusFixture, mode: ViewMode, message: string): CaptureResult {
  return {
    ...baseResult(fixture, mode, mode),
    resultType: 'missing-reference',
    cause: 'reference',
    causeEvidence: message,
    error: { code: 'missing-reference', message },
  };
}

/**
 * Строка отчёта для сломанного контура: повреждённый эталон, нечитаемый
 * артефакт, невалидный manifest. Это код 3, а не «неудачный захват».
 */
function harnessRow(
  fixture: CorpusFixture,
  mode: ViewMode,
  code: string,
  message: string,
  captureId: string = mode,
): CaptureResult {
  return {
    ...baseResult(fixture, mode, captureId),
    resultType: 'harness-error',
    cause: 'harness',
    causeEvidence: message,
    error: { code, message },
  };
}

/** Строка отчёта для сбоя захвата: тип результата берётся из кода. */
function failureRow(
  fixture: CorpusFixture,
  mode: ViewMode,
  code: string,
  message: string,
  captureId: string = mode,
): CaptureResult {
  const classification = classify({
    fixtureId: fixture.id,
    captureId,
    reference: 'present',
    error: { code, message },
    structure: null,
    comparison: null,
  });
  return {
    ...baseResult(fixture, mode, captureId),
    resultType: classification.resultType,
    cause: classification.cause,
    causeEvidence: classification.causeEvidence,
    error: { code, message },
  };
}

/** Поля, общие у всех строк отчёта. */
function baseResult(
  fixture: CorpusFixture,
  mode: ViewMode,
  captureId: string,
): CaptureResult {
  return {
    fixtureId: fixture.id,
    format: fixture.format,
    mode,
    captureId,
    resultType: 'harness-error',
    cause: 'harness',
    causeSource: 'rule',
    causeEvidence: '',
    reference: null,
    actual: null,
    diffImage: null,
    metrics: null,
    regions: [],
    runtime: null,
    error: null,
  };
}

/**
 * Путь артефакта относительно каталога отчёта.
 *
 * HTML-отчёт лежит в `<outDir>`, поэтому ссылки считаются от него: эталон
 * может жить и вне `outDir`, и жёстко заданный префикс соврал бы. Если
 * относительного пути не существует (разные диски на Windows), остаётся
 * абсолютный — `renderHtml` сведёт его к имени файла.
 */
function artifactUri(outDir: string, absolutePath: string): string {
  const rel = relative(outDir, absolutePath);
  if (rel.length === 0) return absolutePath;
  return isAbsolute(rel) ? absolutePath : rel;
}

/** Свернуть ошибки валидации в одно сообщение: первые пять плюс счётчик. */
function summarizeErrors(errors: readonly { path: string; message: string }[]): string {
  const shown = errors.slice(0, 5).map((entry) =>
    entry.path.length === 0 ? entry.message : `${entry.path}: ${entry.message}`,
  );
  const rest = errors.length - shown.length;
  return rest > 0 ? `${shown.join('; ')} (+${rest} more)` : shown.join('; ');
}

/** Ревизия движка для отчёта; без git — честное `unknown`. */
async function engineRevisionOrUnknown(): Promise<string> {
  try {
    return await engineRevision(REPO_ROOT);
  } catch {
    return 'unknown';
  }
}

/** Существует ли файл. */
async function fileExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
