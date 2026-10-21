// Сквозная проверка стримового открытия (ADR-0031): документ едет в воркер
// чанками из `fetch().body` — main-поток держит не файл, а чанк. Проверяем
// промис открытия, кадр на холсте и то, что оборванный поток не роняет воркер.

import { expect, test } from '@playwright/test';
import { boot } from './helpers/boot.js';

test('открывает DOCX потоком и рисует страницу', async ({ page }) => {
  const errors = await boot(page, '/docx');

  const opened = await page.evaluate(async () => {
    const viewer = window.docConverter!.viewer;
    const response = await fetch(
      new URL('fixtures/docx/simple/one_paragraph.docx', document.baseURI),
    );
    if (!response.body) throw new Error('нет тела ответа — поток не из чего собрать');
    const total = Number(response.headers.get('content-length'));
    return viewer.open(response.body, { size: total });
  });
  expect(opened).toHaveLength(1);

  // Кадр доехал до painter'а: PNG непустой, значит кадр нарисовался.
  const pngBytes = await page.evaluate(async () =>
    (await window.docConverter!.viewer.exportPng()).byteLength,
  );
  expect(pngBytes).toBeGreaterThan(100);

  expect(errors).toEqual([]);
});

test('пустой поток — ошибка открытия, воркер остаётся живым', async ({ page }) => {
  const errors = await boot(page, '/docx');

  const rejected = await page.evaluate(async () => {
    const viewer = window.docConverter!.viewer;
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.close();
      },
    });
    try {
      await viewer.open(stream);
      return 'resolved';
    } catch (e) {
      return String(e);
    }
  });
  expect(rejected).toContain('stream aborted');

  // Страница честно пишет отказ открытия в консоль — это ожидаемая ошибка,
  // а не падение: дальше следим только за новыми ошибками.
  errors.length = 0;

  // Отказ одного потока — не падение воркера: обычное открытие работает.
  const pages = await page.evaluate(async () => {
    const viewer = window.docConverter!.viewer;
    const response = await fetch(
      new URL('fixtures/docx/simple/one_paragraph.docx', document.baseURI),
    );
    return viewer.open(await response.arrayBuffer());
  });
  expect(pages).toHaveLength(1);

  expect(errors).toEqual([]);
});
