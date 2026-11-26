// Захват кадров собственного вьюера.
//
// Модуль знает три вещи: как поднять страницу демо, как привести её в состояние
// из manifest и как снять артефакт. Всё остальное — дело вызывающего.
//
// Бэкенд каждого режима зафиксирован матрицей сертификации и не выбирается на
// ходу:
//
// | режим manifest  | формат | бэкенд | артефакт                                   |
// |-----------------|--------|--------|--------------------------------------------|
// | `print-layout`  | docx   | canvas | кадр окна размером с целую страницу         |
// | `normal`        | xlsx   | canvas | `viewer.exportPng()` на каждый экран        |
// | `pdf`           | docx   | pdf    | `viewer.exportPdf()` → `pdftoppm -r 144`    |
// | `print-preview` | xlsx   | pdf    | `viewer.exportPdf({ sheetIndex })` → то же  |
//
// Canvas-режимы снимают то, что видит пользователь; PDF-режимы — то, что движок
// печатает. Смешивать их нельзя: `docx.pdf` сертифицирует пагинацию PDF, а не
// окно, и наоборот.
//
// У `docx.print-layout` единица — страница, а не окно (certification-matrix §2.2:
// в эталон входит только область страницы). `viewer.exportPng()` снимает холст
// размера области прокрутки, и портретная A4 (~1123 CSS px) в окно 900 px не
// помещается. Поэтому окно перед кадром переводится на размер страницы: главная
// ось области прокрутки — flex-элемент, и коробка вьюера вырастает вместе с
// холстом, так что весь кадр попадает в снимок. Для этого canvas-путь получает
// `docxWindow`, а без него (прямой вызов `captureFixture`) честно падает.

import { execFile } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { Browser, BrowserContext, Page } from 'playwright';
import { sha256Hex } from '../hash.js';
import { parsePngHeader } from '../png.js';
import type { RuntimeMetadata } from '../types.js';
import { deferred } from './deferred.js';
import { asCaptureError, CaptureError } from './errors.js';
import type { ManifestCapture } from '../manifest.js';
import type { CapturePlan } from './index.js';
import type { PageWindow } from './page_api.js';
import { rasterizePdf, type RasterPage } from './rasterize.js';
import {
  waitForStableFrame,
  type StableFrameResult,
  type StableFrameViewer,
} from './stable.js';

/** Снятый артефакт одного capture. */
export interface CapturedImage {
  /** ID capture из manifest. */
  captureId: string;
  /** Вид capture: страница документа, экран книги или печатная страница. */
  kind: 'docx-page' | 'xlsx-screen' | 'xlsx-print-page';
  /** Абсолютный путь к PNG. */
  path: string;
  /** SHA-256 записанного PNG. */
  sha256: string;
  width: number;
  height: number;
  /**
   * CSS-размер окна, из которого снят кадр.
   *
   * Нужен `docx.print-layout`: единица сравнения там — страница, и размер окна
   * обязан быть её размером, а не размером области прокрутки. Иначе снимок —
   * обрезанное окно, а `width/height` PNG этого не покажет.
   */
  frame: { cssWidth: number; cssHeight: number } | null;
  /** Номер страницы — у `docx-page` и `xlsx-print-page`. */
  pageNumber?: number;
  /** Номер листа — у XLSX-видов. */
  sheetIndex?: number;
  /**
   * Два совпавших SHA-256 кадра, доказавшего стабильность именно этого
   * артефакта. У PDF-режимов это кадр-привратник, а не источник пикселей.
   */
  stableHashes: [string, string];
  /** Сколько ждал стабильности именно этот capture. */
  timeToStableMs: number;
}

/** Что нужно захвату от вызывающего. */
export interface CaptureFixtureContext {
  /** Браузер сессии: контекст создаётся на фикстуру и закрывается за собой. */
  readonly browser: Browser;
  /** Базовый URL dev-сервера без завершающего слэша. */
  readonly baseUrl: string;
  /** Каталог, куда пишутся PNG и промежуточные PDF. */
  readonly outDir: string;
  /** Корень репозитория — для `git rev-parse` и `git status`. */
  readonly repoRoot: string;
  /** Сколько ждать стабильного кадра. По умолчанию 20 с. */
  readonly stableTimeoutMs?: number;
  /** Сколько ждать открытия документа. По умолчанию 60 с. */
  readonly openTimeoutMs?: number;
  /** Отмена захвата. */
  readonly signal?: AbortSignal;
  /**
   * Действие с окном захвата — только для `docx.print-layout`.
   *
   * Границы окна читает и ставит сам оракул (`pageWindow` в
   * `capture/index.ts`): это браузерная операция страницы, а не часть модели
   * захвата, а копия её здесь разошлась бы с настоящими размерами.
   */
  readonly docxWindow?: DocxCaptureWindow;
}

/** Размер кадра окна в CSS-пикселях. */
export interface PageRegion {
  width: number;
  height: number;
}

/**
 * Окно захвата DOCX-страницы: размер области прокрутки на время кадра.
 *
 * Страница передаётся в каждый вызов, а не запоминается при создании: сессия
 * захвата создаёт окно один раз, а страница у каждой фикстуры своя.
 */
export interface DocxCaptureWindow {
  /** Поставить размер области прокрутки; `null` — снять переопределение. */
  apply(page: Page, size: { width: number; height: number } | null): Promise<void>;
  /**
   * Пиксели `viewer.exportPng()` вместе с размерами кадра.
   *
   * Размер берётся из того же снимка: окно могли пересчитать между вызовом и
   * ответом, и подписанная цифра дала бы подпись, которой нет у артефакта.
   */
  exportPng(page: Page): Promise<{ bytes: Uint8Array; cssWidth: number; cssHeight: number }>;
}

/**
 * Сколько страниц (или печатных страниц) реально имеет документ.
 *
 * Захват знает это число по построению: canvas-режим спрашивает
 * `viewer.pageCount`, PDF-режим считает растеризованные страницы. Наружу оно
 * нужно структурной проверке: manifest обязан покрывать все страницы, и
 * расхождение счётчиков — `page-mismatch`, а не повод сравнивать пиксели.
 */
export interface StructureCount {
  /** Что считалось: страницы документа или печатные страницы листа. */
  unit: 'pages' | 'print-pages';
  count: number;
}

/** Результат захвата одной фикстуры. */
export interface CaptureFixtureResult {
  captures: CapturedImage[];
  runtime: RuntimeMetadata;
  /** Счётчик страниц либо `null`, если режим его не проверяет. */
  structure: StructureCount | null;
}

const DEFAULT_OPEN_TIMEOUT_MS = 60_000;
/** Размер окна для PDF-режимов: PDF снимается не с окна, но странице нужен размер. */
const PDF_VIEWPORT = { width: 1440, height: 900 } as const;

/** Вид вьюера, который поднимает маршрут. */
type ViewerKind = 'docx' | 'xlsx';

/**
 * Ревизия движка: короткий HEAD плюс `-dirty`, если рабочее дерево грязное.
 *
 * Экспортируется, потому что прогон без единого захвата (все эталоны
 * отсутствуют) всё равно обязан записать ревизию в отчёт.
 */
export async function engineRevision(repoRoot: string): Promise<string> {
  const git = (args: readonly string[]): Promise<string> =>
    new Promise((resolve, reject) => {
      execFile('git', [...args], { cwd: repoRoot }, (error, stdout) => {
        if (error) {
          reject(error);
          return;
        }
        resolve(stdout.trim());
      });
    });
  const head = await git(['rev-parse', '--short', 'HEAD']);
  const status = await git(['status', '--porcelain']);
  return status.length === 0 ? head : `${head}-dirty`;
}

/**
 * Значение `<select id="fixture">`: путь от корня фикстур вида.
 *
 * `source.path` отсчитывается от `test-fixtures/` (`docx/tables/x.docx`), а
 * список фикстур — от корня вида (`tables/x.docx`), поэтому первый сегмент
 * отбрасывается.
 */
function fixtureSelectValue(plan: CapturePlan): string {
  const segments = plan.source.path.split('/');
  return segments.length > 1 ? segments.slice(1).join('/') : plan.source.path;
}

/** Параметры контекста: у PDF-режимов окно фиксированное, PDF важнее. */
function contextOptions(plan: CapturePlan): {
  viewport: { width: number; height: number };
  deviceScaleFactor: number;
} {
  const { environment } = plan;
  if (environment.viewportCss !== undefined && environment.deviceScaleFactor !== undefined) {
    return {
      viewport: environment.viewportCss,
      deviceScaleFactor: environment.deviceScaleFactor,
    };
  }
  return { viewport: { ...PDF_VIEWPORT }, deviceScaleFactor: 1 };
}

/** Страница DOCX с проверенными обязательными полями. */
interface DocxPageCapture {
  id: string;
  pageNumber: number;
}

/** Экран XLSX с проверенными обязательными полями. */
interface XlsxScreenCapture {
  id: string;
  sheetIndex: number;
  scroll: { x: number; y: number };
  activeCell: string;
}

/** Печатная страница XLSX с проверенными обязательными полями. */
interface XlsxPrintPageCapture {
  id: string;
  sheetIndex: number;
  pageNumber: number;
}

/**
 * Достать обязательные поля capture.
 *
 * Схема manifest требует их для своего вида, но захват не вправе полагаться на
 * то, что проверка уже была: пропущенное поле обязано стать внятной ошибкой, а
 * не `TypeError` посреди снимка.
 */
function requireDocxPage(capture: ManifestCapture): DocxPageCapture {
  if (capture.pageNumber === undefined) {
    throw new CaptureError('open-failed', `capture "${capture.id}" has no pageNumber`);
  }
  return { id: capture.id, pageNumber: capture.pageNumber };
}

function requireXlsxScreen(capture: ManifestCapture): XlsxScreenCapture {
  if (capture.sheet === undefined || capture.scroll === undefined || capture.activeCell === undefined) {
    throw new CaptureError(
      'open-failed',
      `capture "${capture.id}" needs sheet, scroll and activeCell for xlsx-screen`,
    );
  }
  return {
    id: capture.id,
    sheetIndex: capture.sheet.index,
    scroll: capture.scroll,
    activeCell: capture.activeCell,
  };
}

function requireXlsxPrintPage(capture: ManifestCapture): XlsxPrintPageCapture {
  if (capture.sheet === undefined || capture.pageNumber === undefined) {
    throw new CaptureError(
      'open-failed',
      `capture "${capture.id}" needs sheet and pageNumber for xlsx-print-page`,
    );
  }
  return { id: capture.id, sheetIndex: capture.sheet.index, pageNumber: capture.pageNumber };
}

/** Канал тиков из страницы в Node. */
interface TickChannel {
  subscribe(handler: (stats: unknown) => void): () => void;
  /** Сколько тиков пришло с начала сессии. */
  count(): number;
  /**
   * Дождаться тика после отметки `since`.
   *
   * Отметка, а не «следующий тик»: кадр может быть нарисован между действием и
   * подпиской, и ожидание «следующего» пропустило бы его, уйдя в таймаут на
   * документе, который уже отрисован.
   */
  waitForTick(since: number, timeoutMs: number): Promise<void>;
}

/**
 * Канал тиков: страница дёргает `window.__oracleTick`, который выставлен
 * `page.exposeBinding`, и подписчики получают событие в порядке отрисовки.
 * Опрос счётчика был бы гонкой: тик между подпиской и первым чтением потерялся
 * бы, и ожидание честно упало бы по таймауту на быстром документе.
 */
async function createTickChannel(page: Page): Promise<TickChannel> {
  const listeners = new Set<(stats: unknown) => void>();
  await page.exposeBinding('__oracleTick', () => {
    ticks += 1;
    for (const listener of listeners) listener(undefined);
  });
  let ticks = 0;
  const subscribe = (handler: (stats: unknown) => void): (() => void) => {
    listeners.add(handler);
    return () => {
      listeners.delete(handler);
    };
  };
  return {
    subscribe,
    count: () => ticks,
    async waitForTick(since, timeoutMs) {
      if (ticks > since) return;
      const arrived = deferred<void>();
      const unsubscribe = subscribe(() => {
        arrived.resolve();
      });
      const timer = setTimeout(() => {
        arrived.reject(
          new CaptureError(
            'open-failed',
            `viewer produced no frame within ${timeoutMs} ms after opening the document`,
          ),
        );
      }, timeoutMs);
      try {
        await arrived.promise;
      } finally {
        clearTimeout(timer);
        unsubscribe();
      }
    },
  };
}

/**
 * Снять PNG текущего кадра страницы.
 *
 * `captureDocxPageFrame` берёт снимок через окно (там же читается размер кадра);
 * этот путь — для экранов XLSX и для `exportPng` вьюера.
 */
async function exportPngBytes(page: Page): Promise<Uint8Array> {
  try {
    const base64 = await page.evaluate(async () => {
      const w = globalThis as unknown as PageWindow;
      const viewer = w.docConverter?.viewer;
      if (!viewer) throw new Error('viewer bridge is gone');
      const bytes = await viewer.exportPng();
      let binary = '';
      const chunk = 0x8000;
      for (let offset = 0; offset < bytes.length; offset += chunk) {
        binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
      }
      return w.btoa(binary);
    });
    return new Uint8Array(Buffer.from(base64, 'base64'));
  } catch (error) {
    throw asCaptureError(error, 'export-failed', 'exportPng failed');
  }
}

/** Вьюер страницы в терминах `waitForStableFrame`. */
function createPageViewer(page: Page, ticks: TickChannel): StableFrameViewer {
  return {
    exportPng: () => exportPngBytes(page),
    onTick(handler: (stats: unknown) => void): () => void {
      return ticks.subscribe(handler);
    },
  };
}

/** Дождаться моста нужного вида. */
async function waitForBridge(page: Page, kind: ViewerKind, timeoutMs: number): Promise<void> {
  try {
    await page.waitForFunction(
      (expected) => (globalThis as unknown as PageWindow).docConverter?.kind === expected,
      kind,
      { timeout: timeoutMs },
    );
  } catch (error) {
    const actual = await page.evaluate(
      () => (globalThis as unknown as PageWindow).docConverter?.kind ?? null,
    );
    throw new CaptureError(
      'browser-failed',
      `viewer "${kind}" did not come up: window.docConverter is ${
        actual === null ? 'absent' : `"${actual}"`
      }`,
      { cause: error },
    );
  }
}

/** Подписать вьюер страницы на мост тиков. */
async function installTickBridge(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = globalThis as unknown as PageWindow;
    const bridge = w.docConverter;
    if (!bridge) return;
    w.__oracleUnsub?.();
    w.__oracleUnsub = bridge.viewer.onTick(() => {
      w.__oracleTick?.();
    });
  });
}

/** Текст строки состояния — для диагностики неудачного открытия. */
async function statusText(page: Page): Promise<string> {
  return page.evaluate(
    () => (globalThis as unknown as PageWindow).document.querySelector('#status')?.textContent ?? '',
  );
}

/** Дождаться, пока документ откроется: страницы или листы появились. */
async function waitForOpen(page: Page, kind: ViewerKind, timeoutMs: number): Promise<void> {
  try {
    await page.waitForFunction(
      (expected) => {
        const bridge = (globalThis as unknown as PageWindow).docConverter;
        if (!bridge || bridge.kind !== expected) return false;
        if (expected === 'docx') return (bridge.viewer.pageCount ?? 0) > 0;
        return (bridge.viewer.sheets?.length ?? 0) > 0;
      },
      kind,
      { timeout: timeoutMs },
    );
  } catch (error) {
    const status = await statusText(page);
    throw new CaptureError(
      'open-failed',
      `document did not open within ${timeoutMs} ms${status ? `: ${status}` : ''}`,
      { cause: error },
    );
  }
}

/** Проверить, что фикстура есть в списке страницы. */
async function assertFixtureListed(page: Page, value: string): Promise<void> {
  const options = await page.evaluate(() => {
    const nodes = (globalThis as unknown as PageWindow).document.querySelectorAll('#fixture option');
    const values: string[] = [];
    for (const node of nodes) {
      if (typeof node.value === 'string') values.push(node.value);
    }
    return values;
  });
  if (!options.includes(value)) {
    throw new CaptureError(
      'fixture-not-listed',
      `fixture "${value}" is not among the ${options.length} options of #fixture; ` +
        'refusing to fall back to another document',
    );
  }
}

/** Открыть фикстуру через список страницы. */
async function openFixture(
  page: Page,
  value: string,
  kind: ViewerKind,
  timeoutMs: number,
  ticks: TickChannel,
): Promise<void> {
  await assertFixtureListed(page, value);
  // Отметка до выбора: кадр может быть нарисован раньше, чем `waitForOpen`
  // увидит страницы, и тогда ожидание «следующего» тика повисло бы.
  const mark = ticks.count();
  try {
    await page.selectOption('#fixture', value);
  } catch (error) {
    throw new CaptureError('open-failed', `could not select fixture "${value}"`, { cause: error });
  }
  await waitForOpen(page, kind, timeoutMs);
  // Первый кадр ждём по каналу тиков, а не по счётчику `#frame`: счётчик
  // пишет подвал страницы, и тик, пришедший до его монтирования, оставил бы
  // там ноль навсегда.
  await ticks.waitForTick(mark, timeoutMs);
  // Шрифты — часть кадра: до `fonts.ready` картинка ещё может смениться.
  await page.evaluate(() => (globalThis as unknown as PageWindow).document.fonts.ready);
}

/** Применить общее состояние из manifest: зум и оформление книги. */
async function applyViewState(page: Page, plan: CapturePlan): Promise<void> {
  const zoom = plan.environment.applicationZoomPercent / 100;
  await page.evaluate((value) => {
    (globalThis as unknown as PageWindow).docConverter?.viewer.setZoom(value);
  }, zoom);
  if (plan.view.format === 'xlsx') {
    const view = { showGrid: plan.view.showGridLines, showHeaders: plan.view.showHeadings };
    await page.evaluate((options) => {
      (globalThis as unknown as PageWindow).docConverter?.viewer.setView?.(options);
    }, view);
  }
}

/** Попросить вьюер перерисовать текущее состояние, не меняя его. */
async function repaint(page: Page): Promise<void> {
  await page.evaluate(() => {
    const viewer = (globalThis as unknown as PageWindow).docConverter?.viewer;
    if (!viewer) return;
    viewer.setZoom(viewer.zoom);
  });
}

/** Экспортировать PDF текущего состояния страницы. */
async function exportPdf(page: Page, sheetIndex?: number): Promise<Uint8Array> {
  try {
    const base64 = await page.evaluate(async (index) => {
      const w = globalThis as unknown as PageWindow;
      const viewer = w.docConverter?.viewer;
      if (!viewer) throw new Error('viewer bridge is gone');
      const bytes =
        index === null ? await viewer.exportPdf() : await viewer.exportPdf({ sheetIndex: index });
      let binary = '';
      const chunk = 0x8000;
      for (let offset = 0; offset < bytes.length; offset += chunk) {
        binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
      }
      return w.btoa(binary);
    }, sheetIndex ?? null);
    return new Uint8Array(Buffer.from(base64, 'base64'));
  } catch (error) {
    throw asCaptureError(error, 'export-failed', 'exportPdf failed');
  }
}

/** Доказательство стабильности, привязанное к конкретному артефакту. */
interface CaptureEvidence {
  stableHashes: [string, string];
  timeToStableMs: number;
}

/** Записать PNG и вернуть его описание вместе с доказательством стабильности. */
async function writePng(
  outDir: string,
  capture: ManifestCapture,
  bytes: Uint8Array,
  evidence: CaptureEvidence,
  frame: { cssWidth: number; cssHeight: number } | null = null,
): Promise<CapturedImage> {
  const filePath = path.join(outDir, `${capture.id}.png`);
  await writeFile(filePath, bytes);
  const header = parsePngHeader(bytes);
  if (header === null) {
    throw new CaptureError('export-failed', `capture "${capture.id}" is not a valid PNG`);
  }
  const base = {
    captureId: capture.id,
    kind: capture.kind,
    path: filePath,
    sha256: sha256Hex(bytes),
    width: header.width,
    height: header.height,
    frame,
    stableHashes: evidence.stableHashes,
    timeToStableMs: evidence.timeToStableMs,
  };
  if (capture.kind === 'docx-page') {
    return { ...base, pageNumber: requireDocxPage(capture).pageNumber };
  }
  if (capture.kind === 'xlsx-print-page') {
    const page = requireXlsxPrintPage(capture);
    return { ...base, pageNumber: page.pageNumber, sheetIndex: page.sheetIndex };
  }
  return { ...base, sheetIndex: requireXlsxScreen(capture).sheetIndex };
}

/** Шаг одного canvas-capture: доказать стабильность и записать именно её. */
export interface StableCaptureStep {
  /** Каталог, куда пишется PNG. */
  outDir: string;
  /** Capture из manifest: даёт id, вид и номер страницы/листа. */
  capture: ManifestCapture;
  /** Вьюер, у которого спрашивают кадр. */
  viewer: StableFrameViewer;
  /** Действие, приводящее вьюер в нужное состояние. */
  change: () => void | Promise<void>;
  /** Пределы ожидания и отмена. */
  options?: { timeoutMs?: number; signal?: AbortSignal };
  /** Размер окна снимка, если вызывающий его знает (см. `CapturedImage.frame`). */
  frame?: { cssWidth: number; cssHeight: number } | null;
}

/**
 * Дождаться стабильного кадра и записать его.
 *
 * Записывается `stable.bytes` — буфер, чей хеш совпал с предыдущим. Повторный
 * `exportPng` здесь был бы ошибкой: между подтверждением и снимком может
 * пройти перерисовка, и в артефакт уедет кадр, которого никто не проверял.
 */
export async function captureStableFrame(
  step: StableCaptureStep,
): Promise<{ image: CapturedImage; stable: StableFrameResult }> {
  const stable = await waitForStableFrame(step.viewer, {
    change: step.change,
    ...step.options,
  });
  const image = await writePng(
    step.outDir,
    step.capture,
    stable.bytes,
    {
      stableHashes: stable.hashes,
      timeToStableMs: stable.elapsedMs,
    },
    step.frame ?? null,
  );
  return { image, stable };
}

/**
 * Итог одного бэкенда: артефакты, метрики стабильности и счётчик страниц.
 *
 * `stableHashes`/`timeToStableMs` здесь — значения уровня фикстуры, и они
 * попадают в `runtime.json`. У canvas-режимов это доказательство ПОСЛЕДНЕГО
 * capture (у PDF — единственного кадра-привратника), поэтому привязывать их к
 * конкретной строке отчёта нельзя: правда о каждом артефакте лежит в
 * `CapturedImage.stableHashes`/`timeToStableMs`.
 */
interface BackendResult {
  captures: CapturedImage[];
  stableHashes: [string, string];
  timeToStableMs: number;
  structure: StructureCount | null;
}

/** Настройки ожидания стабильного кадра из контекста. */
function stableOptions(ctx: CaptureFixtureContext): {
  timeoutMs?: number;
  signal?: AbortSignal;
} {
  return {
    ...(ctx.stableTimeoutMs === undefined ? {} : { timeoutMs: ctx.stableTimeoutMs }),
    ...(ctx.signal === undefined ? {} : { signal: ctx.signal }),
  };
}

/**
 * Проверить, что запрошенная страница существует.
 *
 * `showPage` вне диапазона — тихий no-op, и без этой проверки промах manifest'а
 * выглядел бы как «кадр не стабилизировался» через полминуты ожидания.
 */
async function assertPageExists(page: Page, pageNumber: number, captureId: string): Promise<void> {
  const pageCount = await page.evaluate(
    () => (globalThis as unknown as PageWindow).docConverter?.viewer.pageCount ?? 0,
  );
  if (pageNumber < 1 || pageNumber > pageCount) {
    throw new CaptureError(
      'open-failed',
      `capture "${captureId}" wants page ${pageNumber}, but the document has ${pageCount}`,
    );
  }
}

/** Проверить, что запрошенный лист существует. */
async function assertSheetExists(page: Page, sheetIndex: number, captureId: string): Promise<void> {
  const sheetCount = await page.evaluate(
    () => (globalThis as unknown as PageWindow).docConverter?.viewer.sheets?.length ?? 0,
  );
  if (sheetIndex < 0 || sheetIndex >= sheetCount) {
    throw new CaptureError(
      'open-failed',
      `capture "${captureId}" wants sheet ${sheetIndex}, but the workbook has ${sheetCount}`,
    );
  }
}

/**
 * Окно кадра для страницы: весь лист плюс поля прокрутки.
 *
 * `viewer.showPage` ставит верх страницы в `scrollTop = 0`, а `scrollLeft`
 * программой не выставляется. Значит, чтобы страница целиком попала в кадр,
 * окно должно равняться стопке вместе с её полями: у стопки уже области
 * прокрутки поле слева — `(ширина области − стопка) / 2`, и кадр шириной в
 * стопку обрезал бы его (а тень страницы живёт в этом поле).
 *
 * @param pageSize — размер страницы в CSS-пикселях (раскладка × зум)
 */
export function pageFrameSize(
  pageSize: { width: number; height: number },
  viewport: { width: number; height: number },
): { width: number; height: number } {
  // Поле не больше половины окна и не больше `min(ширина, высота)/2`: при зуме
  // 100 % это ровно `padLeft`-подобное поле настоящего просмотрщика.
  const pad = Math.max(0, Math.floor(Math.min(viewport.width, viewport.height) / 2));
  // Кегль полей целочисленный, но стопка может быть дробной: округление вверх
  // не даёт обрезать последний пиксель справа или снизу.
  return {
    width: Math.ceil(pageSize.width) + 2 * pad,
    height: Math.ceil(pageSize.height) + 2 * pad,
  };
}

/** Размер страницы в CSS-пикселях по состоянию вьюера. */
export async function readDocxPageSize(
  page: Page,
  pageNumber: number,
  captureId: string,
): Promise<{ width: number; height: number }> {
  const size = await page.evaluate((index) => {
    const viewer = (globalThis as unknown as PageWindow).docConverter?.viewer;
    const info = viewer?.pages?.[index];
    if (!viewer || !info) return null;
    return { width: info.width * viewer.zoom, height: info.height * viewer.zoom };
  }, pageNumber - 1);
  if (size === null) {
    // Страница за пределами раскладки: `assertPageExists` это уже отсеяла, и
    // нулевого размера здесь быть не может — значит ручка разошлась с manifest.
    throw new CaptureError(
      'open-failed',
      `capture "${captureId}" wants page ${pageNumber}, but the viewer exposes no layout for it`,
    );
  }
  return size;
}

/**
 * Вписать страницу в окно на время снимка и снять кадр.
 *
 * Порядок обязателен: размер окна → `showPage` (он центрирует стопку под новый
 * размер) → первый тик → стабилизация. Переставь первые два шага, и прокрутка
 * встанет по полю прежнего окна, а страница уедет из кадра.
 *
 * @throws {CaptureError} `open-failed` — нет окна захвата или страницы в
 *   раскладке; `export-failed`/`unstable-frame` — как у `captureStableFrame`
 */
export async function captureDocxPageFrame(deps: {
  page: Page;
  viewer: StableFrameViewer;
  window: DocxCaptureWindow;
  capture: ManifestCapture;
  /** Окно браузера, в котором запущен вьюер (manifest viewportCss либо PDF_VIEWPORT). */
  viewport: { width: number; height: number };
  options: { timeoutMs?: number; signal?: AbortSignal };
  outDir: string;
}): Promise<{ image: CapturedImage; stable: StableFrameResult }> {
  const { page, window, capture } = deps;
  const pageNumber = requireDocxPage(capture).pageNumber;
  const pageSize = await readDocxPageSize(page, pageNumber, capture.id);
  // Вьюпорт берётся из параметров запуска, а не с замером области прокрутки:
  // та к моменту второго кадра уже переопределена (или сброшена) предыдущей
  // страницей, и кадры соседних страниц получали бы разную геометрию.
  const frame = pageFrameSize(pageSize, deps.viewport);

  try {
    await window.apply(page, frame);
    return await captureStableFrame({
      outDir: deps.outDir,
      capture,
      viewer: deps.viewer,
      change: async () => {
        await page.evaluate((index) => {
          (globalThis as unknown as PageWindow).docConverter?.viewer.showPage?.(index);
        }, pageNumber - 1);
      },
      options: deps.options,
      frame: { cssWidth: frame.width, cssHeight: frame.height },
    });
  } finally {
    // Окно возвращается до проверки: провал одной страницы не должен
    // оставлять следующий capture в чужой коробке.
    await window.apply(page, null);
  }
}

/** Canvas-режим DOCX: страница целиком, один кадр на страницу. */
async function captureDocxCanvas(
  page: Page,
  viewer: StableFrameViewer,
  plan: CapturePlan,
  ctx: CaptureFixtureContext,
): Promise<BackendResult> {
  if (ctx.docxWindow === undefined) {
    // Снимок окна вместо страницы молча уехал бы в эталон обрезанным: это
    // ровно та ошибка, ради которой страница снимается кадром её размера.
    throw new CaptureError(
      'open-failed',
      'docx.print-layout needs the capture window (ctx.docxWindow): a viewport-sized ' +
        'frame holds only part of a page',
    );
  }
  const window = ctx.docxWindow;
  const pageCount = await page.evaluate(
    () => (globalThis as unknown as PageWindow).docConverter?.viewer.pageCount ?? 0,
  );
  const captures: CapturedImage[] = [];
  let stableHashes: [string, string] = ['', ''];
  let timeToStableMs = 0;
  for (const capture of plan.captures) {
    if (capture.kind !== 'docx-page') {
      throw new CaptureError(
        'open-failed',
        `capture "${capture.id}" is ${capture.kind}, but view mode "${plan.view.mode}" expects docx-page`,
      );
    }
    await assertPageExists(page, requireDocxPage(capture).pageNumber, capture.id);
    const { image, stable } = await captureDocxPageFrame({
      page,
      viewer,
      window,
      capture,
      viewport: contextOptions(plan).viewport,
      options: stableOptions(ctx),
      outDir: ctx.outDir,
    });
    stableHashes = stable.hashes;
    timeToStableMs = Math.max(timeToStableMs, stable.elapsedMs);
    captures.push(image);
  }
  // Ноль означает, что ручка не отдала `pageCount`: сверять нечего, и
  // выдуманный ноль дал бы `page-mismatch` на здоровом документе.
  return {
    captures,
    stableHashes,
    timeToStableMs,
    structure: pageCount > 0 ? { unit: 'pages', count: pageCount } : null,
  };
}

/** Canvas-режим XLSX: экран за экраном. */
async function captureXlsxCanvas(
  page: Page,
  viewer: StableFrameViewer,
  plan: CapturePlan,
  ctx: CaptureFixtureContext,
): Promise<BackendResult> {
  const captures: CapturedImage[] = [];
  let stableHashes: [string, string] = ['', ''];
  let timeToStableMs = 0;
  for (const capture of plan.captures) {
    if (capture.kind !== 'xlsx-screen') {
      throw new CaptureError(
        'open-failed',
        `capture "${capture.id}" is ${capture.kind}, but view mode "${plan.view.mode}" expects xlsx-screen`,
      );
    }
    const { image, stable } = await captureStableFrame({
      outDir: ctx.outDir,
      capture,
      viewer,
      change: async () => {
        const screen = requireXlsxScreen(capture);
        await assertSheetExists(page, screen.sheetIndex, capture.id);
        const state = {
          sheetIndex: screen.sheetIndex,
          x: screen.scroll.x,
          y: screen.scroll.y,
          activeCell: screen.activeCell,
        };
        await page.evaluate(async (target) => {
          const handle = (globalThis as unknown as PageWindow).docConverter?.viewer;
          if (!handle) return;
          // Порядок важен: `showSheet` сбрасывает прокрутку и выделение, а
          // `setSelection` по умолчанию доскролливает до ячейки — это сбило бы
          // позицию из manifest. Поэтому выделение ставится без скролла, а
          // прокрутка — последней, ровно в записанные координаты.
          handle.showSheet?.(target.sheetIndex);
          await handle.setSelection?.(target.activeCell, { scrollIntoView: false });
          handle.scrollTo(target.x, target.y);
        }, state);
      },
      options: stableOptions(ctx),
    });
    stableHashes = stable.hashes;
    timeToStableMs = Math.max(timeToStableMs, stable.elapsedMs);
    captures.push(image);
  }
  // Normal view не имеет постраничной структуры: экран — это видимый
  // диапазон, и сверять его с числом страниц не с чем.
  return { captures, stableHashes, timeToStableMs, structure: null };
}

/** Разложить растеризованные страницы по номерам. */
function pagesByNumber(pages: readonly RasterPage[]): Map<number, Uint8Array> {
  const map = new Map<number, Uint8Array>();
  for (const page of pages) map.set(page.pageNumber, page.bytes);
  return map;
}

/** PDF-режим DOCX: один экспорт, страницы — из растеризации. */
async function captureDocxPdf(
  page: Page,
  viewer: StableFrameViewer,
  plan: CapturePlan,
  ctx: CaptureFixtureContext,
): Promise<BackendResult> {
  // PDF собирается из той же раскладки, что и кадр: дожидаемся стабильного
  // кадра один раз, чтобы экспорт не ушёл в середину пересчёта. Зум на самого
  // себя — запрос перерисовки: без него ждать было бы нечего, если последний
  // тик пришёл до подписки.
  const stable = await waitForStableFrame(viewer, {
    change: () => repaint(page),
    ...stableOptions(ctx),
  });
  const pdf = await exportPdf(page);
  await writeFile(path.join(ctx.outDir, 'export.pdf'), pdf);
  const pages = pagesByNumber(await rasterizePdf(pdf, plan.view.mode));

  const captures: CapturedImage[] = [];
  for (const capture of plan.captures) {
    if (capture.kind !== 'docx-page') {
      throw new CaptureError(
        'open-failed',
        `capture "${capture.id}" is ${capture.kind}, but view mode "pdf" expects docx-page`,
      );
    }
    const page_ = requireDocxPage(capture);
    const bytes = pages.get(page_.pageNumber);
    if (bytes === undefined) {
      throw new CaptureError(
        'rasterize-failed',
        `PDF export has no page ${page_.pageNumber} for capture "${capture.id}" ` +
          `(${pages.size} pages rasterised)`,
      );
    }
    captures.push(
      await writePng(ctx.outDir, capture, bytes, {
        stableHashes: stable.hashes,
        timeToStableMs: stable.elapsedMs,
      }),
    );
  }
  return {
    captures,
    stableHashes: stable.hashes,
    timeToStableMs: stable.elapsedMs,
    structure: { unit: 'pages', count: pages.size },
  };
}

/** PDF-режим XLSX: экспорт по листу, страницы — из растеризации. */
async function captureXlsxPdf(
  page: Page,
  viewer: StableFrameViewer,
  plan: CapturePlan,
  ctx: CaptureFixtureContext,
): Promise<BackendResult> {
  const stable = await waitForStableFrame(viewer, {
    change: () => repaint(page),
    ...stableOptions(ctx),
  });

  // Листы группируются: один экспорт на лист, а не на страницу.
  const bySheet = new Map<number, ManifestCapture[]>();
  for (const capture of plan.captures) {
    if (capture.kind !== 'xlsx-print-page') {
      throw new CaptureError(
        'open-failed',
        `capture "${capture.id}" is ${capture.kind}, but view mode "print-preview" expects xlsx-print-page`,
      );
    }
    const sheetIndex = requireXlsxPrintPage(capture).sheetIndex;
    await assertSheetExists(page, sheetIndex, capture.id);
    const list = bySheet.get(sheetIndex) ?? [];
    list.push(capture);
    bySheet.set(sheetIndex, list);
  }

  const captures: CapturedImage[] = [];
  let printPages = 0;
  for (const [sheetIndex, sheetCaptures] of bySheet) {
    const pdf = await exportPdf(page, sheetIndex);
    await writeFile(path.join(ctx.outDir, `sheet-${sheetIndex}.pdf`), pdf);
    const pages = pagesByNumber(await rasterizePdf(pdf, plan.view.mode));
    printPages += pages.size;
    for (const capture of sheetCaptures) {
      if (capture.kind !== 'xlsx-print-page') continue;
      const page_ = requireXlsxPrintPage(capture);
      const bytes = pages.get(page_.pageNumber);
      if (bytes === undefined) {
        throw new CaptureError(
          'rasterize-failed',
          `PDF export of sheet ${sheetIndex} has no page ${page_.pageNumber} ` +
            `for capture "${capture.id}" (${pages.size} pages rasterised)`,
        );
      }
      captures.push(
        await writePng(ctx.outDir, capture, bytes, {
          stableHashes: stable.hashes,
          timeToStableMs: stable.elapsedMs,
        }),
      );
    }
  }
  return {
    captures,
    stableHashes: stable.hashes,
    timeToStableMs: stable.elapsedMs,
    structure: { unit: 'print-pages', count: printPages },
  };
}

/** Загруженные семейства шрифтов страницы. */
async function loadedFonts(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const families = new Set<string>();
    for (const face of (globalThis as unknown as PageWindow).document.fonts) {
      if (face.status !== 'loaded') continue;
      families.add(face.family.replace(/^["']|["']$/g, ''));
    }
    return [...families].sort();
  });
}

/**
 * Снять одну фикстуру в одном режиме.
 *
 * @throws {CaptureError} с кодом шага: `browser-failed`, `fixture-not-listed`,
 * `open-failed`, `unstable-frame`, `export-failed`, `rasterize-failed`,
 * `missing-tool`.
 */
export async function captureFixture(
  ctx: CaptureFixtureContext,
  fixture: string,
  plan: CapturePlan,
): Promise<CaptureFixtureResult> {
  const kind: ViewerKind = plan.source.format;
  const openTimeoutMs = ctx.openTimeoutMs ?? DEFAULT_OPEN_TIMEOUT_MS;
  const options = contextOptions(plan);
  const backend: 'canvas' | 'pdf' =
    plan.view.mode === 'pdf' || plan.view.mode === 'print-preview' ? 'pdf' : 'canvas';

  let context: BrowserContext | null = null;
  try {
    try {
      context = await ctx.browser.newContext({
        viewport: options.viewport,
        deviceScaleFactor: options.deviceScaleFactor,
        colorScheme: 'light',
        ...(plan.environment.locale === undefined
          ? {}
          : { locale: plan.environment.locale }),
        ...(plan.environment.timezone === undefined
          ? {}
          : { timezoneId: plan.environment.timezone }),
      });
    } catch (error) {
      throw asCaptureError(
        error,
        'browser-failed',
        `could not create a browser context for "${fixture}"`,
      );
    }

    const page = await context.newPage();
    const pageErrors: string[] = [];
    page.on('pageerror', (error) => {
      pageErrors.push(String(error));
    });

    const ticks = await createTickChannel(page);

    try {
      await page.goto(`${ctx.baseUrl}/#/${kind}`, { waitUntil: 'domcontentloaded' });
    } catch (error) {
      throw asCaptureError(error, 'browser-failed', `could not open the ${kind} page`);
    }

    await waitForBridge(page, kind, openTimeoutMs);
    await installTickBridge(page);
    await openFixture(page, fixtureSelectValue(plan), kind, openTimeoutMs, ticks);
    await applyViewState(page, plan);

    const viewer = createPageViewer(page, ticks);
    let result: BackendResult;
    if (backend === 'canvas') {
      result =
        kind === 'docx'
          ? await captureDocxCanvas(page, viewer, plan, ctx)
          : await captureXlsxCanvas(page, viewer, plan, ctx);
    } else {
      result =
        kind === 'docx'
          ? await captureDocxPdf(page, viewer, plan, ctx)
          : await captureXlsxPdf(page, viewer, plan, ctx);
    }

    const handle = await page.evaluate(() => {
      const w = globalThis as unknown as PageWindow;
      const bridge = w.docConverter;
      if (!bridge) return null;
      return {
        transport: bridge.viewer.transport,
        renderMode: bridge.viewer.renderMode,
        zoom: bridge.viewer.zoom,
        dpr: w.devicePixelRatio,
      };
    });
    if (handle === null) {
      throw new CaptureError(
        'browser-failed',
        'viewer bridge disappeared before metadata was read',
      );
    }

    const viewport = page.viewportSize() ?? options.viewport;
    const runtime: RuntimeMetadata = {
      engineRevision: await engineRevision(ctx.repoRoot),
      transport: handle.transport,
      renderMode: handle.renderMode,
      viewport: { width: viewport.width, height: viewport.height },
      dpr: handle.dpr,
      zoom: handle.zoom,
      fonts: await loadedFonts(page),
      // Вьюер пока не отдаёт подстановки шрифтов: пустой список честнее
      // выдуманного.
      fontSubstitutions: [],
      featureSupport: null,
      timeToStableMs: result.timeToStableMs,
      stableHashes: result.stableHashes,
      captureBackend: backend,
    };

    if (pageErrors.length > 0) {
      // Ошибки страницы не отменяют уже снятые кадры, но молчать о них нельзя:
      // они объясняют будущие расхождения.
      process.stderr.write(
        `[visual-oracle] page errors while capturing "${fixture}":\n${pageErrors.join('\n')}\n`,
      );
    }

    return { captures: result.captures, runtime, structure: result.structure };
  } finally {
    await context?.close().catch(() => undefined);
  }
}

/** Записать `runtime.json` рядом с PNG. */
export async function writeRuntime(outDir: string, runtime: RuntimeMetadata): Promise<string> {
  const filePath = path.join(outDir, 'runtime.json');
  await mkdir(outDir, { recursive: true });
  await writeFile(filePath, `${JSON.stringify(runtime, null, 2)}\n`, 'utf8');
  return filePath;
}

/** Создать каталог вывода. */
export async function ensureOutDir(outDir: string): Promise<void> {
  await mkdir(outDir, { recursive: true });
}
