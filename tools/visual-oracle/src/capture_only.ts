// Подкоманда `capture`: снять PNG одного документа нашим вьюером.
//
// Это инструмент «посмотреть, что рисует движок», а не эталон: manifest здесь
// синтетический, живёт только в памяти и никогда не пишется на диск, сравнения
// нет, а артефакты идут строго в `--out`. Политика baseline
// (docs/strict-viewer/baseline-policy.md §3 п.5 и §4) запрещает автоматически
// создавать reference PNG, поэтому каталог `test-fixtures/strict-viewer` закрыт
// на запись даже по ошибке оператора.
//
// Второго пути захвата здесь нет: кадры снимает та же `createCaptureSession`,
// что и `run`, — иначе «посмотреть» и «сертифицировать» показывали бы разные
// картинки.

import { readFile, stat } from 'node:fs/promises';
import { basename, extname, isAbsolute, relative, sep } from 'node:path';

import { captureOutDir, createCaptureSession } from './capture/index.js';
import type { CaptureOutcome, CaptureSession } from './capture/index.js';
import { sha256Hex } from './hash.js';
import { DEFAULT_FIXTURES_ROOT, MANIFEST_SCHEMA_VERSION } from './manifest.js';
import type {
  Manifest,
  ManifestCapture,
  ManifestDocxView,
  ManifestEnvironment,
  ManifestImage,
  ManifestXlsxView,
} from './manifest.js';
import { REPO_ROOT, resolveFromRepo } from './paths.js';
import type { DocFormat } from './types.js';
import { readZipEntry } from './zip.js';

/** Код выхода: PNG записаны. */
const EXIT_OK = 0;
/** Код выхода: захват упал (браузер, таймаут, ошибка вьюера). */
const EXIT_CAPTURE = 3;

/** Корень эталонов: подкоманда не пишет сюда никогда (baseline-policy §3 п.5, §4). */
const STRICT_VIEWER_ROOT = 'test-fixtures/strict-viewer';

/** Режим, который умеет снимать эта подкоманда: только экранные, без PDF. */
export type CaptureOnlyMode = 'print-layout' | 'normal';

/** Ошибка аргументов подкоманды: CLI печатает её со справкой и кодом 2. */
export class CaptureOnlyUsageError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CaptureOnlyUsageError';
  }
}

/**
 * Slug фикстуры из пути документа: он же имя каталога вывода.
 *
 * Имя файла — единственное, что оператор уже назвал осмысленно, поэтому
 * каталог вывода повторяет его. Не-буквы и не-цифры схлопываются в `-`, чтобы
 * пробелы и скобки не превращались в разные каталоги на разных ФС.
 */
export function docFixtureSlug(filePath: string): string {
  const stem = basename(filePath, extname(filePath));
  const slug = stem
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, '-')
    .replace(/^-+|-+$/g, '');
  // Пустой slug дал бы каталог `--out//<mode>`: лучше явное имя, чем путь,
  // который зависит от того, как ФС схлопывает слэши.
  return slug.length > 0 ? slug : 'document';
}

/**
 * Формат документа по расширению.
 *
 * @throws {CaptureOnlyUsageError} расширение не `.docx` и не `.xlsx`.
 */
export function detectDocFormat(filePath: string): DocFormat {
  const extension = extname(filePath).toLowerCase();
  if (extension === '.docx') return 'docx';
  if (extension === '.xlsx') return 'xlsx';
  throw new CaptureOnlyUsageError(
    `unsupported document extension "${extension === '' ? '(none)' : extension}": ` +
      'capture accepts .docx and .xlsx',
  );
}

/**
 * Режим просмотра для формата: умолчание и отказ на неподдерживаемом.
 *
 * `pdf` и `print-preview` снимаются через экспорт PDF и растеризацию — это
 * другой бэкенд и другая матрица, поэтому здесь они запрещены, а не подменены
 * экранным кадром.
 *
 * @throws {CaptureOnlyUsageError} режим не относится к формату или требует PDF.
 */
export function resolveCaptureMode(format: DocFormat, mode: string | null): CaptureOnlyMode {
  if (mode === null) return format === 'docx' ? 'print-layout' : 'normal';
  if (mode === 'pdf' || mode === 'print-preview') {
    throw new CaptureOnlyUsageError(
      `mode "${mode}" is not supported by capture: it needs PDF export and rasterization; ` +
        'use "visual-oracle run" with a reference manifest instead',
    );
  }
  if (format === 'docx' && mode === 'print-layout') return 'print-layout';
  if (format === 'xlsx' && mode === 'normal') return 'normal';
  throw new CaptureOnlyUsageError(
    `mode "${mode}" does not apply to ${format}: capture supports ` +
      `"${format === 'docx' ? 'print-layout' : 'normal'}"`,
  );
}

/**
 * Запретить запись в корень эталонов.
 *
 * Проверка до запуска браузера: упасть после захвата, уже записав PNG в
 * `test-fixtures/strict-viewer`, значило бы нарушить политику baseline.
 *
 * @throws {CaptureOnlyUsageError} `--out` лежит внутри корня эталонов.
 */
export function assertOutDirAllowed(outDir: string): void {
  const target = resolveFromRepo(outDir);
  const forbidden = resolveFromRepo(STRICT_VIEWER_ROOT);
  const inside = relative(forbidden, target);
  if (inside === '' || (!inside.startsWith('..') && !isAbsolute(inside))) {
    throw new CaptureOnlyUsageError(
      `refusing to write into ${STRICT_VIEWER_ROOT}: capture never creates reference artifacts`,
    );
  }
}

/** Вход синтетического manifest: всё, что нельзя узнать из самого документа. */
export interface SyntheticManifestInput {
  /** Путь документа от корня `test-fixtures`, через `/` — так его читает вьюер. */
  docPath: string;
  format: DocFormat;
  mode: CaptureOnlyMode;
  /** Число страниц DOCX; для XLSX не нужно. */
  pageCount?: number;
  /** Имена листов XLSX в порядке книги; для DOCX не нужно. */
  sheetNames?: readonly string[];
  /** Каталог `--out`: он же `corpus.storage.rootUri`. */
  outDir: string;
  sourceSha256: string;
  sourceBytes: number;
  operator: string;
  /** Момент захвата в UTC, ISO 8601. */
  capturedAt: string;
}

/**
 * Собрать минимально валидный по схеме manifest для одного документа.
 *
 * Manifest не валидируется и не пишется: он нужен только как вход
 * `captureFixture`, который читает из него путь, режим, зум и список кадров.
 * Поля эталона (`reference`, `environment`) заполнены формой из импорта
 * операторского drop, чтобы структура оставалась той же, что у настоящего
 * manifest, — но их значения здесь инертны.
 *
 * @throws {CaptureOnlyUsageError} нет страниц, нет листов или режим не от формата.
 */
export function buildSyntheticManifest(input: SyntheticManifestInput): Manifest {
  const captures =
    input.format === 'docx' ? docxCaptures(input.pageCount) : xlsxCaptures(input.sheetNames);
  const view =
    input.format === 'docx'
      ? docxView(input.mode)
      : xlsxView(input.mode, requireSheetNames(input.sheetNames));

  return {
    schemaVersion: MANIFEST_SCHEMA_VERSION,
    corpus: {
      // Корпус синтетический: id и версии нужны схеме, а не читателю.
      id: 'capture-only',
      version: '0.0.0',
      matrixVersion: 'capture-only',
      baselineId: 'capture-only',
      // Артефакты лежат в локальном каталоге, а не в LFS и не в релизе.
      storage: { kind: 'object-store', rootUri: input.outDir },
    },
    source: {
      path: input.docPath,
      format: input.format,
      sha256: input.sourceSha256,
      bytes: input.sourceBytes,
    },
    reference: {
      // Кадр снимает не Office, а наш вьюер в Chromium (`captureFixture`):
      // ни `office-online`, ни `office-desktop` тут не истинны. Схема
      // различает только два провайдера, поэтому значения — заглушка формы:
      // manifest не пишется и не валидируется (см. шапку модуля), а захват
      // читает из него лишь источник, вид и список кадров.
      provider: 'office-desktop',
      application: input.format === 'docx' ? 'word-desktop' : 'excel-desktop',
      capturedAt: input.capturedAt,
      captureMethod: 'automated',
      operator: input.operator,
    },
    environment: captureEnvironment(),
    view,
    dynamicData: {
      dates: 'fixed-value',
      fields: 'cached-result',
      formulaCache: {
        policy: 'cached-values-only',
        dirty: false,
        fullCalcOnLoad: false,
        forceFullCalc: false,
      },
      externalLinks: 'cached-values-no-network',
    },
    captures,
  };
}

/** Параметры подкоманды после разбора аргументов. */
export interface CaptureOnlyOptions {
  doc: string;
  out: string;
  /** `null` — взять умолчание формата. */
  mode: string | null;
  port: number | null;
  headed: boolean;
}

/**
 * Снять один документ и записать PNG в `<out>/<fixture>/<mode>/`.
 *
 * @returns 0 — PNG записаны, 3 — захват упал.
 * @throws {CaptureOnlyUsageError} аргументы, путь документа или режим не годятся.
 */
export async function runCaptureOnly(options: CaptureOnlyOptions): Promise<number> {
  const docPath = resolveFromRepo(options.doc);
  const info = await stat(docPath).catch(() => null);
  if (info === null || !info.isFile()) {
    throw new CaptureOnlyUsageError(`document not found: ${options.doc}`);
  }
  const format = detectDocFormat(docPath);
  const mode = resolveCaptureMode(format, options.mode);
  assertOutDirAllowed(options.out);

  const outDir = resolveFromRepo(options.out);
  const docPathInFixtures = fixtureRelativePath(docPath, format);
  const bytes = await readFile(docPath);
  const fixtureId = docFixtureSlug(docPath);
  const base: Omit<SyntheticManifestInput, 'pageCount' | 'sheetNames'> = {
    docPath: docPathInFixtures,
    format,
    mode,
    outDir,
    sourceSha256: sha256Hex(bytes),
    sourceBytes: bytes.byteLength,
    operator: process.env.USER ?? process.env.USERNAME ?? 'visual-oracle',
    capturedAt: new Date().toISOString(),
  };

  let session: CaptureSession | null = null;
  try {
    session = await createCaptureSession({
      repoRoot: REPO_ROOT,
      outDir,
      ...(options.port === null ? {} : { port: options.port }),
      ...(options.headed ? { headless: false } : {}),
    });

    let manifest = buildSyntheticManifest(
      format === 'docx'
        ? { ...base, pageCount: 1 }
        : { ...base, sheetNames: readSheetNames(bytes) },
    );
    let outcome = await session.capture(fixtureId, manifest);
    if (!outcome.ok) return reportCaptureFailure(outcome);

    // DOCX: число страниц знает только вьюер, и узнаётся оно после первого
    // кадра. Второй проход пересобирает manifest целиком, чтобы каталог вывода
    // был согласован, а не «страница 1 из первого прогона, остальные из
    // второго».
    if (format === 'docx') {
      const pageCount = outcome.structure?.count ?? 1;
      if (pageCount > 1) {
        manifest = buildSyntheticManifest({ ...base, pageCount });
        outcome = await session.capture(fixtureId, manifest);
        if (!outcome.ok) return reportCaptureFailure(outcome);
      }
    }

    for (const capture of outcome.captures) process.stdout.write(`${capture.path}\n`);
    process.stdout.write(
      `visual-oracle: ${outcome.captures.length} capture(s) in ` +
        `${captureOutDir(outDir, fixtureId, manifest)}\n`,
    );
    return EXIT_OK;
  } catch (error) {
    // Ошибка аргументов остаётся ошибкой аргументов: CLI покажет справку.
    if (error instanceof CaptureOnlyUsageError) throw error;
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`error: ${message}\n`);
    return EXIT_CAPTURE;
  } finally {
    await session?.close();
  }
}

/** Кадры DOCX: по одному на страницу, номера непрерывны от 1. */
function docxCaptures(pageCount: number | undefined): ManifestCapture[] {
  if (pageCount === undefined || !Number.isInteger(pageCount) || pageCount < 1) {
    throw new CaptureOnlyUsageError('docx capture needs a positive page count');
  }
  const captures: ManifestCapture[] = [];
  for (let page = 1; page <= pageCount; page += 1) {
    const id = `page-${page}`;
    captures.push({ id, kind: 'docx-page', pageNumber: page, image: placeholderImage(id) });
  }
  return captures;
}

/** Кадры XLSX: по одному экрану на лист, в порядке книги. */
function xlsxCaptures(sheetNames: readonly string[] | undefined): ManifestCapture[] {
  return requireSheetNames(sheetNames).map(
    (name, index): ManifestCapture => ({
      id: `sheet-${index}`,
      kind: 'xlsx-screen',
      sheet: { index, name },
      // Экран фиксирован: подкоманда показывает документ, а не подбирает
      // «интересный» диапазон, и повторный прогон обязан дать тот же кадр.
      visibleRange: 'A1:H20',
      scroll: { x: 0, y: 0 },
      activeCell: 'A1',
      image: placeholderImage(`sheet-${index}`),
    }),
  );
}

/** Имена листов обязательны: без них не построить ни `view`, ни кадры. */
function requireSheetNames(sheetNames: readonly string[] | undefined): readonly string[] {
  if (sheetNames === undefined || sheetNames.length === 0) {
    throw new CaptureOnlyUsageError('xlsx capture needs at least one sheet');
  }
  return sheetNames;
}

/** Ветка `view` для DOCX: режим уже проверен, но литерал нужен типу. */
function docxView(mode: CaptureOnlyMode): ManifestDocxView {
  if (mode !== 'print-layout') {
    throw new CaptureOnlyUsageError(
      `mode "${mode}" does not apply to docx: capture supports "print-layout"`,
    );
  }
  return { format: 'docx', mode, revisions: 'accepted', comments: 'hidden' };
}

/** Ветка `view` для XLSX: активный лист — первый, скрытые листы не исключаем. */
function xlsxView(mode: CaptureOnlyMode, sheetNames: readonly string[]): ManifestXlsxView {
  const firstSheet = sheetNames[0];
  if (firstSheet === undefined) throw new CaptureOnlyUsageError('xlsx capture needs at least one sheet');
  if (mode !== 'normal') {
    throw new CaptureOnlyUsageError(
      `mode "${mode}" does not apply to xlsx: capture supports "normal"`,
    );
  }
  return {
    format: 'xlsx',
    mode,
    activeSheet: { index: 0, name: firstSheet },
    hiddenSheets: 'exclude',
    showGridLines: true,
    showHeadings: true,
  };
}

/**
 * Заглушка изображения в capture.
 *
 * `captureFixture` не читает `capture.image`: настоящий PNG и его хеш считает
 * `writePng` из снятых байтов. Поле обязано быть по схеме, поэтому здесь
 * честные единицы, а не выдуманные размеры.
 */
function placeholderImage(id: string): ManifestImage {
  return {
    uri: `${id}.png`,
    sha256: '0'.repeat(64),
    width: 1,
    height: 1,
    bytes: 1,
    colorType: 'RGBA8',
  };
}

/**
 * Среда захвата: форма скопирована с импорта операторского drop.
 *
 * Значения инертны для кадра, кроме зума, локали и таймзоны: их читает
 * `captureFixture`, когда создаёт контекст браузера.
 */
function captureEnvironment(): ManifestEnvironment {
  return {
    os: { name: 'Windows 11', version: '10.0.22631' },
    browser: { name: 'chromium', version: '131.0.6778.86' },
    viewportCss: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    applicationZoomPercent: 100,
    browserZoomPercent: 100,
    locale: 'ru-RU',
    timezone: 'UTC',
    colorScheme: 'light',
    fonts: {
      package: 'strict-viewer-fonts',
      version: '1.0.0',
      assets: [{ family: 'Carlito', style: 'normal', weight: 400, sha256: 'a'.repeat(64) }],
    },
  };
}

/**
 * Путь документа от корня `test-fixtures`, через `/`.
 *
 * Вьюер отдаёт только это дерево (`/fixtures/<format>/…`), а `captureFixture`
 * отбрасывает первый сегмент `source.path` — значит, путь обязан начинаться с
 * формата, иначе документ просто не найдётся в списке страницы.
 *
 * @throws {CaptureOnlyUsageError} документ вне `test-fixtures/<format>/`.
 */
function fixtureRelativePath(docPath: string, format: DocFormat): string {
  const root = resolveFromRepo(DEFAULT_FIXTURES_ROOT);
  const rel = relative(root, docPath).split(sep).join('/');
  if (rel === '' || rel === '..' || rel.startsWith('../') || isAbsolute(rel)) {
    throw new CaptureOnlyUsageError(
      `document must live under ${DEFAULT_FIXTURES_ROOT}/: the demo viewer serves only that tree`,
    );
  }
  if (!rel.startsWith(`${format}/`)) {
    throw new CaptureOnlyUsageError(
      `document must live under ${DEFAULT_FIXTURES_ROOT}/${format}/: ` +
        `the ${format} viewer lists only that directory`,
    );
  }
  return rel;
}

/**
 * Имена листов из `xl/workbook.xml`.
 *
 * Свой мини-ридер, а не `manifest.ts`: там он приватный и заточен под
 * валидацию эталона, а синтетическому manifest имена нужны до первого кадра —
 * вьюер отдаёт только число листов, не имена.
 */
function readSheetNames(bytes: Uint8Array): string[] {
  const xml = readZipEntry(bytes, 'xl/workbook.xml');
  if (xml === null) return [];
  const text = new TextDecoder('utf-8').decode(xml);
  const names: string[] = [];
  const pattern = /<sheet\b([^>]*)\/?>/g;
  for (let match = pattern.exec(text); match !== null; match = pattern.exec(text)) {
    const name = /(?:^|\s)name\s*=\s*"([^"]*)"/.exec(match[1] ?? '')?.[1];
    if (name !== undefined) names.push(decodeXmlEntities(name));
  }
  return names;
}

/** Раскрыть пять предопределённых XML-сущностей: имена листов приходят из XML. */
function decodeXmlEntities(value: string): string {
  return value
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, '&');
}

/** Сбой захвата: код шага уже в сообщении, оператору нужен только он. */
function reportCaptureFailure(outcome: Extract<CaptureOutcome, { ok: false }>): number {
  process.stderr.write(`error: capture failed (${outcome.code}): ${outcome.message}\n`);
  return EXIT_CAPTURE;
}
