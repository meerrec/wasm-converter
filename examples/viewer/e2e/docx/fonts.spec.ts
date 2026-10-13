// Предпросмотр рисует браузерным canvas'ом: без зарегистрированного `FontFace`
// строка `ctx.font = "…px Carlito"` уходит в подстановку браузера, и текст
// разъезжается с PDF, куда те же сабсеты встроены, — это и ловим.
//
// Точного совпадения с метриками раскладки при этом требовать нельзя: браузер
// применяет кернинг, а `FontRegistry::measure()` его ещё не считает — строка
// собирается суммой advance'ов (ADR-0005), отсюда ≈0,5 % расхождения на этой
// строке.
//
// Замер идёт в самом воркере: `FontFace` регистрируется в его области
// видимости, документу не виден, а `document.fonts.check` для этой проверки
// не годится — она отвечает `true` и при отсутствующем шрифте.

import { expect, test } from '@playwright/test';
import { boot } from '../helpers/boot.js';

/** Эталонная строка: кириллица, латиница и цифры — набор сабсета целиком. */
const PROBE = 'Проверка шрифта Wg 123';

/** Ширина строки настоящим сабсетом Carlito при 32px. */
const SUBSET_PX = 349.05;

/** Подстановка браузера на той же строке — ≈397px: промах видно сразу. */
const FALLBACK_PX = 397.0;

/**
 * Допуск замера: платформенное квантование advance'ов. FreeType на Linux
 * округляет продвижение каждого глифа до целого пикселя, браузер на macOS
 * считает его дробным, — на строке-зонде это даёт худший случай в полпикселя
 * на глиф. Различению «сабсет или подстановка» допуск не мешает: до
 * подстановки (≈397px) остаётся 397 − 349 − 11 ≈ 37px запаса.
 */
const TOLERANCE_PX = PROBE.length / 2;

/** Итог ожидания регистрации. */
type FaceState = 'loaded' | 'no-fonts-api' | 'timeout';

/** Запускается в воркере предпросмотра; живёт без окружающих имён. */
const waitForCarlito = async (): Promise<FaceState> => {
  const scope = globalThis as unknown as { fonts?: FontFaceSet };
  const fonts = scope.fonts;
  if (!fonts) return 'no-fonts-api';
  const deadline = Date.now() + 10_000;
  for (;;) {
    const face = Array.from(fonts).find((candidate) => candidate.family === 'Carlito');
    if (face?.status === 'loaded') return 'loaded';
    if (Date.now() > deadline) return 'timeout';
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
};

/** Замер в воркере; строка шрифта — как у painter'а для 32px regular. */
const measure = async (text: string): Promise<number> => {
  const ctx = new OffscreenCanvas(600, 100).getContext('2d');
  if (!ctx) throw new Error('нет 2d-контекста');
  ctx.font = '32px Carlito, sans-serif';
  return ctx.measureText(text).width;
};

test('воркер меряет текст настоящим сабсетом Carlito', async ({ page }) => {
  const workerPromise = page.waitForEvent('worker', {
    predicate: (candidate) => candidate.url().includes('docx_worker'),
  });
  const errors = await boot(page, '/docx');
  const worker = await workerPromise;

  // Сначала регистрация, потом замер — и только в этом порядке: в Chromium
  // промах по конкретной строке шрифта, записанный движком до регистрации,
  // остаётся в кэше воркера на всю сессию, и замер до неё отравил бы проверку
  // навсегда (проверено зондами слайса).
  const state = await worker.evaluate(waitForCarlito);
  expect(state, 'сабсет Carlito не зарегистрировался в воркере').toBe('loaded');

  const width = await worker.evaluate(measure, PROBE);
  expect(
    Math.abs(width - SUBSET_PX),
    `ширина «${PROBE}» — ${width}px: сабсет Carlito даёт ${SUBSET_PX}px, ` +
      `подстановка браузера ≈${FALLBACK_PX}px`,
  ).toBeLessThanOrEqual(TOLERANCE_PX);

  expect(errors).toEqual([]);
});
