// Импорт операторского захвата эталона: каталог с PNG (и PDF для режима
// `pdf`) плюс `capture.json` превращается в schema-valid `manifest.json`.
//
// ## Раскладка входа
//
// ```text
// <from>/                      # последние три сегмента: <format>/<fixture-id>/<mode>
//   capture.json               # метаданные оператора
//   reference/page-1.png       # артефакты; пути в capture.json — от <from>
//   reference/export.pdf       # только для mode = "pdf"
// ```
//
// Пути артефактов в `capture.json` относительны `<from>` и обязаны начинаться
// с `reference/`: manifest ссылается на `reference/<name>`, и путь вне этой
// папки указывал бы в пустоту.
//
// Выход — `test-fixtures/strict-viewer/<format>/<fixture-id>/<mode>/manifest.json`;
// `image.uri` и `reference.exportedPdf.uri` записываются относительно
// `corpus.storage.rootUri`, поэтому manifest переносим.
//
// ## Что считается, а что берётся у оператора
//
// Импортёр считает сам: `source.sha256`/`source.bytes`, `image.sha256`,
// `image.bytes`, `image.width`/`image.height`/`image.colorType`,
// `reference.exportedPdf.sha256` и `corpus.baselineId`. Всё остальное —
// среда, режим, id captures, маски — приходит из `capture.json`. Отсутствующее
// поле — ошибка, а не значение по умолчанию: выдуманная среда сделала бы
// baseline невоспроизводимым.
//
// ## Алгоритм `baselineId`
//
// ```text
// strict-viewer-<corpusVersion>-<matrixVersion>-<environmentId>-<shortDigest>
// ```
//
// - `environmentId` — первые 8 hex SHA-256 от канонического JSON объекта
//   `environment` (ключи отсортированы, пробелов нет).
// - `shortDigest` — первые 12 hex SHA-256 от канонического JSON объекта
//   `{ captures, environment }`, где `captures` — список
//   `{ sourcePath, sourceSha256, mode, captureId, imageSha256 }`,
//   отсортированный по `sourcePath`, затем `mode`, затем `captureId`.
//
// Версия движка в identity не входит (baseline-policy §2): смена движка не
// создаёт новый эталон.

import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';

import { fixtureById, loadCorpus, slug, type Corpus } from '../corpus.js';
import { sha256Hex, sha256Text } from '../hash.js';
import { canonicalJson, isRecord } from '../json.js';
import {
  MANIFEST_SCHEMA_VERSION,
  validateManifestValue,
  type Manifest,
  type ManifestCapture,
  type ManifestEnvironment,
  type ManifestMask,
  type ManifestReference,
  type ManifestValidationOptions,
} from '../manifest.js';
import { parsePngHeader } from '../png.js';
import type { DocFormat, ViewMode } from '../types.js';

/** Ошибка импорта: одна причина, английский текст для оператора. */
export class ImportError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ImportError';
  }
}

/** Параметры импорта. */
export interface ImportOptions {
  /** Каталог `<format>/<fixture-id>/<mode>` с `capture.json`. */
  from: string;
  /**
   * Corpus index — источник истины для `corpus.*` и списка режимов фикстуры.
   * Импорт без него писал бы manifest, не связанный с корпусом.
   */
  corpusPath: string;
  /** Корень хранилища эталонов, обычно `test-fixtures/strict-viewer`. */
  rootUri: string;
  /** Корень исходных документов, обычно `test-fixtures`. */
  fixturesRoot: string;
  /** Перезаписать существующий manifest. */
  force?: boolean;
  /** Путь к схеме manifest для финальной проверки. */
  schemaPath?: string;
}

/** Итог импорта. */
export interface ImportResult {
  /** Куда записан manifest. */
  manifestPath: string;
  /** Новый baselineId. */
  baselineId: string;
  /** Прежний baselineId, если manifest перезаписан по `--force`. */
  previousBaselineId: string | null;
  /** Сколько captures попало в manifest. */
  captureCount: number;
}

/** Метаданные оператора из `capture.json`. */
interface CaptureInput {
  /**
   * Версии корпуса, с которыми снимался захват. Необязательны: истина живёт
   * в corpus index, а здесь они нужны лишь чтобы поймать устаревший drop.
   */
  corpus?: { version?: string; matrixVersion?: string };
  source: { path: string };
  /**
   * Метаданные захвата из `capture.json` без `exportedPdf.sha256`: хеш PDF
   * считает импортёр. Значения enum проверяет схема manifest, а не вторая
   * ручная копия списка — иначе импорт и валидатор разошлись бы.
   */
  reference: Omit<ManifestReference, 'exportedPdf'> & { exportedPdf?: { uri: string } };
  environment: ManifestEnvironment;
  view: Record<string, unknown> & { format: DocFormat; mode: ViewMode };
  dynamicData: Manifest['dynamicData'];
  captures: Array<{
    id: string;
    kind: ManifestCapture['kind'];
    pageNumber?: number;
    sheet?: { index: number; name: string };
    visibleRange?: string;
    scroll?: { x: number; y: number };
    activeCell?: string;
    image: string;
    masks?: ManifestMask[];
    expectedText?: string[];
  }>;
  notes?: string[];
}

/**
 * Импортировать один операторский захват.
 *
 * @throws {ImportError} если вход неполон, артефакт отсутствует или итоговый
 * manifest не проходит schema/semantic validation.
 */
export async function importReference(options: ImportOptions): Promise<ImportResult> {
  const fromDir = resolve(options.from);
  const rootUri = resolve(options.rootUri);
  const fixturesRoot = resolve(options.fixturesRoot);

  const input = await readCaptureInput(fromDir);
  const format = input.view.format;
  const mode = input.view.mode;
  const fixtureId = slug(input.source.path);
  checkCaptureDirectory(fromDir, format, fixtureId, mode);

  const corpus = await loadCorpusForImport(options.corpusPath);
  const fixture = fixtureById(corpus, fixtureId);
  if (fixture === undefined) {
    throw new ImportError(`fixture "${fixtureId}" is not listed in ${options.corpusPath}`);
  }
  if (fixture.format !== format) {
    throw new ImportError(
      `fixture "${fixtureId}" is ${fixture.format} in the corpus index but the capture is ${format}`,
    );
  }
  if (!fixture.modes.includes(mode)) {
    throw new ImportError(
      `mode "${mode}" is not listed for fixture "${fixtureId}" (corpus index has ${fixture.modes.join(', ')})`,
    );
  }
  if (fixture.source.path !== input.source.path) {
    throw new ImportError(
      `source.path "${input.source.path}" does not match corpus index path "${fixture.source.path}"`,
    );
  }
  if (
    input.corpus?.version !== undefined &&
    input.corpus.version !== corpus.corpus.version
  ) {
    throw new ImportError(
      `capture.json was taken against corpus version ${input.corpus.version}, index has ${corpus.corpus.version}`,
    );
  }
  if (
    input.corpus?.matrixVersion !== undefined &&
    input.corpus.matrixVersion !== corpus.corpus.matrixVersion
  ) {
    throw new ImportError(
      `capture.json was taken against matrix version ${input.corpus.matrixVersion}, index has ${corpus.corpus.matrixVersion}`,
    );
  }

  const sourcePath = resolveInside(fixturesRoot, input.source.path);
  if (sourcePath === null) {
    throw new ImportError(`source.path "${input.source.path}" does not resolve inside ${fixturesRoot}`);
  }
  const sourceBytes = await readBytes(sourcePath, 'source document');

  const captures: ManifestCapture[] = [];
  for (const [index, capture] of input.captures.entries()) {
    const imagePath = resolveInside(fromDir, capture.image);
    if (imagePath === null) {
      throw new ImportError(`captures[${index}].image "${capture.image}" does not resolve inside ${fromDir}`);
    }
    const imageUri = requireReferenceArtifact(capture.image, `captures[${index}].image`);
    if (!imageUri.endsWith('.png')) {
      throw new ImportError(
        `captures[${index}].image "${capture.image}" must be a .png file under reference/`,
      );
    }
    const imageBytes = await readBytes(imagePath, `captures[${index}].image`);
    const header = parsePngHeader(imageBytes);
    if (header === null) {
      throw new ImportError(`captures[${index}].image "${capture.image}" is not a PNG with a valid IHDR chunk`);
    }
    if (header.colorTypeName === null) {
      throw new ImportError(
        `captures[${index}].image "${capture.image}" has unsupported color type ${header.colorType} at bit depth ${header.bitDepth}; expected RGB8 or RGBA8`,
      );
    }
    captures.push({
      id: capture.id,
      kind: capture.kind,
      ...(capture.pageNumber === undefined ? {} : { pageNumber: capture.pageNumber }),
      ...(capture.sheet === undefined ? {} : { sheet: capture.sheet }),
      ...(capture.visibleRange === undefined ? {} : { visibleRange: capture.visibleRange }),
      ...(capture.scroll === undefined ? {} : { scroll: capture.scroll }),
      ...(capture.activeCell === undefined ? {} : { activeCell: capture.activeCell }),
      image: {
        uri: artifactUri(format, fixtureId, mode, imageUri),
        sha256: sha256Hex(imageBytes),
        width: header.width,
        height: header.height,
        bytes: imageBytes.byteLength,
        colorType: header.colorTypeName,
      },
      ...(capture.masks === undefined ? {} : { masks: capture.masks }),
      ...(capture.expectedText === undefined ? {} : { expectedText: capture.expectedText }),
    });
  }

  const reference: Manifest['reference'] = {
    provider: input.reference.provider,
    application: input.reference.application,
    capturedAt: input.reference.capturedAt,
    captureMethod: input.reference.captureMethod,
    operator: input.reference.operator,
    ...(input.reference.applicationVersion === undefined
      ? {}
      : { applicationVersion: input.reference.applicationVersion }),
  };
  if (input.reference.exportedPdf !== undefined) {
    const pdfPath = resolveInside(fromDir, input.reference.exportedPdf.uri);
    if (pdfPath === null) {
      throw new ImportError(
        `reference.exportedPdf.uri "${input.reference.exportedPdf.uri}" does not resolve inside ${fromDir}`,
      );
    }
    const pdfUri = requireReferenceArtifact(
      input.reference.exportedPdf.uri,
      'reference.exportedPdf.uri',
    );
    if (pdfUri !== 'reference/export.pdf') {
      throw new ImportError(
        `reference.exportedPdf.uri "${input.reference.exportedPdf.uri}" must be reference/export.pdf`,
      );
    }
    const pdfBytes = await readBytes(pdfPath, 'reference.exportedPdf');
    reference.exportedPdf = {
      uri: artifactUri(format, fixtureId, mode, pdfUri),
      sha256: sha256Hex(pdfBytes),
    };
  }

  const sourceSha256 = sha256Hex(sourceBytes);
  const baselineId = computeBaselineId({
    corpusVersion: corpus.corpus.version,
    matrixVersion: corpus.corpus.matrixVersion,
    environment: input.environment,
    sourcePath: input.source.path,
    sourceSha256,
    mode,
    captures,
  });

  const manifest: Manifest = {
    schemaVersion: MANIFEST_SCHEMA_VERSION,
    corpus: {
      id: corpus.corpus.id,
      version: corpus.corpus.version,
      matrixVersion: corpus.corpus.matrixVersion,
      baselineId,
      storage: corpus.corpus.storage,
    },
    source: {
      path: input.source.path,
      format,
      sha256: sourceSha256,
      bytes: sourceBytes.byteLength,
    },
    reference,
    environment: input.environment,
    view: input.view as unknown as Manifest['view'],
    dynamicData: input.dynamicData,
    comparisonProfile: 'visual-v1',
    captures,
    ...(input.notes === undefined ? {} : { notes: input.notes }),
  };

  const manifestPath = resolve(rootUri, format, fixtureId, mode, 'manifest.json');
  const previousBaselineId = await readExistingBaselineId(manifestPath);
  if (previousBaselineId !== null && options.force !== true) {
    throw new ImportError(
      `${manifestPath} already exists with baselineId ${previousBaselineId}; pass --force to overwrite`,
    );
  }

  const validationOptions: ManifestValidationOptions = {
    fixturesRoot,
    storageRoot: rootUri,
    ...(options.schemaPath === undefined ? {} : { schemaPath: options.schemaPath }),
  };
  const validation = await validateManifestValue(manifest, manifestPath, validationOptions);
  if (!validation.ok) {
    const details = validation.errors.map((error) => `${error.path || '<root>'}: ${error.message}`).join('; ');
    throw new ImportError(`generated manifest is invalid: ${details}`);
  }

  await mkdir(dirname(manifestPath), { recursive: true });
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  return {
    manifestPath,
    baselineId,
    previousBaselineId,
    captureCount: captures.length,
  };
}

/** Входные данные для `baselineId`. */
export interface BaselineIdInput {
  corpusVersion: string;
  matrixVersion: string;
  environment: ManifestEnvironment;
  sourcePath: string;
  sourceSha256: string;
  mode: ViewMode;
  captures: readonly Pick<ManifestCapture, 'id' | 'image'>[];
}

/**
 * Собрать `baselineId` по алгоритму из шапки модуля.
 *
 * Чистая функция: одинаковый вход даёт одинаковый id, изменение любого
 * хеша изображения — другой.
 */
export function computeBaselineId(input: BaselineIdInput): string {
  const environmentId = sha256Text(canonicalJson(input.environment)).slice(0, 8);
  const entries = input.captures
    .map((capture) => ({
      sourcePath: input.sourcePath,
      sourceSha256: input.sourceSha256,
      mode: input.mode,
      captureId: capture.id,
      imageSha256: capture.image.sha256,
    }))
    .sort(
      (a, b) =>
        a.sourcePath.localeCompare(b.sourcePath) ||
        a.mode.localeCompare(b.mode) ||
        a.captureId.localeCompare(b.captureId),
    );
  const shortDigest = sha256Text(
    canonicalJson({ captures: entries, environment: input.environment }),
  ).slice(0, 12);
  return `strict-viewer-${input.corpusVersion}-${input.matrixVersion}-${environmentId}-${shortDigest}`;
}

/** Последние три сегмента `--from` обязаны совпасть с форматом, id и режимом. */
function checkCaptureDirectory(
  fromDir: string,
  format: DocFormat,
  fixtureId: string,
  mode: ViewMode,
): void {
  const segments = fromDir.split(sep);
  const actual = segments.slice(-3);
  const expected = [format, fixtureId, mode];
  if (actual.length !== 3 || actual.some((segment, index) => segment !== expected[index])) {
    throw new ImportError(
      `input directory ${fromDir} must end with ${expected.join('/')} derived from source.path and view.mode`,
    );
  }
}

/** URI артефакта относительно `corpus.storage.rootUri`. */
function artifactUri(format: DocFormat, fixtureId: string, mode: ViewMode, relativePath: string): string {
  const normalized = relativePath.split(sep).join('/');
  return `${format}/${fixtureId}/${mode}/${normalized}`;
}

/**
 * Артефакт drop обязан быть плоским файлом внутри `reference/`.
 *
 * Возвращает путь с прямыми слэшами: manifest ссылается на
 * `reference/<name>`, поэтому `page-1.png` или `actual/page-1.png` сделали бы
 * URI несуществующим.
 */
function requireReferenceArtifact(uri: string, label: string): string {
  const normalized = uri.split(sep).join('/');
  const prefix = 'reference/';
  if (!normalized.startsWith(prefix)) {
    throw new ImportError(`${label} "${uri}" must live under reference/`);
  }
  const name = normalized.slice(prefix.length);
  if (name.length === 0 || name.includes('/')) {
    throw new ImportError(`${label} "${uri}" must be a file directly under reference/`);
  }
  return normalized;
}

/** Прочитать и проверить `capture.json`. */
async function readCaptureInput(fromDir: string): Promise<CaptureInput> {
  const path = resolve(fromDir, 'capture.json');
  let text: string;
  try {
    text = await readFile(path, 'utf8');
  } catch (error) {
    throw new ImportError(`cannot read ${path}: ${errorText(error)}`);
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new ImportError(`${path} is not valid JSON: ${errorText(error)}`);
  }
  if (!isRecord(value)) throw new ImportError(`${path} must be a JSON object`);

  const source = requireRecord(value['source'], 'source');
  const reference = requireRecord(value['reference'], 'reference');
  const environment = requireRecord(value['environment'], 'environment');
  const view = requireRecord(value['view'], 'view');
  const dynamicData = requireRecord(value['dynamicData'], 'dynamicData');
  const captures = value['captures'];
  if (!Array.isArray(captures) || captures.length === 0) {
    throw new ImportError('captures must be a non-empty array');
  }

  const format = view['format'];
  if (format !== 'docx' && format !== 'xlsx') {
    throw new ImportError('view.format must be docx or xlsx');
  }
  const mode = view['mode'];
  if (mode !== 'print-layout' && mode !== 'pdf' && mode !== 'normal' && mode !== 'print-preview') {
    throw new ImportError('view.mode must be print-layout, pdf, normal or print-preview');
  }

  const parsedCaptures = captures.map((entry, index): CaptureInput['captures'][number] => {
    const capture = requireRecord(entry, `captures[${index}]`);
    const image = capture['image'];
    if (typeof image !== 'string' || image.length === 0) {
      throw new ImportError(`captures[${index}].image must be a non-empty string`);
    }
    const id = capture['id'];
    if (typeof id !== 'string' || id.length === 0) {
      throw new ImportError(`captures[${index}].id must be a non-empty string`);
    }
    const kind = capture['kind'];
    if (kind !== 'docx-page' && kind !== 'xlsx-screen' && kind !== 'xlsx-print-page') {
      throw new ImportError(`captures[${index}].kind must be docx-page, xlsx-screen or xlsx-print-page`);
    }
    const captureKind: ManifestCapture['kind'] = kind;
    return {
      id,
      kind: captureKind,
      image,
      ...(capture['pageNumber'] === undefined ? {} : { pageNumber: capture['pageNumber'] as number }),
      ...(capture['sheet'] === undefined
        ? {}
        : { sheet: capture['sheet'] as { index: number; name: string } }),
      ...(capture['visibleRange'] === undefined
        ? {}
        : { visibleRange: capture['visibleRange'] as string }),
      ...(capture['scroll'] === undefined
        ? {}
        : { scroll: capture['scroll'] as { x: number; y: number } }),
      ...(capture['activeCell'] === undefined ? {} : { activeCell: capture['activeCell'] as string }),
      ...(capture['masks'] === undefined ? {} : { masks: capture['masks'] as ManifestMask[] }),
      ...(capture['expectedText'] === undefined
        ? {}
        : { expectedText: capture['expectedText'] as string[] }),
    };
  });

  const corpus = value['corpus'];
  return {
    ...(corpus === undefined ? {} : { corpus: readCorpusVersions(corpus) }),
    source: { path: requireString(source['path'], 'source.path') },
    reference: readReference(reference, mode),
    environment: environment as unknown as ManifestEnvironment,
    view: view as unknown as CaptureInput['view'],
    dynamicData: dynamicData as unknown as Manifest['dynamicData'],
    captures: parsedCaptures,
    ...(value['notes'] === undefined ? {} : { notes: value['notes'] as string[] }),
  };
}

/**
 * Разобрать `reference` из `capture.json`.
 *
 * Списки значений здесь не дублируются: `provider`, `application` и
 * `captureMethod` обязаны быть непустыми строками, а их допустимость и пара
 * проверяются схемой в `validateManifestValue` — единым источником истины для
 * импортированного manifest и любого другого. Ошибка называет путь поля.
 */
function readReference(
  reference: Record<string, unknown>,
  mode: ViewMode,
): CaptureInput['reference'] {
  const exportedPdf = reference['exportedPdf'];
  let pdfUri: string | undefined;
  if (exportedPdf !== undefined) {
    pdfUri = requireString(
      requireRecord(exportedPdf, 'reference.exportedPdf')['uri'],
      'reference.exportedPdf.uri',
    );
  } else if (mode === 'pdf') {
    throw new ImportError('reference.exportedPdf is required for mode pdf');
  }
  const applicationVersion = reference['applicationVersion'];
  return {
    provider: requireString(
      reference['provider'],
      'reference.provider',
    ) as ManifestReference['provider'],
    application: requireString(
      reference['application'],
      'reference.application',
    ) as ManifestReference['application'],
    ...(applicationVersion === undefined
      ? {}
      : { applicationVersion: requireString(applicationVersion, 'reference.applicationVersion') }),
    capturedAt: requireString(reference['capturedAt'], 'reference.capturedAt'),
    captureMethod: requireString(
      reference['captureMethod'],
      'reference.captureMethod',
    ) as ManifestReference['captureMethod'],
    operator: requireString(reference['operator'], 'reference.operator'),
    ...(pdfUri === undefined ? {} : { exportedPdf: { uri: pdfUri } }),
  };
}

/** Версии корпуса из `capture.json`: обе необязательны, но должны быть строками. */
function readCorpusVersions(value: unknown): NonNullable<CaptureInput['corpus']> {
  const record = requireRecord(value, 'corpus');
  const version = record['version'];
  const matrixVersion = record['matrixVersion'];
  return {
    ...(version === undefined ? {} : { version: requireString(version, 'corpus.version') }),
    ...(matrixVersion === undefined
      ? {}
      : { matrixVersion: requireString(matrixVersion, 'corpus.matrixVersion') }),
  };
}

/** Прочитать corpus index, превратив ошибку загрузки в `ImportError`. */
async function loadCorpusForImport(corpusPath: string): Promise<Corpus> {
  try {
    return await loadCorpus(corpusPath);
  } catch (error) {
    throw new ImportError(errorText(error));
  }
}

/** Прежний `baselineId` существующего manifest либо `null`. */
async function readExistingBaselineId(manifestPath: string): Promise<string | null> {
  try {
    await stat(manifestPath);
  } catch {
    return null;
  }
  try {
    const value: unknown = JSON.parse(await readFile(manifestPath, 'utf8'));
    if (isRecord(value) && isRecord(value['corpus'])) {
      const baselineId = value['corpus']['baselineId'];
      if (typeof baselineId === 'string') return baselineId;
    }
  } catch {
    // Битый прежний manifest не мешает перезаписи: его id просто неизвестен.
  }
  return null;
}

async function readBytes(path: string, label: string): Promise<Uint8Array> {
  try {
    return new Uint8Array(await readFile(path));
  } catch (error) {
    throw new ImportError(`cannot read ${label} at ${path}: ${errorText(error)}`);
  }
}

/** Путь внутри корня либо `null`, если он абсолютный или выходит наружу. */
function resolveInside(root: string, uri: string): string | null {
  if (uri.length === 0 || isAbsolute(uri)) return null;
  const resolved = resolve(root, uri);
  const rel = relative(root, resolved);
  if (rel.length === 0 || rel.startsWith('..') || isAbsolute(rel)) return null;
  return resolved;
}

function requireRecord(value: unknown, path: string): Record<string, unknown> {
  if (!isRecord(value)) throw new ImportError(`${path} must be an object`);
  return value;
}

function requireString(value: unknown, path: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new ImportError(`${path} must be a non-empty string`);
  }
  return value;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
