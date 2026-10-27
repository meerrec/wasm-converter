// Сквозная проверка стримового открытия (ADR-0031, ADR-0049): документ едет в
// воркер чанками из `fetch().body` — main-поток держит не файл, а чанк.
// Проверяем промис открытия, кадр на холсте и то, что оборванный поток не
// роняет воркер. Форматы — все, что подключены в примере: XLSB/ODS/ODT/ODP
// резолверов модулей в `viewer_urls.ts` не имеют, их потоковое покрытие живёт
// в тестах ядра (`test/worker_core_stream.spec.ts`).

import { expect, test } from '@playwright/test';
import { boot } from './helpers/boot.js';

/**
 * По формату: маршрут страницы и фикстура, которую страница отдаёт по HTTP.
 *
 * `ready` нужен там, где страница открывает фикстуру сама: `#/pptx` публикует
 * вьюер до этого открытия, и поток, начатый сразу после `boot`, вытеснял бы
 * его — одноразовый флаг отмены (ADR-0032) доставался уже нашему разбору.
 */
const STREAM_CASES = [
  { route: '/docx', file: 'fixtures/docx/simple/one_paragraph.docx' },
  { route: '/xlsx', file: 'fixtures/xlsx/sheets-three.xlsx' },
  {
    route: '/pptx',
    file: 'fixtures/pptx/simple/two_slides.pptx',
    ready: { selector: '#slide-info', text: '1 / 2' },
  },
] as const;

for (const streamCase of STREAM_CASES) {
  const { route, file } = streamCase;
  test(`открывает ${route} потоком и рисует кадр`, async ({ page }) => {
    const errors = await boot(page, route);
    if ('ready' in streamCase) {
      // Демо уже открыло свою презентацию — ждём её конца, чтобы не вытеснять.
      await expect(page.locator(streamCase.ready.selector)).toHaveText(streamCase.ready.text);
    }

    // Число страниц/листов/слайдов: у каждого формата свой ответ открытия,
    // поэтому проверяется непустота, а не конкретная длина.
    const opened = await page.evaluate(async (path) => {
      const viewer = window.docConverter!.viewer;
      const response = await fetch(new URL(path, document.baseURI));
      if (!response.body) throw new Error('нет тела ответа — поток не из чего собрать');
      const total = Number(response.headers.get('content-length'));
      const result = await viewer.open(response.body, { size: total });
      return Array.isArray(result) ? result.length : 0;
    }, file);
    expect(opened).toBeGreaterThan(0);

    // Кадр доехал до painter'а: PNG непустой, значит кадр нарисовался.
    const pngBytes = await page.evaluate(
      async () => (await window.docConverter!.viewer.exportPng()).byteLength,
    );
    expect(pngBytes).toBeGreaterThan(100);

    expect(errors).toEqual([]);
  });
}

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
