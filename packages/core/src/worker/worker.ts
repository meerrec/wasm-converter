/// <reference lib="webworker" />
import init, {
  alloc_sab,
  drop_bitmap,
  font_bytes,
  init_painter,
  register_bitmap,
  resize_canvas,
  sab_total_bytes,
  xlsx_image_bytes,
  xlsx_images,
} from '@meerrec/wasm';
import type { BuildStatsWire, ImageInfoWire, PaintStatsWire } from '../generated/wire.js';
import type {
  DocxModuleUrls,
  FrameTransport,
  InMsg,
  OutMsg,
  PdfModuleUrls,
  RenderRequest,
  WorkerErrorCode,
  WorkerErrorMsg,
} from '../protocol.js';
import { exportPng } from '../render/export_png.js';
import { frameTarget, toViewport, type FrameTarget, type OpenFormat } from './frame_args.js';
import { registerPaintFonts } from './fonts.js';
import { startFrameLoop, type BuildOutcome } from './frame_loop';
import { FramePool, pickTransport } from './transport.js';
import {
  buildDocxInto,
  buildDocxSab,
  buildXlsxInto,
  buildXlsxSab,
  exportDocxPdf,
  exportPdf,
  exportPdfBook,
  hitTest,
  hyperlinkAt,
  images,
  openDocx,
  openXlsx,
  paintBytes,
  paintSab,
  type DocxWasm,
  type PdfWasm,
} from './wasm_typed.js';

type Ctx = OffscreenCanvasRenderingContext2D;

let ctx: Ctx | null = null;
/** Транспорт кадра: выбирается на `init`, дальше не меняется. */
let transport: FrameTransport = 'bytes';
/** Ring SAB-пути; `null` на пути без SAB. */
let sab: SharedArrayBuffer | null = null;
/** Двойной буфер пути без SAB; `null` на SAB-пути. */
let pool: FramePool | null = null;
let slotCapacity = 0;
let loop: ReturnType<typeof startFrameLoop> | null = null;
/** Последний запрос на кадр: перерисовка идёт по нему, а не по сообщению. */
let pending: RenderRequest | null = null;
/** Формат открытого документа: по нему разбирается запрос кадра. */
let format: OpenFormat = null;
/** Сколько заняла сборка последнего кадра. */
let buildMs = 0;

/**
 * Байты открытого документа: книги XLSX или документа DOCX.
 *
 * Основному модулю они не нужны — документ живёт в `thread_local` уже
 * разобранным, — но экспорт работает поверх исходного файла: ленивый
 * PDF-модуль разбирает его сам, а текстовые выгрузки DOCX получают байтами.
 * Плата — размер файла в памяти воркера; отпускаем при закрытии и на новом
 * `open`.
 */
let sourceBytes: ArrayBuffer | null = null;

/** Откуда грузить PDF-модуль: заполняется на `init`, до него экспорта нет. */
let pdfUrls: PdfModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let pdfWasm: PdfWasm | null = null;
/** Незавершённая загрузка — чтобы два клика подряд не тянули модуль дважды. */
let pdfWasmLoading: Promise<PdfWasm> | null = null;

/**
 * Загрузить PDF-модуль по требованию. Неудача не кэшируется: следующий клик
 * попробует снова, иначе один сетевой сбой навсегда ломал бы экспорт.
 */
async function loadPdfWasm(urls: PdfModuleUrls): Promise<PdfWasm> {
  if (pdfWasm) return pdfWasm;
  pdfWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as PdfWasm;
    await mod.default(urls.binary);
    pdfWasm = mod;
    return mod;
  })();
  try {
    return await pdfWasmLoading;
  } catch (e) {
    pdfWasmLoading = null;
    throw e;
  }
}

/** Откуда грузить DOCX-модуль: заполняется на `init`, до него экспорта нет. */
let docxUrls: DocxModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let docxWasm: DocxWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let docxWasmLoading: Promise<DocxWasm> | null = null;

/**
 * Загрузить DOCX-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadDocxWasm(urls: DocxModuleUrls): Promise<DocxWasm> {
  if (docxWasm) return docxWasm;
  docxWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as DocxWasm;
    await mod.default(urls.binary);
    docxWasm = mod;
    return mod;
  })();
  try {
    return await docxWasmLoading;
  } catch (e) {
    docxWasmLoading = null;
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
 * Поколение набора картинок. Открытие и закрытие документа его увеличивают:
 * декодирование асинхронно, и результат отставшей загрузки не должен попасть
 * в уже сменившийся набор.
 */
let bitmapsGen = 0;

/** Снять с painter'а все зарегистрированные картинки. */
function dropBitmaps(): void {
  bitmapsGen++;
  for (const id of bitmaps.keys()) drop_bitmap(id);
  bitmaps.clear();
}

/**
 * Откуда воркер берёт картинки открытого документа: у книги — функции
 * основного модуля, у документа DOCX — ленивого, который загружается только
 * при открытии `.docx`.
 */
interface BitmapSource {
  /** Метка источника в предупреждениях: `[xlsx]` или `[docx]`. */
  tag: string;
  /** Список картинок: id, MIME и длина байтов. */
  list: () => ImageInfoWire[];
  /** Байты одной картинки; ошибка здесь пропускает только её. */
  bytes: (id: number) => Uint8Array;
}

/**
 * Декодировать картинки открытого документа и отдать их painter'у. Ошибка
 * одной картинки не мешает остальным и не роняет открытие документа.
 */
async function loadBitmaps(source: BitmapSource, gen: number): Promise<void> {
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
        if (gen !== bitmapsGen) {
          bmp.close();
          return;
        }
        register_bitmap(id, bmp);
        bitmaps.set(id, bmp);
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
function reregisterBitmaps(): void {
  for (const [id, bmp] of bitmaps) register_bitmap(id, bmp);
}

/**
 * Отпустить разобранный документ прежнего формата.
 *
 * Книга XLSX живёт в wasm до следующего `xlsx_open`, а у DOCX память держат
 * раскладка и реестр шрифтов: отпускает их только `docx_close` ленивого модуля.
 */
function releaseDocument(): void {
  if (format === 'docx') docxWasm?.docx_close();
  format = null;
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

/** Путь без SAB: кадр пишется в готовый буфер пула. */
function buildBytes(target: FrameTarget): BuildStatsWire | null {
  const buffers = pool;
  if (!buffers) return null;
  const out = buffers.beginBuild();
  let stats: BuildStatsWire | undefined;
  if (target.kind === 'docx') {
    const mod = docxWasm;
    stats = mod ? buildDocxInto(mod, out, target.viewport, target.gap) : undefined;
  } else {
    stats = buildXlsxInto(out, target.sheet, target.viewport, target.options);
  }
  if (!stats) {
    buffers.abortBuild();
    return null;
  }
  if (stats.written) buffers.commitBuild(stats.bytes);
  else buffers.abortBuild();
  return stats;
}

/** SAB-путь: кадр пишется прямо в слот ring'а. */
function buildSab(target: FrameTarget): BuildStatsWire | null {
  const shared = sab;
  if (!shared) return null;
  if (target.kind === 'docx') {
    const mod = docxWasm;
    return mod ? buildDocxSab(mod, shared, slotCapacity, target.viewport, target.gap) : null;
  }
  return buildXlsxSab(shared, slotCapacity, target.sheet, target.viewport, target.options);
}

/** Собрать кадр в свободный буфер выбранного транспорта. */
function buildFrame(): BuildOutcome {
  const req = pending;
  // Документа ещё нет — собирать нечего; это не ошибка, а пустой ход.
  if (!req || !format) return 'idle';
  const target = frameTarget(format, req);
  const stats = transport === 'bytes' ? buildBytes(target) : buildSab(target);
  // Модуля DOCX нет — собирать кадр нечем. Путь страховочный: открытие модуль
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
      if (ctx) {
        // Повторный init — нарушение протокола, а не сбой инициализации:
        // воркер уже работает, и терять его запросы из-за второго init нельзя.
        postError('worker already initialised', { code: 'internal' });
        return;
      }
      try {
        await init(msg.wasmUrl);
        // Сабсеты Carlito — до первого кадра: Chromium запоминает промах по
        // строке `ctx.font` на всю сессию, и нарисованное подставленным лицом
        // осталось бы таким же и после регистрации. Иначе предпросмотр
        // разойдётся с PDF, куда встроены те же сабсеты.
        await registerPaintFonts({ font_bytes });
        ctx = msg.canvas.getContext('2d', {
          alpha: true,
          desynchronized: true,
        }) as Ctx;
        if (!ctx) throw new Error('getContext("2d") returned null');

        pdfUrls = msg.pdf ?? null;
        docxUrls = msg.docx ?? null;
        slotCapacity = msg.slotCapacity;
        // SAB — не transferable, шарится через structured clone; живёт он в
        // воркере, поэтому на main уезжает не он, а решение о транспорте.
        const shared = msg.transport === 'bytes' ? null : tryAllocSab(slotCapacity);
        transport = pickTransport(msg.transport, shared);
        sab = transport === 'sab' ? shared : null;
        if (transport === 'bytes') pool = new FramePool(slotCapacity);
        post({
          type: 'ready',
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
      if (!ctx) return;
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
    case 'open': {
      try {
        // Формат выбирает и былл-функцию разбора, и форму ответа. Открытие
        // отдаёт JsValue: разбирает его serde, тип знает только Rust.
        if (msg.format === 'docx') {
          const urls = docxUrls;
          if (!urls) {
            postError('docx module is not configured: init has no docx urls', {
              id: msg.id,
              code: 'open',
            });
            break;
          }
          // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
          const mod = await loadDocxWasm(urls);
          const pages = openDocx(mod, new Uint8Array(msg.bytes));
          format = 'docx';
          pending = null;
          // Байты документа остаются: их просят экспорт в PDF (тамошний модуль
          // разбирает документ сам) и текстовые выгрузки. Прежняя книга
          // отпускается здесь же.
          sourceBytes = msg.bytes;
          // Картинки прежнего документа больше не нужны, а id нового могут с ними совпасть.
          dropBitmaps();
          const gen = bitmapsGen;
          // Ждём картинки до `docx-opened`: к первому кадру они уже в painter'е.
          await loadBitmaps(
            {
              tag: 'docx',
              list: () => images(mod.docx_images()),
              bytes: (id) => mod.docx_image_bytes(id),
            },
            gen,
          );
          if (gen !== bitmapsGen) {
            // Пока грузились картинки, документ сменили или закрыли: ответа
            // этому открытию не будет, и его промис повис бы без этой ошибки.
            postError('open superseded by a newer request', { id: msg.id, code: 'open' });
            break;
          }
          post({ type: 'docx-opened', id: msg.id, pages });
          loop?.request();
          break;
        }
        const sheets = openXlsx(new Uint8Array(msg.bytes));
        // Раскладку прежнего документа отпускаем: книга займёт место в wasm.
        releaseDocument();
        format = 'xlsx';
        // Байты остаются в воркере до закрытия книги: их просит ленивый
        // PDF-модуль. Прежняя книга отпускается здесь же.
        sourceBytes = msg.bytes;
        pending = null;
        // Картинки прежней книги больше не нужны, а id нового файла могут с ними совпасть.
        dropBitmaps();
        const gen = bitmapsGen;
        // Ждём картинки до `opened`: к первому кадру они уже в painter'е.
        await loadBitmaps(
          {
            tag: 'xlsx',
            list: () => images(xlsx_images()),
            bytes: (id) => xlsx_image_bytes(id),
          },
          gen,
        );
        if (gen !== bitmapsGen) {
          // См. DOCX-ветку: отставшее открытие обязано завершиться ошибкой.
          postError('open superseded by a newer request', { id: msg.id, code: 'open' });
          break;
        }
        post({ type: 'opened', id: msg.id, sheets });
        // Пока декодировались картинки, мог прийти кадр — просим показать полный.
        loop?.request();
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'open' });
      }
      break;
    }
    case 'render': {
      if (msg.req) pending = msg.req;
      if (pending) loop?.request();
      break;
    }
    case 'hit-test': {
      try {
        const viewport = toViewport(msg.viewport);
        const cell = hitTest(msg.sheet, viewport, msg.x, msg.y);
        post({ type: 'hit', id: msg.id, cell });
      } catch {
        // Точка вне книги — не ошибка, а отсутствие ответа.
        post({ type: 'hit', id: msg.id, cell: null });
      }
      break;
    }
    case 'hyperlink-at': {
      try {
        const viewport = toViewport(msg.viewport);
        const link = hyperlinkAt(msg.sheet, viewport, msg.x, msg.y);
        post({ type: 'hyperlink', id: msg.id, link });
      } catch {
        // Книги нет или точка описана неверно — ответ пустой, а не ошибка.
        post({ type: 'hyperlink', id: msg.id, link: null });
      }
      break;
    }
    case 'export-png': {
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
    case 'export-pdf': {
      const urls = pdfUrls;
      // Снимок: за время загрузки модуля книгу могли закрыть или сменить.
      const source = sourceBytes;
      if (!urls) {
        postError('pdf module is not configured: init has no pdf urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no workbook is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await loadPdfWasm(urls);
        const bytes = msg.options.allSheets
          ? exportPdfBook(mod, new Uint8Array(source), msg.options)
          : exportPdf(mod, new Uint8Array(source), msg.sheet, msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: книга остаётся открытой.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-docx-pdf': {
      const urls = pdfUrls;
      // Снимок: за время загрузки модуля документ могли закрыть или сменить.
      const source = sourceBytes;
      if (!urls) {
        postError('pdf module is not configured: init has no pdf urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (format !== 'docx' || !source) {
        postError('no docx document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await loadPdfWasm(urls);
        const bytes = exportDocxPdf(mod, new Uint8Array(source), msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: документ остаётся открытым.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-docx-text': {
      const urls = docxUrls;
      const source = sourceBytes;
      if (!urls) {
        postError('docx module is not configured: init has no docx urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (format !== 'docx' || !source) {
        postError('no docx document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Модуль загружен ещё открытием документа; здесь он уже в памяти.
        const mod = await loadDocxWasm(urls);
        const bytes = new Uint8Array(source);
        const text =
          msg.format === 'markdown'
            ? mod.docx_export_markdown(bytes)
            : msg.format === 'html'
              ? mod.docx_export_html(bytes)
              : mod.docx_export_text(bytes);
        post({ type: 'docx-text', id: msg.id, text });
      } catch (e) {
        // Разбор мог упасть и здесь: документ остаётся открытым, клиент получает ошибку.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'close': {
      pending = null;
      sourceBytes = null;
      dropBitmaps();
      releaseDocument();
      break;
    }
  }
};
