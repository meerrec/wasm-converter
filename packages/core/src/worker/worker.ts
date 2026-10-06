/// <reference lib="webworker" />
import init, {
  alloc_sab,
  drop_bitmap,
  init_painter,
  paint_display_list_sab,
  register_bitmap,
  resize_canvas,
  sab_total_bytes,
  xlsx_build_display_list_sab,
  xlsx_hit_test,
  xlsx_hyperlink_at,
  xlsx_image_bytes,
  xlsx_images,
  xlsx_open,
} from '@doc-converter/wasm';
import type {
  HyperlinkInfo,
  ImageInfo,
  InMsg,
  OutMsg,
  PaintStats,
  PdfModuleUrls,
  RenderRequest,
  SheetInfo,
} from '../protocol.js';
import { exportPng } from '../render/export_png.js';
import { startFrameLoop } from './frame_loop';

type Ctx = OffscreenCanvasRenderingContext2D;

/** Тип ленивого модуля берём у сгенерированных wasm-bindgen деклараций. */
type PdfWasm = typeof import('@doc-converter/wasm-pdf');

let ctx: Ctx | null = null;
let sab: SharedArrayBuffer | null = null;
let slotCapacity = 0;
let loop: ReturnType<typeof startFrameLoop> | null = null;
/** Последний запрос на кадр: перерисовка идёт по нему, а не по сообщению. */
let pending: RenderRequest | null = null;
/** Сколько заняла сборка последнего кадра. */
let buildMs = 0;

/**
 * Байты открытой книги. Основному модулю они не нужны — книга живёт в
 * `thread_local` уже разобранной, — но ленивый PDF-модуль разбирает книгу
 * сам и получает исходный XLSX. Плата — размер файла в памяти воркера;
 * отпускаем при закрытии книги и на новом `open`.
 */
let bookBytes: ArrayBuffer | null = null;

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

const post = (msg: OutMsg, transfer: Transferable[] = []) => {
  (self as unknown as Worker).postMessage(msg, transfer);
};

/** Битмапы, зарегистрированные в painter'е: их возвращают после ресайза и снимают при закрытии. */
const bitmaps = new Map<number, ImageBitmap>();
/**
 * Поколение набора картинок. Открытие и закрытие книги его увеличивают:
 * декодирование асинхронно, и результат отставшей загрузки не должен попасть
 * в уже сменившийся набор.
 */
let bitmapsGen = 0;

/** Снять с painter'а все зарегистрированные картинки. */
function dropBookBitmaps(): void {
  bitmapsGen++;
  for (const id of bitmaps.keys()) drop_bitmap(id);
  bitmaps.clear();
}

/**
 * Декодировать картинки открытой книги и отдать их painter'у. Ошибка одной
 * картинки не мешает остальным и не роняет открытие книги.
 */
async function loadBookBitmaps(gen: number): Promise<void> {
  let images: ImageInfo[];
  try {
    images = xlsx_images() as ImageInfo[];
  } catch (e) {
    console.warn('[xlsx] image list:', String(e));
    return;
  }
  await Promise.all(
    images.map(async ({ id, mime }) => {
      try {
        const bytes = xlsx_image_bytes(id) as Uint8Array;
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
        console.warn(`[xlsx] image ${id} skipped:`, String(e));
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

/** Окно и настройки в том виде, в каком их ждёт Rust. */
const renderArgs = (req: RenderRequest) => ({
  viewport: {
    scrollX: req.viewport.x,
    scrollY: req.viewport.y,
    width: req.viewport.w,
    height: req.viewport.h,
    scale: req.viewport.scale,
  },
  options: {
    showGrid: req.config.showGrid,
    showHeaders: req.config.showHeaders,
    dark: req.config.theme === 'dark',
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  switch (msg.type) {
    case 'init': {
      if (ctx) {
        post({ type: 'error', message: 'worker already initialised' });
        return;
      }
      try {
        await init(msg.wasmUrl);
        ctx = msg.canvas.getContext('2d', {
          alpha: true,
          desynchronized: true,
        }) as Ctx;
        if (!ctx) throw new Error('getContext("2d") returned null');

        pdfUrls = msg.pdf ?? null;
        slotCapacity = msg.slotCapacity;
        // SAB — не transferable, шарится через structured clone.
        sab = alloc_sab(slotCapacity) as unknown as SharedArrayBuffer;
        post({
          type: 'ready',
          sab,
          slotCapacity,
          totalBytes: sab_total_bytes(slotCapacity),
        });

        init_painter(ctx);
        loop = startFrameLoop({
          build: () => {
            if (!pending || !sab) return false;
            const { viewport, options } = renderArgs(pending);
            const stats = xlsx_build_display_list_sab(
              sab,
              slotCapacity,
              pending.sheet ?? 0,
              viewport,
              options,
            ) as { written: boolean; cmds: number; buildMs: number };
            buildMs = stats.buildMs;
            // Свободного слота не было — кадр пропущен намеренно.
            return stats.written;
          },
          paint: () => {
            const stats = paint_display_list_sab(sab!, slotCapacity) as Omit<
              PaintStats,
              'frameId'
            >;
            return stats;
          },
          onTick: (s) => post({ type: 'tick', stats: { ...s, buildMs } }),
        });
      } catch (e) {
        post({ type: 'error', message: String(e) });
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
        // wasm-bindgen отдаёт JsValue: разбирает его serde, тип знает только Rust.
        const sheets = xlsx_open(new Uint8Array(msg.bytes)) as SheetInfo[];
        // Байты остаются в воркере до закрытия книги: их просит ленивый
        // PDF-модуль. Прежняя книга отпускается здесь же.
        bookBytes = msg.bytes;
        pending = null;
        // Картинки прежней книги больше не нужны, а id нового файла могут с ними совпасть.
        dropBookBitmaps();
        const gen = bitmapsGen;
        // Ждём картинки до `opened`: к первому кадру они уже в painter'е.
        await loadBookBitmaps(gen);
        if (gen !== bitmapsGen) return;
        post({ type: 'opened', sheets });
        // Пока декодировались картинки, мог прийти кадр — просим показать полный.
        loop?.request();
      } catch (e) {
        post({ type: 'error', message: String(e) });
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
        const { viewport, options } = renderArgs({
          sheet: msg.sheet,
          viewport: msg.viewport,
          config: msg.config,
        });
        const cell = xlsx_hit_test(msg.sheet, viewport, msg.x, msg.y) as [number, number];
        post({ type: 'hit', id: msg.id, cell: cell ?? null });
      } catch {
        // Точка вне книги — не ошибка, а отсутствие ответа.
        post({ type: 'hit', id: msg.id, cell: null });
      }
      break;
    }
    case 'hyperlink-at': {
      try {
        const { viewport, options } = renderArgs({
          sheet: msg.sheet,
          viewport: msg.viewport,
          config: msg.config,
        });
        const link = xlsx_hyperlink_at(msg.sheet, viewport, msg.x, msg.y) as HyperlinkInfo | null;
        post({ type: 'hyperlink', id: msg.id, link: link ?? null });
      } catch {
        // Книги нет или точка описана неверно — ответ пустой, а не ошибка.
        post({ type: 'hyperlink', id: msg.id, link: null });
      }
      break;
    }
    case 'export-png': {
      if (!ctx) {
        post({ type: 'error', message: 'export-png before init' });
        break;
      }
      try {
        const bytes = await exportPng(ctx.canvas);
        post({ type: 'png', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'export-pdf': {
      const urls = pdfUrls;
      // Снимок: за время загрузки модуля книгу могли закрыть или сменить.
      const source = bookBytes;
      if (!urls) {
        post({ type: 'error', message: 'pdf module is not configured: init has no pdf urls' });
        break;
      }
      if (!source) {
        post({ type: 'error', message: 'no workbook is open' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await loadPdfWasm(urls);
        const bytes = mod.export_pdf(new Uint8Array(source), msg.sheet, msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: книга остаётся открытой.
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'close': {
      pending = null;
      bookBytes = null;
      dropBookBitmaps();
      break;
    }
  }
};
