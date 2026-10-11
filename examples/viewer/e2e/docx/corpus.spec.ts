// Корпусный прогон: вьюер обязан открыть каждую фикстуру из `test-fixtures/docx`,
// а не одну-две, на которых ходят точечные спеки.
//
// Ожидание фикстуры берётся из её сайдкара (`<имя>.json`), а не из имени
// каталога: `broken/` — не синоним «не открывается». Сайдкар помечает
// `metadata.fatal` только то, что разбор обязан отвергнуть (ADR-0016 §2), и
// таких документов в корпусе три; остальные три из `broken/` открываются с
// предупреждением — того же деления держится `crates/docx/tests/warnings.rs`.
// Класс берётся из сайдкара, поэтому ни одна фикстура не выпадает из
// проверки молча.
//
// Движок — только chromium: 94 открытия на трёх проектах превратили бы набор в
// марафон, а кроссбраузерность держат точечные спеки (viewer, pages, zoom,
// repaint) — они и так идут на всех трёх. Отсюда `test.skip` в первом тесте.
//
// Диагностика собирается по каждой фикстуре (имя → страницы/ошибка), поэтому
// падение называет сломавшийся документ, а не «упало на 47-м».
//
// Отдельная находка обхода: одна фикстура открывается, но лист остаётся белым —
// её единственное содержимое VML-фолбэк без поддерживаемой ветки `Choice`, до
// кадра он не доходит. Она перечислена в `DRAWINGS_NOT_PAINTED` ниже и остаётся в
// корпусе: это состояние продукта, а не повод выкинуть фикстуру из проверки.

import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameId, pageCount, waitForFrame } from '../helpers/docx_frame.js';
import { frameStats, type FrameStats } from '../helpers/png.js';

/** Корень корпуса: рядом с каждым `.docx` лежит сайдкар с ожиданиями. */
const FIXTURES = path.resolve(import.meta.dirname, '../../../../test-fixtures/docx');

/**
 * Сколько проблем достаточно, чтобы остановить обход: каждая следующая не
 * добавила бы к диагнозу ничего нового, а прогон растянулся бы на таймауты.
 */
const MAX_PROBLEMS = 5;

/** Ниже этой доли кадра лист не считается нарисованным: так же судит `viewer.spec.ts`. */
const MIN_PAGE_RATIO = 0.2;

/**
 * Фикстуры, у которых на листе нет чернил, потому что рисовать нечего.
 *
 * Осталась одна: `alternate_content/fallback_only.docx` — `AlternateContent` без
 * единой поддерживаемой ветки `Choice`, один VML-фолбэк. Разбор держит блок
 * `Unknown` (сайдкар: `expectedImages: 0`), до раскладки он не доходит, и текста
 * документ не отдаёт: `exportText` у него пуст.
 *
 * Остальные фикстуры этой категории из списка ушли, когда рисунки начали
 * рисоваться: встроенные и плавающие картинки (`images/*.docx`) идут в кадр
 * командами `Image` с настоящими пикселями, а фигуры и текстовые поля
 * (`alternate_content/*`) — прямоугольником-плейсхолдером: геометрия доезжает до
 * кадра, а байтов у фигуры нет (`draw_image` без `image_id`).
 *
 * Это состояние продукта, а не отказ: открывается она как все остальные (один
 * белый лист, ни ошибок, ни исключений), и корпусный прогон держит её в общем
 * списке. Список поимённый и работает в обе стороны: у этих фикстур чернил быть
 * НЕ должно, поэтому, когда фикстура начнёт рисоваться, тест укажет на список, а
 * не проглотит изменение.
 */
const DRAWINGS_NOT_PAINTED = new Set(['alternate_content/fallback_only.docx']);

/** Ожидания фикстуры из сайдкара. */
interface Fixture {
  /** Имя с категорией: `simple/one_paragraph.docx`. */
  name: string;
  /** Обещан ли видимый элемент — абзац, таблица или рисунок. */
  hasContent: boolean;
  /** Разбор обязан отказать (`metadata.fatal`, ADR-0016 §2). */
  fatal: boolean;
}

/** Сайдкар фикстуры: берём из него только то, что задаёт ожидания. */
interface Manifest {
  metadata?: {
    fatal?: string;
    expectedParagraphs?: number;
    expectedTables?: number;
    expectedImages?: number;
  };
}

/** Текст ошибки как строка: у исключений бывает и не-`Error`. */
function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Прочитать корпус с диска: имена всех `.docx` и ожидания из их сайдкаров. */
async function readCorpus(): Promise<Fixture[]> {
  const entries = await readdir(FIXTURES, { recursive: true });
  const corpus: Fixture[] = [];
  for (const name of entries.filter((entry) => entry.endsWith('.docx')).sort()) {
    const sidecar = path.join(FIXTURES, `${name.replace(/\.docx$/, '')}.json`);
    let manifest: Manifest;
    try {
      manifest = JSON.parse(await readFile(sidecar, 'utf8')) as Manifest;
    } catch (e) {
      throw new Error(`${sidecar}: сайдкар не читается — ${message(e)}`);
    }
    const metadata = manifest.metadata;
    if (!metadata) throw new Error(`${sidecar}: в сайдкаре нет metadata`);
    const items =
      (metadata.expectedParagraphs ?? 0) +
      (metadata.expectedTables ?? 0) +
      (metadata.expectedImages ?? 0);
    corpus.push({ name, hasContent: items > 0, fatal: Boolean(metadata.fatal) });
  }
  return corpus;
}

/**
 * Поднять страницу и разложить сообщения браузера по источникам.
 *
 * `boot` из `helpers/boot.ts` складывает оба вида в один список — корпусу этого
 * мало: отказ открытия вьюер печатает в консоль (это разобранная ошибка, а не
 * падение), а `pageerror` — необработанное исключение, которого не должно быть
 * ни на одной фикстуре.
 */
async function bootSplit(page: Page): Promise<{ consoleErrors: string[]; pageErrors: string[] }> {
  const consoleErrors: string[] = [];
  const pageErrors: string[] = [];
  page.on('pageerror', (error) => pageErrors.push(String(error)));
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });

  await boot(page, '/docx');
  return { consoleErrors, pageErrors };
}

/** Итог открытия одной фикстуры. */
interface Outcome {
  /** Число страниц; у отвергнутого документа — 0. */
  pages: number;
  /** Текст отказа из строки состояния: `null` — документ открылся. */
  error: string | null;
  /** Кадр: `null` — документа нет, рисовать было нечего. */
  frame: FrameStats | null;
}

/**
 * Выбрать фикстуру и дождаться исхода.
 *
 * Ход и итог различает строка состояния: пока документ разбирается, в ней
 * «загружаю <имя>…», после — «<имя>: N стр. …» либо «не открылось: …».
 * Двоеточие сразу после имени отделяет итог от хода загрузки — по одному
 * вхождению имени исход не отличить.
 */
async function open(page: Page, name: string): Promise<Outcome> {
  const before = await frameId(page);
  await page.selectOption('#fixture', name);
  await page.waitForFunction(
    (fixture) => {
      const text = document.querySelector('#status')?.textContent ?? '';
      return text.startsWith('не открылось:') || text.startsWith(`${fixture}: `);
    },
    name,
    { timeout: 30_000 },
  );

  const status = (await page.locator('#status').textContent()) ?? '';
  if (status.startsWith('не открылось:')) {
    // Страницы читаем и у отказа: вьюер мог оставить раскладку прошлого
    // документа, и проверка «отвергнутый документ страниц не даёт» — про это.
    return {
      pages: await pageCount(page),
      error: status.replace('не открылось:', '').trim(),
      frame: null,
    };
  }
  // Разбор закончен, но кадр ещё не нарисован: painter получает раскладку
  // асинхронно. Ждём новый кадр, и только потом меряем.
  await waitForFrame(page, before);
  return { pages: await pageCount(page), error: null, frame: await frameStats(page) };
}

/** Что не так с открытой фикстурой: пустой список — всё в порядке. */
function complaints(fixture: Fixture, outcome: Outcome): string[] {
  if (outcome.error !== null) return [`не открылся: ${outcome.error}`];
  const frame = outcome.frame;
  if (frame === null) return ['кадр не нарисован'];

  const problems: string[] = [];
  if (outcome.pages < 1) problems.push('в документе нет ни одной страницы');
  if (frame.pageBox === null || frame.pageRatio < MIN_PAGE_RATIO) {
    problems.push(`лист не нарисован (занято ${frame.pageRatio.toFixed(3)} кадра)`);
  }
  if (DRAWINGS_NOT_PAINTED.has(fixture.name)) {
    if (frame.inkBox !== null) {
      problems.push('на листе появились чернила: рисунки, похоже, начали рисоваться — уберите фикстуру из DRAWINGS_NOT_PAINTED и проверьте отрисовку глазами');
    }
  } else if (fixture.hasContent && frame.inkBox === null) {
    // Пустой документ (`simple/empty.docx`, `basic/empty_body.docx`) пуст и по
    // сайдкару — ни абзацев, ни таблиц, ни рисунков, — и белый лист без чернил
    // это верная отрисовка; поэтому спрашиваем сайдкар, а не гадаем.
    problems.push('холст пуст: на листе нет ни текста, ни другого содержимого');
  }
  return problems;
}

test('открывает и рисует каждую фикстуру корпуса', async ({ page, browserName }) => {
  // Решение намеренное: корпус — только chromium (см. шапку файла). Три
  // движка на 94 документа дали бы марафон без новой информации: точечные
  // спеки уже проверяют отрисовку на firefox и webkit.
  test.skip(browserName !== 'chromium', 'корпус идёт только в chromium');
  // 94 открытия не влезают в общий бюджет: корпусу нужен свой.
  test.setTimeout(300_000);

  const { consoleErrors, pageErrors } = await bootSplit(page);
  const corpus = await readCorpus();
  // Пустой корпус прошёл бы все проверки вхолостую.
  expect(corpus.length, 'фикстуры корпуса должны найтись').toBeGreaterThan(90);

  const renderable = corpus.filter((fixture) => !fixture.fatal);
  const fatal = corpus.length - renderable.length;
  // Список исключений не должен пережить фикстуру: переименование или удаление
  // файла обязано падать здесь, а не превращаться в мёртвую строку.
  for (const name of DRAWINGS_NOT_PAINTED) {
    expect(
      renderable.some((fixture) => fixture.name === name),
      `${name}: фикстуры из DRAWINGS_NOT_PAINTED нет среди открываемых`,
    ).toBe(true);
  }
  const problems: string[] = [];
  const pageRatios: number[] = [];
  const inkRatios: number[] = [];
  const started = Date.now();
  let rendered = 0;
  let blank = 0;
  let stopped = false;

  for (const fixture of renderable) {
    if (problems.length >= MAX_PROBLEMS) {
      stopped = true;
      break;
    }
    await test.step(fixture.name, async () => {
      const seenConsole = consoleErrors.length;
      const seenPage = pageErrors.length;

      let outcome: Outcome;
      try {
        outcome = await open(page, fixture.name);
      } catch (e) {
        problems.push(`${fixture.name}: открытие не завершилось — ${message(e)}`);
        return;
      }

      const found = complaints(fixture, outcome);
      for (const text of consoleErrors.slice(seenConsole)) found.push(`ошибка в консоли: ${text}`);
      for (const text of pageErrors.slice(seenPage)) found.push(`необработанное исключение: ${text}`);

      if (found.length > 0) {
        problems.push(`${fixture.name}: ${found.join('; ')}`);
        return;
      }
      rendered += 1;
      const frame = outcome.frame;
      const blankSheet = DRAWINGS_NOT_PAINTED.has(fixture.name);
      if (blankSheet) blank += 1;
      // В диапазоны идут только листы, которым сайдкар обещал содержимое:
      // у пустых документов и у нерисованных рисунков нули по определению.
      if (frame !== null && fixture.hasContent && !blankSheet) {
        pageRatios.push(frame.pageRatio);
        inkRatios.push(frame.inkRatio);
      }
    });
  }

  const seconds = Math.round((Date.now() - started) / 1000);
  const range = (values: number[]): string =>
    values.length === 0 ? '—' : `${Math.min(...values).toFixed(4)}…${Math.max(...values).toFixed(4)}`;
  console.log(
    [
      `корпус: открылось ${rendered} из ${renderable.length} — с чернилами ${rendered - blank}, пустых листов ${blank} (по списку DRAWINGS_NOT_PAINTED); отвергнуто по metadata.fatal: ${fatal}`,
      `корпус: обход занял ${seconds} с, доля листа в кадре ${range(pageRatios)}, доля чернил у непустых ${range(inkRatios)}`,
      ...(stopped ? [`корпус: обход остановлен после ${problems.length} проблем`] : []),
    ].join('\n'),
  );

  expect(problems, `корпус: проблем ${problems.length}\n${problems.join('\n')}`).toEqual([]);
  expect(consoleErrors, 'ошибки в консоли за обход корпуса').toEqual([]);
  expect(pageErrors, 'необработанные исключения за обход корпуса').toEqual([]);
});

test('отвергает нечитаемые фикстуры внятной ошибкой и остаётся живым', async ({ page }) => {
  const { consoleErrors, pageErrors } = await bootSplit(page);
  const corpus = await readCorpus();
  const fatal = corpus.filter((fixture) => fixture.fatal);
  // Класс не должен опустеть: иначе тест прошёл бы, ничего не проверив.
  expect(fatal.length, 'фикстуры с metadata.fatal').toBeGreaterThan(0);

  const rejected: string[] = [];
  for (const fixture of fatal) {
    await test.step(fixture.name, async () => {
      const outcome = await open(page, fixture.name);
      expect(outcome.error, `${fixture.name}: вьюер обязан показать отказ`).not.toBeNull();
      expect(
        outcome.error?.trim().length ?? 0,
        `${fixture.name}: сообщение об отказе пустое`,
      ).toBeGreaterThan(0);
      expect(outcome.pages, `${fixture.name}: отвергнутый документ страниц не даёт`).toBe(0);
      // Отказ не гасит управление: сломалась одна загрузка, а не просмотрщик.
      expect(await page.locator('#fixture').isEnabled(), `${fixture.name}: управление живо`).toBe(
        true,
      );
      rejected.push(`${fixture.name} → ${outcome.error ?? ''}`);
    });
  }

  // Интерфейс живёт после всех отказов подряд: следующий документ открывается
  // и рисуется. Это и есть проверка «вьюер не упал» — список `pageerror` ниже
  // только дополняет её.
  const healthy = await open(page, 'simple/one_paragraph.docx');
  expect(healthy.error, 'после отказов вьюер обязан открыть нормальный документ').toBeNull();
  expect(
    healthy.frame?.inkBox ?? null,
    'нормальный документ после отказов должен отрисоваться',
  ).not.toBeNull();

  console.log(
    [
      `битые фикстуры: отвергнуто ${rejected.length} из ${fatal.length}`,
      `битые фикстуры: сообщений об отказе в консоли — ${consoleErrors.length} (отказ разобран, а не проглочен)`,
      ...rejected.map((line) => `  ${line}`),
    ].join('\n'),
  );

  expect(pageErrors, 'отвергнутая фикстура не должна ронять страницу').toEqual([]);
});
