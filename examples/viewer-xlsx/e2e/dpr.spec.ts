// Плотность пикселей 2: кадр должен рисоваться в физическом разрешении
// холста, а не растягиваться браузером.
//
// Все проверки — структурные, у каждой точный ответ, не зависящий от
// сглаживания: размер кадра, край серой заливки, координата линии сетки,
// адрес ячейки. Корреляции яркостных профилей здесь намеренно нет: на сдвиге
// в один физический пиксель она падает на проценты, и порог по ней — флак.
//
// Эталон — та же страница в контексте с DPR=1 и тем же CSS-окном.

import { expect, test, type Page } from '@playwright/test';

/** Оба контекста получают одно CSS-окно: кадры отличаются только DPR. */
const VIEWPORT = { width: 1100, height: 620 };

/** API вьюера, вывешенное примером на `window` (см. `src/main.ts`). */
interface ViewerBridge {
  exportPng(): Promise<Uint8Array>;
  scrollTo(x: number, y: number): void;
  hitTest(clientX: number, clientY: number): Promise<[number, number] | null>;
}

/** Геометрия кадра: размеры, края полос заголовков, сетка и резкость краёв. */
interface FrameShape {
  width: number;
  height: number;
  /** Правый край серой заливки заголовков строк, физические пиксели. */
  headerW: number;
  /** Нижний край серой заливки заголовков столбцов, физические пиксели. */
  headerH: number;
  /** Центры вертикальных линий сетки, физические пиксели. */
  gridX: number[];
  /** Медианный шаг сетки по x, физические пиксели. */
  gridStep: number;
  /** Сколько строк кадра участвовало в замере перехода «заливка → фон». */
  edgeRampRows: number;
  /** Самый длинный такой переход по числу полутонов, физические пиксели. */
  maxEdgeRamp: number;
}

/** Открыть пример с плотной фикстурой (лист больше окна — есть что прокручивать). */
async function openFixture(page: Page, errors: string[]): Promise<void> {
  page.on('pageerror', (error) => errors.push(String(error)));
  await page.goto('/');
  // Список фикстур подгружается запросом: пока опции нет, `selectOption` не сработает.
  await page.locator('#fixture option[value="content-dense.xlsx"]').waitFor({ state: 'attached' });
  await page.selectOption('#fixture', 'content-dense.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });
  await expect(page.locator('#cmds')).not.toHaveText('0');
}

/** PNG кадра в base64: перенести картинку в другую страницу иначе нечем. */
async function exportPngBase64(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const hook = (window as unknown as { docConverter?: { viewer?: ViewerBridge } }).docConverter?.viewer;
    if (!hook) throw new Error('вьюер не поднялся');
    const bytes = await hook.exportPng();
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x4000) {
      for (const byte of bytes.subarray(i, i + 0x4000)) binary += String.fromCharCode(byte);
    }
    return btoa(binary);
  });
}

/** Прокрутить лист одним и тем же вызовом API, что из основного кода примера. */
async function scrollTo(page: Page, x: number, y: number): Promise<void> {
  await page.evaluate(
    ([px, py]) => {
      const hook = (window as unknown as { docConverter?: { viewer?: ViewerBridge } }).docConverter?.viewer;
      if (!hook) throw new Error('вьюер не поднялся');
      hook.scrollTo(px, py);
    },
    [x, y] as [number, number],
  );
}

/**
 * Разобрать оба PNG и вернуть их геометрию. Декодер PNG живёт в странице:
 * в Node его нет, а `ImageBitmap` есть.
 *
 * Третий кадр — контроль метрики резкости: эталон, растянутый вдвое со
 * сглаживанием, то есть ровно тот артефакт, который тест обязан отличать от
 * честной отрисовки в DPR=2.
 */
async function measureFrames(
  page: Page,
  ownBase64: string,
  refBase64: string,
): Promise<{ own: FrameShape; ref: FrameShape; upscaled: FrameShape }> {
  return page.evaluate(async ({ own: ownB64, ref: refB64 }) => {
    interface Img {
      w: number;
      h: number;
      data: Uint8ClampedArray<ArrayBuffer>;
    }

    const decode = async (b64: string): Promise<Img> => {
      const binary = atob(b64);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      const image = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
      return { w: bitmap.width, h: bitmap.height, data: image.data };
    };

    const luma = (img: Img, x: number, y: number): number => {
      const i = (y * img.w + x) * 4;
      return 0.299 * img.data[i]! + 0.587 * img.data[i + 1]! + 0.114 * img.data[i + 2]!;
    };
    /** Полосы заголовков залиты ровно `#f5f5f5` (`dl_color::HEADER_BG`). */
    const isHeaderFill = (img: Img, x: number, y: number): boolean =>
      Math.abs(luma(img, x, y) - 245) <= 4;

    const shape = (img: Img): FrameShape => {
      /** Доля пикселей заливки в прямоугольнике x0..x1, y0..y1. */
      const grayShare = (x0: number, x1: number, y0: number, y1: number): number => {
        let gray = 0;
        for (let y = y0; y < y1; y += 1) {
          for (let x = x0; x < x1; x += 1) if (isHeaderFill(img, x, y)) gray += 1;
        }
        return gray / ((x1 - x0) * (y1 - y0));
      };

      // Края полос заголовков: полоса серая почти на всей длине, а за краем
      // серая лишь узкая полоска ортогональной полосы (44 px из 1100 — это
      // 4%). Порог 0.2 отделяет их даже в столбцах с цифрами номеров строк,
      // где заливку местами перекрывают глифы. Замер по одной строке не
      // годится — луч попадает в букву или цифру.
      const SHARE = 0.2;
      let headerH = 0;
      while (headerH < img.h && grayShare(0, img.w, headerH, headerH + 1) > SHARE) headerH += 1;
      let headerW = 0;
      while (
        headerW < img.w &&
        grayShare(headerW, headerW + 1, Math.min(headerH + 2, img.h), img.h) > SHARE
      ) {
        headerW += 1;
      }

      // Вертикальные линии сетки: столбец тёмный сверху донизу. У текста
      // между строками и глифами белые промежутки, у линии их нет; просто
      // «тёмный» столбец даёт и выровненный по правому краю текст.
      const gridX: number[] = [];
      let runStart = -1;
      for (let x = headerW + 2; x <= img.w; x += 1) {
        let isLine = false;
        if (x < img.w) {
          isLine = true;
          for (let y = headerH; y < img.h; y += 1) {
            if (luma(img, x, y) >= 251) {
              isLine = false;
              break;
            }
          }
        }
        if (isLine && runStart < 0) runStart = x;
        if (!isLine && runStart >= 0) {
          gridX.push((runStart + x - 1) / 2);
          runStart = -1;
        }
      }
      const steps: number[] = [];
      for (let i = 1; i < gridX.length; i += 1) steps.push(gridX[i]! - gridX[i - 1]!);
      steps.sort((a, b) => a - b);
      const gridStep = steps[Math.floor(steps.length / 2)] ?? 0;

      // Переход «серая заливка → белый фон»: сколько физических пикселей
      // лежит между заливкой и фоном (в переход входит разделительная линия
      // полосы). У честного кадра это ширина линии, у растянутого — широкая
      // рампа. Строки, где за краем сразу текст, пропускаем: там за первым
      // белым нет настоящего фона.
      const ramps: number[] = [];
      for (let y = headerH + 4; y < img.h - 4; y += 1) {
        let firstWhite = -1;
        const right = Math.min(img.w, headerW + 16);
        for (let x = Math.max(0, headerW - 1); x < right; x += 1) {
          if (luma(img, x, y) >= 253) {
            firstWhite = x;
            break;
          }
        }
        if (firstWhite < 0) continue;
        let background = true;
        for (let x = firstWhite; x < Math.min(img.w, firstWhite + 6); x += 1) {
          if (luma(img, x, y) < 250) {
            background = false;
            break;
          }
        }
        if (!background) continue;
        let ramp = 0;
        for (let x = Math.max(0, headerW - 1); x < firstWhite; x += 1) {
          const value = luma(img, x, y);
          if (value < 253 && Math.abs(value - 245) > 3) ramp += 1;
        }
        ramps.push(ramp);
      }

      return {
        width: img.w,
        height: img.h,
        headerW,
        headerH,
        gridX,
        gridStep,
        edgeRampRows: ramps.length,
        maxEdgeRamp: ramps.length > 0 ? Math.max(...ramps) : -1,
      };
    };

    const own = await decode(ownB64);
    const ref = await decode(refB64);
    const control = new OffscreenCanvas(ref.w * 2, ref.h * 2);
    const controlCtx = control.getContext('2d')!;
    const source = new OffscreenCanvas(ref.w, ref.h);
    source.getContext('2d')!.putImageData(new ImageData(ref.data, ref.w, ref.h), 0, 0);
    controlCtx.drawImage(source, 0, 0, control.width, control.height);
    const controlImage = controlCtx.getImageData(0, 0, control.width, control.height);

    return {
      own: shape(own),
      ref: shape(ref),
      upscaled: shape({ w: control.width, h: control.height, data: controlImage.data }),
    };
  }, { own: ownBase64, ref: refBase64 });
}

test('DPR=2: вдвое больше физических пикселей при том же кадре', async ({ browser, page }) => {
  const errors: string[] = [];

  await page.setViewportSize(VIEWPORT);
  await openFixture(page, errors);
  expect(await page.evaluate(() => devicePixelRatio)).toBe(2);

  const reference = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
  try {
    const refPage = await reference.newPage();
    await openFixture(refPage, errors);
    expect(await refPage.evaluate(() => devicePixelRatio)).toBe(1);

    // Окно в CSS-пикселях общее, иначе сравнивать кадры нечего.
    const scrollerBox = (target: Page) =>
      target.evaluate(() => {
        const scroller = document.querySelector('#viewer > div') as HTMLElement;
        const rect = scroller.getBoundingClientRect();
        return {
          w: scroller.clientWidth,
          h: scroller.clientHeight,
          left: rect.left,
          top: rect.top,
        };
      });
    const ownBox = await scrollerBox(page);
    const refBox = await scrollerBox(refPage);
    expect(ownBox.w).toBe(refBox.w);
    expect(ownBox.h).toBe(refBox.h);
    expect(Math.abs(ownBox.left - refBox.left)).toBeLessThanOrEqual(1);
    expect(Math.abs(ownBox.top - refBox.top)).toBeLessThanOrEqual(1);

    /** Проверить кадр: он отличается от DPR=1 только разрешением. */
    const expectDoubleResolution = (frames: {
      own: FrameShape;
      ref: FrameShape;
      upscaled: FrameShape;
    }): void => {
      const { own, ref } = frames;

      // 1. Физическое разрешение: кадр ровно вдвое больше окна по каждой стороне.
      expect(own.width).toBe(ownBox.w * 2);
      expect(own.height).toBe(ownBox.h * 2);
      expect(ref.width).toBe(ownBox.w);
      expect(ref.height).toBe(ownBox.h);

      // 2. Полосы заголовков: 44 и 20 логических px (`crates/xlsx/src/paint.rs`).
      // Сверяем абсолютную величину с DPR=1 и удвоенную — с DPR=2: если детектор
      // ошибётся и вернёт 0, проверка на удвоение прошла бы вхолостую.
      expect(ref.headerW).toBeGreaterThanOrEqual(41);
      expect(ref.headerW).toBeLessThanOrEqual(47);
      expect(ref.headerH).toBeGreaterThanOrEqual(17);
      expect(ref.headerH).toBeLessThanOrEqual(23);
      expect(Math.abs(own.headerW - ref.headerW * 2)).toBeLessThanOrEqual(2);
      expect(Math.abs(own.headerH - ref.headerH * 2)).toBeLessThanOrEqual(2);

      // 3. Та же область листа: каждой линии сетки эталона отвечает линия кадра
      // на удвоенной координате, а шаг сетки вдвое больше. Другая область или
      // масштаб развели бы их на десятки пикселей.
      expect(own.gridX.length).toBeGreaterThan(6);
      const matched = ref.gridX.filter((center) =>
        own.gridX.some((x) => Math.abs(x - center * 2) <= 3),
      );
      expect(matched.length).toBeGreaterThanOrEqual(ref.gridX.length - 2);
      expect(Math.abs(own.gridStep - ref.gridStep * 2)).toBeLessThanOrEqual(2);
    };

    /**
     * Резкость края заливки — только на кадре без прокрутки: после сдвига
     * у самой кромки оказывается обрезанное число, и замер перехода перестаёт
     * быть замером заливки.
     *
     * В переход входит разделительная линия полосы, поэтому у честного кадра
     * это её два физических пикселя. Контроль — тот же замер на эталоне,
     * растянутом вдвое: размытие обязано дать заметно более длинную рампу,
     * иначе метрика ничего не различает.
     */
    const expectSharpFillEdge = (frames: { own: FrameShape; upscaled: FrameShape }): void => {
      expect(frames.own.edgeRampRows).toBeGreaterThan(100);
      expect(frames.own.maxEdgeRamp).toBeLessThanOrEqual(2);
      expect(frames.upscaled.maxEdgeRamp).toBeGreaterThanOrEqual(frames.own.maxEdgeRamp + 3);
    };

    /** Адрес ячейки под точкой окна: общий для обоих контекстов. */
    const cellsAt = (target: Page, points: Array<[number, number]>) =>
      target.evaluate(async (pts) => {
        const hook = (window as unknown as { docConverter?: { viewer?: ViewerBridge } }).docConverter
          ?.viewer;
        if (!hook) throw new Error('вьюер не поднялся');
        return Promise.all(pts.map(([x, y]) => hook.hitTest(x, y)));
      }, points);

    const contentPoints: Array<[number, number]> = [
      [ownBox.left + 150, ownBox.top + 100],
      [ownBox.left + ownBox.w / 2, ownBox.top + ownBox.h / 2],
      [ownBox.left + ownBox.w - 80, ownBox.top + ownBox.h - 50],
    ];

    const firstFrames = await measureFrames(page, await exportPngBase64(page), await exportPngBase64(refPage));
    const brief = (s: FrameShape) => ({ ...s, gridX: [s.gridX.length, s.gridX[0], s.gridX.at(-1)] });
    console.log('CALIB', JSON.stringify({ own: brief(firstFrames.own), ref: brief(firstFrames.ref), up: brief(firstFrames.upscaled) }));
    expectDoubleResolution(firstFrames);
    expectSharpFillEdge(firstFrames);

    // Те же ячейки под теми же точками окна: координаты холста переводятся в
    // координаты листа с учётом DPR, но не зависят от него.
    const ownCells = await cellsAt(page, contentPoints);
    expect(ownCells).not.toContain(null);
    expect(ownCells).toEqual(await cellsAt(refPage, contentPoints));

    // Прокрутка: окно в единицах раскладки не должно умножаться на DPR.
    const ownBefore = await page.locator('#frame').textContent();
    const refBefore = await refPage.locator('#frame').textContent();
    await scrollTo(page, 40, 30);
    await scrollTo(refPage, 40, 30);
    for (const [target, before] of [
      [page, ownBefore],
      [refPage, refBefore],
    ] as const) {
      await target.waitForFunction(
        (previous) => document.querySelector('#frame')?.textContent !== previous,
        before,
      );
    }
    const scrolledAt = await page.evaluate(() => {
      const scroller = document.querySelector('#viewer > div') as HTMLElement;
      return { x: scroller.scrollLeft, y: scroller.scrollTop };
    });
    expect(scrolledAt.x).toBeGreaterThan(0);
    expect(scrolledAt.y).toBeGreaterThan(0);

    expectDoubleResolution(await measureFrames(page, await exportPngBase64(page), await exportPngBase64(refPage)));
    expect(await cellsAt(page, contentPoints)).toEqual(await cellsAt(refPage, contentPoints));

    expect(errors).toEqual([]);
  } finally {
    await reference.close();
  }
});
