import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  TextExportFormat,
  DocxTextMsg,
  InMsg,
  OutMsg,
  PdfMsg,
  PdfOptions,
} from '../src/protocol.js';

/**
 * Экспорт DOCX: PDF-выгрузка обязана идти в ленивый pdf-модуль и не грузить его
 * раньше первого клика, текстовая — в уже загруженный docx-модуль и не трогать
 * pdf-модуль вовсе. Заглушки `test/fixtures/*-module.js` — настоящие файлы,
 * которые воркер грузит рантайм-URL из `init` тем же динамическим импортом,
 * что и боевые модули: `vi.mock` подменил бы модуль, но не проверил бы, что до
 * первого экспорта его никто не грузит.
 */

/** Ответы основного wasm-модуля: настоящего `@meerrec/wasm` в тесте нет. */
const wasm = vi.hoisted(() => {
  return {
    init: vi.fn(async () => {}),
    alloc_sab: vi.fn(() => ({})),
    sab_total_bytes: vi.fn(() => 4096),
    init_painter: vi.fn(),
    resize_canvas: vi.fn(),
    register_bitmap: vi.fn(),
    drop_bitmap: vi.fn(() => true),
    font_bytes: vi.fn(() => new Uint8Array(4)),
    paint_display_list_sab: vi.fn(() => ({ cmds: 1, dropped: false, paintMs: 0 })),
    xlsx_build_display_list_sab: vi.fn(
      (_sab: unknown, _slot: number, _sheet: number, _viewport: unknown, _options: unknown) => ({
        written: true,
        cmds: 1,
        buildMs: 0,
      }),
    ),
    xlsx_hit_test: vi.fn(() => null),
    xlsx_hyperlink_at: vi.fn(() => null),
    xlsx_open: vi.fn(() => []),
    xlsx_images: vi.fn(() => []),
    xlsx_image_bytes: vi.fn(() => new Uint8Array()),
  };
});

vi.mock('@meerrec/wasm', () => ({ default: wasm.init, ...wasm }));

/**
 * Отметки заглушек: вычисление модуля отмечается в `loads`, `init()` — в
 * `initCalls`, каждый вызов функции — в `calls`. Пока объекта нет, модуль не
 * грузили и его функции не звали — этим и проверяется ленивость.
 */
interface Stub {
  loads: number;
  initCalls: number;
  calls: Record<string, unknown[][]>;
}

const pdfStub = (): Stub | undefined => (globalThis as { __pdfStub?: Stub }).__pdfStub;
const docxStub = (): Stub | undefined => (globalThis as { __docxStub?: Stub }).__docxStub;

/** Вызовы функции заглушки: аргументы каждого вызова. */
const pdfCalls = (name: string): unknown[][] => pdfStub()?.calls[name] ?? [];
const docxCalls = (name: string): unknown[][] => docxStub()?.calls[name] ?? [];

/** URL заглушек — те же пути, какими их увидит воркер: рантайм-строками. */
const DOCX_URL = new URL('./fixtures/docx-module.js', import.meta.url).href;
const PDF_URL = new URL('./fixtures/pdf-module.js', import.meta.url).href;
/** Бинарник заглушка не читает: важно лишь, что воркер передал его в `init()`. */
const PDF_BINARY_URL = new URL('./fixtures/pdf-module.wasm', import.meta.url).href;

type Handler = (ev: { data: InMsg }) => Promise<void>;

interface SelfStub {
  onmessage: Handler | null;
  postMessage: (msg: OutMsg, transfer?: Transferable[]) => void;
}

let handler: Handler;
let posted: OutMsg[];
/** Списки перевода владения: по ним видно, что байты PDF ушли без копии. */
let transfers: (Transferable[] | undefined)[];

const SLOT_CAPACITY = 1024;

const initMsg = (withUrls: { pdf?: boolean; docx?: boolean }): InMsg => ({
  type: 'init',
  canvas: { getContext: () => ({}) } as unknown as OffscreenCanvas,
  pdf: withUrls.pdf ? { module: PDF_URL, binary: PDF_BINARY_URL } : undefined,
  docx: withUrls.docx ? { module: DOCX_URL } : undefined,
  slotCapacity: SLOT_CAPACITY,
});

/** Байты файла: непустые и различимые по метке — видно, чей файл ушёл в экспорт. */
const fileBytes = (mark: number): Uint8Array =>
  new Uint8Array([0x50, 0x4b, 0x03, 0x04, mark, mark + 1, 0xee]);

const openMsg = (bytes: Uint8Array, format: 'xlsx' | 'docx' = 'docx'): InMsg => ({
  type: 'open',
  id: 1,
  format,
  bytes: bytes.buffer as ArrayBuffer,
});

const exportPdfMsg = (id: number, options: PdfOptions): InMsg => ({
  type: 'export-docx-pdf',
  id,
  options,
});

const exportTextMsg = (id: number, format: TextExportFormat): InMsg => ({
  type: 'export-docx-text',
  id,
  format,
});

/**
 * Байты, какими их увидел ленивый модуль: воркер делает из буфера документа
 * `Uint8Array` целиком — без сдвига и без хвоста чужой длины.
 */
const expectBytes = (value: unknown, expected: Uint8Array): void => {
  expect(value).toBeInstanceOf(Uint8Array);
  const bytes = value as Uint8Array;
  expect(Array.from(bytes)).toEqual(Array.from(expected));
  expect(bytes.byteOffset).toBe(0);
  expect(bytes.buffer.byteLength).toBe(expected.byteLength);
};

const pdfIds = (msgs: OutMsg[]): number[] => msgs.flatMap((m) => (m.type === 'pdf' ? [m.id] : []));
const textIds = (msgs: OutMsg[]): number[] =>
  msgs.flatMap((m) => (m.type === 'docx-text' ? [m.id] : []));

/** Единственный ответ `pdf`: его отсутствие — тоже провал, но с внятной причиной. */
const pdfOf = (msgs: OutMsg[]): PdfMsg => {
  const msg = msgs.find((m): m is PdfMsg => m.type === 'pdf');
  if (!msg) throw new Error('no pdf message posted');
  return msg;
};

/** Единственный ответ `docx-text`: см. `pdfOf`. */
const textOf = (msgs: OutMsg[]): DocxTextMsg => {
  const msg = msgs.find((m): m is DocxTextMsg => m.type === 'docx-text');
  if (!msg) throw new Error('no docx-text message posted');
  return msg;
};

/** Поднять воркер заново: своя копия модуля и чистые отметки обеих заглушек. */
async function startWorker(withUrls: { pdf?: boolean; docx?: boolean }): Promise<void> {
  vi.resetModules();
  vi.clearAllMocks();
  // Модули заглушек рантайм кэширует по URL: отметки заводим заново, чтобы
  // `loads` считал вычисления только этого сценария.
  (globalThis as { __pdfStub?: Stub }).__pdfStub = undefined;
  (globalThis as { __docxStub?: Stub }).__docxStub = undefined;

  posted = [];
  transfers = [];
  const selfStub: SelfStub = {
    onmessage: null,
    postMessage: (msg, transfer) => {
      posted.push(msg);
      transfers.push(transfer);
    },
  };
  vi.stubGlobal('self', selfStub);
  // Кадр собирается сразу: тесту нужен ответ на экспорт, а не кадры.
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0);
    return 0;
  });
  // Пропущенный кадр воркер логирует — для теста это ожидаемый ход событий.
  vi.spyOn(console, 'error').mockImplementation(() => {});

  await import('../src/worker/docx_worker.js');
  const onmessage = selfStub.onmessage;
  if (!onmessage) throw new Error('worker did not subscribe to messages');
  handler = onmessage;

  await handler({ data: initMsg(withUrls) });
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('экспорт DOCX в PDF', () => {
  it('первый экспорт грузит pdf-модуль и отдаёт ему байты документа и настройки', async () => {
    await startWorker({ pdf: true, docx: true });
    const bytes = fileBytes(0x11);
    await handler({ data: openMsg(bytes) });

    // Открытие документа — дело docx-модуля: pdf-модуль до первого экспорта не грузили.
    expect(pdfStub()).toBeUndefined();

    const options: PdfOptions = {
      pageSize: 'A4',
      orientation: 'landscape',
      margins: { top: 1, right: 2, bottom: 3, left: 4 },
      scale: 1.5,
      sheetIndex: 7,
    };
    await handler({ data: exportPdfMsg(31, options) });

    expect(pdfStub()?.loads).toBe(1);
    expect(pdfStub()?.initCalls).toBe(1);
    // Бинарник из init-сообщения доходит до `init()`: без него wasm-bindgen
    // искал бы .wasm рядом с модулем — в тесте его там нет.
    expect(pdfCalls('default')[0]?.[0]).toBe(PDF_BINARY_URL);

    // Экспорт получает документ теми же байтами, а настройки — как есть.
    expect(pdfCalls('export_docx_pdf')).toHaveLength(1);
    expectBytes(pdfCalls('export_docx_pdf')[0]?.[0], bytes);
    expect(pdfCalls('export_docx_pdf')[0]?.[1]).toEqual(options);

    const pdf = pdfOf(posted);
    expect(pdf.id).toBe(31);
    expect(new TextDecoder().decode(pdf.bytes)).toBe('%PDF-1.7 docx stub');
    // Байты уходят переводом владения: копия на большом PDF — лишняя работа.
    expect(transfers.at(-1)).toEqual([pdf.bytes.buffer]);
  });

  it('повторный экспорт не грузит модуль заново', async () => {
    await startWorker({ pdf: true, docx: true });
    const bytes = fileBytes(0x21);
    await handler({ data: openMsg(bytes) });
    await handler({ data: exportPdfMsg(1, {}) });
    await handler({ data: exportPdfMsg(2, {}) });

    // `import()` и `init()` платятся один раз за воркер: второй клик — только расчёт.
    expect(pdfStub()?.loads).toBe(1);
    expect(pdfStub()?.initCalls).toBe(1);
    expect(pdfCalls('export_docx_pdf')).toHaveLength(2);
    expectBytes(pdfCalls('export_docx_pdf')[1]?.[0], bytes);
    expect(pdfIds(posted)).toEqual([1, 2]);
  });

  it('без pdf в init документ не спасает: ошибка про конфигурацию, модуль не грузится', async () => {
    await startWorker({ docx: true });
    // Документ открыт: ошибка здесь про отсутствие URL, а не про документ.
    await handler({ data: openMsg(fileBytes(0x31)) });
    await handler({ data: exportPdfMsg(41, {}) });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'pdf module is not configured: init has no pdf urls',
      id: 41,
      code: 'export',
    });
    expect(pdfIds(posted)).toEqual([]);
    expect(pdfStub()).toBeUndefined();
  });

  it('без открытого документа отвечает ошибкой и pdf-модуль не грузит', async () => {
    await startWorker({ pdf: true, docx: true });
    await handler({ data: exportPdfMsg(51, {}) });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'no docx document is open',
      id: 51,
      code: 'export',
    });
    expect(pdfIds(posted)).toEqual([]);
    expect(pdfStub()).toBeUndefined();
  });

  it('открытая книга XLSX документом DOCX не считается', async () => {
    await startWorker({ pdf: true, docx: true });
    await handler({ data: openMsg(fileBytes(0x41), 'xlsx') });
    await handler({ data: exportPdfMsg(61, {}) });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'no docx document is open',
      id: 61,
      code: 'export',
    });
    expect(pdfStub()).toBeUndefined();
  });

  it('после close экспорт отвечает ошибкой и модуль не грузит', async () => {
    await startWorker({ pdf: true, docx: true });
    await handler({ data: openMsg(fileBytes(0x51)) });
    await handler({ data: { type: 'close' } });
    await handler({ data: exportPdfMsg(71, {}) });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'no docx document is open',
      id: 71,
      code: 'export',
    });
    expect(pdfIds(posted)).toEqual([]);
    expect(pdfStub()).toBeUndefined();
  });
});

/** Формат → своя функция docx-модуля и своя узнаваемая строка в ответе. */
const TEXT_CASES: { format: TextExportFormat; fn: string; text: string }[] = [
  { format: 'markdown', fn: 'docx_export_markdown', text: '# markdown stub' },
  {
    format: 'html',
    fn: 'docx_export_html',
    text: '<!DOCTYPE html><html><body>html stub</body></html>',
  },
  { format: 'text', fn: 'docx_export_text', text: 'text stub' },
];

describe('текстовая выгрузка DOCX', () => {
  it.each(TEXT_CASES)(
    'формат $format зовёт $fn и отдаёт её строку с id запроса',
    async ({ format, fn, text }) => {
      await startWorker({ pdf: true, docx: true });
      const bytes = fileBytes(0x61);
      await handler({ data: openMsg(bytes) });
      await handler({ data: exportTextMsg(81, format) });

      // Каждый формат — своя функция модуля, а не общая с параметром.
      expect(docxCalls(fn)).toHaveLength(1);
      expectBytes(docxCalls(fn)[0]?.[0], bytes);
      for (const other of TEXT_CASES) {
        if (other.fn !== fn) expect(docxCalls(other.fn)).toHaveLength(0);
      }

      const reply = textOf(posted);
      expect(reply.id).toBe(81);
      expect(reply.text).toBe(text);
      // Текст считает уже загруженный docx-модуль: pdf-модуль не тронут.
      expect(pdfStub()).toBeUndefined();
    },
  );

  it('три формата подряд pdf-модуль так и не грузят', async () => {
    await startWorker({ pdf: true, docx: true });
    await handler({ data: openMsg(fileBytes(0x71)) });
    await handler({ data: exportTextMsg(91, 'markdown') });
    await handler({ data: exportTextMsg(92, 'html') });
    await handler({ data: exportTextMsg(93, 'text') });

    expect(textIds(posted)).toEqual([91, 92, 93]);
    expect(pdfStub()).toBeUndefined();
    // docx-модуль загружен открытием документа, и то один раз.
    expect(docxStub()?.initCalls).toBe(1);
  });

  it('без docx в init отвечает ошибкой и модуль не грузит', async () => {
    await startWorker({ pdf: true });
    await handler({ data: exportTextMsg(101, 'markdown') });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'docx module is not configured: init has no docx urls',
      id: 101,
      code: 'export',
    });
    expect(docxStub()).toBeUndefined();
    expect(pdfStub()).toBeUndefined();
  });

  it('без открытого документа текст не выгружается', async () => {
    await startWorker({ pdf: true, docx: true });
    await handler({ data: exportTextMsg(111, 'markdown') });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'no docx document is open',
      id: 111,
      code: 'export',
    });
    // Открытия не было — модуль не грузили; ошибка не тянет за собой загрузку.
    expect(docxStub()).toBeUndefined();
    expect(pdfStub()).toBeUndefined();
  });

  it('открытая книга XLSX текстом не выгружается', async () => {
    await startWorker({ pdf: true, docx: true });
    await handler({ data: openMsg(fileBytes(0x81), 'xlsx') });
    await handler({ data: exportTextMsg(121, 'html') });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'no docx document is open',
      id: 121,
      code: 'export',
    });
    expect(pdfStub()).toBeUndefined();
  });

  it('после close текст не выгружается', async () => {
    await startWorker({ pdf: true, docx: true });
    await handler({ data: openMsg(fileBytes(0x91)) });
    await handler({ data: { type: 'close' } });
    await handler({ data: exportTextMsg(131, 'text') });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'no docx document is open',
      id: 131,
      code: 'export',
    });
    expect(docxCalls('docx_export_text')).toHaveLength(0);
    expect(pdfStub()).toBeUndefined();
  });
});

describe('байты открытого документа', () => {
  it('повторное открытие заменяет байты, которыми экспортируют', async () => {
    await startWorker({ pdf: true, docx: true });
    const first = fileBytes(0x11);
    const second = fileBytes(0x22);

    await handler({ data: openMsg(first) });
    await handler({ data: exportPdfMsg(1, {}) });
    expectBytes(pdfCalls('export_docx_pdf')[0]?.[0], first);

    await handler({ data: openMsg(second) });
    await handler({ data: exportPdfMsg(2, {}) });
    expectBytes(pdfCalls('export_docx_pdf')[1]?.[0], second);
    // Текстовый путь читает те же байты, что и PDF-путь: свежие, а не прежние.
    await handler({ data: exportTextMsg(3, 'markdown') });
    expectBytes(docxCalls('docx_export_markdown')[0]?.[0], second);

    // Смена документа модули не перезагружает: они живут в воркере дальше.
    expect(pdfStub()?.loads).toBe(1);
    expect(docxStub()?.initCalls).toBe(1);
    const opened = docxCalls('docx_open_bytes');
    expectBytes(opened[0]?.[0], first);
    expectBytes(opened[1]?.[0], second);
  });
});
