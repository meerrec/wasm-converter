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
  DocxModuleUrls,
  HyperlinkInfo,
  ImageInfo,
  InMsg,
  OutMsg,
  PageInfo,
  PaintStats,
  PdfModuleUrls,
  RenderRequest,
  SheetInfo,
} from '../protocol.js';
import { exportPng } from '../render/export_png.js';
import { frameTarget, toViewport, type OpenFormat } from './frame_args.js';
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

/** Тип ленивого DOCX-модуля берём у сгенерированных wasm-bindgen деклараций. */
type DocxWasm = typeof import('@doc-converter/wasm-docx');

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
        docxUrls = msg.docx ?? null;
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
            const target = frameTarget(format, pending);
            const stats = (
              target.kind === 'docx'
                ? docxWasm?.docx_build_display_list_sab(
                    sab,
                    slotCapacity,
                    target.page,
                    target.viewport,
                  )
                : xlsx_build_display_list_sab(
                    sab,
                    slotCapacity,
                    target.sheet,
                    target.viewport,
                    target.options,
                  )
            ) as { written: boolean; cmds: number; buildMs: number } | undefined;
            // Модуля DOCX нет — собирать кадр нечем: он дропается, как и при
            // отсутствии свободного слота. Путь страховочный: открытие модуль
            // уже загрузило.
            if (!stats) return false;
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
        // Формат выбирает и былл-функцию разбора, и форму ответа. Открытие
        // отдаёт JsValue: разбирает его serde, тип знает только Rust.
        if (msg.format === 'docx') {
          const urls = docxUrls;
          if (!urls) {
            post({
              type: 'error',
              message: 'docx module is not configured: init has no docx urls',
            });
            break;
          }
          // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
          const mod = await loadDocxWasm(urls);
          const pages = mod.docx_open_bytes(new Uint8Array(msg.bytes)) as PageInfo[];
          format = 'docx';
          pending = null;
          // Байты документа остаются: их просят экспорт в PDF (тамошний модуль
          // разбирает документ сам) и текстовые выгрузки. Прежняя книга
          // отпускается здесь же.
          sourceBytes = msg.bytes;
          // Картинки прежней книги больше не нужны, а id нового документа могут с ними совпасть.
          dropBookBitmaps();
          post({ type: 'docx-opened', pages });
          loop?.request();
          break;
        }
        const sheets = xlsx_open(new Uint8Array(msg.bytes)) as SheetInfo[];
        // Раскладку прежнего документа отпускаем: книга займёт место в wasm.
        releaseDocument();
        format = 'xlsx';
        // Байты остаются в воркере до закрытия книги: их просит ленивый
        // PDF-модуль. Прежняя книга отпускается здесь же.
        sourceBytes = msg.bytes;
        pending = null;
        // Картинки прежней книги больше не нужны, а id нового файла могут с ними совпасть.
        dropBookBitmaps();
        const gen = bitmapsGen;
        // Ждём картинки до `opened`: к первому кадру они уже в painter'е.
        await loadBookBitmaps(gen);
        if (gen !== bitmapsGen) break;
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
        const viewport = toViewport(msg.viewport);
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
        const viewport = toViewport(msg.viewport);
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
      const source = sourceBytes;
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
        const bytes = msg.options.allSheets
          ? mod.export_pdf_book(new Uint8Array(source), msg.options)
          : mod.export_pdf(new Uint8Array(source), msg.sheet, msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: книга остаётся открытой.
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'export-docx-pdf': {
      const urls = pdfUrls;
      // Снимок: за время загрузки модуля документ могли закрыть или сменить.
      const source = sourceBytes;
      if (!urls) {
        post({ type: 'error', message: 'pdf module is not configured: init has no pdf urls' });
        break;
      }
      if (format !== 'docx' || !source) {
        post({ type: 'error', message: 'no docx document is open' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await loadPdfWasm(urls);
        const bytes = mod.export_docx_pdf(new Uint8Array(source), msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: документ остаётся открытым.
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'export-docx-text': {
      const urls = docxUrls;
      const source = sourceBytes;
      if (!urls) {
        post({ type: 'error', message: 'docx module is not configured: init has no docx urls' });
        break;
      }
      if (format !== 'docx' || !source) {
        post({ type: 'error', message: 'no docx document is open' });
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
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'close': {
      pending = null;
      sourceBytes = null;
      dropBookBitmaps();
      releaseDocument();
      break;
    }
  }
};
