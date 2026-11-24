import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  FONT_READINESS_POINTS,
  PAINT_FONT_PROBE_FAMILY,
  registerPaintFonts,
  resolvePaintFonts,
  waitPaintFontsVisible,
  type FontBytesSource,
  type PaintFontContext,
} from '../src/worker/fonts.js';

/**
 * Лицо-заглушка: регистрация трогает конструктор, `add` и `load` — больше от
 * `FontFace` ей ничего не нужно.
 */
class FakeFontFace {
  /** Все созданные лица: по ним видно, что и с какими дескрипторами завели. */
  static created: FakeFontFace[] = [];
  /** Ворота разбора: пока не отпущены, `load()` не резолвится. */
  static loadGate: Promise<void> | null = null;

  readonly family: string;
  readonly source: Uint8Array | string;
  readonly descriptors: FontFaceDescriptors;
  status = 'unloaded';

  constructor(
    family: string,
    source: Uint8Array | string,
    descriptors: FontFaceDescriptors = {},
  ) {
    this.family = family;
    this.source = source;
    this.descriptors = descriptors;
    FakeFontFace.created.push(this);
  }

  load(): Promise<FontFace> {
    const gate = FakeFontFace.loadGate ?? Promise.resolve();
    return gate.then(() => {
      this.status = 'loaded';
      return this as unknown as FontFace;
    });
  }
}

/** Набор шрифтов-заглушка: регистрации важен только `add`. */
function fakeFontSet(): { added: FakeFontFace[]; set: FontFaceSet } {
  const added: FakeFontFace[] = [];
  const set = {
    add: (face: FakeFontFace) => {
      added.push(face);
      return set;
    },
  };
  return { added, set: set as unknown as FontFaceSet };
}

/** Модуль-заглушка: байты узнаются по номеру лица, вызовы записаны. */
function fakeModule(): { calls: number[]; mod: FontBytesSource } {
  const calls: number[] = [];
  return {
    calls,
    mod: {
      font_bytes: (face: number) => {
        calls.push(face);
        return new Uint8Array([face]);
      },
    },
  };
}

let fonts: ReturnType<typeof fakeFontSet>;

beforeEach(() => {
  FakeFontFace.created = [];
  FakeFontFace.loadGate = null;
  fonts = fakeFontSet();
  vi.stubGlobal('FontFace', FakeFontFace);
  vi.stubGlobal('fonts', fonts.set);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('registerPaintFonts', () => {
  it('заводит четыре начертания под Carlito и Calibri и дожидается их разбора', async () => {
    const { calls, mod } = fakeModule();

    await expect(registerPaintFonts(mod)).resolves.toBe(true);

    expect(calls).toEqual([0, 1, 2, 3, 0]);
    // Восемь рабочих лиц + канарейка: она нужна `waitPaintFontsVisible`, чтобы
    // отличить «load() резолвнулся» от «матчер реально видит лицо».
    expect(
      fonts.added.map((face) => [face.family, face.descriptors.style, face.descriptors.weight]),
    ).toEqual([
      ['Carlito', 'normal', '400'],
      ['Calibri', 'normal', '400'],
      ['Carlito', 'normal', '700'],
      ['Calibri', 'normal', '700'],
      ['Carlito', 'italic', '400'],
      ['Calibri', 'italic', '400'],
      ['Carlito', 'italic', '700'],
      ['Calibri', 'italic', '700'],
      [PAINT_FONT_PROBE_FAMILY, undefined, undefined],
    ]);
    // Байты уезжают в `FontFace` как есть: своя копия или подстановка — то же
    // самое расхождение, ради которого функция и написана. Один номер лица
    // покрывает оба canvas-имени, чтобы XLSX с `Calibri` не уходил в fallback.
    // Канарейка — теми же байтами regular, что и лицо 0.
    expect(fonts.added.map((face) => face.source)).toEqual([
      new Uint8Array([0]),
      new Uint8Array([0]),
      new Uint8Array([1]),
      new Uint8Array([1]),
      new Uint8Array([2]),
      new Uint8Array([2]),
      new Uint8Array([3]),
      new Uint8Array([3]),
      new Uint8Array([0]),
    ]);
    expect(fonts.added.every((face) => face.status === 'loaded')).toBe(true);
  });

  it('не резолвится раньше разбора: первый кадр ждёт готовый шрифт', async () => {
    const { mod } = fakeModule();
    let release!: () => void;
    FakeFontFace.loadGate = new Promise<void>((resolve) => {
      release = resolve;
    });

    let settled = false;
    const done = registerPaintFonts(mod).then((registered) => {
      settled = true;
      return registered;
    });
    await Promise.resolve();
    expect(settled).toBe(false);

    release();
    await expect(done).resolves.toBe(true);
  });

  it('без FontFace/self.fonts возвращает «не зарегистрировано», а не падает', async () => {
    vi.stubGlobal('FontFace', undefined);
    vi.stubGlobal('fonts', undefined);
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { mod } = fakeModule();

    await expect(registerPaintFonts(mod)).resolves.toBe(false);

    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('сломанный разбор лица — тоже «не зарегистрировано», без исключения', async () => {
    class BrokenFace extends FakeFontFace {
      override load(): Promise<FontFace> {
        return Promise.reject(new Error('font parse failed'));
      }
    }
    vi.stubGlobal('FontFace', BrokenFace);
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { mod } = fakeModule();

    await expect(registerPaintFonts(mod)).resolves.toBe(false);

    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]?.[1])).toContain('font parse failed');
  });
});

describe('resolvePaintFonts', () => {
  it('разрешает строки точно в формате painter’а до первого кадра', () => {
    const measured: Array<{ font: string; text: string }> = [];
    const ctx: PaintFontContext = {
      font: '',
      measureText(text: string) {
        measured.push({ font: this.font, text });
        return {};
      },
    };

    resolvePaintFonts(ctx);

    // Формат совпадает с `PaintState::font_css` посимвольно, включая суффикс
    // `, sans-serif` и Rust-Display для f32-кегля: canvas кэширует строку
    // `ctx.font` целиком, и любое расхождение оставляет painter-строку непрогретой.
    const pxPerPoint = new Float32Array([4 / 3])[0]!;
    const toPxF32 = (point: number): number =>
      new Float32Array([new Float32Array([point])[0]! * pxPerPoint])[0]!;
    const rustF32 = (px: number): string => {
      if (Number.isInteger(px)) return String(px);
      for (let precision = 1; precision <= 9; precision += 1) {
        const candidate = px.toFixed(precision);
        if (new Float32Array([Number(candidate)])[0] === px) return candidate;
      }
      return String(px);
    };
    const expected = (['Carlito', 'Calibri'] as const).flatMap((family) =>
      ([false, true] as const).flatMap((bold) =>
        ([false, true] as const).flatMap((italic) =>
          FONT_READINESS_POINTS.map((point) => ({
            font: `${italic ? 'italic ' : ''}${bold ? 'bold ' : ''}${rustF32(toPxF32(point))}px ${family}, sans-serif`,
            text: 'Font readiness',
          })),
        ),
      ),
    );
    expect(measured).toEqual(expected);
  });

  it('кегль 11pt печатается как «14.666667px» — ровно как в кадре XLSX', () => {
    const measured: string[] = [];
    const ctx: PaintFontContext = {
      font: '',
      measureText() {
        measured.push(this.font);
        return {};
      },
    };

    resolvePaintFonts(ctx);

    expect(measured).toContain('14.666667px Calibri, sans-serif');
    expect(measured).toContain('italic bold 14.666667px Calibri, sans-serif');
    expect(measured).toContain('14.666667px Carlito, sans-serif');
  });
});

describe('waitPaintFontsVisible', () => {
  /** Контекст-заглушка: видимость канарейки переключается извне. */
  function fakeProbeContext(visibleAfterMs: number): {
    ctx: PaintFontContext;
    fontsSeen: string[];
  } {
    const fontsSeen: string[] = [];
    const startedAt = performance.now();
    return {
      fontsSeen,
      ctx: {
        font: '',
        measureText(text: string) {
          fontsSeen.push(this.font);
          const visible = performance.now() - startedAt >= visibleAfterMs;
          const width = this.font.includes(PAINT_FONT_PROBE_FAMILY)
            ? visible
              ? 42
              : 40
            : // Заведомо отсутствующее семейство: системный fallback.
              40;
          return { width };
        },
      },
    };
  }

  it('резолвится, как только канарейка отличается по ширине от fallback', async () => {
    const { ctx } = fakeProbeContext(80);

    await expect(waitPaintFontsVisible(ctx, 5_000)).resolves.toBe(true);
  });

  it('возвращает false по таймауту, если канарейка так и не стала видна', async () => {
    const { ctx } = fakeProbeContext(Number.POSITIVE_INFINITY);

    await expect(waitPaintFontsVisible(ctx, 150)).resolves.toBe(false);
  });

  it('каждый замер использует новую строку: повторы кэша матчером не читаются', async () => {
    const { ctx, fontsSeen } = fakeProbeContext(120);

    await waitPaintFontsVisible(ctx, 5_000);

    const probeFonts = fontsSeen.filter((font) => font.includes(PAINT_FONT_PROBE_FAMILY));
    expect(new Set(probeFonts).size).toBe(probeFonts.length);
  });
});
