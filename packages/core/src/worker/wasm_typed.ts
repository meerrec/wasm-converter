// Единственное место, где ответы wasm-bindgen (`any`) сужаются до wire-типов.
// Сами типы сгенерированы из Rust (`packages/core/src/generated/wire.ts`), а
// их форма сверяется с публичными типами в `wire_assertions.ts`. Касты вне
// этого файла — признак того, что граница поехала: возвращайте их сюда.
import {
  paint_display_list_bytes,
  paint_display_list_sab,
  stream_open_xlsx,
  xlsx_build_display_list_into,
  xlsx_build_display_list_sab,
  xlsx_edits,
  xlsx_hit_test,
  xlsx_hyperlink_at,
  xlsx_images,
  xlsx_open,
  xlsx_set_cell_text,
} from '@meerrec/wasm';
import type {
  BuildStatsWire,
  HyperlinkInfoWire,
  ImageInfoWire,
  PageInfoWire,
  PaintStatsWire,
  PdfOptionsWire,
  RenderOptionsWire,
  SheetInfoWire,
  SlideInfoWire,
  TextLayerWire,
  ViewportWire,
} from '../generated/wire.js';

/** Тип ленивого DOCX-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type DocxWasm = typeof import('@meerrec/wasm-docx');

/** Тип ленивого PPTX-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type PptxWasm = typeof import('@meerrec/wasm-pptx');

/** Тип ленивого PPTX-PDF-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type PptxPdfWasm = typeof import('@meerrec/wasm-pptx-pdf');

/** Тип ленивого PDF-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type PdfWasm = typeof import('@meerrec/wasm-pdf');

/** Тип ленивого XLSB-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type XlsbWasm = typeof import('@meerrec/wasm-xlsb');

/** Тип ленивого ODS-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type OdsWasm = typeof import('@meerrec/wasm-ods');

/** Тип ленивого ODP-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type OdpWasm = typeof import('@meerrec/wasm-odp');

/** Тип ленивого ODT-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type OdtWasm = typeof import('@meerrec/wasm-odt');

/** Открыть книгу XLSX: список листов с размерами раскладки. */
export const openXlsx = (bytes: Uint8Array): SheetInfoWire[] =>
  xlsx_open(bytes) as unknown as SheetInfoWire[];

/** Открыть документ DOCX: размеры страниц раскладки. Модуль ленивый. */
export const openDocx = (mod: DocxWasm, bytes: Uint8Array): PageInfoWire[] =>
  mod.docx_open_bytes(bytes) as unknown as PageInfoWire[];

/** Завершить поток XLSX: разобрать накопленный буфер, ответ — список листов. */
export const openXlsxStream = (streamId: number): SheetInfoWire[] =>
  stream_open_xlsx(streamId) as unknown as SheetInfoWire[];

/** Завершить поток DOCX: разобрать накопленный буфер, ответ — страницы. */
export const openDocxStream = (mod: DocxWasm, streamId: number): PageInfoWire[] =>
  mod.stream_open_docx(streamId) as unknown as PageInfoWire[];

/** TextLayer одной страницы открытого DOCX. Модуль ленивый. */
export const docxTextLayer = (mod: DocxWasm, page: number): TextLayerWire =>
  mod.docx_get_text_layer(page) as unknown as TextLayerWire;

/** TextLayer всего документа — для семантического дерева и Ctrl+A. */
export const docxFullTextLayer = (mod: DocxWasm): TextLayerWire =>
  mod.docx_get_full_text_layer() as unknown as TextLayerWire;

/**
 * DOCX-модуль с правками run'ов. Каст живёт здесь же, в граничном файле: срез
 * появится в сгенерированных биндингах после сборки wasm
 * (`scripts/build-wasm.sh`).
 */
type DocxEditWasm = DocxWasm & {
  docx_set_run_text(paragraph: number, run: number, text: string): void;
  docx_pages(): unknown;
  docx_edits(): unknown;
};

/** Привести ленивый DOCX-модуль к срезу правок. */
function withRunEdits(mod: DocxWasm): DocxEditWasm {
  return mod as DocxEditWasm;
}

/** Правка run'а: снимок для экспортёра PDF, поля — как в Rust `DocxRunEditWire`. */
export interface DocxRunEdit {
  paragraph: number;
  run: number;
  text: string;
}

/** Заменить текст run'а и перераскладывать документ. Ошибки wasm — исключения JS. */
export const docxSetRunText = (mod: DocxWasm, paragraph: number, run: number, text: string): void => {
  withRunEdits(mod).docx_set_run_text(paragraph, run, text);
};

/** Размеры страниц после перераскладки — их несёт ответ `docx-edited`. */
export const docxPages = (mod: DocxWasm): PageInfoWire[] =>
  withRunEdits(mod).docx_pages() as unknown as PageInfoWire[];

/** Все правки открытого документа, для экспортёра PDF. */
export const docxEdits = (mod: DocxWasm): DocxRunEdit[] => {
  // Старый wasm-модуль без экспорта правок даёт пустой снимок: путь без
  // правок остаётся байт-в-байт, как до фичи.
  const edits = withRunEdits(mod).docx_edits?.();
  return (edits ?? []) as unknown as DocxRunEdit[];
};

/** Открыть презентацию PPTX: слайды с именами и размерами раскладки. */
export const openPptx = (mod: PptxWasm, bytes: Uint8Array): SlideInfoWire[] =>
  mod.pptx_open_bytes(bytes) as unknown as SlideInfoWire[];

/** Завершить поток PPTX: разобрать накопленный буфер, ответ — слайды. */
export const openPptxStream = (mod: PptxWasm, streamId: number): SlideInfoWire[] =>
  mod.stream_open_pptx(streamId) as unknown as SlideInfoWire[];

/** Открыть книгу XLSB: список листов с размерами раскладки. Модуль ленивый. */
export const openXlsb = (mod: XlsbWasm, bytes: Uint8Array): SheetInfoWire[] =>
  mod.xlsb_open_bytes(bytes) as unknown as SheetInfoWire[];

/** Завершить поток XLSB: разобрать накопленный буфер, ответ — список листов. */
export const openXlsbStream = (mod: XlsbWasm, streamId: number): SheetInfoWire[] =>
  mod.stream_open_xlsb(streamId) as unknown as SheetInfoWire[];

/** Открыть книгу ODS: список листов с размерами раскладки. Модуль ленивый. */
export const openOds = (mod: OdsWasm, bytes: Uint8Array): SheetInfoWire[] =>
  mod.ods_open_bytes(bytes) as unknown as SheetInfoWire[];

/** Завершить поток ODS: разобрать накопленный буфер, ответ — список листов. */
export const openOdsStream = (mod: OdsWasm, streamId: number): SheetInfoWire[] =>
  mod.stream_open_ods(streamId) as unknown as SheetInfoWire[];

/** Открыть презентацию ODP: слайды с именами и размерами раскладки. */
export const openOdp = (mod: OdpWasm, bytes: Uint8Array): SlideInfoWire[] =>
  mod.odp_open_bytes(bytes) as unknown as SlideInfoWire[];

/** Завершить поток ODP: разобрать накопленный буфер, ответ — слайды. */
export const openOdpStream = (mod: OdpWasm, streamId: number): SlideInfoWire[] =>
  mod.stream_open_odp(streamId) as unknown as SlideInfoWire[];

/** Открыть документ ODT: размеры страниц раскладки. Модуль ленивый. */
export const openOdt = (mod: OdtWasm, bytes: Uint8Array): PageInfoWire[] =>
  mod.odt_open_bytes(bytes) as unknown as PageInfoWire[];

/** Завершить поток ODT: разобрать накопленный буфер, ответ — страницы. */
export const openOdtStream = (mod: OdtWasm, streamId: number): PageInfoWire[] =>
  mod.stream_open_odt(streamId) as unknown as PageInfoWire[];

/** TextLayer одной страницы открытого ODT. Модуль ленивый. */
export const odtTextLayer = (mod: OdtWasm, page: number): TextLayerWire =>
  mod.odt_get_text_layer(page) as unknown as TextLayerWire;

/** TextLayer всего документа ODT — для семантического дерева и Ctrl+A. */
export const odtFullTextLayer = (mod: OdtWasm): TextLayerWire =>
  mod.odt_get_full_text_layer() as unknown as TextLayerWire;

/** Список картинок: id, MIME и длина байтов. */
export const images = (value: unknown): ImageInfoWire[] => value as unknown as ImageInfoWire[];

/** Ячейка под точкой окна. */
export const hitTest = (
  sheet: number,
  viewport: ViewportWire,
  x: number,
  y: number,
): [number, number] => xlsx_hit_test(sheet, viewport, x, y) as unknown as [number, number];

/** Правка ячейки: снимок для экспортёра PDF, поля — как в Rust `CellEditWire`. */
export interface CellEdit {
  sheet: number;
  row: number;
  col: number;
  text: string;
}

/** Заменить показ ячейки сырым текстом. Ошибки wasm — исключения JS. */
export const setCellText = (sheet: number, row: number, col: number, text: string): void => {
  xlsx_set_cell_text(sheet, row, col, text);
};

/** Все правки открытой книги, для экспортёра PDF. */
export const cellEdits = (): CellEdit[] => xlsx_edits() as unknown as CellEdit[];

/** Гиперссылка под точкой окна либо `null`, если её нет. */
export const hyperlinkAt = (
  sheet: number,
  viewport: ViewportWire,
  x: number,
  y: number,
): HyperlinkInfoWire | null =>
  xlsx_hyperlink_at(sheet, viewport, x, y) as unknown as HyperlinkInfoWire | null;

/** Собрать кадр в буфер ring'а или вызывающего: статистика сборки. */
export const buildStats = (build: () => unknown): BuildStatsWire =>
  build() as unknown as BuildStatsWire;

/** Нарисовать готовый кадр: статистика отрисовки. */
export const paintStats = (paint: () => unknown): PaintStatsWire =>
  paint() as unknown as PaintStatsWire;

/** XLSX-кадр в буфер вызывающего. */
export const buildXlsxInto = (
  out: Uint8Array,
  sheet: number,
  viewport: ViewportWire,
  options: RenderOptionsWire,
): BuildStatsWire =>
  buildStats(() => xlsx_build_display_list_into(out, sheet, viewport, options));

/** XLSX-кадр в слот ring'а. */
export const buildXlsxSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  sheet: number,
  viewport: ViewportWire,
  options: RenderOptionsWire,
): BuildStatsWire =>
  buildStats(() => xlsx_build_display_list_sab(sab, slotCapacity, sheet, viewport, options));

/** DOCX-кадр в буфер вызывающего. Модуль ленивый. */
export const buildDocxInto = (
  mod: DocxWasm,
  out: Uint8Array,
  viewport: ViewportWire,
  gap: number,
): BuildStatsWire =>
  buildStats(() => mod.docx_build_display_list_into(out, viewport, gap));

/** DOCX-кадр в слот ring'а. Модуль ленивый. */
export const buildDocxSab = (
  mod: DocxWasm,
  sab: SharedArrayBuffer,
  slotCapacity: number,
  viewport: ViewportWire,
  gap: number,
): BuildStatsWire =>
  buildStats(() => mod.docx_build_display_list_sab(sab, slotCapacity, viewport, gap));

/** PPTX-кадр в буфер вызывающего. Модуль ленивый. */
export const buildPptxInto = (
  mod: PptxWasm,
  out: Uint8Array,
  slide: number,
  viewport: ViewportWire,
): BuildStatsWire =>
  buildStats(() => mod.pptx_build_display_list_into(out, viewport, slide));

/** PPTX-кадр в слот ring'а. Модуль ленивый. */
export const buildPptxSab = (
  mod: PptxWasm,
  sab: SharedArrayBuffer,
  slotCapacity: number,
  slide: number,
  viewport: ViewportWire,
): BuildStatsWire =>
  buildStats(() => mod.pptx_build_display_list_sab(sab, slotCapacity, viewport, slide));

/** XLSB-кадр в буфер вызывающего. Модуль ленивый. */
export const buildXlsbInto = (
  mod: XlsbWasm,
  out: Uint8Array,
  sheet: number,
  viewport: ViewportWire,
  options: RenderOptionsWire,
): BuildStatsWire =>
  buildStats(() => mod.xlsb_build_display_list_into(out, sheet, viewport, options));

/** XLSB-кадр в слот ring'а. Модуль ленивый. */
export const buildXlsbSab = (
  mod: XlsbWasm,
  sab: SharedArrayBuffer,
  slotCapacity: number,
  sheet: number,
  viewport: ViewportWire,
  options: RenderOptionsWire,
): BuildStatsWire =>
  buildStats(() => mod.xlsb_build_display_list_sab(sab, slotCapacity, sheet, viewport, options));

/** ODS-кадр в буфер вызывающего. Модуль ленивый. */
export const buildOdsInto = (
  mod: OdsWasm,
  out: Uint8Array,
  sheet: number,
  viewport: ViewportWire,
  options: RenderOptionsWire,
): BuildStatsWire =>
  buildStats(() => mod.ods_build_display_list_into(out, sheet, viewport, options));

/** ODS-кадр в слот ring'а. Модуль ленивый. */
export const buildOdsSab = (
  mod: OdsWasm,
  sab: SharedArrayBuffer,
  slotCapacity: number,
  sheet: number,
  viewport: ViewportWire,
  options: RenderOptionsWire,
): BuildStatsWire =>
  buildStats(() => mod.ods_build_display_list_sab(sab, slotCapacity, sheet, viewport, options));

/** ODP-кадр в буфер вызывающего. Модуль ленивый. */
export const buildOdpInto = (
  mod: OdpWasm,
  out: Uint8Array,
  slide: number,
  viewport: ViewportWire,
): BuildStatsWire =>
  buildStats(() => mod.odp_build_display_list_into(out, viewport, slide));

/** ODP-кадр в слот ring'а. Модуль ленивый. */
export const buildOdpSab = (
  mod: OdpWasm,
  sab: SharedArrayBuffer,
  slotCapacity: number,
  slide: number,
  viewport: ViewportWire,
): BuildStatsWire =>
  buildStats(() => mod.odp_build_display_list_sab(sab, slotCapacity, viewport, slide));

/** ODT-кадр в буфер вызывающего. Модуль ленивый. */
export const buildOdtInto = (
  mod: OdtWasm,
  out: Uint8Array,
  viewport: ViewportWire,
  gap: number,
): BuildStatsWire =>
  buildStats(() => mod.odt_build_display_list_into(out, viewport, gap));

/** ODT-кадр в слот ring'а. Модуль ленивый. */
export const buildOdtSab = (
  mod: OdtWasm,
  sab: SharedArrayBuffer,
  slotCapacity: number,
  viewport: ViewportWire,
  gap: number,
): BuildStatsWire =>
  buildStats(() => mod.odt_build_display_list_sab(sab, slotCapacity, viewport, gap));

/** Painter: байты DisplayList (путь без SAB). */
export const paintBytes = (frame: Uint8Array): PaintStatsWire =>
  paintStats(() => paint_display_list_bytes(frame));

/** Painter: самый свежий слот ring'а (SAB-путь). */
export const paintSab = (sab: SharedArrayBuffer, slotCapacity: number): PaintStatsWire =>
  paintStats(() => paint_display_list_sab(sab, slotCapacity));

/** Экспортировать лист книги в PDF; `edits` — правки интерфейса, если есть. */
export const exportPdf = (
  mod: PdfWasm,
  bytes: Uint8Array,
  sheet: number,
  options: PdfOptionsWire,
  edits: CellEdit[] = [],
): Uint8Array => {
  // Без правок путь прежний — байт-в-байт, как до фичи.
  if (edits.length === 0) return mod.export_pdf(bytes, sheet, options);
  return withEditsPdf(mod).export_pdf_edits(bytes, sheet, options, edits);
};

/** Экспортировать всю книгу в один PDF; `edits` — правки интерфейса, если есть. */
export const exportPdfBook = (
  mod: PdfWasm,
  bytes: Uint8Array,
  options: PdfOptionsWire,
  edits: CellEdit[] = [],
): Uint8Array => {
  // Без правок путь прежний — байт-в-байт, как до фичи.
  if (edits.length === 0) return mod.export_pdf_book(bytes, options);
  return withEditsPdf(mod).export_pdf_book_edits(bytes, options, edits);
};

/**
 * PDF-модуль с правками ячеек. Каст живёт здесь же, в граничном файле: срез
 * появится в сгенерированных биндингах после сборки wasm
 * (`scripts/build-wasm.sh`).
 */
function withEditsPdf(mod: PdfWasm): PdfWasm & {
  export_pdf_edits(
    bytes: Uint8Array,
    sheet: number,
    options: PdfOptionsWire,
    edits: CellEdit[],
  ): Uint8Array;
  export_pdf_book_edits(bytes: Uint8Array, options: PdfOptionsWire, edits: CellEdit[]): Uint8Array;
} {
  return mod as PdfWasm & {
    export_pdf_edits(
      bytes: Uint8Array,
      sheet: number,
      options: PdfOptionsWire,
      edits: CellEdit[],
    ): Uint8Array;
    export_pdf_book_edits(
      bytes: Uint8Array,
      options: PdfOptionsWire,
      edits: CellEdit[],
    ): Uint8Array;
  };
}

/** Экспортировать документ DOCX в PDF; `edits` — правки интерфейса, если есть. */
export const exportDocxPdf = (
  mod: PdfWasm,
  bytes: Uint8Array,
  options: PdfOptionsWire,
  edits: DocxRunEdit[] = [],
): Uint8Array => {
  // Без правок путь прежний — байт-в-байт, как до фичи.
  if (edits.length === 0) return mod.export_docx_pdf(bytes, options);
  return withDocxEditsPdf(mod).export_docx_pdf_edits(bytes, options, edits);
};

/**
 * PDF-модуль с правками run'ов DOCX. Каст живёт в граничном файле: срез
 * появится в сгенерированных биндингах после сборки wasm
 * (`scripts/build-wasm.sh`).
 */
function withDocxEditsPdf(mod: PdfWasm): PdfWasm & {
  export_docx_pdf_edits(bytes: Uint8Array, options: PdfOptionsWire, edits: DocxRunEdit[]): Uint8Array;
} {
  return mod as PdfWasm & {
    export_docx_pdf_edits(
      bytes: Uint8Array,
      options: PdfOptionsWire,
      edits: DocxRunEdit[],
    ): Uint8Array;
  };
}

/** Экспортировать презентацию PPTX в PDF. Модуль ленивый и свой у формата. */
export const exportPptxPdf = (
  mod: PptxPdfWasm,
  bytes: Uint8Array,
  options: PdfOptionsWire,
): Uint8Array => mod.export_pptx_pdf(bytes, options);

/** Экспортировать презентацию PPTX в текст: формат выбирает воркер. */
export const exportPptxText = (
  mod: PptxWasm,
  bytes: Uint8Array,
  format: 'markdown' | 'html' | 'text',
): string => {
  if (format === 'markdown') return mod.pptx_export_markdown(bytes);
  if (format === 'html') return mod.pptx_export_html(bytes);
  return mod.pptx_export_text(bytes);
};

/** Экспортировать презентацию ODP в текст: формат выбирает воркер. */
export const exportOdpText = (
  mod: OdpWasm,
  bytes: Uint8Array,
  format: 'markdown' | 'html' | 'text',
): string => {
  if (format === 'markdown') return mod.odp_export_markdown(bytes);
  if (format === 'html') return mod.odp_export_html(bytes);
  return mod.odp_export_text(bytes);
};

/** Экспортировать документ ODT в текст: формат выбирает воркер. */
export const exportOdtText = (
  mod: OdtWasm,
  bytes: Uint8Array,
  format: 'markdown' | 'html' | 'text',
): string => {
  if (format === 'markdown') return mod.odt_export_markdown(bytes);
  if (format === 'html') return mod.odt_export_html(bytes);
  return mod.odt_export_text(bytes);
};
