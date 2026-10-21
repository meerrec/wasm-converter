/// <reference lib="webworker" />
// Отдельный воркер формата PPTX (ADR-0030 §е, направление «воркер на формат»):
// обслуживает одну презентацию, не трогая состояние воркера XLSX/DOCX. Painter,
// SAB-аллокатор и шрифты остаются в основном wasm-модуле, а разбор и раскладка
// презентации живут в ленивом модуле `@meerrec/wasm-pptx`. Экспорт в PDF —
// отдельный ленивый модуль `@meerrec/wasm-pdf` (просмотрщик и экспортёр
// разнесены, как у DOCX).
import init, {
  alloc_sab,
  drop_bitmap,
  font_bytes,
  init_painter,
  register_bitmap,
  resize_canvas,
  sab_total_bytes,
} from '@meerrec/wasm';
import type { BuildStatsWire, ImageInfoWire, PaintStatsWire, ViewportWire } from '../generated/wire.js';
import type {
  FrameTransport,
  InMsg,
  OutMsg,
  PdfModuleUrls,
  PptxModuleUrls,
  RenderRequest,
  WorkerErrorCode,
  WorkerErrorMsg,
} from '../protocol.js';
import { exportPng } from '../render/export_png.js';
import { frameTarget } from './frame_args.js';
import { registerPaintFonts } from './fonts.js';
import { startFrameLoop, type BuildOutcome } from './frame_loop';
import { FrameRelay } from './frame_relay.js';
import { startRelayLoop, type RelayBuildOutcome } from './relay_loop.js';
import { FramePool, pickTransport } from './transport.js';
import {
  buildPptxInto,
  buildPptxSab,
  exportPptxPdf,
  exportPptxText,
  images,
  openPptx,
  openPptxStream,
  paintBytes,
  paintSab,
  type PptxPdfWasm,
  type PptxWasm,
} from './wasm_typed.js';

type Ctx = OffscreenCanvasRenderingContext2D;

let ctx: Ctx | null = null;
/** Где рисуется кадр. Решается один раз на `init` и дальше не меняется. */
let backend: 'worker' | 'main' = 'worker';
/** `init` уже прошёл (успешно или нет): повторный — нарушение протокола. */
let initialized = false;
/** Транспорт кадра: выбирается на `init`, дальше не меняется. */
let transport: FrameTransport = 'bytes';
/** Ring SAB-пути; `null` на пути без SAB. */
let sab: SharedArrayBuffer | null = null;
/** Двойной буфер пути без SAB; `null` на SAB-пути. */
let pool: FramePool | null = null;
/** Двойной буфер main-режима: владение уезжает на main и возвращается ack'ом. */
let relay: FrameRelay | null = null;
let slotCapacity = 0;
let loop: ReturnType<typeof startFrameLoop> | null = null;
let relayLoop: ReturnType<typeof startRelayLoop> | null = null;
/** Последний запрос на кадр: перерисовка идёт по нему, а не по сообщению. */
let pending: RenderRequest | null = null;
/** Сколько заняла сборка последнего кадра. */
let buildMs = 0;

/**
 * Байты открытой презентации.
 *
 * Основному модулю они не нужны — презентация живёт в `thread_local` уже
 * разобранной, — но экспорт работает поверх исходного файла: ленивый
 * PDF-модуль разбирает его сам, а текстовые выгрузки получают байтами.
 * Плата — размер файла в памяти воркера; отпускаем при закрытии и на новом
 * `open`.
 */
let sourceBytes: ArrayBuffer | null = null;

/** Накопленный поток открытия: чанки копятся до `end-stream` (ADR-0031). */
interface OpenStream {
  /**
   * id буфера в wasm-модуле. Модуль грузится асинхронно, поэтому до его
   * готовности здесь 0 — wasm-аппенды отложены до конца потока.
   */
  streamId: number;
  /** Поколение битмапов на старте потока: если оно уехало — открытие устарело. */
  gen: number;
  /**
   * Куда пишутся чанки, когда размер известен заранее (`open-stream` несёт
   * `totalSize`): одна аллокация на весь вход, сборки второй копии в конце
   * потока нет — пик памяти воркера равен пути через буфер (ADR-0031 §3).
   */
  target: Uint8Array | null;
  /** Чанки пути без известного размера: соберутся в `sourceBytes` в конце. */
  chunks: Uint8Array[];
  /** Сколько байт уже приехало (оно же — смещение записи в `target`). */
  received: number;
  /** Загрузка PPTX-модуля. */
  moduleReady: Promise<PptxWasm> | null;
}

const openStreams = new Map<number, OpenStream>();

/** Освободить буфер потока в wasm-модуле (ADR-0031 §6). */
function freeStream(entry: OpenStream): void {
  if (entry.streamId === 0) return;
  pptxWasm?.stream_free(entry.streamId);
}

/** Откуда грузить PPTX-PDF-модуль: заполняется на `init`, до него экспорта нет. */
let pptxPdfUrls: PdfModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let pptxPdfWasm: PptxPdfWasm | null = null;
/** Незавершённая загрузка — чтобы два клика подряд не тянули модуль дважды. */
let pptxPdfWasmLoading: Promise<PptxPdfWasm> | null = null;

/**
 * Загрузить PPTX-PDF-модуль по требованию. Неудача не кэшируется: следующий
 * клик попробует снова, иначе один сетевой сбой навсегда ломал бы экспорт.
 */
async function loadPptxPdfWasm(urls: PdfModuleUrls): Promise<PptxPdfWasm> {
  if (pptxPdfWasm) return pptxPdfWasm;
  pptxPdfWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as PptxPdfWasm;
    await mod.default(urls.binary);
    pptxPdfWasm = mod;
    return mod;
  })();
  try {
    return await pptxPdfWasmLoading;
  } catch (e) {
    pptxPdfWasmLoading = null;
    throw e;
  }
}

/** Откуда грузить PPTX-модуль: заполняется на `init`, до него открытия нет. */
let pptxUrls: PptxModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let pptxWasm: PptxWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let pptxWasmLoading: Promise<PptxWasm> | null = null;

/**
 * Загрузить PPTX-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadPptxWasm(urls: PptxModuleUrls): Promise<PptxWasm> {
  if (pptxWasm) return pptxWasm;
  pptxWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as PptxWasm;
    await mod.default(urls.binary);
    pptxWasm = mod;
    return mod;
  })();
  try {
    return await pptxWasmLoading;
  } catch (e) {
    pptxWasmLoading = null;
    throw e;
  }
}

const post = (msg: OutMsg, transfer: Transferable[] = []) => {
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
function postError(
  message: string,
  opts: { id?: number; code?: WorkerErrorCode } = {},
): void {
  const msg: WorkerErrorMsg = { type: 'error', message };
  if (opts.id !== undefined) msg.id = opts.id;
  if (opts.code !== undefined) msg.code = opts.code;
  post(msg);
}

/** Битмапы, зарегистрированные в painter'е: их возвращают после ресайза и снимают при закрытии. */
const bitmaps = new Map<number, ImageBitmap>();
/**
 * Поколение набора картинок. Открытие и закрытие презентации его увеличивают:
 * декодирование асинхронно, и результат отставшей загрузки не должен попасть
 * в уже сменившийся набор.
 */
let bitmapsGen = 0;

/** Снять с painter'а все зарегистрированные картинки. */
function dropBitmaps(): void {
  bitmapsGen++;
  if (backend === 'main') {
    // Id новой презентации могут совпасть со старыми: main снимает весь набор
    // разом, до того как приедут новые картинки.
    post({ type: 'clear-bitmaps' });
    return;
  }
  for (const id of bitmaps.keys()) drop_bitmap(id);
  bitmaps.clear();
}

/**
 * Код ошибки открытия: wasm помечает кооперативную отмену префиксом
 * `[cancelled]` (ADR-0032), всё остальное — обычная ошибка разбора.
 */
function openErrorCode(e: unknown): WorkerErrorCode {
  return String(e).startsWith('[cancelled]') ? 'cancelled' : 'open';
}

/**
 * Общий хвост открытия (оба пути — байты и поток, ADR-0031): снять картинки
 * прежней презентации, дождаться картинок новой (к первому кадру они уже в
 * painter'е), ответить и перерисовать. Отставшее открытие отвечает ошибкой:
 * иначе его промис повис бы навсегда.
 */
async function finishOpen(id: number, response: OutMsg): Promise<void> {
  // Картинки прежней презентации больше не нужны, а id новой могут с ними совпасть.
  dropBitmaps();
  const gen = bitmapsGen;
  // Ждём картинки до ответа открытия: к первому кадру они уже в painter'е.
  await loadBitmaps(gen);
  if (gen !== bitmapsGen) {
    // Пока грузились картинки, презентацию сменили или закрыли: ответа этому
    // открытию не будет.
    postError('open superseded by a newer request', { id, code: 'open' });
    return;
  }
  post(response);
  // Пока декодировались картинки, мог прийти кадр — просим показать полный.
  requestFrame();
}

/**
 * Декодировать картинки открытой презентации и отдать их painter'у. Ошибка
 * одной картинки не мешает остальным и не роняет открытие презентации.
 */
async function loadBitmaps(gen: number): Promise<void> {
  let list: ImageInfoWire[];
  try {
    list = images(pptxWasm!.pptx_images());
  } catch (e) {
    console.warn('[pptx] image list:', String(e));
    return;
  }
  await Promise.all(
    list.map(async ({ id, mime }) => {
      try {
        const bytes = pptxWasm!.pptx_image_bytes(id);
        // Байты приходят копией из памяти wasm: буфер целиком принадлежит
        // интерфейсу, и Blob забирает его без ещё одной копии.
        const bmp = await createImageBitmap(
          new Blob([bytes.buffer as ArrayBuffer], { type: mime }),
        );
        if (gen !== bitmapsGen) {
          bmp.close();
          return;
        }
        if (backend === 'main') {
          // В main-режиме painter живёт на main: картинка уезжает туда
          // переводом владения, и после этого воркер её не удерживает.
          post({ type: 'bitmap', id, bitmap: bmp }, [bmp]);
        } else {
          register_bitmap(id, bmp);
          bitmaps.set(id, bmp);
        }
      } catch (e) {
        console.warn(`[pptx] image ${id} skipped:`, String(e));
      }
    }),
  );
}

/**
 * `resize_canvas` сбрасывает состояние painter'а вместе с кэшем `ImageBitmap`
 * (установка размеров холста сбрасывает и 2D-контекст). Битмапы к размеру
 * холста не привязаны, поэтому сразу возвращаем их в кэш.
 */
function reregisterBitmaps(): void {
  if (backend === 'main') return;
  for (const [id, bmp] of bitmaps) register_bitmap(id, bmp);
}

/** Отпустить разобранную презентацию прежнего открытия. */
function releaseDocument(): void {
  pptxWasm?.pptx_close();
}

/**
 * Выделить ring. Неудача — не ошибка: без COOP/COEP `SharedArrayBuffer` в
 * странице просто нет, и это штатный случай, а не сбой.
 */
function tryAllocSab(capacity: number): SharedArrayBuffer | null {
  if (typeof SharedArrayBuffer === 'undefined') return null;
  try {
    return alloc_sab(capacity) as unknown as SharedArrayBuffer;
  } catch (e) {
    console.warn('[frame] SharedArrayBuffer недоступен:', String(e));
    return null;
  }
}

/** Размер слота: у обоих путей он означает одно — сколько влезает кадр. */
function frameCapacity(): number {
  return transport === 'bytes' ? (pool?.slotCapacity ?? slotCapacity) : slotCapacity;
}

/** Вырасти под кадр длиной `bytes`. */
function growFrameBuffer(bytes: number): void {
  if (transport === 'bytes') {
    pool?.grow(bytes);
    return;
  }
  const grown = tryAllocSab(Math.max(bytes, slotCapacity * 2));
  // Памяти на новый ring не нашлось — кадр пропущен, как раньше: следующий
  // может оказаться меньше и влезет в старый слот.
  if (!grown) return;
  slotCapacity = Math.max(bytes, slotCapacity * 2);
  sab = grown;
}

/** Собрать кадр в свободный буфер выбранного транспорта. */
function buildFrame(): BuildOutcome {
  const req = pending;
  // Презентации ещё нет — собирать нечего; это не ошибка, а пустой ход.
  if (!req) return 'idle';
  const target = frameTarget('pptx', req);
  // frameTarget отдаёт объединение: воркер PPTX принимает только свой вариант.
  if (target.kind !== 'pptx') return 'idle';
  const stats =
    transport === 'bytes'
      ? buildBytes(target)
      : (() => {
          const shared = sab;
          const mod = pptxWasm;
          if (!shared || !mod) return null;
          return buildPptxSab(mod, shared, slotCapacity, target.slide, target.viewport);
        })();
  // Модуля PPTX нет — собирать кадр нечем. Путь страховочный: открытие модуль
  // уже загрузило.
  if (!stats) return 'idle';
  buildMs = stats.buildMs;
  if (stats.written) return 'built';
  // Слота не хватило: либо он занят (backpressure — кадр пропущен намеренно),
  // либо кадр больше слота. Во втором случае буфер растят: кадр, который
  // никогда не влезает, — это замерший экран.
  if (stats.bytes > frameCapacity()) growFrameBuffer(stats.bytes);
  return 'dropped';
}

/** Путь без SAB: кадр пишется в готовый буфер пула. */
function buildBytes(target: { kind: 'pptx'; slide: number; viewport: ViewportWire }): BuildStatsWire | null {
  const buffers = pool;
  const mod = pptxWasm;
  if (!buffers || !mod) return null;
  const out = buffers.beginBuild();
  const stats = buildPptxInto(mod, out, target.slide, target.viewport);
  if (stats.written) buffers.commitBuild(stats.bytes);
  else buffers.abortBuild();
  return stats;
}

/**
 * main-режим: кадр пишется в свободный буфер релея, отправляет его цикл.
 *
 * `busy` — оба буфера в полёте, ждём `frame-ack`; `retry` — кадр не влез в
 * буфер, ёмкость уже выросла и сборка повторяется. Занятость буферов сюда не
 * попадает: `beginBuild` отдаёт только свободный.
 */
function buildFrameMain(): RelayBuildOutcome {
  const req = pending;
  const buffers = relay;
  if (!req || !buffers) return { kind: 'idle' };
  const target = frameTarget('pptx', req);
  // frameTarget отдаёт объединение: воркер PPTX принимает только свой вариант.
  if (target.kind !== 'pptx') return { kind: 'idle' };
  const out = buffers.beginBuild();
  if (!out) return { kind: 'busy' };
  let stats: BuildStatsWire | null;
  try {
    const mod = pptxWasm;
    stats = mod ? buildPptxInto(mod, out, target.slide, target.viewport) : null;
  } catch (e) {
    buffers.abort();
    throw e;
  }
  // Модуля PPTX нет — собирать кадр нечем. Путь страховочный: открытие модуль
  // уже загрузило.
  if (!stats) {
    buffers.abort();
    return { kind: 'idle' };
  }
  if (!stats.written) {
    buffers.abort();
    if (stats.bytes > buffers.slotCapacity) {
      buffers.grow(stats.bytes);
      return { kind: 'retry' };
    }
    return { kind: 'busy' };
  }
  buffers.commit(stats.bytes);
  const frame = buffers.takeFrame();
  if (!frame) return { kind: 'busy' };
  return {
    kind: 'frame',
    frame: {
      slot: frame.slot,
      bytes: frame.bytes,
      len: frame.len,
      cmds: stats.cmds,
      buildMs: stats.buildMs,
    },
  };
}

/** Запросить кадр на перерисовку: какой цикл, решает режим. */
function requestFrame(): void {
  if (backend === 'main') relayLoop?.request();
  else loop?.request();
}

/** Нарисовать самый свежий готовый кадр; промежуточные транспорт отпускает сам. */
function paintFrame(): PaintStatsWire {
  if (transport === 'bytes') {
    const frame = pool?.takeLatest();
    if (!frame) return { cmds: 0, dropped: true, paintMs: 0 };
    return paintBytes(frame);
  }
  const shared = sab;
  if (!shared) return { cmds: 0, dropped: true, paintMs: 0 };
  return paintSab(shared, slotCapacity);
}

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  switch (msg.type) {
    case 'init': {
      if (initialized) {
        // Повторный init — нарушение протокола, а не сбой инициализации:
        // воркер уже работает, и терять его запросы из-за второго init нельзя.
        postError('worker already initialised', { code: 'internal' });
        return;
      }
      // Занимаем воркер до `await`: два параллельных init иначе оба прошли бы
      // проверку и подняли два painter'а в одном потоке.
      initialized = true;
      try {
        await init(msg.wasmUrl);
        backend = msg.backend ?? 'worker';
        pptxPdfUrls = msg.pptxPdf ?? null;
        pptxUrls = msg.pptx ?? null;
        slotCapacity = msg.slotCapacity;

        if (backend === 'main') {
          // В main-режиме воркер только разбирает и собирает кадры: контекста
          // у него нет, шрифты и painter живут на main, SAB не нужен.
          if (msg.transport === 'sab') {
            throw new Error(
              "renderMode 'main' несовместим с transport 'sab': кадры едут буферами",
            );
          }
          transport = 'bytes';
          relay = new FrameRelay(slotCapacity);
          post({
            type: 'ready',
            backend: 'main',
            transport,
            slotCapacity,
            totalBytes: slotCapacity * 2,
            crossOriginIsolated:
              typeof crossOriginIsolated === 'boolean' ? crossOriginIsolated : false,
          });
          relayLoop = startRelayLoop({
            build: buildFrameMain,
            send: (frame) => post(frame, [frame.bytes]),
          });
          break;
        }

        // Сабсеты Carlito — до первого кадра: Chromium запоминает промах по
        // строке `ctx.font` на всю сессию, и нарисованное подставленным лицом
        // осталось бы таким же и после регистрации. Иначе предпросмотр
        // разойдётся с PDF, куда встроены те же сабсеты.
        await registerPaintFonts({ font_bytes });
        const canvas = msg.canvas;
        if (!canvas) throw new Error('init: worker backend requires canvas');
        ctx = canvas.getContext('2d', {
          alpha: true,
          desynchronized: true,
        }) as Ctx;
        if (!ctx) throw new Error('getContext("2d") returned null');

        // SAB — не transferable, шарится через structured clone; живёт он в
        // воркере, поэтому на main уезжает не он, а решение о транспорте.
        const shared = msg.transport === 'bytes' ? null : tryAllocSab(slotCapacity);
        transport = pickTransport(msg.transport, shared);
        sab = transport === 'sab' ? shared : null;
        if (transport === 'bytes') pool = new FramePool(slotCapacity);
        post({
          type: 'ready',
          backend: 'worker',
          transport,
          slotCapacity,
          totalBytes: transport === 'sab' ? sab_total_bytes(slotCapacity) : slotCapacity * 2,
          crossOriginIsolated:
            typeof crossOriginIsolated === 'boolean' ? crossOriginIsolated : false,
        });

        init_painter(ctx);
        loop = startFrameLoop({
          build: buildFrame,
          paint: paintFrame,
          onTick: (s) => post({ type: 'tick', stats: { ...s, buildMs } }),
        });
      } catch (e) {
        // Ошибка без id: воркер не поднялся, ответить на запросы нечем.
        postError(String(e), { code: 'init' });
      }
      break;
    }
    case 'resize': {
      // В main-режиме размеры холста ставит main; воркеру тут делать нечего.
      if (backend === 'main' || !ctx) return;
      resize_canvas(msg.cssW, msg.cssH, msg.dpr);
      reregisterBitmaps();
      if (pending) loop?.request();
      break;
    }
    case 'bitmap': {
      register_bitmap(msg.id, msg.bitmap);
      break;
    }
    case 'drop-bitmap': {
      drop_bitmap(msg.id);
      break;
    }
    case 'frame-ack': {
      // main вернул буфер после отрисовки (или сразу, если кадр вытеснен).
      relay?.adopt(msg.slot, msg.buffer);
      relayLoop?.ack();
      break;
    }
    case 'open': {
      try {
        const urls = pptxUrls;
        if (!urls) {
          postError('pptx module is not configured: init has no pptx urls', {
            id: msg.id,
            code: 'open',
          });
          break;
        }
        // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
        const mod = await loadPptxWasm(urls);
        const slides = openPptx(mod, new Uint8Array(msg.bytes));
        releaseDocument();
        pending = null;
        // Байты остаются: их просят экспорт в PDF и текстовые выгрузки.
        sourceBytes = msg.bytes;
        await finishOpen(msg.id, { type: 'pptx-opened', id: msg.id, slides });
      } catch (e) {
        postError(String(e), { id: msg.id, code: openErrorCode(e) });
      }
      break;
    }
    case 'open-stream': {
      try {
        const urls = pptxUrls;
        if (!urls) {
          postError('pptx module is not configured: init has no pptx urls', {
            id: msg.id,
            code: 'open',
          });
          break;
        }
        // Запись в реестре появляется синхронно: сообщения воркера
        // обрабатываются асинхронно, и append/end-stream могли бы обогнать
        // await загрузки PPTX-модуля.
        const entry: OpenStream = {
          streamId: 0,
          gen: bitmapsGen,
          target: msg.totalSize ? new Uint8Array(msg.totalSize) : null,
          chunks: [],
          received: 0,
          moduleReady: null,
        };
        openStreams.set(msg.id, entry);
        entry.moduleReady = loadPptxWasm(urls).then((mod) => {
          // id буфера выдаётся самой записи, а не через поиск по карте:
          // end-stream к этому моменту запись уже мог удалить.
          entry.streamId = mod.stream_new();
          // Поток могли закрыть, пока грузился модуль: разбирать буфер некому.
          if (!openStreams.has(msg.id)) {
            mod.stream_free(entry.streamId);
            entry.streamId = 0;
          }
          return mod;
        });
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'open' });
      }
      break;
    }
    case 'append': {
      const entry = openStreams.get(msg.id);
      // Поток отменён или уже разобран: чанки-сироты пропадают молча — ответ
      // на такое открытие уже дан или не будет вовсе.
      if (!entry) break;
      try {
        const bytes = new Uint8Array(msg.bytes);
        if (entry.target) {
          // Заявленный размер и факт обязаны совпасть: враньё content-length
          // или обрыв сети ловятся здесь, а не при сборке.
          if (entry.received + bytes.byteLength > entry.target.byteLength) {
            openStreams.delete(msg.id);
            freeStream(entry);
            postError('stream exceeds declared size', { id: msg.id, code: 'open' });
            break;
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
        openStreams.delete(msg.id);
      }
      break;
    }
    case 'end-stream': {
      const entry = openStreams.get(msg.id);
      if (!entry) {
        postError('end-stream: unknown stream', { id: msg.id, code: 'open' });
        break;
      }
      try {
        // Оборванный поток (читатель отменил) или пустой вход: открывать нечего.
        if (msg.aborted || entry.received === 0) {
          openStreams.delete(msg.id);
          freeStream(entry);
          postError('stream aborted', { id: msg.id, code: 'open' });
          break;
        }
        // Пока качались чанки, могло открыться или закрыться что-то другое.
        if (entry.gen !== bitmapsGen) {
          openStreams.delete(msg.id);
          freeStream(entry);
          postError('open superseded by a newer request', { id: msg.id, code: 'open' });
          break;
        }
        // Дожидаемся модуля формата; запись удаляется только после этого:
        // closure загрузки проверяет карту, и раннее удаление выглядело бы
        // как закрытие потока. Если поток закрыли во время ожидания —
        // streamId останется нулём, а буфер уже освободил сам closure.
        const mod = entry.moduleReady ? await entry.moduleReady : null;
        if (entry.streamId === 0) {
          postError('stream aborted', { id: msg.id, code: 'open' });
          break;
        }
        openStreams.delete(msg.id);
        // sourceBytes: при известном размере буфер уже заполнен (второй копии
        // нет), иначе чанки собираются в контигуальный буфер — экспортам нужен
        // один ArrayBuffer, а не список.
        let source: ArrayBuffer;
        if (entry.target) {
          if (entry.received !== entry.target.byteLength) {
            freeStream(entry);
            postError('stream aborted', { id: msg.id, code: 'open' });
            break;
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
        mod?.stream_append(entry.streamId, new Uint8Array(source));
        const slides = openPptxStream(mod!, entry.streamId);
        releaseDocument();
        pending = null;
        sourceBytes = source;
        await finishOpen(msg.id, { type: 'pptx-opened', id: msg.id, slides });
      } catch (e) {
        postError(String(e), { id: msg.id, code: openErrorCode(e) });
      }
      break;
    }
    case 'cancel': {
      // Кооперативная отмена (ADR-0032): флаг ставится в модуле презентации.
      // Ближайшая точка проверки (между слайдами раскладки) завершит операцию
      // ошибкой, воркер ответит кодом 'cancelled'. Флаг одноразовый: модуль
      // гасит его сам после того, как заметил, поэтому отмена не достаётся
      // следующей операции.
      pptxWasm?.set_cancel(true);
      break;
    }
    case 'render': {
      if (msg.req) pending = msg.req;
      if (pending) requestFrame();
      break;
    }
    case 'export-png': {
      if (backend === 'main') {
        // Снимок делает main: холст и painter живут там же, гонять его через
        // воркер значило бы копировать PNG лишний раз.
        postError('export-png is handled on the main thread in main mode', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!ctx) {
        postError('export-png before init', { id: msg.id, code: 'export' });
        break;
      }
      try {
        const bytes = await exportPng(ctx.canvas);
        post({ type: 'png', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-pptx-pdf': {
      const urls = pptxPdfUrls;
      // Снимок: за время загрузки модуля презентацию могли закрыть или сменить.
      const source = sourceBytes;
      if (!urls) {
        postError('pptx-pdf module is not configured: init has no pptxPdf urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no pptx presentation is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await loadPptxPdfWasm(urls);
        const bytes = exportPptxPdf(mod, new Uint8Array(source), msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: презентация остаётся открытой.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-pptx-text': {
      const urls = pptxUrls;
      const source = sourceBytes;
      if (!urls) {
        postError('pptx module is not configured: init has no pptx urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no pptx presentation is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Модуль загружен ещё открытием презентации; здесь он уже в памяти.
        const mod = await loadPptxWasm(urls);
        const text = exportPptxText(mod, new Uint8Array(source), msg.format);
        post({ type: 'pptx-text', id: msg.id, text });
      } catch (e) {
        // Разбор мог упасть и здесь: презентация остаётся открытой, клиент получает ошибку.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'close': {
      pending = null;
      sourceBytes = null;
      dropBitmaps();
      releaseDocument();
      // Незавершённые потоки: буферы в wasm-модуле больше не нужны.
      for (const entry of openStreams.values()) freeStream(entry);
      openStreams.clear();
      break;
    }
  }
};
