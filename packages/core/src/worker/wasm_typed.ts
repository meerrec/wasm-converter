// Единственное место, где ответы wasm-bindgen (`any`) сужаются до wire-типов.
// Сами типы сгенерированы из Rust (`packages/core/src/generated/wire.ts`), а
// их форма сверяется с публичными типами в `wire_assertions.ts`. Касты вне
// этого файла — признак того, что граница поехала: возвращайте их сюда.
import {
  paint_display_list_bytes,
  paint_display_list_sab,
  xlsx_build_display_list_into,
  xlsx_build_display_list_sab,
  xlsx_hit_test,
  xlsx_hyperlink_at,
  xlsx_images,
  xlsx_open,
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
  ViewportWire,
} from '../generated/wire.js';

/** Тип ленивого DOCX-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type DocxWasm = typeof import('@meerrec/wasm-docx');

/** Тип ленивого PDF-модуля берём у сгенерированных wasm-bindgen деклараций. */
export type PdfWasm = typeof import('@meerrec/wasm-pdf');

/** Открыть книгу XLSX: список листов с размерами раскладки. */
export const openXlsx = (bytes: Uint8Array): SheetInfoWire[] =>
  xlsx_open(bytes) as unknown as SheetInfoWire[];

/** Открыть документ DOCX: размеры страниц раскладки. Модуль ленивый. */
export const openDocx = (mod: DocxWasm, bytes: Uint8Array): PageInfoWire[] =>
  mod.docx_open_bytes(bytes) as unknown as PageInfoWire[];

/** Список картинок: id, MIME и длина байтов. */
export const images = (value: unknown): ImageInfoWire[] => value as unknown as ImageInfoWire[];

/** Ячейка под точкой окна. */
export const hitTest = (
  sheet: number,
  viewport: ViewportWire,
  x: number,
  y: number,
): [number, number] => xlsx_hit_test(sheet, viewport, x, y) as unknown as [number, number];

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

/** Painter: байты DisplayList (путь без SAB). */
export const paintBytes = (frame: Uint8Array): PaintStatsWire =>
  paintStats(() => paint_display_list_bytes(frame));

/** Painter: самый свежий слот ring'а (SAB-путь). */
export const paintSab = (sab: SharedArrayBuffer, slotCapacity: number): PaintStatsWire =>
  paintStats(() => paint_display_list_sab(sab, slotCapacity));

/** Экспортировать лист книги в PDF. */
export const exportPdf = (
  mod: PdfWasm,
  bytes: Uint8Array,
  sheet: number,
  options: PdfOptionsWire,
): Uint8Array => mod.export_pdf(bytes, sheet, options);

/** Экспортировать всю книгу в один PDF. */
export const exportPdfBook = (
  mod: PdfWasm,
  bytes: Uint8Array,
  options: PdfOptionsWire,
): Uint8Array => mod.export_pdf_book(bytes, options);

/** Экспортировать документ DOCX в PDF. */
export const exportDocxPdf = (
  mod: PdfWasm,
  bytes: Uint8Array,
  options: PdfOptionsWire,
): Uint8Array => mod.export_docx_pdf(bytes, options);
