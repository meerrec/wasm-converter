import { describe, expect, it } from 'vitest';

import { classify, type ClassifyInput } from '../src/diff/classify.js';
import {
  COLOR_TOLERANCE,
  MAX_REGIONS,
  MIN_REGION_PIXELS,
  REGION_CELL,
  compareImages,
} from '../src/diff/compare.js';
import { PngError, decodePng, encodePng, type RgbaImage } from '../src/diff/png.js';
import type { Rect } from '../src/types.js';

/** Однотонное изображение: остальные тесты меняют отдельные пиксели. */
function solid(width: number, height: number, rgba: [number, number, number, number]): RgbaImage {
  const data = new Uint8Array(width * height * 4);
  for (let pixel = 0; pixel < width * height; pixel += 1) {
    data[pixel * 4] = rgba[0];
    data[pixel * 4 + 1] = rgba[1];
    data[pixel * 4 + 2] = rgba[2];
    data[pixel * 4 + 3] = rgba[3];
  }
  return { width, height, data };
}

/** Копия изображения: тесты правят её, не трогая эталон. */
function clone(image: RgbaImage): RgbaImage {
  return { width: image.width, height: image.height, data: new Uint8Array(image.data) };
}

/** Поставить пиксель по координатам. */
function setPixel(image: RgbaImage, x: number, y: number, rgba: [number, number, number, number]): void {
  const offset = (y * image.width + x) * 4;
  image.data[offset] = rgba[0];
  image.data[offset + 1] = rgba[1];
  image.data[offset + 2] = rgba[2];
  image.data[offset + 3] = rgba[3];
}

/** Заполнить прямоугольник одним цветом. */
function fillRect(image: RgbaImage, rect: Rect, rgba: [number, number, number, number]): void {
  for (let y = rect.y; y < rect.y + rect.height; y += 1) {
    for (let x = rect.x; x < rect.x + rect.width; x += 1) setPixel(image, x, y, rgba);
  }
}

describe('compareImages: размеры', () => {
  it('разные размеры дают page-mismatch без метрик', () => {
    const outcome = compareImages(solid(4, 4, [0, 0, 0, 255]), solid(4, 5, [0, 0, 0, 255]));
    expect(outcome.kind).toBe('page-mismatch');
    if (outcome.kind !== 'page-mismatch') return;
    expect(outcome.reason).toBe('dimensions');
    expect(outcome.reference).toEqual({ width: 4, height: 4 });
    expect(outcome.actual).toEqual({ width: 4, height: 5 });
    expect('metrics' in outcome).toBe(false);
  });
});

describe('compareImages: цветовой допуск', () => {
  it('дельта ровно 2 по каналу — совпадение', () => {
    const reference = solid(8, 8, [100, 100, 100, 255]);
    const actual = clone(reference);
    setPixel(actual, 3, 3, [100 + COLOR_TOLERANCE, 100, 100, 255]);
    const outcome = compareImages(reference, actual);
    expect(outcome.kind).toBe('compared');
    if (outcome.kind !== 'compared') return;
    expect(outcome.metrics.diffPixels).toBe(0);
    expect(outcome.metrics.maxChannelDelta).toBe(COLOR_TOLERANCE);
    expect(outcome.regions).toEqual([]);
  });

  it('дельта 3 по каналу — расхождение', () => {
    const reference = solid(8, 8, [100, 100, 100, 255]);
    const actual = clone(reference);
    setPixel(actual, 3, 3, [100 + COLOR_TOLERANCE + 1, 100, 100, 255]);
    const outcome = compareImages(reference, actual);
    expect(outcome.kind).toBe('compared');
    if (outcome.kind !== 'compared') return;
    expect(outcome.metrics.diffPixels).toBe(1);
    expect(outcome.metrics.maxChannelDelta).toBe(COLOR_TOLERANCE + 1);
  });

  it('отрицательная дельта считается по модулю', () => {
    const reference = solid(8, 8, [100, 100, 100, 255]);
    const actual = clone(reference);
    setPixel(actual, 0, 0, [100 - COLOR_TOLERANCE - 1, 100, 100, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(1);
  });

  it('alpha обязана совпадать точно: дельта 1 — расхождение', () => {
    const reference = solid(8, 8, [10, 20, 30, 255]);
    const actual = clone(reference);
    setPixel(actual, 1, 1, [10, 20, 30, 254]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(1);
    expect(outcome.metrics.maxChannelDelta).toBe(1);
  });

  it('diffRatio считается по числу сравнённых пикселей', () => {
    const reference = solid(10, 10, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 0, y: 0, width: 10, height: 5 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(50);
    expect(outcome.metrics.diffRatio).toBeCloseTo(0.5, 10);
  });
});

describe('compareImages: маски', () => {
  it('расхождение внутри маски не считается', () => {
    const reference = solid(16, 16, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 4, y: 4, width: 8, height: 8 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual, [{ x: 4, y: 4, width: 8, height: 8 }]);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(0);
    expect(outcome.metrics.diffRatio).toBe(0);
    expect(outcome.regions).toEqual([]);
  });

  it('маска не скрывает расхождение за своими границами', () => {
    const reference = solid(16, 16, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 4, y: 4, width: 8, height: 8 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual, [{ x: 4, y: 4, width: 4, height: 4 }]);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(64 - 16);
  });

  it('маска за границей изображения обрезается', () => {
    const reference = solid(8, 8, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 0, y: 0, width: 8, height: 8 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual, [{ x: -100, y: -100, width: 1000, height: 1000 }]);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(0);
  });

  it('полностью замаскированное изображение даёт diffRatio 0 и ssim null', () => {
    const reference = solid(8, 8, [0, 0, 0, 255]);
    const actual = solid(8, 8, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual, [{ x: 0, y: 0, width: 8, height: 8 }]);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(0);
    expect(outcome.metrics.diffRatio).toBe(0);
    expect(outcome.metrics.ssim).toBeNull();
  });
});

describe('compareImages: SSIM', () => {
  it('идентичные изображения дают 1', () => {
    const image = solid(16, 16, [120, 130, 140, 255]);
    const outcome = compareImages(image, clone(image));
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.ssim).toBe(1);
  });

  it('контрастное расхождение даёт меньше 1', () => {
    const reference = solid(16, 16, [0, 0, 0, 255]);
    const actual = solid(16, 16, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.ssim).not.toBeNull();
    expect(outcome.metrics.ssim ?? 1).toBeLessThan(0.5);
  });

  it('изображение меньше окна даёт ssim null', () => {
    const reference = solid(4, 4, [0, 0, 0, 255]);
    const outcome = compareImages(reference, clone(reference));
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.ssim).toBeNull();
  });

  it('окно, задетое маской, пропускается', () => {
    const reference = solid(16, 16, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 0, y: 0, width: 8, height: 8 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual, [{ x: 0, y: 0, width: 8, height: 8 }]);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    // Осталось три окна из четырёх, все идентичны.
    expect(outcome.metrics.ssim).toBe(1);
  });
});

describe('compareImages: регионы', () => {
  it('соседние расходящиеся пиксели сливаются в один регион', () => {
    const reference = solid(32, 32, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 8, y: 8, width: 4, height: 4 }, [255, 255, 255, 255]);
    fillRect(actual, { x: 12, y: 8, width: 4, height: 4 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.regions).toHaveLength(1);
    // Регион — объединение ячеек сетки 8×8, а не точный bounding box пикселей.
    expect(outcome.regions[0]).toEqual({ x: 8, y: 8, width: 8, height: 8 });
  });

  it('далёкие пятна дают разные регионы, отсортированные по площади', () => {
    const reference = solid(64, 64, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 0, y: 0, width: 8, height: 8 }, [255, 255, 255, 255]);
    fillRect(actual, { x: 40, y: 40, width: 16, height: 16 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.regions).toHaveLength(2);
    expect(outcome.regions[0]).toEqual({ x: 40, y: 40, width: 16, height: 16 });
    expect(outcome.regions[1]).toEqual({ x: 0, y: 0, width: 8, height: 8 });
  });

  it('пятно меньше порога отбрасывается', () => {
    const reference = solid(32, 32, [0, 0, 0, 255]);
    const actual = clone(reference);
    const side = Math.floor(Math.sqrt(MIN_REGION_PIXELS - 1));
    fillRect(actual, { x: 0, y: 0, width: side, height: side }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.metrics.diffPixels).toBe(side * side);
    expect(outcome.regions).toEqual([]);
  });

  it('diffBounds — точный bounding box расходящихся пикселей', () => {
    const reference = solid(32, 32, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 5, y: 7, width: 3, height: 2 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.diffBounds).toEqual({ x: 5, y: 7, width: 3, height: 2 });
  });

  it('без расхождений diffBounds равен null', () => {
    const reference = solid(16, 16, [0, 0, 0, 255]);
    const outcome = compareImages(reference, clone(reference));
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.diffBounds).toBeNull();
  });

  it('число регионов ограничено', () => {
    const reference = solid(512, 512, [0, 0, 0, 255]);
    const actual = clone(reference);
    let painted = 0;
    for (let y = 0; y < 512 && painted <= MAX_REGIONS; y += REGION_CELL * 2) {
      for (let x = 0; x < 512 && painted <= MAX_REGIONS; x += REGION_CELL * 2) {
        fillRect(actual, { x, y, width: REGION_CELL, height: REGION_CELL }, [255, 255, 255, 255]);
        painted += 1;
      }
    }
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.regions.length).toBe(MAX_REGIONS);
  });
});

describe('compareImages: overlay', () => {
  it('расхождение закрашивается красным, маска — синим', () => {
    const reference = solid(16, 16, [200, 200, 200, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 0, y: 0, width: 8, height: 8 }, [0, 0, 0, 255]);
    const outcome = compareImages(reference, actual, [{ x: 8, y: 0, width: 8, height: 8 }]);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    const overlay = outcome.overlay;
    expect(overlay.width).toBe(16);
    expect(overlay.height).toBe(16);
    expect([...overlay.data.slice(0, 4)]).toEqual([255, 0, 0, 255]);
    const masked = (0 * 16 + 12) * 4;
    expect(overlay.data[masked + 2] ?? 0).toBeGreaterThan(overlay.data[masked] ?? 0);
    const clean = (12 * 16 + 12) * 4;
    expect(overlay.data[clean]).toBe(overlay.data[clean + 1]);
    expect(overlay.data[clean + 1]).toBe(overlay.data[clean + 2]);
  });
});

describe('PNG: кодек', () => {
  it('round-trip сохраняет пиксели', () => {
    const image = solid(5, 3, [12, 34, 56, 78]);
    setPixel(image, 4, 2, [255, 0, 128, 200]);
    const decoded = decodePng(encodePng(image));
    expect(decoded.width).toBe(5);
    expect(decoded.height).toBe(3);
    expect([...decoded.data]).toEqual([...image.data]);
  });

  it('битые байты дают PngError', () => {
    expect(() => decodePng(new Uint8Array([1, 2, 3]))).toThrow(PngError);
  });

  it('неверная длина буфера при кодировании даёт PngError', () => {
    expect(() => encodePng({ width: 2, height: 2, data: new Uint8Array(4) })).toThrow(PngError);
  });

  it('палитровый PNG отвергается до декодирования', () => {
    // Сигнатура + IHDR с colorType 3 (палитра): профиль `visual-v1` такое не
    // принимает, и отказ обязан случиться до разбора пикселей.
    const bytes = new Uint8Array(8 + 8 + 13);
    bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0);
    const view = new DataView(bytes.buffer);
    view.setUint32(8, 13, false);
    bytes.set([0x49, 0x48, 0x44, 0x52], 12);
    view.setUint32(16, 2, false);
    view.setUint32(20, 2, false);
    bytes[24] = 8;
    bytes[25] = 3;
    expect(() => decodePng(bytes)).toThrow(/unsupported PNG color type 3/);
  });
});

/** Минимальный вход классификатора: остальные тесты меняют одно поле. */
function classifyInput(overrides: Partial<ClassifyInput> = {}): ClassifyInput {
  return {
    fixtureId: 'docx-tables-long-table',
    captureId: 'page-1',
    reference: 'present',
    error: null,
    structure: null,
    comparison: null,
    ...overrides,
  };
}

/** Результат сравнения с заданными регионами и метриками. */
function compared(
  regions: Rect[],
  width = 100,
  height = 100,
  diffPixels = 10,
): ClassifyInput['comparison'] {
  const first = regions[0];
  return {
    kind: 'compared',
    width,
    height,
    metrics: {
      diffPixels,
      diffRatio: diffPixels / (width * height),
      maxChannelDelta: 255,
      ssim: 0.9,
    },
    regions,
    diffBounds: first ?? null,
    overlay: solid(width, height, [0, 0, 0, 255]),
  };
}

describe('classify: жёсткие правила', () => {
  it('нет эталона — missing-reference/reference', () => {
    const result = classify(classifyInput({ reference: 'missing' }));
    expect(result.resultType).toBe('missing-reference');
    expect(result.cause).toBe('reference');
    expect(result.causeSource).toBe('rule');
  });

  it('невалидный эталон — missing-reference/reference', () => {
    const result = classify(classifyInput({ reference: 'invalid' }));
    expect(result.resultType).toBe('missing-reference');
    expect(result.cause).toBe('reference');
  });

  it('сбой захвата — capture-error/harness', () => {
    const result = classify(
      classifyInput({ error: { code: 'open-failed', message: 'document did not open' } }),
    );
    expect(result.resultType).toBe('capture-error');
    expect(result.cause).toBe('harness');
    expect(result.causeEvidence).toContain('open-failed');
  });

  it('сломанный контур — harness-error/harness', () => {
    for (const code of ['missing-tool', 'browser-failed', 'fixture-not-listed', 'rasterize-failed']) {
      const result = classify(classifyInput({ error: { code, message: 'x' } }));
      expect(result.resultType).toBe('harness-error');
      expect(result.cause).toBe('harness');
    }
  });

  it('неизвестный код сбоя не выдаёт себя за harness-error', () => {
    const result = classify(classifyInput({ error: { code: 'weird', message: 'x' } }));
    expect(result.resultType).toBe('capture-error');
  });

  it('расхождение числа страниц — page-mismatch/layout', () => {
    const result = classify(
      classifyInput({ structure: { reference: 3, actual: 4, unit: 'pages' } }),
    );
    expect(result.resultType).toBe('page-mismatch');
    expect(result.cause).toBe('layout');
    expect(result.causeEvidence).toBe('pages 3 vs 4');
  });

  it('расхождение размеров — page-mismatch/layout', () => {
    const result = classify(
      classifyInput({
        comparison: {
          kind: 'page-mismatch',
          reason: 'dimensions',
          reference: { width: 1240, height: 1754 },
          actual: { width: 1240, height: 1753 },
        },
      }),
    );
    expect(result.resultType).toBe('page-mismatch');
    expect(result.cause).toBe('layout');
    expect(result.causeEvidence).toBe('dimensions 1240x1754 vs 1240x1753');
  });

  it('ноль расходящихся пикселей — match', () => {
    const result = classify(classifyInput({ comparison: compared([], 100, 100, 0) }));
    expect(result.resultType).toBe('match');
    expect(result.causeSource).toBe('rule');
    expect(result.causeEvidence).toContain('0 diff pixels');
  });

  it('захват без сравнимого изображения — harness-error', () => {
    const result = classify(classifyInput({ comparison: null }));
    expect(result.resultType).toBe('harness-error');
    expect(result.cause).toBe('harness');
  });
});

describe('classify: предварительная причина visual-diff', () => {
  it('регионы только в нижней полосе — layout', () => {
    const result = classify(
      classifyInput({ comparison: compared([{ x: 0, y: 90, width: 10, height: 10 }]) }),
    );
    expect(result.resultType).toBe('visual-diff');
    expect(result.cause).toBe('layout');
    expect(result.causeEvidence).toContain('rule=layout');
  });

  it('нижняя полоса считается по точному bounding box, а не по сетке', () => {
    // Пятно начинается на 85-й строке из 100 — ровно на границе нижней
    // полосы. Регион сетки 8×8 начинается с 80-й строки и выглядел бы как
    // расхождение выше полосы; точный box границу не размывает.
    const reference = solid(100, 100, [0, 0, 0, 255]);
    const actual = clone(reference);
    fillRect(actual, { x: 0, y: 85, width: 4, height: 4 }, [255, 255, 255, 255]);
    const outcome = compareImages(reference, actual);
    if (outcome.kind !== 'compared') throw new Error('expected compared');
    expect(outcome.regions[0]?.y).toBe(80);
    const result = classify(classifyInput({ comparison: outcome }));
    expect(result.cause).toBe('layout');
  });

  it('регион в верхней части — не layout по полосе', () => {
    const result = classify(
      classifyInput({ comparison: compared([{ x: 0, y: 0, width: 10, height: 10 }]) }),
    );
    expect(result.cause).toBe('render');
  });

  it('покрытие ≥ 60 % площади — layout', () => {
    const result = classify(
      classifyInput({ comparison: compared([{ x: 0, y: 0, width: 80, height: 80 }]) }),
    );
    expect(result.cause).toBe('layout');
  });

  it('много мелких регионов — font', () => {
    const regions: Rect[] = [];
    for (let index = 0; index < 20; index += 1) {
      regions.push({ x: (index % 10) * 10, y: Math.floor(index / 10) * 10, width: 2, height: 2 });
    }
    const result = classify(classifyInput({ comparison: compared(regions) }));
    expect(result.cause).toBe('font');
  });

  it('мало мелких регионов — render', () => {
    const regions: Rect[] = [];
    for (let index = 0; index < 5; index += 1) {
      regions.push({ x: index * 10, y: 0, width: 2, height: 2 });
    }
    const result = classify(classifyInput({ comparison: compared(regions) }));
    expect(result.cause).toBe('render');
  });
});

describe('classify: переопределения', () => {
  it('переопределение по паре fixture/capture побеждает правило', () => {
    const result = classify(
      classifyInput({
        comparison: compared([{ x: 0, y: 0, width: 10, height: 10 }]),
        overrides: { 'docx-tables-long-table/page-1': 'font' },
      }),
    );
    expect(result.cause).toBe('font');
    expect(result.causeSource).toBe('override');
    expect(result.causeEvidence).toContain('rule=render');
    expect(result.causeEvidence).toContain('override=font');
  });

  it('переопределение по фикстуре работает как запасное', () => {
    const result = classify(
      classifyInput({
        comparison: compared([{ x: 0, y: 0, width: 10, height: 10 }]),
        overrides: { 'docx-tables-long-table': 'model' },
      }),
    );
    expect(result.cause).toBe('model');
    expect(result.causeSource).toBe('override');
  });

  it('переопределение не меняет тип результата', () => {
    const result = classify(
      classifyInput({ reference: 'missing', overrides: { 'docx-tables-long-table': 'parse' } }),
    );
    expect(result.resultType).toBe('missing-reference');
    expect(result.cause).toBe('parse');
    expect(result.causeSource).toBe('override');
  });
});
