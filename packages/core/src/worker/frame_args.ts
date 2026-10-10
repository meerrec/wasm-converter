// Разбор запроса кадра в аргументы былл-функций. Отдельно от воркера: это
// чистая логика, и её видно тестам — без wasm и без настоящего воркера.
import type { DocFormat, RenderConfig, RenderRequest, Viewport } from '../protocol.js';

/** Формат открытого документа; `null` — не открыто ничего. */
export type OpenFormat = DocFormat | null;

/**
 * Окно в единицах Rust. Единицы намеренно разные: смещение — пиксели
 * раскладки (до зума), размер — физические пиксели canvas.
 */
export interface RustViewport {
  scrollX: number;
  scrollY: number;
  width: number;
  height: number;
  scale: number;
}

/** Настройки показа в том виде, в каком их ждёт Rust. Нужны только кадру XLSX. */
export interface XlsxRenderOptions {
  showGrid: boolean;
  showHeaders: boolean;
  dark: boolean;
}

/** Окно в единицах Rust. */
export const toViewport = (v: Viewport): RustViewport => ({
  scrollX: v.x,
  scrollY: v.y,
  width: v.w,
  height: v.h,
  scale: v.scale,
});

/** Настройки показа в единицах Rust. */
export const toOptions = (config: RenderConfig): XlsxRenderOptions => ({
  showGrid: config.showGrid,
  showHeaders: config.showHeaders,
  dark: config.theme === 'dark',
});

/**
 * Что рисовать в следующем кадре.
 *
 * Ветки различаются не только номером листа или страницы, но и набором
 * аргументов былл-функции: у DOCX настроек показа нет — кадр страницы от них
 * не зависит.
 */
export type FrameTarget =
  | { kind: 'xlsx'; sheet: number; viewport: RustViewport; options: XlsxRenderOptions }
  | { kind: 'docx'; page: number; viewport: RustViewport };

/**
 * Разобрать запрос кадра под формат открытого документа.
 *
 * @throws Error, если документ не открыт или в запросе нет поля его формата:
 * кадр лучше пропустить, чем нарисовать чужой лист или первую страницу.
 */
export function frameTarget(format: OpenFormat, req: RenderRequest): FrameTarget {
  if (format === null) throw new Error('render: no document is open');
  const viewport = toViewport(req.viewport);
  if (format === 'docx') {
    if (req.page === undefined) throw new Error('render: docx frame has no page number');
    return { kind: 'docx', page: req.page, viewport };
  }
  return { kind: 'xlsx', sheet: req.sheet ?? 0, viewport, options: toOptions(req.config) };
}
