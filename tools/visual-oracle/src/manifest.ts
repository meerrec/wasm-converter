// Проверка manifest эталонного захвата: JSON Schema плюс семантические
// правила из `docs/strict-viewer/baseline-policy.md` §4, которым нужна
// файловая система. Схема ловит форму, здесь — существование артефактов,
// их хеши, непрерывность страниц и видимость листа.
//
// Валидация никогда не бросает исключение на нарушении: вызывающий получает
// список ошибок и сам решает, печатать их таблицей или падать с кодом 3.

import { readFile } from 'node:fs/promises';
import { isAbsolute, relative, resolve, sep } from 'node:path';

import type { AnySchema, ValidateFunction } from 'ajv';
import { Ajv2020 } from 'ajv/dist/2020.js';
import * as ajvFormats from 'ajv-formats';

import { slug } from './corpus.js';
import { sha256Hex } from './hash.js';
import { isRecord } from './json.js';
import { resolveFromRepo } from './paths.js';
import { parsePngHeader } from './png.js';
import type { DocFormat } from './types.js';
import { readZipEntry } from './zip.js';

/** Путь схемы manifest по умолчанию — от корня репозитория. */
export const DEFAULT_SCHEMA_PATH = 'docs/strict-viewer/manifest.schema.json';

/**
 * Версия формы manifest — она же `const` поля `schemaVersion` в схеме.
 *
 * `1.1.0` расширила только `reference`: провайдер `office-online`/`office-desktop`,
 * приложения `*-online`/`*-desktop` и способ `automated`, появление которого
 * связано с COM-автоматизацией локального Office (ADR-0056).
 */
export const MANIFEST_SCHEMA_VERSION = '1.1.0' as const;

/** Корень исходных документов по умолчанию — от корня репозитория. */
export const DEFAULT_FIXTURES_ROOT = 'test-fixtures';

/** Одно нарушение manifest. */
export interface ManifestErrorEntry {
  /** Путь до поля в документе, например `captures[0].image.sha256`. */
  path: string;
  /** Английское сообщение: его читает оператор и CI. */
  message: string;
}

/** Прямоугольник маски в пикселях изображения. */
export interface ManifestRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Маска динамической области. */
export interface ManifestMask {
  id: string;
  reason: 'dynamic-date' | 'documented-nondeterminism';
  rect: ManifestRect;
  textRule: string;
}

/** Изображение эталонного захвата. */
export interface ManifestImage {
  uri: string;
  sha256: string;
  width: number;
  height: number;
  bytes: number;
  colorType: 'RGB8' | 'RGBA8';
}

/** Селектор листа XLSX. */
export interface ManifestSheetSelector {
  index: number;
  name: string;
}

/** Один захват внутри manifest. */
export interface ManifestCapture {
  id: string;
  kind: 'docx-page' | 'xlsx-screen' | 'xlsx-print-page';
  pageNumber?: number;
  sheet?: ManifestSheetSelector;
  visibleRange?: string;
  scroll?: { x: number; y: number };
  activeCell?: string;
  image: ManifestImage;
  masks?: ManifestMask[];
  expectedText?: string[];
}

/** Хранилище артефактов корпуса. */
export interface ManifestStorage {
  kind: 'git-lfs' | 'release-asset' | 'object-store';
  rootUri: string;
  maxCorpusBytes?: number;
}

/** Паспорт корпуса внутри manifest. */
export interface ManifestCorpus {
  id: string;
  version: string;
  matrixVersion: string;
  baselineId: string;
  storage: ManifestStorage;
}

/** Исходный документ. */
export interface ManifestSource {
  path: string;
  format: DocFormat;
  sha256: string;
  bytes: number;
}

/** Шрифтовой ассет среды. */
export interface ManifestFontAsset {
  family: string;
  style: string;
  weight: number;
  sha256: string;
  source?: string;
}

/** Среда эталонного захвата. */
export interface ManifestEnvironment {
  os: { name: string; version: string; architecture?: string };
  browser: { name: 'chromium'; version: string };
  viewportCss?: { width: number; height: number };
  deviceScaleFactor?: number;
  applicationZoomPercent: number;
  browserZoomPercent: number;
  locale: string;
  timezone: string;
  colorScheme?: 'light';
  fonts: { package: string; version: string; assets: ManifestFontAsset[] };
  rasterizer?: { name: 'pdftoppm'; version: string; dpi: 144 };
}

/**
 * Метаданные эталонного захвата.
 *
 * Провайдер называет рендер, приложение — программу: `office-online` снимает
 * веб-версия (`*-online`), `office-desktop` — локальный Office через COM
 * (`*-desktop`). Пара проверяется схемой и `checkReferenceIdentity`: захват
 * не переписывается к ближайшему допустимому значению.
 */
export interface ManifestReference {
  provider: 'office-online' | 'office-desktop';
  application: 'word-online' | 'excel-online' | 'word-desktop' | 'excel-desktop';
  /** Версия приложения: для desktop — `Word.Version`/`Excel.Version` из COM. */
  applicationVersion?: string;
  capturedAt: string;
  /** `automated` — каждый шаг сделал скрипт; `manual` — человек целиком. */
  captureMethod: 'manual' | 'semi-automated' | 'automated';
  operator: string;
  exportedPdf?: { uri: string; sha256: string };
}

/** Режим просмотра DOCX. */
export interface ManifestDocxView {
  format: 'docx';
  mode: 'print-layout' | 'pdf';
  revisions: 'accepted';
  comments: 'hidden';
}

/** Режим просмотра XLSX. */
export interface ManifestXlsxView {
  format: 'xlsx';
  mode: 'normal' | 'print-preview';
  activeSheet: ManifestSheetSelector;
  hiddenSheets: 'exclude';
  showGridLines: boolean;
  showHeadings: boolean;
}

/** Политика динамических данных. */
export interface ManifestDynamicData {
  dates: 'fixed-value' | 'masked';
  fields: 'cached-result';
  formulaCache: {
    policy: 'cached-values-only';
    dirty: boolean;
    fullCalcOnLoad: boolean;
    forceFullCalc: boolean;
  };
  externalLinks: 'cached-values-no-network';
}

/** Разобранный manifest. */
export interface Manifest {
  schemaVersion: typeof MANIFEST_SCHEMA_VERSION;
  corpus: ManifestCorpus;
  source: ManifestSource;
  reference: ManifestReference;
  environment: ManifestEnvironment;
  view: ManifestDocxView | ManifestXlsxView;
  dynamicData: ManifestDynamicData;
  comparisonProfile?: 'visual-v1';
  captures: ManifestCapture[];
  notes?: string[];
}

/** Куда разрешать пути: корень фикстур и схема. */
export interface ManifestValidationOptions {
  /** Корень, относительно которого разрешается `source.path`. */
  fixturesRoot?: string;
  /** Путь к JSON Schema manifest. */
  schemaPath?: string;
  /**
   * Корень хранилища эталонов. По умолчанию — `corpus.storage.rootUri`
   * относительно текущего каталога; CLI передаёт сюда разрешённый путь,
   * чтобы результат не зависел от cwd.
   */
  storageRoot?: string;
}

/** Результат проверки manifest. */
export type ManifestValidation =
  | { ok: true; manifest: Manifest }
  | { ok: false; errors: ManifestErrorEntry[] };

/** Кэш скомпилированных валидаторов: схема одна на процесс. */
const validatorCache = new Map<string, ValidateFunction>();

/**
 * Прочитать и проверить manifest по пути.
 *
 * Ошибки чтения файла тоже возвращаются списком, а не исключением: CLI
 * печатает их в общей таблице.
 */
export async function validateManifestFile(
  manifestPath: string,
  options: ManifestValidationOptions = {},
): Promise<ManifestValidation> {
  let text: string;
  try {
    text = await readFile(manifestPath, 'utf8');
  } catch (error) {
    return { ok: false, errors: [{ path: '', message: `cannot read manifest: ${errorText(error)}` }] };
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    return { ok: false, errors: [{ path: '', message: `manifest is not valid JSON: ${errorText(error)}` }] };
  }
  return validateManifestValue(value, manifestPath, options);
}

/**
 * Проверить уже разобранный manifest.
 *
 * `manifestPath` нужен для правила «путь заканчивается на `/<mode>/manifest.json`»
 * и для сообщений; он не обязан существовать.
 */
export async function validateManifestValue(
  value: unknown,
  manifestPath: string,
  options: ManifestValidationOptions = {},
): Promise<ManifestValidation> {
  const schemaPath = resolveFromRepo(options.schemaPath ?? DEFAULT_SCHEMA_PATH);
  const fixturesRoot = resolveFromRepo(options.fixturesRoot ?? DEFAULT_FIXTURES_ROOT);

  const errors: ManifestErrorEntry[] = [];
  const validate = await loadValidator(schemaPath, errors);
  if (validate === null) return { ok: false, errors };

  // Пара provider/application проверяется и на форме, которую схема уже
  // отвергла: иначе оператор увидит «must be equal to one of the allowed
  // values» вместо причины — несовпадения семейства или формата.
  checkReferenceIdentity(value, errors);
  if (!validate(value)) {
    errors.push(...schemaErrors(validate.errors ?? []));
    // Семантические проверки бессмысленны на форме, которую не разобрать.
    return { ok: false, errors };
  }

  const manifest = value as Manifest;
  const storageRoot = resolveFromRepo(options.storageRoot ?? manifest.corpus.storage.rootUri);
  await checkSemantics(manifest, manifestPath, fixturesRoot, storageRoot, errors);
  if (errors.length > 0) return { ok: false, errors };
  return { ok: true, manifest };
}

/** Скомпилировать схему один раз на путь. */
async function loadValidator(
  schemaPath: string,
  errors: ManifestErrorEntry[],
): Promise<ValidateFunction | null> {
  const cached = validatorCache.get(schemaPath);
  if (cached !== undefined) return cached;
  let schema: unknown;
  try {
    schema = JSON.parse(await readFile(schemaPath, 'utf8'));
  } catch (error) {
    errors.push({ path: '', message: `cannot read manifest schema ${schemaPath}: ${errorText(error)}` });
    return null;
  }
  // `strict: false` — принятая схема использует условные подcхемы без
  // `type: "object"`, а strictTypes считает это ошибкой. Схема неизменяема,
  // поэтому строгость к её форме ничего не даёт.
  const ajv = new Ajv2020({ allErrors: true, strict: false });
  // `ajv-formats` — CJS без `exports`-карты: под NodeNext без
  // `esModuleInterop` типы видят `default` как сам namespace, хотя в рантайме
  // это функция. Приведение локализовано здесь и проверено тестом.
  const addFormats = ajvFormats.default as unknown as (instance: Ajv2020) => Ajv2020;
  addFormats(ajv);
  let compiled: ValidateFunction;
  try {
    compiled = ajv.compile(schema as AnySchema);
  } catch (error) {
    errors.push({ path: '', message: `manifest schema does not compile: ${errorText(error)}` });
    return null;
  }
  validatorCache.set(schemaPath, compiled);
  return compiled;
}

/** Семантические проверки §4, которым нужна файловая система. */
async function checkSemantics(
  manifest: Manifest,
  manifestPath: string,
  fixturesRoot: string,
  storageRoot: string,
  errors: ManifestErrorEntry[],
): Promise<void> {
  checkManifestPath(manifest, manifestPath, errors);
  const sourceBytes = await checkSource(manifest, fixturesRoot, errors);
  await checkReferencePdf(manifest, storageRoot, errors);
  await checkCaptures(manifest, storageRoot, errors);
  checkModeEnvironment(manifest, errors);
  if (manifest.source.format === 'xlsx' && sourceBytes !== null) {
    await checkSheets(manifest, sourceBytes, errors);
  }
}

/** Путь manifest обязан заканчиваться на `/<view.mode>/manifest.json`. */
function checkManifestPath(
  manifest: Manifest,
  manifestPath: string,
  errors: ManifestErrorEntry[],
): void {
  const expected = `${sep}${manifest.view.mode}${sep}manifest.json`;
  if (!manifestPath.endsWith(expected)) {
    errors.push({
      path: '',
      message: `manifest path must end with "${manifest.view.mode}/manifest.json"`,
    });
  }
}

/** Исходный документ: путь внутри `test-fixtures/`, размер и SHA-256. */
async function checkSource(
  manifest: Manifest,
  fixturesRoot: string,
  errors: ManifestErrorEntry[],
): Promise<Uint8Array | null> {
  const resolved = resolveInside(fixturesRoot, manifest.source.path);
  if (resolved === null) {
    errors.push({
      path: 'source.path',
      message: `"${manifest.source.path}" does not resolve inside ${fixturesRoot}`,
    });
    return null;
  }
  let bytes: Uint8Array;
  try {
    bytes = new Uint8Array(await readFile(resolved));
  } catch (error) {
    errors.push({ path: 'source.path', message: `source file is missing: ${errorText(error)}` });
    return null;
  }
  if (bytes.byteLength !== manifest.source.bytes) {
    errors.push({
      path: 'source.bytes',
      message: `declared ${manifest.source.bytes} bytes, actual ${bytes.byteLength}`,
    });
  }
  const actual = sha256Hex(bytes);
  if (actual !== manifest.source.sha256) {
    errors.push({
      path: 'source.sha256',
      message: `declared ${manifest.source.sha256}, actual ${actual}`,
    });
  }
  return bytes;
}

/** Сохранённый PDF от Word Online: URI под rootUri и совпадение SHA-256. */
async function checkReferencePdf(
  manifest: Manifest,
  rootUri: string,
  errors: ManifestErrorEntry[],
): Promise<void> {
  const exported = manifest.reference.exportedPdf;
  if (exported === undefined) return;
  const expected = `${artifactPrefix(manifest)}reference/export.pdf`;
  if (exported.uri !== expected) {
    errors.push({
      path: 'reference.exportedPdf.uri',
      message: `"${exported.uri}" must be "${expected}"`,
    });
    return;
  }
  const resolved = resolveInside(rootUri, exported.uri);
  if (resolved === null) {
    errors.push({
      path: 'reference.exportedPdf.uri',
      message: `"${exported.uri}" does not resolve inside ${rootUri}`,
    });
    return;
  }
  let bytes: Uint8Array;
  try {
    bytes = new Uint8Array(await readFile(resolved));
  } catch (error) {
    errors.push({
      path: 'reference.exportedPdf.uri',
      message: `exported PDF is missing: ${errorText(error)}`,
    });
    return;
  }
  const actual = sha256Hex(bytes);
  if (actual !== exported.sha256) {
    errors.push({
      path: 'reference.exportedPdf.sha256',
      message: `declared ${exported.sha256}, actual ${actual}`,
    });
  }
}

/** Уникальность id, артефакты, маски и непрерывность страниц. */
async function checkCaptures(
  manifest: Manifest,
  rootUri: string,
  errors: ManifestErrorEntry[],
): Promise<void> {
  const captureIds = new Set<string>();
  const maskIds = new Set<string>();

  for (const [index, capture] of manifest.captures.entries()) {
    const base = `captures[${index}]`;
    if (captureIds.has(capture.id)) {
      errors.push({ path: `${base}.id`, message: `duplicate capture id "${capture.id}"` });
    }
    captureIds.add(capture.id);

    const imageBase = `${base}.image`;
    if (!isReferenceImageUri(manifest, capture.image.uri)) {
      errors.push({
        path: `${imageBase}.uri`,
        message: `"${capture.image.uri}" must be <format>/<fixture-id>/<mode>/reference/<name>.png`,
      });
    }

    const image = await checkImage(capture.image, imageBase, rootUri, errors);
    if (image === null) continue;

    for (const [maskIndex, mask] of (capture.masks ?? []).entries()) {
      const maskBase = `${base}.masks[${maskIndex}]`;
      if (maskIds.has(mask.id)) {
        errors.push({ path: `${maskBase}.id`, message: `duplicate mask id "${mask.id}"` });
      }
      maskIds.add(mask.id);
      checkMaskRect(mask, maskBase, image, errors);
    }
    const masks = capture.masks ?? [];
    if (masks.length > 0) {
      const covered = unionArea(masks.map((mask) => mask.rect));
      if (covered >= image.width * image.height) {
        errors.push({
          path: `${base}.masks`,
          message: 'masks cover the whole image; a full-image mask is forbidden',
        });
      }
    }
  }

  if (manifest.source.format === 'docx') {
    checkContinuousPages(manifest, 'docx', errors);
  } else if (manifest.view.mode === 'print-preview') {
    // Print Preview обязан содержать все печатные страницы листа
    // (baseline-policy §4), поэтому нумерация тоже непрерывна от 1.
    checkContinuousPages(manifest, 'xlsx print-preview', errors);
  }
}

/** Изображение: URI под rootUri, PNG-сигнатура, размеры, тип цвета, хеш. */
async function checkImage(
  image: ManifestImage,
  base: string,
  rootUri: string,
  errors: ManifestErrorEntry[],
): Promise<ManifestImage | null> {
  const resolved = resolveInside(rootUri, image.uri);
  if (resolved === null) {
    errors.push({ path: `${base}.uri`, message: `"${image.uri}" does not resolve inside ${rootUri}` });
    return null;
  }
  let bytes: Uint8Array;
  try {
    bytes = new Uint8Array(await readFile(resolved));
  } catch (error) {
    errors.push({ path: `${base}.uri`, message: `image is missing: ${errorText(error)}` });
    return null;
  }
  if (bytes.byteLength !== image.bytes) {
    errors.push({ path: `${base}.bytes`, message: `declared ${image.bytes} bytes, actual ${bytes.byteLength}` });
  }
  const actualHash = sha256Hex(bytes);
  if (actualHash !== image.sha256) {
    errors.push({ path: `${base}.sha256`, message: `declared ${image.sha256}, actual ${actualHash}` });
  }
  const header = parsePngHeader(bytes);
  if (header === null) {
    errors.push({ path: `${base}.uri`, message: 'file is not a PNG with a valid IHDR chunk' });
    return null;
  }
  if (header.width !== image.width || header.height !== image.height) {
    errors.push({
      path: `${base}.width`,
      message: `declared ${image.width}x${image.height}, actual ${header.width}x${header.height}`,
    });
  }
  if (header.colorTypeName !== image.colorType) {
    errors.push({
      path: `${base}.colorType`,
      message: `declared ${image.colorType}, actual ${header.colorTypeName ?? `colorType ${header.colorType} at bit depth ${header.bitDepth}`}`,
    });
  }
  return image;
}

/** Маска внутри изображения и не во весь кадр. */
function checkMaskRect(
  mask: ManifestMask,
  base: string,
  image: ManifestImage,
  errors: ManifestErrorEntry[],
): void {
  const { x, y, width, height } = mask.rect;
  if (x + width > image.width || y + height > image.height) {
    errors.push({
      path: `${base}.rect`,
      message: `rect ${x},${y} ${width}x${height} exceeds image ${image.width}x${image.height}`,
    });
  }
  if (x === 0 && y === 0 && width === image.width && height === image.height) {
    errors.push({ path: `${base}.rect`, message: 'mask covers the whole image' });
  }
}

/** Номера страниц непрерывны от 1, без пропусков и дублей. */
function checkContinuousPages(
  manifest: Manifest,
  label: string,
  errors: ManifestErrorEntry[],
): void {
  const pages: number[] = [];
  for (const [index, capture] of manifest.captures.entries()) {
    if (capture.pageNumber === undefined) {
      errors.push({
        path: `captures[${index}].pageNumber`,
        message: `${label} capture requires pageNumber`,
      });
      continue;
    }
    pages.push(capture.pageNumber);
  }
  const sorted = [...pages].sort((a, b) => a - b);
  for (const [index, page] of sorted.entries()) {
    if (page !== index + 1) {
      errors.push({
        path: 'captures',
        message: `${label} page numbers must be continuous from 1; expected ${index + 1}, got ${page}`,
      });
      return;
    }
  }
}

/** XLSX: активный лист и листы captures существуют и видимы. */
async function checkSheets(
  manifest: Manifest,
  sourceBytes: Uint8Array,
  errors: ManifestErrorEntry[],
): Promise<void> {
  if (manifest.view.format !== 'xlsx') return;
  const sheets = readWorkbookSheets(sourceBytes);
  if (sheets === null) {
    errors.push({ path: 'source.path', message: 'cannot read xl/workbook.xml from the XLSX source' });
    return;
  }
  checkSheetSelector(manifest.view.activeSheet, 'view.activeSheet', sheets, errors);
  for (const [index, capture] of manifest.captures.entries()) {
    if (capture.sheet === undefined) continue;
    checkSheetSelector(capture.sheet, `captures[${index}].sheet`, sheets, errors);
  }
}

/** Лист по индексу: имя совпадает и лист видим. */
function checkSheetSelector(
  selector: ManifestSheetSelector,
  base: string,
  sheets: readonly WorkbookSheet[],
  errors: ManifestErrorEntry[],
): void {
  const sheet = sheets[selector.index];
  if (sheet === undefined) {
    errors.push({
      path: `${base}.index`,
      message: `sheet index ${selector.index} is out of range (workbook has ${sheets.length})`,
    });
    return;
  }
  if (sheet.name !== selector.name) {
    errors.push({
      path: `${base}.name`,
      message: `declared "${selector.name}", workbook sheet ${selector.index} is "${sheet.name}"`,
    });
  }
  if (sheet.state !== 'visible') {
    errors.push({
      path: `${base}.index`,
      message: `sheet "${sheet.name}" is ${sheet.state}; hidden sheets must not be captured`,
    });
  }
}

/** Лист книги: имя и состояние видимости. */
interface WorkbookSheet {
  name: string;
  state: 'visible' | 'hidden' | 'veryHidden';
}

/** Прочитать список листов из `xl/workbook.xml`. */
function readWorkbookSheets(xlsxBytes: Uint8Array): WorkbookSheet[] | null {
  const xml = readZipEntry(xlsxBytes, 'xl/workbook.xml');
  if (xml === null) return null;
  const text = new TextDecoder('utf-8').decode(xml);
  const sheets: WorkbookSheet[] = [];
  const pattern = /<sheet\b([^>]*)\/?>/g;
  for (let match = pattern.exec(text); match !== null; match = pattern.exec(text)) {
    const attributes = match[1] ?? '';
    const name = attribute(attributes, 'name');
    if (name === null) continue;
    const state = attribute(attributes, 'state');
    sheets.push({
      name: decodeXmlEntities(name),
      state: state === 'hidden' || state === 'veryHidden' ? state : 'visible',
    });
  }
  return sheets;
}

/** Значение атрибута XML по имени. */
function attribute(attributes: string, name: string): string | null {
  const pattern = new RegExp(`(?:^|\\s)${name}\\s*=\\s*"([^"]*)"`);
  const match = pattern.exec(attributes);
  return match?.[1] ?? null;
}

/** Раскрыть пять предопределённых XML-сущностей. */
function decodeXmlEntities(value: string): string {
  return value
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, '&');
}

/**
 * Provider, application и формат документа согласованы.
 *
 * `office-online` снимает веб-версия (*-online), `office-desktop` — локальный
 * Word/Excel (*-desktop); DOCX снимает Word, XLSX — Excel. Схема проверяет
 * пару ветками `allOf` по `provider` и по формату; здесь она повторяется,
 * чтобы у ошибки была причина словами, а не «must be equal to one of the
 * allowed values». Проверка не зависит от формы: если поля — строки, пара
 * проверяется даже поверх ошибки схемы. Способ съёмки (`captureMethod`) от
 * провайдера не зависит: `automated` допустим в обоих семействах.
 */
function checkReferenceIdentity(value: unknown, errors: ManifestErrorEntry[]): void {
  if (!isRecord(value)) return;
  const reference = value['reference'];
  const source = value['source'];
  if (!isRecord(reference) || !isRecord(source)) return;
  const provider = reference['provider'];
  const application = reference['application'];
  const format = source['format'];
  if (typeof provider !== 'string' || typeof application !== 'string') return;

  const providerFamily = familyOf(provider);
  const applicationFamily = familyOf(application);
  if (providerFamily !== null && applicationFamily !== null && applicationFamily !== providerFamily) {
    errors.push({
      path: 'reference.application',
      message: `application "${application}" does not match provider "${provider}": office-online requires a *-online application, office-desktop requires a *-desktop application`,
    });
  }

  const applicationFormat = application.startsWith('word-')
    ? 'docx'
    : application.startsWith('excel-')
      ? 'xlsx'
      : null;
  if (
    applicationFormat !== null &&
    (format === 'docx' || format === 'xlsx') &&
    applicationFormat !== format
  ) {
    errors.push({
      path: 'reference.application',
      message: `application "${application}" does not match source format "${format}": docx requires word-*, xlsx requires excel-*`,
    });
  }
}

/** Семейство приложения: `office-online` и `word-online` — `online`. */
function familyOf(applicationOrProvider: string): 'online' | 'desktop' | null {
  if (applicationOrProvider === 'office-online' || applicationOrProvider.endsWith('-online')) {
    return 'online';
  }
  if (applicationOrProvider === 'office-desktop' || applicationOrProvider.endsWith('-desktop')) {
    return 'desktop';
  }
  return null;
}

/** `docx.pdf` требует PDF-среду, остальные режимы — экранную. */
function checkModeEnvironment(manifest: Manifest, errors: ManifestErrorEntry[]): void {
  const isPdf = manifest.view.mode === 'pdf';
  const environment = manifest.environment;
  if (isPdf) {
    if (manifest.reference.exportedPdf === undefined) {
      errors.push({ path: 'reference.exportedPdf', message: 'docx.pdf requires exportedPdf' });
    }
    if (environment.rasterizer === undefined) {
      errors.push({ path: 'environment.rasterizer', message: 'docx.pdf requires environment.rasterizer' });
    }
    for (const field of ['viewportCss', 'deviceScaleFactor', 'colorScheme'] as const) {
      if (environment[field] !== undefined) {
        errors.push({ path: `environment.${field}`, message: `docx.pdf must not declare ${field}` });
      }
    }
    return;
  }
  if (environment.rasterizer !== undefined) {
    errors.push({
      path: 'environment.rasterizer',
      message: `mode ${manifest.view.mode} must not declare environment.rasterizer`,
    });
  }
  for (const field of ['viewportCss', 'deviceScaleFactor', 'colorScheme'] as const) {
    if (environment[field] === undefined) {
      errors.push({ path: `environment.${field}`, message: `mode ${manifest.view.mode} requires ${field}` });
    }
  }
}

/**
 * Разрешить относительный URI внутри корня.
 *
 * Возвращает `null`, если путь абсолютный или выходит за корень: manifest
 * переносим, поэтому локальные и выходящие наружу пути запрещены.
 */
function resolveInside(root: string, uri: string): string | null {
  if (uri.length === 0 || isAbsolute(uri)) return null;
  const resolved = resolve(root, uri);
  const rel = relative(root, resolved);
  if (rel.length === 0 || rel.startsWith('..') || isAbsolute(rel)) return null;
  return resolved;
}

/**
 * Префикс артефактов захвата относительно `corpus.storage.rootUri`.
 *
 * Fixture id — slug от `source.path`, как в corpus index; manifest без
 * `reference/`-путей не может лежать в своей папке.
 */
function artifactPrefix(manifest: Manifest): string {
  return `${manifest.source.format}/${slug(manifest.source.path)}/${manifest.view.mode}/`;
}

/**
 * `captures[].image.uri` обязан быть `reference/<name>.png` внутри папки
 * захвата. Иначе manifest ссылается на файл, которого нет.
 */
function isReferenceImageUri(manifest: Manifest, uri: string): boolean {
  const prefix = `${artifactPrefix(manifest)}reference/`;
  if (!uri.startsWith(prefix)) return false;
  return /^[^/\\]+\.png$/.test(uri.slice(prefix.length));
}

/** Площадь объединения прямоугольников: координатное сжатие по краям. */
function unionArea(rects: readonly ManifestRect[]): number {
  if (rects.length === 0) return 0;
  const xs = [...new Set(rects.flatMap((rect) => [rect.x, rect.x + rect.width]))].sort((a, b) => a - b);
  const ys = [...new Set(rects.flatMap((rect) => [rect.y, rect.y + rect.height]))].sort((a, b) => a - b);
  let area = 0;
  for (let xi = 0; xi + 1 < xs.length; xi += 1) {
    const left = xs[xi] ?? 0;
    const right = xs[xi + 1] ?? 0;
    for (let yi = 0; yi + 1 < ys.length; yi += 1) {
      const top = ys[yi] ?? 0;
      const bottom = ys[yi + 1] ?? 0;
      const covered = rects.some(
        (rect) => rect.x <= left && rect.x + rect.width >= right && rect.y <= top && rect.y + rect.height >= bottom,
      );
      if (covered) area += (right - left) * (bottom - top);
    }
  }
  return area;
}

/**
 * Ошибки схемы в терминах полей manifest.
 *
 * ajv сообщает о нарушении условной ветки на родительском пути
 * (`environment: must have required property 'rasterizer'`), поэтому имя
 * недостающего поля дописывается в путь. Служебные ошибки `if`/`then`/`else`
 * отбрасываются, когда есть конкретная причина: иначе оператор видит шум.
 */
function schemaErrors(issues: readonly SchemaIssue[]): ManifestErrorEntry[] {
  const mapped = issues.map((issue) => ({
    path: formatInstancePath(issue),
    message: issueMessage(issue),
    branch: isBranchMessage(issue.message),
  }));
  const concrete = mapped.filter((entry) => !entry.branch);
  const chosen = concrete.length > 0 ? concrete : mapped;
  return chosen.map(({ path, message }) => ({ path, message }));
}

/**
 * Текст нарушения: к сообщению ajv добавляется список допустимых значений.
 *
 * Без него оператор видит только «must be equal to one of the allowed values» —
 * и не узнаёт, что `full-auto` невозможен, а `automated` возможен.
 */
function issueMessage(issue: SchemaIssue): string {
  const message = issue.message ?? 'schema violation';
  const allowed = issue.params?.['allowedValues'];
  if (!Array.isArray(allowed) || allowed.length === 0) return message;
  return `${message} (${allowed.map((value) => JSON.stringify(value)).join(', ')})`;
}

/** Служебное сообщение о несовпадении условной ветки схемы. */
function isBranchMessage(message: string | undefined): boolean {
  return message !== undefined && /^must match "(if|then|else)" schema$/.test(message);
}

/** `/captures/0/image/sha256` → `captures[0].image.sha256`. */
function formatInstancePath(issue: SchemaIssue): string {
  const raw = issue.instancePath ?? '';
  const segments = raw.split('/').filter((segment) => segment.length > 0);
  let path = '';
  for (const segment of segments) {
    if (/^\d+$/.test(segment)) path += `[${segment}]`;
    else path += path.length === 0 ? segment : `.${segment}`;
  }
  const missing = issue.params?.['missingProperty'];
  const additional = issue.params?.['additionalProperty'];
  const suffix = typeof missing === 'string' ? missing : typeof additional === 'string' ? additional : null;
  if (suffix !== null) path = path.length === 0 ? suffix : `${path}.${suffix}`;
  return path;
}

/** Минимум полей ошибки ajv, который здесь используется. */
interface SchemaIssue {
  instancePath?: string;
  message?: string;
  params?: Record<string, unknown>;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
