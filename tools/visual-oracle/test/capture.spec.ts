// Тесты чистых частей захвата: без браузера и без сети.
//
// Здесь проверяется то, что можно проверить без Chromium: хеширование,
// состояние ожидания стабильного кадра, аргументы растеризатора и таблица
// кодов ошибок. Сам захват проверяется сквозным прогоном, а не этим файлом.

import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { captureStableFrame } from '../src/capture/capture.js';
import { CAPTURE_ERROR_CODES, CaptureError, resultTypeForCode } from '../src/capture/errors.js';
import { encodePng } from '../src/diff/png.js';
import type { ManifestCapture } from '../src/manifest.js';
import { pdftoppmArgs, RASTER_DPI } from '../src/capture/rasterize.js';
import {
  sha256Hex,
  waitForStableFrame,
  type StableFrameViewer,
} from '../src/capture/stable.js';

/** Байты из строки: содержимое важно только как различимый вход хеша. */
function bytes(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

/** Поддельный вьюер: сценарий кадров и ручной тик. */
interface FakeViewer extends StableFrameViewer {
  /** Выдать тик всем подписчикам. */
  tick(): void;
  /** Сколько раз спросили `exportPng`. */
  readonly polls: number;
}

/**
 * Вьюер со сценарием кадров.
 *
 * `frames` — очередь ответов `exportPng`; когда она кончается, отдаётся
 * последний кадр (стабильное состояние). С `cycle` кадры идут по кругу — так
 * проверяется кадр, который не перестаёт меняться. `onTick` запоминает
 * подписчиков, чтобы тест сам решал, когда painter «отработал».
 */
function fakeViewer(frames: readonly Uint8Array[], options: { cycle?: boolean } = {}): FakeViewer {
  const handlers = new Set<(stats: unknown) => void>();
  let polls = 0;
  return {
    get polls() {
      return polls;
    },
    tick() {
      for (const handler of handlers) handler(undefined);
    },
    onTick(handler) {
      handlers.add(handler);
      return () => {
        handlers.delete(handler);
      };
    },
    async exportPng() {
      const index =
        options.cycle === true ? polls % frames.length : Math.min(polls, frames.length - 1);
      polls += 1;
      const frame = frames[index];
      if (frame === undefined) throw new Error('no frames scripted');
      return frame;
    },
  };
}

describe('sha256Hex', () => {
  it('хеширует пустой ввод', () => {
    expect(sha256Hex(new Uint8Array())).toBe(
      'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    );
  });

  it('хеширует известный вектор «abc»', () => {
    expect(sha256Hex(bytes('abc'))).toBe(
      'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
    );
  });

  it('хеширует только срез буфера, а не весь буфер', () => {
    const padded = bytes('xxabcxx');
    const slice = padded.subarray(2, 5);
    expect(sha256Hex(slice)).toBe(sha256Hex(bytes('abc')));
  });
});

describe('waitForStableFrame', () => {
  it('ждёт двух одинаковых хешей подряд и возвращает оба', async () => {
    const viewer = fakeViewer([bytes('frame-a'), bytes('frame-b'), bytes('frame-b')]);
    const result = await waitForStableFrame(viewer, {
      change: () => {
        viewer.tick();
      },
      pollIntervalMs: 1,
      timeoutMs: 5_000,
    });
    expect(result.hashes[0]).toBe(sha256Hex(bytes('frame-b')));
    expect(result.hashes[1]).toBe(result.hashes[0]);
    expect(viewer.polls).toBe(3);
    expect(result.elapsedMs).toBeGreaterThanOrEqual(0);
  });

  it('возвращает байты кадра, чей хеш равен hashes[1]', async () => {
    const viewer = fakeViewer([bytes('frame-a'), bytes('frame-b'), bytes('frame-b')]);
    const result = await waitForStableFrame(viewer, {
      change: () => {
        viewer.tick();
      },
      pollIntervalMs: 1,
      timeoutMs: 5_000,
    });
    expect(sha256Hex(result.bytes)).toBe(result.hashes[1]);
    expect(new TextDecoder().decode(result.bytes)).toBe('frame-b');
  });

  it('не считает стабильным кадр, который меняется каждый опрос', async () => {
    // Кадры чередуются: двух одинаковых подряд не будет никогда.
    const viewer = fakeViewer([bytes('a'), bytes('b')], { cycle: true });
    await expect(
      waitForStableFrame(viewer, {
        change: () => {
          viewer.tick();
        },
        pollIntervalMs: 1,
        timeoutMs: 40,
      }),
    ).rejects.toMatchObject({ code: 'unstable-frame' });
  });

  it('падает, если тик не пришёл после изменения', async () => {
    const viewer = fakeViewer([bytes('a'), bytes('a')]);
    await expect(
      waitForStableFrame(viewer, {
        change: () => {
          // Тика нет: painter не отчитался.
        },
        tickTimeoutMs: 20,
        pollIntervalMs: 1,
        timeoutMs: 1_000,
      }),
    ).rejects.toMatchObject({ code: 'unstable-frame' });
    expect(viewer.polls).toBe(0);
  });

  it('засчитывает тик, пришедший во время асинхронного изменения', async () => {
    const viewer = fakeViewer([bytes('a'), bytes('a')]);
    const result = await waitForStableFrame(viewer, {
      change: async () => {
        viewer.tick();
        await Promise.resolve();
      },
      pollIntervalMs: 1,
      timeoutMs: 1_000,
    });
    expect(result.hashes[0]).toBe(result.hashes[1]);
  });

  it('не засчитывает совпадение, если во время снимка пришёл тик', async () => {
    // Первые два снимка совпали бы, но между ними painter нарисовал кадр:
    // сравнивать их нельзя, иначе оракул принял бы старые байты за стабильные.
    const handlers = new Set<(stats: unknown) => void>();
    const frames = [bytes('a'), bytes('a'), bytes('b'), bytes('b')];
    let polls = 0;
    const viewer: StableFrameViewer = {
      onTick(handler) {
        handlers.add(handler);
        return () => {
          handlers.delete(handler);
        };
      },
      async exportPng() {
        const frame = frames[Math.min(polls, frames.length - 1)];
        polls += 1;
        if (frame === undefined) throw new Error('no frames scripted');
        if (polls === 2) {
          // Тик приходит, пока снимок ещё не вернулся.
          for (const handler of handlers) handler(undefined);
        }
        return frame;
      },
    };
    const result = await waitForStableFrame(viewer, {
      change: () => {
        for (const handler of handlers) handler(undefined);
      },
      pollIntervalMs: 1,
      timeoutMs: 1_000,
    });
    expect(result.hashes[0]).toBe(sha256Hex(bytes('b')));
    expect(polls).toBe(4);
  });

  it('переводит сбой exportPng в код export-failed', async () => {
    const viewer: StableFrameViewer = {
      onTick(handler) {
        handler(undefined);
        return () => undefined;
      },
      exportPng() {
        return Promise.reject(new Error('worker crashed'));
      },
    };
    await expect(
      waitForStableFrame(viewer, { pollIntervalMs: 1, timeoutMs: 1_000 }),
    ).rejects.toMatchObject({ code: 'export-failed' });
  });

  it('отписывается от тиков после завершения', async () => {
    const viewer = fakeViewer([bytes('a'), bytes('a')]);
    let subscribed = 0;
    const counting: StableFrameViewer = {
      onTick(handler) {
        subscribed += 1;
        return viewer.onTick(handler);
      },
      exportPng: () => viewer.exportPng(),
    };
    await waitForStableFrame(counting, {
      change: () => {
        viewer.tick();
      },
      pollIntervalMs: 1,
      timeoutMs: 1_000,
    });
    expect(subscribed).toBe(1);
    // После отписки тик не должен ничего ломать.
    expect(() => {
      viewer.tick();
    }).not.toThrow();
  });
});

/** Настоящий PNG заданного цвета: `writePng` проверяет заголовок. */
function pngOf(red: number, green: number, blue: number): Uint8Array {
  return encodePng({
    width: 2,
    height: 2,
    data: new Uint8Array([
      red, green, blue, 255, red, green, blue, 255,
      red, green, blue, 255, red, green, blue, 255,
    ]),
  });
}

describe('captureStableFrame', () => {
  it('пишет доказанный кадр, а не тот, что вьюер отдаёт после стабильности', async () => {
    const stableFrame = pngOf(10, 20, 30);
    const laterFrame = pngOf(200, 100, 50);
    const handlers = new Set<(stats: unknown) => void>();
    let polls = 0;
    const viewer: StableFrameViewer = {
      onTick(handler) {
        handlers.add(handler);
        return () => {
          handlers.delete(handler);
        };
      },
      async exportPng() {
        polls += 1;
        // Первые два опроса — доказанный кадр, дальше «перерисовка».
        return polls <= 2 ? stableFrame : laterFrame;
      },
    };
    const capture: ManifestCapture = {
      id: 'page-1',
      kind: 'docx-page',
      pageNumber: 1,
      image: { uri: 'x', sha256: '', width: 2, height: 2, bytes: 0, colorType: 'RGBA8' },
    };
    const outDir = await mkdtemp(path.join(tmpdir(), 'oracle-stable-'));
    try {
      const { image, stable } = await captureStableFrame({
        outDir,
        capture,
        viewer,
        change: () => {
          for (const handler of handlers) handler(undefined);
        },
        options: { timeoutMs: 5_000 },
      });
      const written = new Uint8Array(await readFile(image.path));
      expect(sha256Hex(written)).toBe(sha256Hex(stableFrame));
      expect(sha256Hex(written)).not.toBe(sha256Hex(laterFrame));
      expect(image.sha256).toBe(sha256Hex(stableFrame));
      expect(image.stableHashes).toEqual(stable.hashes);
      expect(image.timeToStableMs).toBe(stable.elapsedMs);
      expect(image.pageNumber).toBe(1);
      // Третий снимок не запрашивался: писать нужно уже доказанное.
      expect(polls).toBe(2);
    } finally {
      await rm(outDir, { recursive: true, force: true });
    }
  });
});

describe('pdftoppmArgs', () => {
  it('строит аргументы с зафиксированным dpi', () => {
    expect(pdftoppmArgs('/tmp/doc.pdf', '/tmp/page')).toEqual([
      '-png',
      '-r',
      '144',
      '/tmp/doc.pdf',
      '/tmp/page',
    ]);
  });

  it('по умолчанию берёт dpi из матрицы сертификации', () => {
    expect(RASTER_DPI).toBe(144);
    expect(pdftoppmArgs('a.pdf', 'p')[2]).toBe(String(RASTER_DPI));
  });

  it('позволяет задать dpi явно', () => {
    expect(pdftoppmArgs('a.pdf', 'p', 72)).toEqual(['-png', '-r', '72', 'a.pdf', 'p']);
  });
});

describe('коды ошибок захвата', () => {
  it('покрывает все коды таблицей', () => {
    expect([...CAPTURE_ERROR_CODES].sort()).toEqual(
      [
        'browser-failed',
        'export-failed',
        'fixture-not-listed',
        'missing-tool',
        'open-failed',
        'rasterize-failed',
        'unstable-frame',
      ].sort(),
    );
  });

  it('относит сбои движка к capture-error', () => {
    expect(resultTypeForCode('open-failed')).toBe('capture-error');
    expect(resultTypeForCode('unstable-frame')).toBe('capture-error');
    expect(resultTypeForCode('export-failed')).toBe('capture-error');
  });

  it('относит сбои контура к harness-error', () => {
    expect(resultTypeForCode('missing-tool')).toBe('harness-error');
    expect(resultTypeForCode('fixture-not-listed')).toBe('harness-error');
    expect(resultTypeForCode('browser-failed')).toBe('harness-error');
    expect(resultTypeForCode('rasterize-failed')).toBe('harness-error');
  });

  it('несёт код в самом исключении', () => {
    const error = new CaptureError('missing-tool', 'pdftoppm is not available');
    expect(error).toBeInstanceOf(Error);
    expect(error.code).toBe('missing-tool');
    expect(error.message).toBe('pdftoppm is not available');
  });
});
