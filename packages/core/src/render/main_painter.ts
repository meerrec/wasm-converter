import type { PaintStatsWire } from '../generated/wire.js';
import { registerPaintFonts } from '../worker/fonts.js';

/**
 * Тип wasm-модуля берём из сгенерированных wasm-bindgen деклараций, но сам
 * модуль грузим динамически: статический импорт вернул бы wasm-биндинги в
 * основной бандл (`index.js`), а main-режим нужен не всем страницам.
 */
type WasmModule = typeof import('@meerrec/wasm');

/** Загруженный модуль: `import()` и `init()` платятся один раз за страницу. */
let modulePromise: Promise<WasmModule> | null = null;
let module: WasmModule | null = null;
/** Painter установлен и ещё не снят. */
let active = false;

/**
 * Поднять painter на main-потоке.
 *
 * Зовётся один раз на страницу: у wasm-модуля один singleton-painter, и
 * второй вьюер в main-режиме затёр бы контекст первого. Ошибка повторной
 * инициализации возникает до загрузки модуля — второй вьюер не тратит на неё
 * ни времени, ни памяти.
 *
 * @param ctx — 2D-контекст обычного `HTMLCanvasElement`
 * @param wasmUrl — URL wasm-бинаря; без него wasm-bindgen ищет его рядом с
 *   модулем
 * @throws Error, если painter уже инициализирован этим или другим вьюером
 */
export async function initMainPainter(
  ctx: CanvasRenderingContext2D,
  wasmUrl?: string,
): Promise<void> {
  if (active) {
    throw new Error('main-режим: один вьюер на страницу');
  }
  // Занимаем singleton до `await`: два параллельных `init` иначе оба прошли
  // бы проверку и второй установил бы painter поверх первого.
  active = true;
  try {
    modulePromise ??= import('@meerrec/wasm');
    const mod = await modulePromise;
    await mod.default(wasmUrl);
    // Сабсеты Carlito — до первого кадра: Chromium запоминает промах по
    // строке `ctx.font` на всю сессию, и предпросмотр разошёлся бы с PDF.
    await registerPaintFonts({ font_bytes: mod.font_bytes });
    mod.init_painter_main(ctx);
    module = mod;
  } catch (e) {
    // Неудача не оставляет страницу в подвешенном состоянии: следующий
    // вьюер вправе попробовать снова.
    active = false;
    modulePromise = null;
    throw e;
  }
}

function wasm(): WasmModule {
  if (!module) {
    throw new Error('main-режим: painter не инициализирован');
  }
  return module;
}

/** Нарисовать DisplayList тем же Rust-painter'ом, что и воркер. */
export function paintDisplayList(bytes: Uint8Array): PaintStatsWire {
  return wasm().paint_display_list_bytes(bytes) as PaintStatsWire;
}

/**
 * Пересчитать холст под DPR. Установка размеров сбрасывает 2D-контекст и кэш
 * битмапов painter'а, поэтому вызывающий регистрирует картинки заново.
 */
export function resizeMainCanvas(cssW: number, cssH: number, dpr: number): void {
  wasm().resize_canvas(cssW, cssH, dpr);
}

/** Вернуть `ImageBitmap` в кэш painter'а: воркер отдал его переводом. */
export function registerMainBitmap(id: number, bitmap: ImageBitmap): void {
  wasm().register_bitmap(id, bitmap);
}

/** Снять битмап с painter'а; `true`, если он был зарегистрирован. */
export function dropMainBitmap(id: number): boolean {
  return wasm().drop_bitmap(id) as boolean;
}

/** Снять painter: контекст страницы освобождается для следующего вьюера. */
export function disposeMainPainter(): void {
  module?.dispose_painter();
  active = false;
}
