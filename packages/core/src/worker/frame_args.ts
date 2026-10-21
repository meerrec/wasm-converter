// Разбор запроса кадра в аргументы былл-функций. Отдельно от воркера: это
// чистая логика, и её видно тестам — без wasm и без настоящего воркера.
import type { RenderOptionsWire, ViewportWire } from '../generated/wire.js';
import type { DocFormat, RenderConfig, RenderRequest, Viewport } from '../protocol.js';

/** Формат открытого документа; `null` — не открыто ничего. */
export type OpenFormat = DocFormat | null;

/** Окно в единицах Rust. */
export const toViewport = (v: Viewport): ViewportWire => ({
  scrollX: v.x,
  scrollY: v.y,
  width: v.w,
  height: v.h,
  scale: v.scale,
});

/** Настройки показа в единицах Rust. */
export const toOptions = (config: RenderConfig): RenderOptionsWire => ({
  showGrid: config.showGrid,
  showHeaders: config.showHeaders,
  dark: config.theme === 'dark',
});

/**
 * Что рисовать в следующем кадре.
 *
 * Ветки различаются не только номером листа или зазором стопки, но и набором
 * аргументов былл-функции: у DOCX настроек показа нет — кадр стопки страниц от
 * них не зависит.
 */
export type FrameTarget =
  | { kind: 'xlsx'; sheet: number; viewport: ViewportWire; options: RenderOptionsWire }
  | { kind: 'docx'; gap: number; viewport: ViewportWire };

/**
 * Разобрать запрос кадра под формат открытого документа.
 *
 * @throws Error, если документ не открыт или в запросе нет поля его формата:
 * кадр лучше пропустить, чем нарисовать чужой лист или стопку, съехавшую
 * относительно прокрутки.
 */
export function frameTarget(format: OpenFormat, req: RenderRequest): FrameTarget {
  if (format === null) throw new Error('render: no document is open');
  const viewport = toViewport(req.viewport);
  if (format === 'docx') {
    if (req.gap === undefined) throw new Error('render: docx frame has no page gap');
    return { kind: 'docx', gap: req.gap, viewport };
  }
  return { kind: 'xlsx', sheet: req.sheet ?? 0, viewport, options: toOptions(req.config) };
}
