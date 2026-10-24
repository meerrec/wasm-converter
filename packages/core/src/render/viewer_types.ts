// Контракты ядра просмотрщиков: описание формата, состояние и общий handle.
//
// Вынесены из `viewer_core.ts`, чтобы жизненный цикл не тонул в типах;
// `viewer_core.ts` реэкспортирует всё отсюда — публичная поверхность одна.

import type {
  DocFormat,
  FrameTransport,
  HyperlinkInfo,
  InMsg,
  InitMsg,
  OutMsg,
  PaintStats,
  PdfOptions,
  RenderMode,
  TextExportFormat,
  TextLayer,
  Viewport,
} from '../protocol.js';
import type { AbortGuard } from './abort.js';
import type { MainFrameChain } from './main_frame_chain.js';
import type { TextLayerController } from './text_layer.js';

export type { TextExportFormat };

/** Общие поля опций, которые читает ядро. */
export interface ViewerBaseOptions {
  workerUrl?: string | URL;
  wasmUrl?: string;
  slotCapacity?: number;
  transport?: 'auto' | FrameTransport;
  renderMode?: RenderMode;
  zoom?: number;
  a11y?: boolean;
  onProgress?: (loaded: number, total?: number) => void;
}

/** Что формат рассказывает ядру о себе. */
export interface ViewerFormatSpec<S, O> {
  /** Имя воркера: `doc-converter-xlsx` и т.п. */
  workerName: string;
  /** URL воркера: свой из опций либо дефолт рядом с бандлом. */
  workerUrl: (options: { workerUrl?: URL }) => URL;
  /** Формат-тег телеметрии (ADR-0037). */
  telemetry: 'xlsx' | 'docx' | 'pptx';
  /** Формат в конверте `open`/`open-stream`. */
  formatTag: DocFormat;
  /** Тег ответа на открытие: свой у каждого формата. */
  openedResponse: 'opened' | 'docx-opened' | 'pptx-opened';
  /** Запомнить sheets/pages/slides + перестроить раскладку + запланировать кадр. */
  onOpened: (st: ViewerState<S>, msg: OutMsg) => void;
  /** Размеры распорки: лист / стопка страниц / слайд. */
  applyLayout: (st: ViewerState<S>) => void;
  /** Окно в единицах рендера: `x`/`y` — раскладка, `w`/`h` — физические пиксели. */
  viewport: (st: ViewerState<S>) => Viewport;
  /** Сообщение открытия байтами: формат и id проставляет формат. */
  openMsg: (st: ViewerState<S>, id: number, bytes: ArrayBuffer) => InMsg;
  /** Текстовая выгрузка формата; у XLSX её нет. */
  exportText?: (st: ViewerState<S>, format: TextExportFormat) => Promise<string>;
  /** Обновить доступный слой по требованию ядра (DOCX). */
  requestTextLayer?: (st: ViewerState<S>, page: number | null) => void;
  /** Дополнительная разметка формата: textLayer, role, подписки canvas. */
  extraInit?: (st: ViewerState<S>, canvas: HTMLCanvasElement, scroller: HTMLDivElement) => void;
  /** Форматное сообщение; `true` — ядру его обрабатывать не нужно. */
  extraMsg?: (st: ViewerState<S>, msg: OutMsg) => boolean;
}

/** Внутренние хуки, которых нет в публичном описании формата. */
export interface ViewerCoreHooks<S, O extends ViewerBaseOptions> {
  /** Начальное форматное состояние: листы, страницы или слайды. */
  createState: (options: O) => S;
  /** Дополнительные поля `init` — URL ленивых модулей формата. */
  initOptions?: (options: O) => Partial<InitMsg>;
  /** Сообщение кадра; `null` — рисовать нечего (документ ещё не открыт). */
  renderMsg: (st: ViewerState<S, O>) => InMsg | null;
  /** Сообщение экспорта PDF: у форматов разные теги и поля. */
  exportPdfMsg: (st: ViewerState<S, O>, id: number, options: PdfOptions) => InMsg;
}

/** Описание формата вместе с внутренними хуками ядра. */
export type ViewerSpec<S, O extends ViewerBaseOptions> = ViewerFormatSpec<S, O> &
  ViewerCoreHooks<S, O>;

/** Общее состояние просмотрщика: DOM, воркер, запросы и форматные данные. */
export interface ViewerState<S, O extends ViewerBaseOptions = ViewerBaseOptions> {
  readonly host: HTMLElement;
  readonly options: O;
  readonly scroller: HTMLDivElement;
  readonly spacer: HTMLDivElement;
  readonly canvas: HTMLCanvasElement;
  readonly worker: Worker;
  readonly renderMode: 'worker' | 'main';
  /** Приедет из `ready`: воркер сам решает, каким путём возить кадры. */
  transport: FrameTransport;
  /** Форматные данные: `sheets` у XLSX, `pages` у DOCX, `slides` у PPTX. */
  readonly data: S;
  zoom: number;
  dpr: number;
  /** Поле слева от стопки/слайда на экране; у XLSX всегда нуль. */
  padLeft: number;
  destroyed: boolean;
  frameScheduled: boolean;
  mainChain: MainFrameChain | null;
  /** Битмапы main-режима: main перерегистрирует их после resize. */
  readonly mainBitmaps: Map<number, ImageBitmap>;
  readonly tickHandlers: Set<(stats: PaintStats) => void>;
  readonly pendingOpen: Map<number, AbortGuard<void>>;
  readonly pendingHit: Map<number, { resolve: (cell: [number, number] | null) => void }>;
  readonly pendingHyperlink: Map<number, { resolve: (link: HyperlinkInfo | null) => void }>;
  readonly pendingPng: Map<number, AbortGuard<Uint8Array>>;
  readonly pendingPdf: Map<number, AbortGuard<Uint8Array>>;
  readonly pendingText: Map<number, AbortGuard<string>>;
  readonly pendingTextLayer: Map<number, { resolve: (layer: TextLayer) => void; reject: (e: Error) => void }>;
  nextRequestId: number;
  /** Доступный DOM-слой DOCX; у остальных форматов — `null`. */
  textLayer: TextLayerController | null;
  /** Размер последнего открытия — для атрибута спана. */
  openedCount: number;
  /** Уборка форматных подписок: зовётся из `destroy`. */
  readonly disposers: Set<() => void>;
  applyLayout(): void;
  schedule(): void;
  placeCanvas(): void;
  /** Сдвинуть холст и запланировать кадр — общий отклик на прокрутку. */
  refresh(): void;
  /** Запросить TextLayer страницы (`null` — всего документа). */
  requestTextLayer(page: number | null): Promise<TextLayer>;
}

/** Общий handle: формат добавляет к нему свои геттеры и операции. */
export interface ViewerBaseHandle<S, O extends ViewerBaseOptions = ViewerBaseOptions> {
  /** Внутреннее состояние; обёртка формата не публикует его наружу. */
  readonly state: ViewerState<S, O>;
  readonly transport: FrameTransport;
  readonly renderMode: 'worker' | 'main';
  open(
    source: ArrayBuffer | ReadableStream<Uint8Array>,
    opts?: { size?: number; signal?: AbortSignal },
  ): Promise<void>;
  setZoom(zoom: number): void;
  scrollTo(x: number, y: number): void;
  exportPng(opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  exportPdf(options?: PdfOptions, opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  exportText(format: TextExportFormat, opts?: { signal?: AbortSignal }): Promise<string>;
  getTextLayer(page: number): Promise<TextLayer>;
  getFullTextLayer(): Promise<TextLayer>;
  onTick(handler: (stats: PaintStats) => void): () => void;
  destroy(): void;
}
