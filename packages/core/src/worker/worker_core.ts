/// <reference lib="webworker" />
// Общее ядро воркера формата: painter, транспорт кадра, SAB/пул, реестр
// битмапов, потоки открытия и ленивый PDF-модуль. Форматная часть — разбор,
// сборка кадра и экспорт — приходит хуками из `xlsx_worker`/`docx_worker`/`pptx_worker`.
// Ядро не знает про `DocFormat` за пределами собственного формата: чужое
// открытие отбрасывается ошибкой `open`.
import init, {
  alloc_sab,
  drop_bitmap,
  register_bitmap,
  sab_total_bytes,
} from '@meerrec/wasm';
import type { BuildStatsWire, DiagnosticWire, ImageInfoWire } from '../generated/wire.js';
import type {
  InMsg,
  InitMsg,
  OpenMsg,
  OpenStreamMsg,
  OutMsg,
  PdfModuleUrls,
  RenderRequest,
  WorkerErrorCode,
  WorkerErrorMsg,
} from '../protocol.js';
import { resolveExecutionPlan, type ExecutionPlan } from '../execution/plan.js';
import {
  createTransportAdapter,
  type LocalBuildResult,
  type TransportAdapter,
} from '../execution/transport.js';
import { exportPng } from '../render/export_png.js';
import { frameTarget } from './frame_args.js';
import { ForwardingRenderAdapter } from './execution/render_forward.js';
import { OffscreenRenderAdapter } from './execution/render_offscreen.js';
import type { RenderAdapter } from './execution/render.js';
import type { RelayBuildOutcome } from './relay_loop.js';
import type { PdfWasm } from './wasm_typed.js';

/** Откуда воркер берёт картинки открытого документа. */
export interface BitmapSource {
  /** Метка источника в предупреждениях: `[xlsx]` или `[docx]`. */
  tag: string;
  /** Список картинок: id, MIME и длина байтов. */
  list: () => ImageInfoWire[];
  /** Байты одной картинки; ошибка здесь пропускает только её. */
  bytes: (id: number) => Uint8Array;
}

/**
 * Накопленный поток открытия: чанки копятся до `end-stream` (ADR-0031).
 *
 * `streamId` для ленивого модуля появляется асинхронно: до готовности модуля
 * здесь 0, и wasm-аппенды отложены до конца потока.
 */
export interface OpenStreamEntry {
  /** id буфера в wasm-модуле формата; 0 — модуль ещё не готов. */
  streamId: number;
  /** Поколение битмапов на старте потока: если оно уехало — открытие устарело. */
  gen: number;
  /**
   * Куда пишутся чанки, когда размер известен заранее (`open-stream` несёт
   * `totalSize`): одна аллокация на весь вход, второй копии в конце потока нет.
   */
  target: Uint8Array | null;
  /** Чанки пути без известного размера: соберутся в буфер в конце потока. */
  chunks: Uint8Array[];
  /** Сколько байт уже приехало (оно же — смещение записи в `target`). */
  received: number;
  /** Загрузка модуля формата; `null`, если модуль уже в памяти. */
  moduleReady: Promise<unknown> | null;
}

/** Форматная часть воркера: всё, что зависит от XLSX или DOCX. */
export interface FormatHooks {
  /** Формат, который обслуживает воркер. */
  readonly format: 'xlsx' | 'docx' | 'pptx';
  /** Запомнить URL-ы формата из `init` (DOCX-модуль грузится лениво). */
  configure?: (msg: InitMsg) => void;
  /** Причина, по которой открывать нельзя (нет URL модуля), или `null`. */
  openBlockedReason: () => string | null;
  /** Разобрать документ из байтов и вернуть ответ `opened`/`docx-opened`. */
  open: (msg: OpenMsg) => Promise<OutMsg>;
  /**
   * Начать приём потока: выделить буфер в модуле формата. Для ленивого модуля
   * `entry.moduleReady` получает загрузку, а `streamId` — после её завершения.
   */
  beginStream: (
    msg: OpenStreamMsg,
    entry: OpenStreamEntry,
    alive: () => boolean,
  ) => void;
  /** Дописать собранный буфер в wasm-модуль формата. */
  appendStream: (streamId: number, bytes: Uint8Array) => void;
  /** Освободить буфер потока в модуле формата. */
  freeStream: (streamId: number) => void;
  /** Разобрать поток и вернуть ответ `opened`/`docx-opened`. */
  finishStream: (streamId: number, id: number) => OutMsg;
  /** Картинки открытого документа. */
  images: () => BitmapSource;
  /** Собрать кадр в буфер вызывающего. */
  buildInto: (out: Uint8Array, req: RenderRequest) => BuildStatsWire | null;
  /** Собрать кадр в слот ring'а. */
  buildSab: (
    sab: SharedArrayBuffer,
    slotCapacity: number,
    req: RenderRequest,
  ) => BuildStatsWire | null;
  /** Отпустить разобранный документ при закрытии. */
  release: () => void;
  /** Поставить кооперативный флаг отмены в модуле формата. */
  cancel: () => void;
}

/** Отправить сообщение на main-сторону. */
export const post = (msg: OutMsg, transfer: Transferable[] = []): void => {
  (self as unknown as Worker).postMessage(msg, transfer);
};

/**
 * Ошибка на main-сторону.
 *
 * `id` адресует её конкретному запросу, `code` называет стадию. Без `id`
 * ошибка относится к воркеру целиком (инициализация): ответа не будет ни на
 * один запрос, и просмотрщик по этому признаку отклоняет все ожидающие.
 * Поля собираются условно: `exactOptionalPropertyTypes` не даёт положить
 * `undefined` в необязательное поле.
 */
export function postError(
  message: string,
  opts: { id?: number; code?: WorkerErrorCode; diagnostic?: DiagnosticWire } = {},
): void {
  const msg: WorkerErrorMsg = { type: 'error', message };
  if (opts.id !== undefined) msg.id = opts.id;
  if (opts.code !== undefined) msg.code = opts.code;
  if (opts.diagnostic !== undefined) msg.diagnostic = opts.diagnostic;
  post(msg);
}

/**
 * Код ошибки открытия: wasm помечает кооперативную отмену префиксом
 * `[cancelled]` (ADR-0032), всё остальное — обычная ошибка разбора.
 */
export function openErrorCode(e: unknown): WorkerErrorCode {
  return String(e).startsWith('[cancelled]') ? 'cancelled' : 'open';
}

/** Общее состояние и обработчики воркера одного формата. */
export class WorkerCore {
  /** `init` уже прошёл (успешно или нет): повторный — нарушение протокола. */
  private initialized = false;
  /** Выбранный путь кадра: место рендера и транспорт. */
  private plan: ExecutionPlan | null = null;
  /** Транспорт кадра: живёт от `init` до остановки воркера. */
  private transportAdapter: TransportAdapter | null = null;
  /** Рендер-адаптер: offscreen-путь рисует здесь, main-путь отправляет кадры. */
  private render: RenderAdapter | null = null;
  private slotCapacity = 0;
  /** Последний запрос на кадр: перерисовка идёт по нему, а не по сообщению. */
  private pending: RenderRequest | null = null;
  /** Формат открытого документа: по нему разбирается запрос кадра. */
  private format: 'xlsx' | 'docx' | 'pptx' | null = null;
  /** Сколько заняла сборка последнего кадра. */
  private buildMs = 0;

  /**
   * Байты открытого документа.
   *
   * Основному модулю они не нужны — документ живёт в `thread_local` уже
   * разобранным, — но экспорт работает поверх исходного файла: ленивый
   * PDF-модуль разбирает его сам, а текстовые выгрузки DOCX получают байтами.
   * Плата — размер файла в памяти воркера; отпускаем при закрытии и на новом
   * `open`.
   */
  private source: ArrayBuffer | null = null;

  /** Накопленные потоки открытия по id запроса. */
  private readonly openStreams = new Map<number, OpenStreamEntry>();

  /** Откуда грузить PDF-модуль: заполняется на `init`, до него экспорта нет. */
  private pdfUrls: PdfModuleUrls | null = null;
  /** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
  private pdfWasm: PdfWasm | null = null;
  /** Незавершённая загрузка — чтобы два клика подряд не тянули модуль дважды. */
  private pdfWasmLoading: Promise<PdfWasm> | null = null;

  /** Битмапы, зарегистрированные в painter'е. */
  private readonly bitmaps = new Map<number, ImageBitmap>();
  /**
   * Поколение набора картинок. Открытие и закрытие документа его увеличивают:
   * декодирование асинхронно, и результат отставшей загрузки не должен попасть
   * в уже сменившийся набор.
   */
  private bitmapsGen = 0;

  constructor(private readonly hooks: FormatHooks) {}

  /** Байты открытого документа: нужны форматным обработчикам экспорта. */
  get sourceBytes(): ArrayBuffer | null {
    return this.source;
  }

  /** URL-ы PDF-модуля: без них экспорт отвечает ошибкой. */
  get pdfModuleUrls(): PdfModuleUrls | null {
    return this.pdfUrls;
  }

  /**
   * Загрузить PDF-модуль по требованию. Неудача не кэшируется: следующий клик
   * попробует снова, иначе один сетевой сбой навсегда ломал бы экспорт.
   */
  async loadPdfWasm(urls: PdfModuleUrls): Promise<PdfWasm> {
    if (this.pdfWasm) return this.pdfWasm;
    this.pdfWasmLoading ??= (async () => {
      // URL динамический: модуль лежит отдельным файлом, и статический импорт
      // вернул бы его в основной бандл — ровно то, от чего мы ушли.
      const mod = (await import(/* @vite-ignore */ urls.module)) as PdfWasm;
      await mod.default(urls.binary);
      this.pdfWasm = mod;
      return mod;
    })();
    try {
      return await this.pdfWasmLoading;
    } catch (e) {
      this.pdfWasmLoading = null;
      throw e;
    }
  }

  /**
   * Обработать сообщение, общее для всех форматов.
   *
   * @returns `true`, если сообщение обработано здесь; `false` — форматный
   * обработчик воркера должен разобрать его сам.
   */
  async handle(msg: InMsg): Promise<boolean> {
    switch (msg.type) {
      case 'init': {
        if (this.initialized) {
          // Повторный init — нарушение протокола, а не сбой инициализации:
          // воркер уже работает, и терять его запросы из-за второго init нельзя.
          postError('worker already initialised', { code: 'internal' });
          return true;
        }
        // Занимаем воркер до `await`: два параллельных init иначе оба прошли бы
        // проверку и подняли два painter'а в одном потоке.
        this.initialized = true;
        try {
          await init(msg.wasmUrl);
          this.pdfUrls = msg.pdf ?? null;
          this.hooks.configure?.(msg);
          this.slotCapacity = msg.slotCapacity;

          // Одно решение о пути кадра: место рендера, транспорт и допустимость
          // пары. SAB выделяется только для воркер-пути: main всегда едет
          // буферами, и аллокация там была бы мёртвой памятью.
          const renderTarget = msg.backend ?? 'worker';
          const wantsSab = renderTarget === 'worker' && msg.transport !== 'bytes';
          const shared = wantsSab ? this.tryAllocSab(this.slotCapacity) : null;
          const plan = resolveExecutionPlan({
            render: renderTarget,
            requested: msg.transport,
            sabAvailable: shared !== null,
          });
          this.plan = plan;

          const transport = createTransportAdapter(plan.transport, {
            slotCapacity: this.slotCapacity,
            target: plan.render,
            sab: plan.transport === 'sab' ? shared : null,
            allocSab: (bytes) => this.tryAllocSab(bytes),
            buildInto: this.hooks.buildInto,
            buildSab: this.hooks.buildSab,
          });
          this.transportAdapter = transport;

          const render: RenderAdapter =
            plan.render === 'worker'
              ? new OffscreenRenderAdapter({
                  transport,
                  build: () => this.buildLocal(),
                  onTick: (stats) =>
                    post({
                      type: 'tick',
                      stats: { ...stats, buildMs: this.buildMs },
                    }),
                  onAfterResize: () => this.reregisterBitmaps(),
                })
              : new ForwardingRenderAdapter({
                  transport,
                  build: () => this.buildForward(),
                  send: (frame) => post(frame, [frame.bytes]),
                });
          this.render = render;

          // Полная инициализация до `ready`: main не должен успеть прислать
          // первый кадр раньше, чем painter и шрифты готовы.
          await render.start(msg.canvas ?? null);

          post({
            type: 'ready',
            backend: plan.render,
            transport: plan.transport,
            slotCapacity: this.slotCapacity,
            totalBytes:
              plan.transport === 'sab'
                ? sab_total_bytes(this.slotCapacity)
                : this.slotCapacity * 2,
            crossOriginIsolated:
              typeof crossOriginIsolated === 'boolean' ? crossOriginIsolated : false,
          });
        } catch (e) {
          // Ошибка без id: воркер не поднялся, ответить на запросы нечем.
          this.render?.stop();
          this.transportAdapter?.close();
          this.render = null;
          this.transportAdapter = null;
          postError(String(e), { code: 'init' });
        }
        return true;
      }
      case 'resize': {
        // В main-режиме размеры холста ставит main; адаптер сам решает, что делать.
        if (!this.render) return true;
        this.render.resize(msg.cssW, msg.cssH, msg.dpr);
        if (this.plan?.render === 'worker' && this.pending) this.render.request();
        return true;
      }
      case 'bitmap': {
        register_bitmap(msg.id, msg.bitmap);
        return true;
      }
      case 'drop-bitmap': {
        drop_bitmap(msg.id);
        return true;
      }
      case 'frame-ack': {
        // main вернул буфер после отрисовки (или сразу, если кадр вытеснен).
        this.render?.ack(msg.slot, msg.buffer);
        return true;
      }
      case 'open': {
        if (msg.format !== this.hooks.format) {
          // Формат выбирает воркер: чужое открытие не должно трогать
          // состояние формата и обязано получить внятный ответ.
          postError(`open: unsupported format '${msg.format}'`, {
            id: msg.id,
            code: 'open',
          });
          return true;
        }
        const blocked = this.hooks.openBlockedReason();
        if (blocked) {
          postError(blocked, { id: msg.id, code: 'open' });
          return true;
        }
        try {
          const response = await this.hooks.open(msg);
          this.format = this.hooks.format;
          this.pending = null;
          // Байты документа остаются: их просят экспорт в PDF и текстовые
          // выгрузки. Прежний документ отпускается на новом `open`.
          this.source = msg.bytes;
          await this.finishOpen(msg.id, response);
        } catch (e) {
          postError(String(e), { id: msg.id, code: openErrorCode(e) });
        }
        return true;
      }
      case 'open-stream': {
        if (msg.format !== this.hooks.format) {
          postError(`open-stream: unsupported format '${msg.format}'`, {
            id: msg.id,
            code: 'open',
          });
          return true;
        }
        const blocked = this.hooks.openBlockedReason();
        if (blocked) {
          postError(blocked, { id: msg.id, code: 'open' });
          return true;
        }
        try {
          // Запись в реестре появляется синхронно: сообщения воркера
          // обрабатываются асинхронно, и append/end-stream могли бы обогнать
          // await загрузки ленивого модуля.
          const entry: OpenStreamEntry = {
            streamId: 0,
            gen: this.bitmapsGen,
            target: msg.totalSize ? new Uint8Array(msg.totalSize) : null,
            chunks: [],
            received: 0,
            moduleReady: null,
          };
          this.openStreams.set(msg.id, entry);
          this.hooks.beginStream(msg, entry, () => this.openStreams.has(msg.id));
        } catch (e) {
          this.openStreams.delete(msg.id);
          postError(String(e), { id: msg.id, code: 'open' });
        }
        return true;
      }
      case 'append': {
        const entry = this.openStreams.get(msg.id);
        // Поток отменён или уже разобран: чанки-сироты пропадают молча — ответ
        // на такое открытие уже дан или не будет вовсе.
        if (!entry) return true;
        try {
          const bytes = new Uint8Array(msg.bytes);
          if (entry.target) {
            // Заявленный размер и факт обязаны совпасть: враньё content-length
            // или обрыв сети ловятся здесь, а не при сборке.
            if (entry.received + bytes.byteLength > entry.target.byteLength) {
              this.openStreams.delete(msg.id);
              this.freeEntry(entry);
              postError('stream exceeds declared size', { id: msg.id, code: 'open' });
              return true;
            }
            entry.target.set(bytes, entry.received);
          } else {
            entry.chunks.push(bytes);
          }
          entry.received += bytes.byteLength;
          // В wasm байты не уезжают: единый append всего буфера случится в
          // end-stream, когда модуль формата точно загружен.
        } catch (e) {
          postError(String(e), { id: msg.id, code: 'open' });
          this.openStreams.delete(msg.id);
        }
        return true;
      }
      case 'end-stream': {
        const entry = this.openStreams.get(msg.id);
        if (!entry) {
          postError('end-stream: unknown stream', { id: msg.id, code: 'open' });
          return true;
        }
        try {
          // Оборванный поток (читатель отменил) или пустой вход: открывать нечего.
          if (msg.aborted || entry.received === 0) {
            this.openStreams.delete(msg.id);
            this.freeEntry(entry);
            postError('stream aborted', { id: msg.id, code: 'open' });
            return true;
          }
          // Пока качались чанки, могло открыться или закрыться что-то другое.
          if (entry.gen !== this.bitmapsGen) {
            this.openStreams.delete(msg.id);
            this.freeEntry(entry);
            postError('open superseded by a newer request', { id: msg.id, code: 'open' });
            return true;
          }
          // Дожидаемся модуля формата; запись удаляется только после этого:
          // closure загрузки проверяет карту, и раннее удаление выглядело бы
          // как закрытие потока. Если поток закрыли во время ожидания —
          // streamId останется нулём, а буфер уже освободил сам closure.
          if (entry.moduleReady) await entry.moduleReady;
          if (entry.streamId === 0) {
            postError('stream aborted', { id: msg.id, code: 'open' });
            return true;
          }
          this.openStreams.delete(msg.id);
          // source: при известном размере буфер уже заполнен (второй копии
          // нет), иначе чанки собираются в контигуальный буфер — экспортам
          // нужен один ArrayBuffer, а не список.
          let source: ArrayBuffer;
          if (entry.target) {
            if (entry.received !== entry.target.byteLength) {
              this.freeEntry(entry);
              postError('stream aborted', { id: msg.id, code: 'open' });
              return true;
            }
            source = entry.target.buffer as ArrayBuffer;
          } else {
            const joined = new Uint8Array(entry.received);
            let offset = 0;
            for (const chunk of entry.chunks) {
              joined.set(chunk, offset);
              offset += chunk.byteLength;
            }
            entry.chunks.length = 0;
            source = joined.buffer;
          }
          // Единый append всего буфера: порядок чанков не зависит от гонки
          // загрузки модуля, копия в wasm живёт только на время парса — пик
          // памяти равен пути через буфер.
          this.hooks.appendStream(entry.streamId, new Uint8Array(source));
          const response = this.hooks.finishStream(entry.streamId, msg.id);
          this.format = this.hooks.format;
          this.pending = null;
          this.source = source;
          await this.finishOpen(msg.id, response);
        } catch (e) {
          postError(String(e), { id: msg.id, code: openErrorCode(e) });
        }
        return true;
      }
      case 'cancel': {
        // Кооперативная отмена (ADR-0032): флаг ставится в модуле формата.
        // Ближайшая точка проверки завершит операцию ошибкой, воркер ответит
        // кодом 'cancelled'. Флаг одноразовый: модуль гасит его сам после
        // того, как заметил, поэтому отмена не достаётся следующей операции.
        this.hooks.cancel();
        return true;
      }
      case 'render': {
        if (msg.req) this.pending = msg.req;
        if (this.pending) this.requestFrame();
        return true;
      }
      case 'export-png': {
        if (this.plan?.render === 'main') {
          // Снимок делает main: холст и painter живут там же, гонять его через
          // воркер значило бы копировать PNG лишний раз.
          postError('export-png is handled on the main thread in main mode', {
            id: msg.id,
            code: 'export',
          });
          return true;
        }
        const canvas = this.render?.canvas;
        if (!canvas) {
          postError('export-png before init', { id: msg.id, code: 'export' });
          return true;
        }
        try {
          const bytes = await exportPng(canvas);
          post({ type: 'png', id: msg.id, bytes }, [bytes.buffer]);
        } catch (e) {
          postError(String(e), { id: msg.id, code: 'export' });
        }
        return true;
      }
      case 'close': {
        this.pending = null;
        this.source = null;
        // Формат сбрасывается до сборки: `render` после закрытия — пустой ход,
        // а не кадр по исчезнувшему документу.
        this.format = null;
        this.dropBitmaps();
        this.hooks.release();
        // Незавершённые потоки: буферы в wasm-модулях больше не нужны.
        for (const entry of this.openStreams.values()) this.freeEntry(entry);
        this.openStreams.clear();
        return true;
      }
      default:
        return false;
    }
  }

  /** Освободить буфер потока, если он уже выделен. */
  private freeEntry(entry: OpenStreamEntry): void {
    if (entry.streamId === 0) return;
    this.hooks.freeStream(entry.streamId);
  }

  /** Снять с painter'а все зарегистрированные картинки. */
  private dropBitmaps(): void {
    this.bitmapsGen++;
    if (this.plan?.render === 'main') {
      // Id нового документа могут совпасть со старыми: main снимает весь набор
      // разом, до того как приедут новые картинки.
      post({ type: 'clear-bitmaps' });
      return;
    }
    for (const id of this.bitmaps.keys()) drop_bitmap(id);
    this.bitmaps.clear();
  }

  /**
   * Общий хвост открытия (оба пути — байты и поток, ADR-0031): снять картинки
   * прежнего документа, дождаться картинок нового (к первому кадру они уже в
   * painter'е), ответить и перерисовать. Отставшее открытие отвечает ошибкой:
   * иначе его промис повис бы навсегда.
   */
  private async finishOpen(id: number, response: OutMsg): Promise<void> {
    // Картинки прежнего документа больше не нужны, а id нового могут с ними совпасть.
    this.dropBitmaps();
    const gen = this.bitmapsGen;
    // Ждём картинки до ответа открытия: к первому кадру они уже в painter'е.
    await this.loadBitmaps(this.hooks.images(), gen);
    if (gen !== this.bitmapsGen) {
      // Пока грузились картинки, документ сменили или закрыли: ответа этому
      // открытию не будет.
      postError('open superseded by a newer request', { id, code: 'open' });
      return;
    }
    post(response);
    // Пока декодировались картинки, мог прийти кадр — просим показать полный.
    this.requestFrame();
  }

  /**
   * Декодировать картинки открытого документа и отдать их painter'у. Ошибка
   * одной картинки не мешает остальным и не роняет открытие документа.
   */
  private async loadBitmaps(source: BitmapSource, gen: number): Promise<void> {
    let list: ImageInfoWire[];
    try {
      list = source.list();
    } catch (e) {
      console.warn(`[${source.tag}] image list:`, String(e));
      return;
    }
    await Promise.all(
      list.map(async ({ id, mime }) => {
        try {
          const bytes = source.bytes(id);
          // Байты приходят копией из памяти wasm: буфер целиком принадлежит
          // интерфейсу, и Blob забирает его без ещё одной копии.
          const bmp = await createImageBitmap(
            new Blob([bytes.buffer as ArrayBuffer], { type: mime }),
          );
          if (gen !== this.bitmapsGen) {
            bmp.close();
            return;
          }
          if (this.plan?.render === 'main') {
            // В main-режиме painter живёт на main: картинка уезжает туда
            // переводом владения, и после этого воркер её не удерживает.
            post({ type: 'bitmap', id, bitmap: bmp }, [bmp]);
          } else {
            register_bitmap(id, bmp);
            this.bitmaps.set(id, bmp);
          }
        } catch (e) {
          console.warn(`[${source.tag}] image ${id} skipped:`, String(e));
        }
      }),
    );
  }

  /**
   * `resize_canvas` сбрасывает состояние painter'а вместе с кэшем `ImageBitmap`
   * (установка размеров холста сбрасывает и 2D-контекст). Битмапы к размеру
   * холста не привязаны, поэтому сразу возвращаем их в кэш.
   */
  private reregisterBitmaps(): void {
    if (this.plan?.render === 'main') return;
    for (const [id, bmp] of this.bitmaps) register_bitmap(id, bmp);
  }

  /**
   * Выделить ring. Неудача — не ошибка: без COOP/COEP `SharedArrayBuffer` в
   * странице просто нет, и это штатный случай, а не сбой.
   */
  private tryAllocSab(capacity: number): SharedArrayBuffer | null {
    if (typeof SharedArrayBuffer === 'undefined') return null;
    try {
      return alloc_sab(capacity) as unknown as SharedArrayBuffer;
    } catch (e) {
      console.warn('[frame] SharedArrayBuffer недоступен:', String(e));
      return null;
    }
  }

  /**
   * Собрать кадр под локальный painter.
   *
   * Разбор запроса (`frameTarget`) остаётся здесь, а не в транспорте: без
   * обязательного поля формата кадр лучше пропустить, чем оставить пул в
   * состоянии сборки. Транспорт отвечает только за буфер и его судьбу.
   */
  private buildLocal(): LocalBuildResult {
    const req = this.pending;
    // Документа ещё нет — собирать нечего; это не ошибка, а пустой ход.
    if (!req || !this.format || !this.transportAdapter) {
      return { outcome: 'idle', stats: null };
    }
    frameTarget(this.format, req);
    const result = this.transportAdapter.buildLocal(req);
    if (result.stats) this.buildMs = result.stats.buildMs;
    return result;
  }

  /** Собрать кадр для отправки на main; буферы и backpressure — у транспорта. */
  private buildForward(): RelayBuildOutcome {
    const req = this.pending;
    if (!req || !this.format || !this.transportAdapter) return { kind: 'idle' };
    frameTarget(this.format, req);
    return this.transportAdapter.buildForward(req);
  }

  /** Запросить кадр на перерисовку: цикл живёт в рендер-адаптере. */
  private requestFrame(): void {
    this.render?.request();
  }
}
