import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { registerPaintFonts, type FontBytesSource } from '../src/worker/fonts.js';

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
  it('заводит четыре лица Carlito и дожидается их разбора', async () => {
    const { calls, mod } = fakeModule();

    await expect(registerPaintFonts(mod)).resolves.toBe(true);

    expect(calls).toEqual([0, 1, 2, 3]);
    expect(
      fonts.added.map((face) => [face.family, face.descriptors.style, face.descriptors.weight]),
    ).toEqual([
      ['Carlito', 'normal', '400'],
      ['Carlito', 'normal', '700'],
      ['Carlito', 'italic', '400'],
      ['Carlito', 'italic', '700'],
    ]);
    // Байты уезжают в `FontFace` как есть: своя копия или подстановка — то же
    // самое расхождение, ради которого функция и написана.
    expect(fonts.added.map((face) => face.source)).toEqual([
      new Uint8Array([0]),
      new Uint8Array([1]),
      new Uint8Array([2]),
      new Uint8Array([3]),
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
