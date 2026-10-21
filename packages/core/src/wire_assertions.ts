// Проверки формы wire-типов на этапе компиляции.
//
// Файл ниоткуда не импортируется: в бандл он не попадает, а `tsc --noEmit`
// проверяет, что публичные типы в `protocol.ts` не разъехались с
// сгенерированными из Rust (`generated/wire.ts`), а единственный маппинг
// public → wire (`frame_args.ts`) возвращает ровно wire-типы. Расхождение —
// ошибка компиляции, а не сюрприз в рантайме (ADR-0028).

import type {
  HyperlinkInfoWire,
  ImageInfoWire,
  PageInfoWire,
  PaintStatsWire,
  PdfOptionsWire,
  RenderOptionsWire,
  SheetInfoWire,
  ViewportWire,
} from './generated/wire.js';
import type {
  HyperlinkInfo,
  ImageInfo,
  PageInfo,
  PaintStats,
  PdfOptions,
  SheetInfo,
  Viewport,
} from './protocol.js';
import type { toOptions, toViewport } from './worker/frame_args.js';

/** Формы A и B совпадают: присваиваемость в обе стороны. */
type Equals<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;
/** Утверждение, которое не компилируется, если условие ложно. */
type Assert<T extends true> = T;

// Публичные алиасы обязаны совпадать с порождёнными типами.
type _SheetInfo = Assert<Equals<SheetInfo, SheetInfoWire>>;
type _PageInfo = Assert<Equals<PageInfo, PageInfoWire>>;
type _ImageInfo = Assert<Equals<ImageInfo, ImageInfoWire>>;
type _HyperlinkInfo = Assert<Equals<HyperlinkInfo, HyperlinkInfoWire>>;

// PaintStats = PaintStatsWire + поля, которые добавляет воркер.
type _PaintStatsBase = Assert<Equals<Pick<PaintStats, keyof PaintStatsWire>, PaintStatsWire>>;
type _PaintStatsExtra = Assert<
  Equals<Omit<PaintStats, keyof PaintStatsWire>, { frameId: number; buildMs?: number }>
>;

// PdfOptions = PdfOptionsWire + allSheets: режим выбирает воркер, Rust о нём
// не знает.
type _PdfOptionsBase = Assert<Equals<Pick<PdfOptions, keyof PdfOptionsWire>, PdfOptionsWire>>;
type _PdfOptionsExtra = Assert<
  Equals<Omit<PdfOptions, keyof PdfOptionsWire>, { allSheets?: boolean }>
>;

// Viewport на проводе — отдельная форма: публичный Viewport переводит
// `frame_args.toViewport`, и обратного маппинга нет.
type _ViewportKeys = Assert<Equals<keyof Viewport, 'x' | 'y' | 'w' | 'h' | 'scale'>>;
type _ViewportWireKeys = Assert<
  Equals<keyof ViewportWire, 'scrollX' | 'scrollY' | 'width' | 'height' | 'scale'>
>;

// Единственный маппинг public → wire возвращает именно wire-типы.
type _ToViewport = Assert<Equals<ReturnType<typeof toViewport>, ViewportWire>>;
type _ToOptions = Assert<Equals<ReturnType<typeof toOptions>, RenderOptionsWire>>;
