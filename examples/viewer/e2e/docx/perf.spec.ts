// Производительность виртуализации: 200-страничный документ, scroll p95.
//
// В headless-Chromium rAF не всегда репрезентативен (SwiftShader), но гейт
// ловит именно провал виртуализации: если spans всех страниц попадут в DOM,
// частота кадров упадёт независимо от растеризации.

import { expect, test } from '@playwright/test';

import { boot } from '../helpers/boot.js';
import { openFixture, pageCount } from '../helpers/docx_frame.js';

test.setTimeout(180_000);

// fps-гейт меряется только в Chromium: в WebKit/Firefox headless rAF
// троттлится и число не сравнимо; виртуализация там проверяется структурно
// (число страниц/строк в DOM) в a11y/selection-спеках.
test.skip(
  ({ browserName }) => browserName !== 'chromium',
  'p95 fps меряется только в Chromium',
);

test('200 страниц: p95 ≥ 55 fps и ограниченный DOM', async ({ page }) => {
  await boot(page, '/docx', { a11y: true });
  await openFixture(page, 'a11y/a11y_perf.docx');
  expect(await pageCount(page)).toBe(200);

  const result = await page.evaluate(async () => {
    const scroller = document.querySelector('#viewer > div');
    if (!(scroller instanceof HTMLElement)) throw new Error('нет области прокрутки');
    const deltas: number[] = [];
    let last = performance.now();
    const start = last;
    const timer = window.setInterval(() => {
      scroller.scrollTop += 80;
    }, 16);
    await new Promise<void>((resolve) => {
      const tick = (now: number): void => {
        deltas.push(now - last);
        last = now;
        if (now - start < 3000) requestAnimationFrame(tick);
        else resolve();
      };
      requestAnimationFrame(tick);
    });
    window.clearInterval(timer);
    deltas.shift();
    deltas.sort((left, right) => left - right);
    const p95 = deltas[Math.floor(deltas.length * 0.95)] ?? 0;
    return {
      frames: deltas.length,
      p95,
      fps: p95 === 0 ? 0 : 1000 / p95,
      lines: document.querySelectorAll('.dc-text-line').length,
      pages: document.querySelectorAll('.dc-text-page').length,
    };
  });

  expect(result.frames).toBeGreaterThan(30);
  expect(result.pages).toBeLessThanOrEqual(4);
  expect(result.lines).toBeLessThanOrEqual(400);
  expect(result.fps).toBeGreaterThanOrEqual(55);
});
