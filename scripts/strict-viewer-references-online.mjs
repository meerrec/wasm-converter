#!/usr/bin/env node
// Захват эталонов strict-viewer из Word Online / Excel Online.
//
// Ручной путь по docs/strict-viewer/reference-capture.md автоматизирован
// настолько, насколько это разрешено политикой: браузер ведёт скрипт
// (captureMethod: "semi-automated"), вход в учётную запись Microsoft делает
// человек один раз в persistent-профиле Chromium. Скрипт живёт вне
// tools/visual-oracle намеренно: политика baseline (§4) запрещает Office URL,
// сеть и учётные данные в capture/diff runtime оракула.
//
// Команды:
//   login    — открыть office.com в профиле и дождаться входа пользователя.
//   probe    — снять URL/скриншот/элементы текущей страницы (диагностика DOM).
//   capture  — снять эталоны по corpus.json: --filter <glob>, --out <dir>,
//              --fixtures <dir>, --corpus <path>, --only <id>.
//
// Playwright и pngjs резолвятся из tools/visual-oracle (там они
// devDependency): корневой node_modules pnpm их не экспортирует.

import { createRequire } from 'node:module';
import { execFile } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const execFileAsync = promisify(execFile);

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PROFILE_DIR = path.join(REPO_ROOT, 'target', 'office-online-profile');
const PROBE_DIR = path.join(REPO_ROOT, 'target', 'office-online-probe');
const DEFAULT_OUT = path.join(REPO_ROOT, 'target', 'strict-viewer-online-out');
const DEFAULT_CORPUS = path.join(REPO_ROOT, 'test-fixtures', 'strict-viewer', 'corpus.json');
const DEFAULT_FIXTURES = path.join(REPO_ROOT, 'test-fixtures');

const requireFromOracle = createRequire(
  path.join(REPO_ROOT, 'tools', 'visual-oracle', 'package.json'),
);
const { chromium } = requireFromOracle('playwright');
const { PNG } = requireFromOracle('pngjs');

const HOME_URL = 'https://www.office.com/';

/** Стартовая страница Word Online: отсюда идёт загрузка файла. */
const WORD_HOME = 'https://word.cloud.microsoft/';

/**
 * Альфа-профиль кромок страницы демо-вьюера: страница рисуется с дробным
 * смещением по X, поэтому крайние колонки получают частичное покрытие
 * (замерено по actual-кадрам: слева 221, справа 220 при span 794 для A4).
 * Внутренние колонки совпадают 1:1 со скриншотом PageContent Word Online —
 * ресемплинг не нужен, только эти две колонки доводятся до профиля вьюера.
 */
const DOCX_EDGE_ALPHA = { left: 221, right: 220 };

// Геометрия actual-кадров демо-вьюера при viewport 1440×900, DPR 1, zoom 1.
// DOCX print-layout — модель «страница + поля» (b7444c9): кадр равен
// странице плюс 2×pad (pad = 450), страница вверху, следующая страница
// стека видна снизу, ниже последней — белое продолжение листа. Геометрия
// per fixture берётся из geometry.json (замер по actual-кадрам оракула).
// Сетка XLSX — непрозрачные 1440×682, содержимое A1 начинается в (44, 21).
// Эталон обязан совпасть с actual по размерам, иначе сравнение даст
// page-mismatch без пиксельных метрик (visual-v1, правило 1).
const XLSX_FRAME = { width: 1440, height: 682, cellA1X: 44, cellA1Y: 21 };

// Матрица §4: строки excluded/planned не имеют визуального профиля — их
// сертификация идёт по FeatureSupportReport, а не по эталону. WAC к тому же
// не открывает эти документы (крах рендерера на OLE-превью, цикл
// перезагрузок на endnote), поэтому эталоны для них не снимаются.
const CAPTURE_SKIP = new Set([
  'docx-embedded-ole-object',
  'docx-embedded-smartart-basic',
  'docx-notes-endnote-basic',
  // Строка xlsx.external-links — unsupported-visible; WAC к тому же не может
  // резолвить внешнюю книгу и навсегда застревает со скелетоном загрузки,
  // из-за чего контейнер сетки короче кадра вьюера.
  'xlsx-external-workbook-link',
  'xlsx-layout-broken-link',
]);

/** slug фикстуры из source.path — тот же алгоритм, что в оракуле. */
function fixtureSlug(sourcePath) {
  const stem = sourcePath.replace(/^.*\//, '').replace(/\.[^.]+$/, '');
  const slug = stem
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, '-')
    .replace(/^-+|-+$/g, '');
  return slug.length > 0 ? slug : 'document';
}

/** Минимальный glob (`*` — любая последовательность) для --filter по id. */
function globMatch(glob, value) {
  const re = new RegExp(
    `^${glob.split('*').map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.*')}$`,
  );
  return re.test(value);
}

/** Разобрать argv в карту флагов: `--name value` и `--flag`. */
function parseArgs(argv) {
  const args = { _: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (token.startsWith('--')) {
      const next = argv[i + 1];
      if (next !== undefined && !next.startsWith('--')) {
        args[token.slice(2)] = next;
        i += 1;
      } else {
        args[token.slice(2)] = true;
      }
    } else {
      args._.push(token);
    }
  }
  return args;
}

/** Corpus index: только фикстуры экранных режимов (их и снимает Office Online). */
async function loadScreenFixtures(corpusPath, filter) {
  const corpus = JSON.parse(await readFile(corpusPath, 'utf8'));
  const wanted = [];
  for (const fixture of corpus.fixtures) {
    for (const mode of fixture.modes) {
      const screen = (fixture.format === 'docx' && mode === 'print-layout') ||
        (fixture.format === 'xlsx' && mode === 'normal');
      if (!screen) continue;
      if (filter && !globMatch(filter, fixture.id)) continue;
      wanted.push({
        id: fixture.id,
        slug: fixtureSlug(fixture.source.path),
        format: fixture.format,
        mode,
        sourcePath: fixture.source.path,
        title: fixture.title ?? fixture.id,
      });
    }
  }
  return { corpus, wanted };
}

async function launch() {
  await mkdir(PROFILE_DIR, { recursive: true });
  const args = ['--disable-blink-features=AutomationControlled'];
  // OFFICE_NO_GPU=1 — софтовый рендеринг: диагностика воспроизводимости
  // канвас-текста (Excel Online) и обход крахов на OLE-превью.
  if (process.env.OFFICE_NO_GPU === '1') {
    args.push('--disable-gpu', '--disable-accelerated-2d-canvas');
  }
  const context = await chromium.launchPersistentContext(PROFILE_DIR, {
    headless: false,
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    locale: 'ru-RU',
    timezoneId: 'UTC',
    colorScheme: 'light',
    args,
  });
  return context;
}

async function cmdLogin() {
  const context = await launch();
  const page = context.pages()[0] ?? (await context.newPage());
  await page.goto('https://word.office.com/', { waitUntil: 'domcontentloaded' }).catch(() => {});
  process.stdout.write('Довожу сессию до Word Online; при необходимости войдите в окне…\n');
  const deadline = Date.now() + 15 * 60 * 1000;
  for (;;) {
    const url = page.url();
    // Готово: стартовая Word Online с созданием документа (word.office.com
    // редиректит на word.cloud.microsoft).
    if (/word\.(office|cloud\.microsoft)\./.test(url) || /word\.cloud\.microsoft/.test(url)) {
      const ready = await page
        .evaluate(() =>
          /Пустой документ|Blank document|Создать|New/.test(document.body?.innerText ?? ''),
        )
        .catch(() => false);
      if (ready) break;
    }
    // SSO-шаги, которые можно пройти без пользователя: кнопка «Войти» и
    // выбор уже аутентифицированного аккаунта. Пароль/2FA — за человеком.
    const clicked = await page
      .evaluate(() => {
        const visible = (el) => {
          const r = el.getBoundingClientRect();
          return r.width > 0 && r.height > 0;
        };
        const buttons = [...document.querySelectorAll('button, a, [role="button"], [role="link"]')];
        const signIn = buttons.find(
          (el) => visible(el) && /^(Войти|Sign in|Пријава|Prijava)$/i.test((el.textContent ?? '').trim()),
        );
        if (signIn) {
          signIn.click();
          return 'sign-in';
        }
        const tile = [...document.querySelectorAll('[role="button"], button, div, a')].find(
          (el) => visible(el) && /@/.test(el.textContent ?? '') && (el.textContent ?? '').length < 120,
        );
        if (tile) {
          tile.click();
          return 'account-tile';
        }
        return null;
      })
      .catch(() => null);
    if (clicked !== null) {
      process.stdout.write(`шаг SSO: ${clicked}\n`);
      await page.waitForTimeout(4000);
    } else {
      await page.waitForTimeout(3000);
    }
    // Видимость для отладки: куда попали и что на странице.
    const snippet = await page
      .evaluate(() => (document.body?.innerText ?? '').replace(/\s+/g, ' ').slice(0, 160))
      .catch(() => '<no page>');
    process.stdout.write(`[poll] ${page.url()} :: ${snippet}\n`);
    if (Date.now() > deadline) {
      throw new Error('Word Online session was not established within 15 minutes');
    }
  }
  process.stdout.write(`Вход выполнен. Профиль сохранён в ${PROFILE_DIR}\n`);
  await context.close();
}

async function cmdProbe(args) {
  const context = await launch();
  const page = context.pages()[0] ?? (await context.newPage());
  await page.goto(args.url ?? args._[0] ?? HOME_URL, { waitUntil: 'domcontentloaded' }).catch(() => {});
  await page.waitForTimeout(5000);
  await mkdir(PROBE_DIR, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  // Произвольный сценарий поверх открытой страницы: export default async
  // ({ page, context }) => ({...}) — результат попадёт в JSON дампа.
  let extra = null;
  if (typeof args.eval === 'string') {
    const { pathToFileURL } = await import('node:url');
    const fn = (await import(pathToFileURL(path.resolve(REPO_ROOT, args.eval)).href)).default;
    extra = await fn({ page, context });
  }
  await page.screenshot({ path: path.join(PROBE_DIR, `${stamp}.png`), fullPage: false });
  const info = {
    url: page.url(),
    title: await page.title(),
    text: (await page.evaluate(() => document.body?.innerText ?? '')).slice(0, 4000),
    extra,
  };
  await writeFile(path.join(PROBE_DIR, `${stamp}.json`), JSON.stringify(info, null, 2));
  process.stdout.write(`${path.join(PROBE_DIR, `${stamp}.json`)}\n`);
  await context.close();
}

const isMainModule =
  process.argv[1] !== undefined &&
  import.meta.url === (await import('node:url')).pathToFileURL(process.argv[1]).href;

if (isMainModule) {
  const command = process.argv[2];
  if (command === 'login') {
    await cmdLogin();
  } else if (command === 'probe') {
    await cmdProbe(parseArgs(process.argv.slice(3)));
  } else if (command === 'capture') {
    await cmdCapture(parseArgs(process.argv.slice(3)));
  } else {
    process.stderr.write(
      'usage: strict-viewer-references-online.mjs <login|probe|capture> [options]\n',
    );
    process.exitCode = 2;
  }
}

export { composeDocxStackFrame, cropToWhitePage };

// ---------------------------------------------------------------------------
// Сборка drop и импорт manifest
// ---------------------------------------------------------------------------

/** Среда захвата для capture.json: значения принятой матрицы (§2.2). */
async function captureEnvironment(mode) {
  const fonts = [];
  const fontsDir = path.join(REPO_ROOT, 'crates', 'render', 'src', 'fonts');
  const { createHash } = await import('node:crypto');
  const { readdir, readFile: read } = await import('node:fs/promises');
  const styles = {
    'carlito-subset.ttf': { style: 'normal', weight: 400 },
    'carlito-bold-subset.ttf': { style: 'bold', weight: 700 },
    'carlito-italic-subset.ttf': { style: 'italic', weight: 400 },
    'carlito-bolditalic-subset.ttf': { style: 'italic', weight: 700 },
  };
  for (const file of await readdir(fontsDir)) {
    const meta = styles[file];
    if (meta === undefined) continue;
    const bytes = await read(path.join(fontsDir, file));
    fonts.push({
      family: 'Carlito',
      style: meta.style,
      weight: meta.weight,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    });
  }
  const environment = {
    os: {
      name: 'macOS',
      version: await execFileAsync('sw_vers', ['-productVersion']).then((r) => r.stdout.trim()),
      architecture: process.arch === 'arm64' ? 'arm64' : process.arch,
    },
    // Браузер записывается для воспроизводимости среды (матрица §2.2):
    // кадр рисует Office Online, но в том же Chromium.
    browser: { name: 'chromium', version: browserVersion() },
    applicationZoomPercent: 100,
    browserZoomPercent: 100,
    locale: 'ru-RU',
    timezone: 'UTC',
    fonts: { package: 'carlito-subset', version: '1.104', assets: fonts },
  };
  // Экранным режимам схема требует viewport/DPR/colorScheme; rasterizer
  // запрещён (он только у docx.pdf).
  environment.viewportCss = { width: 1440, height: 900 };
  environment.deviceScaleFactor = 1;
  environment.colorScheme = 'light';
  return environment;
}

function browserVersion() {
  // Версия Chromium, которым снимается эталон, — из установленного Playwright.
  try {
    const pkg = requireFromOracle('playwright-core/package.json');
    return pkg.version;
  } catch {
    return 'unknown';
  }
}

/** Записать drop `<out>/drop/<format>/<id>/<mode>/` и вызвать импорт. */
async function writeDropAndImport({ outDir, corpus, fixture, captures, notes }) {
  const dropDir = path.join(outDir, 'drop', fixture.format, fixture.id, fixture.mode);
  await mkdir(path.join(dropDir, 'reference'), { recursive: true });
  for (const capture of captures) {
    await writeFile(path.join(dropDir, 'reference', `${capture.id}.png`), capture.png);
  }
  const environment = await captureEnvironment(fixture.mode);
  const view =
    fixture.format === 'docx'
      ? { format: 'docx', mode: fixture.mode, revisions: 'accepted', comments: 'hidden' }
      : {
          format: 'xlsx',
          mode: fixture.mode,
          activeSheet: { index: 0, name: captures[0].sheetName },
          hiddenSheets: 'exclude',
          showGridLines: true,
          showHeadings: true,
        };
  const captureJson = {
    corpus: { version: corpus.corpus.version, matrixVersion: corpus.corpus.matrixVersion },
    source: { path: fixture.sourcePath },
    reference: {
      provider: 'office-online',
      application: fixture.format === 'docx' ? 'word-online' : 'excel-online',
      capturedAt: new Date().toISOString().replace(/\.\d+Z$/, 'Z'),
      captureMethod: 'semi-automated',
      operator: process.env.USER ?? 'unknown',
    },
    environment,
    view,
    dynamicData: {
      dates: 'fixed-value',
      fields: 'cached-result',
      formulaCache: {
        policy: 'cached-values-only',
        dirty: false,
        fullCalcOnLoad: false,
        forceFullCalc: false,
      },
      externalLinks: 'cached-values-no-network',
    },
    captures: captures.map((capture) =>
      fixture.format === 'docx'
        ? {
            id: capture.id,
            kind: 'docx-page',
            pageNumber: capture.pageNumber,
            image: `reference/${capture.id}.png`,
          }
        : {
            id: capture.id,
            kind: 'xlsx-screen',
            sheet: { index: capture.sheetIndex, name: capture.sheetName },
            visibleRange: capture.visibleRange,
            scroll: { x: 0, y: 0 },
            activeCell: 'A1',
            image: `reference/${capture.id}.png`,
          },
    ),
    ...(notes === undefined ? {} : { notes }),
  };
  await writeFile(path.join(dropDir, 'capture.json'), `${JSON.stringify(captureJson, null, 2)}\n`);

  const { stdout } = await execFileAsync(
    'pnpm',
    [
      'visual-oracle',
      'reference',
      'import',
      '--from',
      dropDir,
      '--corpus',
      DEFAULT_CORPUS,
      '--references',
      path.join(outDir, 'drop'),
      // Drop — транзитный артефакт, он пересобирается каждым прогоном;
      // защиту от перезаписи имеет сам корпус эталонов, не drop.
      '--force',
    ],
    { cwd: REPO_ROOT },
  );
  return { dropDir, importOutput: stdout.trim() };
}

// ---------------------------------------------------------------------------
// Захват (заполняется после разведки DOM Office Online)
// ---------------------------------------------------------------------------

/**
 * Геометрия страницы DOCX из actual-кадра нашего вьюера: левый край и ширина
 * белой страницы. Эталон компонуется в те же координаты, иначе сравнение
 * увидит сдвиг всего кадра. Поля страниц различаются между документами
 * (A4/Letter, секции), поэтому замер — по каждой странице, а не константа.
 */
function measureDocxPageRect(png) {
  const img = PNG.sync.read(png);
  const at = (x, y) => {
    const i = (y * img.width + x) * 4;
    return [img.data[i], img.data[i + 1], img.data[i + 2], img.data[i + 3]];
  };
  const isPage = (x, y) => {
    const [r, g, b, a] = at(x, y);
    return a > 0 && r > 240 && g > 240 && b > 240;
  };
  const y = Math.min(100, img.height - 1);
  let left = -1;
  let right = -1;
  for (let x = 0; x < img.width; x += 1) {
    if (isPage(x, y)) {
      left = x;
      break;
    }
  }
  for (let x = img.width - 1; x >= 0; x -= 1) {
    if (isPage(x, y)) {
      right = x;
      break;
    }
  }
  if (left < 0 || right < 0) {
    throw new Error('не удалось найти белую страницу в actual-кадре');
  }
  return { left, width: right - left + 1, frameWidth: img.width, frameHeight: img.height };
}

/**
 * Начало содержимого A1 из actual-кадра XLSX: левая колонка — заголовки строк
 * (заливка 245), дальше белые ячейки; высота строки заголовков колонок ищется
 * по той же границе в первой колонке.
 */
function measureXlsxA1Origin(png) {
  const img = PNG.sync.read(png);
  const at = (x, y) => {
    const i = (y * img.width + x) * 4;
    return [img.data[i], img.data[i + 1], img.data[i + 2], img.data[i + 3]];
  };
  const isHeader = (x, y) => {
    const [r, g, b] = at(x, y);
    return r >= 235 && r <= 250 && Math.abs(r - g) < 6 && Math.abs(g - b) < 6;
  };
  // Сканируем строку внутри первой ячейки по вертикали: до границы — заголовок
  // строки (серый), после — белая ячейка. Берём y чуть ниже середины кадра,
  // где точно ячейки, а не закреплённые области.
  const y = Math.min(200, img.height - 1);
  let a1x = -1;
  for (let x = 0; x < img.width; x += 1) {
    if (!isHeader(x, y)) {
      a1x = x;
      break;
    }
  }
  // Высота заголовков колонок: по первой колонке поиска A1 нет — сканируем
  // колонку левее a1x (внутри заголовков строк): сверху угол и заголовок
  // колонок, граница — первая горизонтальная линия сетки под ними. Проще:
  // идём по x = a1x + 2 (внутри ячеек) сверху вниз: заголовок колонок серый,
  // первая ячейка белая.
  let a1y = -1;
  for (let yy = 0; yy < img.height; yy += 1) {
    if (!isHeader(a1x + 2, yy)) {
      a1y = yy;
      break;
    }
  }
  if (a1x < 0 || a1y < 0) {
    throw new Error('не удалось найти начало A1 в actual-кадре');
  }
  return { a1x, a1y, frameWidth: img.width, frameHeight: img.height };
}

/** Перенести drop в корпус эталонов (test-fixtures/strict-viewer). */
async function transferToCorpus(outDir, fixture) {
  const { cp } = await import('node:fs/promises');
  const from = path.join(outDir, 'drop', fixture.format, fixture.slug, fixture.mode);
  const to = path.join(
    REPO_ROOT,
    'test-fixtures',
    'strict-viewer',
    fixture.format,
    fixture.slug,
    fixture.mode,
  );
  await mkdir(to, { recursive: true });
  await cp(path.join(from, 'reference'), path.join(to, 'reference'), { recursive: true });
  await cp(path.join(from, 'manifest.json'), path.join(to, 'manifest.json'));
  return to;
}

// ---------------------------------------------------------------------------
// Захват Word Online (print-layout)
// ---------------------------------------------------------------------------

/** Убрать осиротевшие браузеры профиля: persistent-контекст на macOS не всегда гасит процесс. */
async function killStaleProfileBrowsers() {
  try {
    await execFileAsync('pkill', ['-f', 'office-online-profile']);
  } catch {
    // pkill завершается с кодом 1, когда процессов нет, — это норма.
  }
  await new Promise((resolve) => setTimeout(resolve, 1500));
}

/** Дождаться фрейма редактора Word Online (wordeditorframe). */
async function waitForWordEditorFrame(page, timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frame = page.frames().find((candidate) => candidate.url().includes('wordeditorframe'));
    if (frame !== undefined) return frame;
    await page.waitForTimeout(1000);
  }
  throw new Error('не появился фрейм редактора Word Online');
}

/** Текущий зум Word Online по ярлыку в статус-баре (строка вида «100%»). */
async function readWordZoom(frame) {
  // Статус-бар догружается после фрейма и при навигации может пересоздаться;
  // опрашиваем с повторами, иначе медленные документы (OLE, embedded)
  // ловят «ярлык не найден» или уничтоженный контекст исполнения.
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const zoom = await frame
      .evaluate(() => {
        const leaf = [...document.querySelectorAll('*')].find((el) => {
          if (el.children.length !== 0) return false;
          return /^\d+\s*%$/.test((el.textContent ?? '').trim());
        });
        return leaf === undefined ? null : leaf.textContent.trim().replace(/\s+/g, '');
      })
      .catch(() => null);
    if (zoom !== null) return zoom;
    await frame.page().waitForTimeout(2000).catch(() => {});
  }
  return null;
}

/** Зум обязан быть 100%: иначе масштаб текста не совпадёт с actual. */
async function ensureWordZoom100(frame) {
  // Статус-бар отрисовывается позже страниц — опрашиваем до минуты.
  const deadline = Date.now() + 60000;
  let zoom = null;
  while (Date.now() < deadline) {
    zoom = await readWordZoom(frame);
    if (zoom !== null) break;
    await frame.page().waitForTimeout(2000);
  }
  if (zoom === null) throw new Error('не найден ярлык зума в статус-баре Word Online');
  if (zoom === '100%') return;
  // Клик по ярлыку открывает меню; выбрать в нём 100%.
  await frame.getByText(zoom, { exact: true }).first().click().catch(() => {});
  await frame.getByText('100%', { exact: true }).first().click().catch(() => {});
  await frame.page().waitForTimeout(2000);
  const after = await readWordZoom(frame);
  if (after !== '100%') throw new Error(`зум Word Online: было ${zoom}, стало ${after}, нужно 100%`);
}

/** Загрузить docx через «Отправить файл» на стартовой Word Online. */
async function uploadDocxToWord(page, absoluteSourcePath) {
  await page.goto(WORD_HOME, { waitUntil: 'domcontentloaded' }).catch(() => {});
  const upload = page.getByRole('button', { name: /Отправить файл|Upload/i }).first();
  await upload.waitFor({ state: 'visible', timeout: 60000 });
  const [chooser] = await Promise.all([
    page.waitForEvent('filechooser', { timeout: 60000 }),
    upload.click(),
  ]);
  await chooser.setFiles(absoluteSourcePath);
}

/**
 * Клип скриншота OOPIF-элемента округляется в координатах внешнего фрейма и
 * от сессии к сессии может захватить разное число строк выше страницы
 * (тень/градиент кромки WAC — полоса ~240,240,240). Эталону это не нужно:
 * обрезаем всё до первой строки чистой белой страницы. Порог ≥252 отделяет
 * белое поле страницы от градиента кромки; текст тёмен и на детекцию не
 * влияет, потому что ищется строка с ≥90% белых пикселей.
 */
function cropToWhitePage(pngBuffer) {
  const img = PNG.sync.read(pngBuffer);
  const isPageWhite = (x, y) => {
    const i = (y * img.width + x) * 4;
    return (
      img.data[i + 3] > 0 &&
      img.data[i] >= 252 &&
      img.data[i + 1] >= 252 &&
      img.data[i + 2] >= 252
    );
  };
  let top = -1;
  const margin = 8; // края клипа не считаем: там может быть рамка/тень
  const span = img.width - 2 * margin;
  for (let y = 0; y < img.height; y += 1) {
    let white = 0;
    for (let x = margin; x < img.width - margin; x += 1) {
      if (isPageWhite(x, y)) white += 1;
    }
    if (white / span >= 0.9) {
      top = y;
      break;
    }
  }
  if (top < 0) {
    throw new Error('не найдена белая страница в скриншоте Word Online');
  }
  const out = new PNG({ width: img.width, height: img.height - top });
  img.data.copy(out.data, 0, top * img.width * 4);
  return PNG.sync.write(out);
}

/**
 * Скриншот содержимого страницы Word Online; повторяется до пиксельной
 * стабильности (аналог stableHashes оракула: два одинаковых кадра подряд).
 * Свежезагруженная копия документа может догружать раскладку десятки
 * секунд — поэтому окно до 60 с. Нестабильная страница — ошибка захвата,
 * а не повод отдавать промежуточный кадр в эталон. Стабильность считается
 * по обрезанному кадру — по тому, что реально попадёт в эталон.
 */
async function stableContentShot(frame, index, { maxTries = 20, gapMs = 3000 } = {}) {
  const locator = frame.locator('div.Page .PageContent').nth(index);
  let previous = null;
  for (let attempt = 0; attempt < maxTries; attempt += 1) {
    const shot = await locator.screenshot({ timeout: 60000, animations: 'disabled' });
    const cropped = cropToWhitePage(shot);
    if (previous !== null && previous.equals(cropped)) return { cropped, raw: shot };
    previous = cropped;
    await frame.page().waitForTimeout(gapMs);
  }
  throw new Error(`страница ${index + 1} не стабилизировалась за ${maxTries} попыток`);
}

/**
 * Компоновка эталона страницы DOCX: стек страниц Word Online в кадр
 * actual-кадра демо-вьюера. Модель кадра из b7444c9: «страница + поля» —
 * кадр равен странице плюс 2×pad, страница стоит вверху (y=0), следующая
 * страница стека видна ниже, ниже последней страницы вьюер рисует белое
 * продолжение листа шириной в страницу.
 *
 * Word Online округляет размер страницы вниз, вьюер — вверх (A4: 793×1122
 * против 794×1123), поэтому снимок кладётся со сдвигом +1 по X; нижний
 * полуряд страницы (дробная высота листа, alpha 128 у вьюера) и верхний
 * полуряд следующей страницы синтезируются. Кромочные колонки доводятся до
 * альфа-профиля вьюера (DOCX_EDGE_ALPHA): это границы кадра, а не
 * содержимое документа.
 */
/**
 * Масштабировать PNG по ширине (билинейно). Нужно редко: страницы
 * нестандартных размеров (sections) WAC и вьюер округляют по-разному —
 * например 794 px против 795 px. Масштабирование — ближайшее честное
 * представление; расхождение раскладки остаётся дефектом движка и его
 * покажет сертификация.
 */
function scaleToWidth(pngBuffer, targetWidth) {
  const img = PNG.sync.read(pngBuffer);
  const scale = targetWidth / img.width;
  const outW = targetWidth;
  const outH = Math.max(1, Math.round(img.height * scale));
  const out = new PNG({ width: outW, height: outH });
  for (let y = 0; y < outH; y += 1) {
    const sy = y / scale;
    const y0 = Math.floor(sy);
    const y1 = Math.min(y0 + 1, img.height - 1);
    const fy = sy - y0;
    for (let x = 0; x < outW; x += 1) {
      const sx = x / scale;
      const x0 = Math.floor(sx);
      const x1 = Math.min(x0 + 1, img.width - 1);
      const fx = sx - x0;
      const ti = (y * outW + x) * 4;
      const i00 = (y0 * img.width + x0) * 4;
      const i10 = (y0 * img.width + x1) * 4;
      const i01 = (y1 * img.width + x0) * 4;
      const i11 = (y1 * img.width + x1) * 4;
      for (let c = 0; c < 4; c += 1) {
        const p00 = img.data[i00 + c];
        const p10 = img.data[i10 + c];
        const p01 = img.data[i01 + c];
        const p11 = img.data[i11 + c];
        out.data[ti + c] = Math.round(
          p00 * (1 - fx) * (1 - fy) + p10 * fx * (1 - fy) + p01 * (1 - fx) * fy + p11 * fx * fy,
        );
      }
    }
  }
  return PNG.sync.write(out);
}

function composeDocxStackFrame(shots, geo, index) {
  const page = geo.pages[`page-${index + 1}`];
  // Кадр — измеренный размер actual-PNG (у sections страницы разного размера,
  // у дефектов пагинации вьюера кадр не равен странице + 2×pad).
  // Высота последней страницы не замеряется (низ невидим) — тогда она
  // берётся из скриншота Word Online (+1 на нижний полуряд).
  const shotPng = PNG.sync.read(shots[index]);
  const pageRows = page.height ?? shotPng.height + 1;
  const frame = page.frame ?? {
    width: page.width + 2 * page.left,
    height: pageRows + 2 * page.left,
  };
  const out = new PNG({ width: frame.width, height: frame.height });
  const left = page.left;
  const right = page.left + page.width - 1;

  const paste = (shot, destX, destY) => {
    const rows = Math.min(shot.height, frame.height - destY);
    const cols = Math.min(shot.width, frame.width - destX);
    if (rows <= 0 || cols <= 0) return;
    for (let y = 0; y < rows; y += 1) {
      shot.data.copy(
        out.data,
        ((destY + y) * frame.width + destX) * 4,
        y * shot.width * 4,
        (y * shot.width + cols) * 4,
      );
    }
  };
  const fillWhite = (x0, x1, y0, y1, alpha) => {
    for (let y = y0; y < y1; y += 1) {
      const start = (y * frame.width + x0) * 4;
      for (let x = x0; x <= x1; x += 1) {
        const i = start + (x - x0) * 4;
        out.data[i] = 255;
        out.data[i + 1] = 255;
        out.data[i + 2] = 255;
        out.data[i + 3] = alpha;
      }
    }
  };

  const last = index + 1 >= shots.length || page.nextTop === null;
  // Скриншот Word Online включает собственные AA-полуколонки клипа (794 px
  // у A4): он ложится ровно в левый край страницы вьюера, а кромочные
  // колонки (450/1243) перезаписываются профилем вьюера ниже. Внутренние
  // колонки при этом совпадают 1:1.
  let shot = PNG.sync.read(shots[index]);
  const wantWidth = page.width;
  if (shot.width !== wantWidth) {
    process.stderr.write(
      `[warn] ширина PageContent Word Online ${shot.width} px, кадр вьюера ждёт ${wantWidth} px — масштабирую\n`,
    );
    shot = PNG.sync.read(scaleToWidth(shots[index], wantWidth));
  }
  paste(shot, left, 0);

  let nextRows = 0;
  if (!last && page.nextTop < frame.height) {
    // Следующая страница стека видна в кадре снизу.
    let nextShot = PNG.sync.read(shots[index + 1]);
    if (nextShot.width !== wantWidth) {
      nextShot = PNG.sync.read(scaleToWidth(shots[index + 1], wantWidth));
    }
    paste(nextShot, left, page.nextTop);
    nextRows = Math.min(nextShot.height, frame.height - page.nextTop);
    // Верхний полуряд следующей страницы: дробная позиция в стеке.
    fillWhite(left + 1, right - 1, page.nextTop, page.nextTop + 1, 128);
    // Нижний полуряд самой страницы: дробная высота листа у вьюера.
    fillWhite(left + 1, right - 1, pageRows - 1, pageRows, 128);
  } else {
    // Белое продолжение листа ниже последней страницы (Word Online рисует
    // пустую страницу, вьюер повторяет её белым).
    fillWhite(left, right, pageRows - 1, frame.height, 255);
  }

  // Кромочные колонки — профиль вьюера: на высоту страницы, на видимую часть
  // следующей страницы стека и на продолжение листа. Зазор между страницами
  // остаётся прозрачным.
  const setEdgeColumns = (y0, y1) => {
    for (let y = y0; y < y1; y += 1) {
      const li = (y * frame.width + left) * 4;
      out.data[li] = 255;
      out.data[li + 1] = 255;
      out.data[li + 2] = 255;
      out.data[li + 3] = DOCX_EDGE_ALPHA.left;
      const ri = (y * frame.width + right) * 4;
      out.data[ri] = 255;
      out.data[ri + 1] = 255;
      out.data[ri + 2] = 255;
      out.data[ri + 3] = DOCX_EDGE_ALPHA.right;
    }
  };
  setEdgeColumns(0, pageRows);
  if (last) {
    setEdgeColumns(pageRows, frame.height);
  } else {
    setEdgeColumns(page.nextTop, page.nextTop + nextRows);
  }
  return PNG.sync.write(out);
}

/** Снять print-layout эталон одного docx: загрузка, страницы, компоновка. */
async function captureDocxPrintLayout(page, fixture, pageGeo, fixturesDir) {
  await uploadDocxToWord(page, path.join(fixturesDir, fixture.sourcePath));
  const frame = await waitForWordEditorFrame(page);
  await ensureWordZoom100(frame);
  // Фокус уходит с поверхности редактирования: blinking caret не входит
  // в изображение эталона (матрица §2.2: курсоры не входят; §3: состояние
  // захвата отключаем, а не маскируем).
  await frame.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  }).catch(() => {});
  await page.waitForTimeout(1000);

  const expected = Object.keys(pageGeo.pages).length;
  // Страницы материализуются постепенно: ждём полного состава по
  // наблюдаемому состоянию, а не по первому кадру DOM
  // (reference-capture §2 п.2).
  let pages = [];
  const deadline = Date.now() + 120000;
  while (Date.now() < deadline) {
    try {
      pages = await frame.evaluate(() =>
        [...document.querySelectorAll('div.Page')].map((el) => {
          const content = el.querySelector('.PageContent');
          const rect = content === null ? null : content.getBoundingClientRect();
          return rect === null ? null : {
            x: rect.x,
            y: rect.y,
            width: Math.round(rect.width),
            height: Math.round(rect.height),
          };
        }),
      );
    } catch {
      // Навигация (переход на /open/onedrive) может уничтожить контекст
      // исполнения; ждём и пробуем снова в пределах дедлайна.
      pages = [];
      await page.waitForTimeout(3000);
      continue;
    }
    if (pages.length === expected && pages.every((entry) => entry !== null)) break;
    await page.waitForTimeout(2000);
  }
  if (pages.length !== expected || pages.some((entry) => entry === null)) {
    throw new Error(`страниц в Word Online: ${pages.length}, в actual: ${expected}`);
  }

  // Сырые скриншоты и метаданные — для диагностики воспроизводимости между
  // сессиями (reference-capture §2 п.4). В drop не входят.
  const rawDir = path.join(REPO_ROOT, 'target', 'office-online-probe', 'raw');
  await mkdir(rawDir, { recursive: true });
  // Сначала все страницы: кадр каждой страницы включает верх следующей
  // (модель «страница + поля»), поэтому компоновка идёт по всему стеку.
  const shots = [];
  for (let index = 0; index < pages.length; index += 1) {
    const { cropped, raw } = await stableContentShot(frame, index);
    shots.push(cropped);
    const stamp = Date.now();
    await writeFile(path.join(rawDir, `${fixture.slug}-p${index + 1}-${stamp}.png`), raw);
    const meta = await frame.evaluate((pageIndex) => {
      const content = document.querySelectorAll('div.Page .PageContent')[pageIndex];
      const rect = content.getBoundingClientRect();
      const iframeRect = window.frameElement?.getBoundingClientRect?.() ?? null;
      return {
        dpr: window.devicePixelRatio,
        content: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        iframe: iframeRect === null
          ? null
          : { x: iframeRect.x, y: iframeRect.y, width: iframeRect.width, height: iframeRect.height },
      };
    }, index);
    meta.mainBanner = await page
      .evaluate(() => /Покупка|Purchase/i.test(document.body?.innerText ?? ''))
      .catch(() => false);
    await writeFile(
      path.join(rawDir, `${fixture.slug}-p${index + 1}-${stamp}.json`),
      `${JSON.stringify(meta, null, 2)}\n`,
    );
  }

  const captures = [];
  for (let index = 0; index < pages.length; index += 1) {
    const composed = composeDocxStackFrame(shots, pageGeo, index);
    captures.push({ id: `page-${index + 1}`, pageNumber: index + 1, png: composed });
    process.stdout.write(
      `[page ${index + 1}] ${pages[index].width}×${pages[index].height} → кадр ${pageGeo.frame.width}×${pageGeo.frame.height}\n`,
    );
  }
  return captures;
}

// ---------------------------------------------------------------------------
// Захват Excel Online (normal)
// ---------------------------------------------------------------------------

/** Загрузить книгу через «Отправить файл» на стартовой Excel Online. */
async function uploadXlsxToExcel(page, absoluteSourcePath) {
  await page.goto('https://excel.cloud.microsoft/', { waitUntil: 'domcontentloaded' }).catch(() => {});
  const upload = page
    .getByRole('button', { name: /Отправить файл|Загрузить и открыть|Upload/i })
    .first();
  await upload.waitFor({ state: 'visible', timeout: 60000 });
  const [chooser] = await Promise.all([
    page.waitForEvent('filechooser', { timeout: 60000 }),
    upload.click(),
  ]);
  await chooser.setFiles(absoluteSourcePath);
}

/** Дождаться фрейма редактора Excel Online (xlviewer). */
async function waitForExcelEditorFrame(page, timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frame = page.frames().find((candidate) => candidate.url().includes('xlviewer'));
    if (frame !== undefined) return frame;
    await page.waitForTimeout(1000);
  }
  throw new Error('не появился фрейм редактора Excel Online');
}

/**
 * Дождаться отрисованной сетки: канвас листа большого размера и зум в
 * статус-баре. Зум в Excel Online — input со значением вида «100 %»,
 * поэтому текст страницы для поиска непригоден (там течёт CSS/JS сплеша).
 */
async function waitForExcelGrid(frame, timeoutMs = 240000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const state = await frame
      .evaluate(() => {
        const sheet = [...document.querySelectorAll('canvas.ewr-sheettable')].find((c) => {
          const r = c.getBoundingClientRect();
          return r.width > 1000 && r.height > 300;
        });
        const zoomInput = [...document.querySelectorAll('input')].find((el) =>
          /^\d+\s*%$/.test(el.value.trim()),
        );
        const grid = document.querySelector('#m_excelWebRenderer_ewaCtl_gridDiv');
        return {
          sheet: sheet
            ? { w: Math.round(sheet.getBoundingClientRect().width), h: Math.round(sheet.getBoundingClientRect().height) }
            : null,
          zoom: zoomInput === undefined ? null : zoomInput.value.trim(),
          // Контейнер сетки должен вмещать кадр 1440×682: при скелетоне
          // загрузки (36 px над шапкой) он короче и нижние ряды кадра
          // теряются. Окно среза заголовков компоновка двигает сама,
          // поэтому достаточно высоты, а не конкретного gridTop.
          gridH: grid === null ? null : Math.round(grid.getBoundingClientRect().height),
        };
      })
      .catch(() => ({ sheet: null, zoom: null, gridH: null }));
    if (state.sheet !== null && state.zoom !== null && state.gridH !== null && state.gridH >= XLSX_FRAME.height) {
      return state;
    }
    await frame.page().waitForTimeout(3000).catch(() => {});
  }
  throw new Error('сетка Excel Online не отрисовалась');
}

/** Зум обязан быть 100% — иначе размеры ячеек не совпадут с actual. */
async function ensureExcelZoom100(frame, state) {
  const zoom = state.zoom.replace(/\s+/g, '');
  if (zoom === '100%') return;
  // Ввести 100 в поле зума и подтвердить Enter.
  await frame.evaluate(() => {
    const el = [...document.querySelectorAll('input')].find((i) => /^\d+\s*%$/.test(i.value.trim()));
    if (el === undefined) return;
    el.focus();
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    setter.call(el, '100');
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    el.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', bubbles: true }));
  });
  await frame.page().waitForTimeout(3000);
  const after = await frame.evaluate(() => {
    const el = [...document.querySelectorAll('input')].find((i) => /^\d+\s*%$/.test(i.value.trim()));
    return el === undefined ? null : el.value.trim();
  });
  if (after === null || after.replace(/\s+/g, '') !== '100%') {
    throw new Error(`зум Excel Online: было ${zoom}, стало ${after}, нужно 100%`);
  }
}

/**
 * Геометрия заголовков: угол A1 в координатах контейнера сетки равен
 * ширине канваса заголовков строк и высоте канваса заголовков столбцов.
 * Ширина заголовков строк зависит от числа строк листа (2–3 разряда),
 * поэтому измеряется каждый раз, а не берётся константой.
 */
async function measureExcelGrid(frame) {
  return frame.evaluate(() => {
    const grid = document.querySelector('#m_excelWebRenderer_ewaCtl_gridDiv');
    if (grid === null) return { a1x: null, a1y: null, gridTop: null };
    const g = grid.getBoundingClientRect();
    const canvases = [...document.querySelectorAll('canvas.ewr-sheettable')].map((c) => {
      const r = c.getBoundingClientRect();
      return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
    });
    const rowHeader = canvases.find((c) => c.x === Math.round(g.x) && c.h > 300);
    const colHeader = canvases.find((c) => c.y === Math.round(g.y) && c.w > 1000);
    return {
      a1x: rowHeader === undefined ? null : rowHeader.w,
      a1y: colHeader === undefined ? null : colHeader.h,
      gridTop: Math.round(g.y),
      gridH: Math.round(g.height),
    };
  });
}

/**
 * Стабильный кадр контейнера сетки: два одинаковых скриншота подряд.
 * Сетка — канвасы, DOM-ячеек нет, поэтому снимается весь контейнер.
 */
async function stableExcelGridShot(frame, { maxTries = 20, gapMs = 3000 } = {}) {
  const locator = frame.locator('#m_excelWebRenderer_ewaCtl_gridDiv');
  let previous = null;
  for (let attempt = 0; attempt < maxTries; attempt += 1) {
    const shot = await locator.screenshot({ timeout: 60000, animations: 'disabled' });
    if (previous !== null && previous.equals(shot)) return shot;
    previous = shot;
    await frame.page().waitForTimeout(gapMs);
  }
  throw new Error('сетка Excel Online не стабилизировалась');
}

/**
 * Компоновка эталона XLSX normal: видимый диапазон сетки Excel Online в
 * кадр 1440×682. Угол A1 кладётся в позицию вьюера из geometry.json
 * (a1x−1, a1y−1); заголовки Excel Online (43×30 для типового листа) при
 * этом попадают в полосы вьюера (44×20) с расхождением — это дефект
 * геометрии движка, который сертификация обязана показать. Пиксели, для
 * которых в кадре Excel Online нет источника (заголовки уже, чем у
 * вьюера), заполняются белым.
 */
function composeXlsxReference(gridPng, pageGeo, a1InGrid, oy) {
  const grid = PNG.sync.read(gridPng);
  const targetX = pageGeo.a1.a1x - 1;
  const targetY = pageGeo.a1.a1y - 1;
  const ox = a1InGrid.x - targetX;
  if (oy < 0 || oy + XLSX_FRAME.height > grid.height) {
    throw new Error(`окно среза oy=${oy} не помещается в сетку ${grid.width}×${grid.height}`);
  }
  const frame = new PNG({ width: XLSX_FRAME.width, height: XLSX_FRAME.height });
  for (let y = 0; y < XLSX_FRAME.height; y += 1) {
    for (let x = 0; x < XLSX_FRAME.width; x += 1) {
      const sx = x + ox;
      const sy = y + oy;
      const ti = (y * XLSX_FRAME.width + x) * 4;
      if (sx >= 0 && sx < grid.width && sy >= 0 && sy < grid.height) {
        const si = (sy * grid.width + sx) * 4;
        frame.data[ti] = grid.data[si];
        frame.data[ti + 1] = grid.data[si + 1];
        frame.data[ti + 2] = grid.data[si + 2];
        frame.data[ti + 3] = grid.data[si + 3];
      } else {
        frame.data[ti] = 255;
        frame.data[ti + 1] = 255;
        frame.data[ti + 2] = 255;
        frame.data[ti + 3] = 255;
      }
    }
  }
  return PNG.sync.write(frame);
}

/**
 * Адрес ячейки под правым нижним углом кадра по линиям сетки в скриншоте.
 * Фолбэк на случай, когда клик не долетает до ячейки (плавающие диаграммы,
 * срезы). Считаются прогоны серых линий (цвет ~175..240, равные каналы)
 * правее A1 по горизонтали и ниже A1 по вертикали; прогон = одна граница.
 */
function addressFromGridlines(gridPng, a1InGrid, pageGeo, oy) {
  const img = PNG.sync.read(gridPng);
  const ox = a1InGrid.x - (pageGeo.a1.a1x - 1);
  const sx = XLSX_FRAME.width - 1 + ox;
  const sy = XLSX_FRAME.height - 1 + oy;
  const isLine = (x, y) => {
    if (x < 0 || y < 0 || x >= img.width || y >= img.height) return false;
    const i = (y * img.width + x) * 4;
    const [r, g, b] = [img.data[i], img.data[i + 1], img.data[i + 2]];
    return Math.abs(r - g) < 8 && Math.abs(g - b) < 8 && r >= 175 && r <= 240;
  };
  const countRuns = (start, end, at) => {
    let runs = 0;
    let prev = false;
    for (let v = start; v < end; v += 1) {
      const cur = at(v);
      if (cur && !prev) runs += 1;
      prev = cur;
    }
    return runs;
  };
  const rowY = Math.min(Math.max(sy - 6, a1InGrid.y + 4), img.height - 1);
  const cols = countRuns(a1InGrid.x + 2, sx, (x) => isLine(x, rowY));
  const colX = Math.min(Math.max(sx - 6, a1InGrid.x + 4), img.width - 1);
  const rows = countRuns(a1InGrid.y + 2, sy, (y) => isLine(colX, y));
  const letter = (n) => {
    let s = '';
    let v = n + 1;
    while (v > 0) {
      const r = (v - 1) % 26;
      s = String.fromCharCode(65 + r) + s;
      v = Math.floor((v - 1) / 26);
    }
    return s;
  };
  return `${letter(cols)}${rows + 1}`;
}

/** Снять xlsx.normal эталон: загрузка, сетка, компоновка, видимый диапазон. */
async function captureXlsxNormal(page, fixture, pageGeo, fixturesDir) {
  await uploadXlsxToExcel(page, path.join(fixturesDir, fixture.sourcePath));
  const frame = await waitForExcelEditorFrame(page);
  const gridState = await waitForExcelGrid(frame);
  await ensureExcelZoom100(frame, gridState);
  // Скелетон загрузки укорачивает контейнер сетки и делает эталон
  // зависимым от тайминга сессии: он может сойтись сам за 10 секунд, а
  // может висеть минутами. Единообразное состояние захвата: короткая
  // пауза и, если заглушка всё ещё есть, скрываем её — это UI, а не
  // содержимое документа (матрица §2.2: состояние захвата отключаем, а
  // не маскируем). Стабильность кадра дальше обеспечивает цикл съёмки.
  await frame.page().waitForTimeout(15000).catch(() => {});
  const skeleton = await frame
    .evaluate(() => {
      const el = document.querySelector('.pageLoadSkeleton');
      return el === null ? false : el.getBoundingClientRect().height > 0;
    })
    .catch(() => false);
  if (skeleton) {
    process.stderr.write('[info] прячу заглушку загрузки\n');
    await frame
      .evaluate(() => {
        const el = document.querySelector('.pageLoadSkeleton');
        if (el !== null) el.style.display = 'none';
      })
      .catch(() => {});
    await frame.page().waitForTimeout(5000).catch(() => {});
  }
  // Фокус не снимаем: выделение A1 — принятое состояние (вьюер тоже рисует
  // рамку активной ячейки), и оно должно попасть в эталон.
  const { a1x, a1y, gridTop, gridH } = await measureExcelGrid(frame);
  if (a1x === null || a1y === null || gridTop === null || gridH === null) {
    throw new Error('не найдены заголовки сетки Excel Online');
  }
  // Окно среза заголовков столбцов: обычно 10 px (низ 30-пиксельной
  // полосы), но при укороченном контейнере (скелетон загрузки) сдвигаем
  // вверх, чтобы кадр 1440×682 поместился целиком.
  const oy = Math.min(a1y - (pageGeo.a1.a1y - 1), gridH - XLSX_FRAME.height);
  if (oy < 0) {
    throw new Error(`контейнер сетки ${gridH} px не вмещает кадр ${XLSX_FRAME.height}`);
  }
  const shot = await stableExcelGridShot(frame);

  // Видимый диапазон: клик в правый нижний угол кадра 1440×682 → имя
  // ячейки в name box. Кадр уже снят, перемещение выделения не мешает.
  const frameBox = await page.locator('iframe[name="WacFrame_Excel_0"]').boundingBox();
  if (frameBox === null) throw new Error('не найден iframe Excel Online');
  const brFrameX = XLSX_FRAME.width - 1;
  const brFrameY = XLSX_FRAME.height - 1;
  const clickX = frameBox.x + (a1x - (pageGeo.a1.a1x - 1)) + brFrameX;
  const clickY = frameBox.y + gridTop + oy + brFrameY;
  await page.mouse.click(clickX, clickY).catch(() => {});
  await page.waitForTimeout(800);
  let addr = await frame.evaluate(() => {
    const el = [...document.querySelectorAll('input')].find((i) => /^[A-Z]{1,3}\$?\d+$/.test(i.value));
    return el === undefined ? null : el.value;
  });
  if (addr === null || addr === 'A1') {
    // Клик не долетел до ячейки (плавающий объект, статус-бар) — считаем
    // адрес по линиям сетки в снятом кадре.
    addr = addressFromGridlines(shot, { x: a1x, y: a1y }, pageGeo, oy);
    process.stderr.write(`[info] клик не долетел — адрес по сетке: ${addr}\n`);
  }
  const sheetName = await frame.evaluate(() => {
    // Вкладки листов — внизу фрейма (y > 700); у ленты свои табы с
    // aria-selected, и в DOM-порядке они идут первыми.
    const tabs = [...document.querySelectorAll('[role="tab"]')].filter(
      (el) => el.getBoundingClientRect().y > 700,
    );
    const selected = tabs.find((el) => el.getAttribute('aria-selected') === 'true');
    const fromTabs = (selected ?? tabs[tabs.length - 1])?.textContent?.trim();
    if (fromTabs) return fromTabs;
    // Фолбэк: a11y-текст фрейма содержит «Текущий лист: "Имя"».
    const status = (document.body?.innerText ?? '').replace(/\s+/g, ' ');
    const m = status.match(/Текущий лист:\s*[«"]([^»"]+)[»"]/);
    return m === null ? null : m[1];
  });
  if (sheetName === null || sheetName.length === 0) throw new Error('не найдено имя активного листа');

  const composed = composeXlsxReference(shot, pageGeo, { x: a1x, y: a1y }, oy);
  process.stdout.write(
    `[sheet] A1@(${a1x},${a1y}) → кадр ${XLSX_FRAME.width}×${XLSX_FRAME.height} @A1=(${pageGeo.a1.a1x - 1},${pageGeo.a1.a1y - 1}), диапазон A1:${addr}, лист «${sheetName}»\n`,
  );
  return [{ id: 'sheet-0', sheetIndex: 0, sheetName, visibleRange: `A1:${addr}`, png: composed }];
}

async function cmdCapture(args) {
  const outDir = path.resolve(REPO_ROOT, args.out ?? DEFAULT_OUT);
  const corpusPath = path.resolve(REPO_ROOT, args.corpus ?? DEFAULT_CORPUS);
  const { corpus, wanted } = await loadScreenFixtures(corpusPath, args.filter ?? null);
  if (wanted.length === 0) {
    process.stderr.write('пустой отбор: --filter не совпал ни с одной фикстурой\n');
    process.exitCode = 2;
    return;
  }
  const only = args.only ?? null;
  const double = args.double === true;
  const fixtures = only === null ? wanted : wanted.filter((f) => f.id === only || f.slug === only);
  if (fixtures.length === 0) {
    process.stderr.write(`--only ${only} не совпал ни с одной фикстурой (id или slug)\n`);
    process.exitCode = 2;
    return;
  }
  process.stdout.write(`к захвату: ${fixtures.length} пар fixture/mode\n`);
  const fixturesDir = path.resolve(REPO_ROOT, args.fixtures ?? DEFAULT_FIXTURES);
  const geometry = JSON.parse(
    await readFile(path.join(REPO_ROOT, 'target', 'actual-frames', 'geometry.json'), 'utf8'),
  );
  await killStaleProfileBrowsers();
  let context = await launch();
  let page = context.pages()[0] ?? (await context.newPage());
  let done = 0;
  let failed = 0;
  try {
    for (const fixture of fixtures) {
      if (CAPTURE_SKIP.has(fixture.id)) {
        process.stderr.write(
          `[skip] ${fixture.id}: строка матрицы без визуального профиля; WAC не открывает документ\n`,
        );
        continue;
      }
      const key = `${fixture.slug}/${fixture.mode}`;
      const pageGeo = geometry[key];
      if (pageGeo === undefined) {
        process.stderr.write(`[fail] ${fixture.id}: нет геометрии в geometry.json (${key})\n`);
        failed += 1;
        continue;
      }
      try {
        const captures =
          fixture.format === 'docx'
            ? await captureDocxPrintLayout(page, fixture, pageGeo, fixturesDir)
            : await captureXlsxNormal(page, fixture, pageGeo, fixturesDir);
        if (double) {
          // Вторая чистая сессия той же среды: два эталона обязаны
          // совпасть байтово, расхождение классифицируется как проблема
          // среды или документа и не маскируется (reference-capture §2 п.4).
          await context.close().catch(() => {});
          await killStaleProfileBrowsers();
          context = await launch();
          page = context.pages()[0] ?? (await context.newPage());
          const captures2 =
            fixture.format === 'docx'
              ? await captureDocxPrintLayout(page, fixture, pageGeo, fixturesDir)
              : await captureXlsxNormal(page, fixture, pageGeo, fixturesDir);
          for (let i = 0; i < captures.length; i += 1) {
            if (!captures[i].png.equals(captures2[i].png)) {
              // Диагностика: сохраняем оба кадра, чтобы разобрать причину.
              await writeFile(
                path.join(REPO_ROOT, 'target', `double-${fixture.slug}-${captures[i].id}-1.png`),
                captures[i].png,
              );
              await writeFile(
                path.join(REPO_ROOT, 'target', `double-${fixture.slug}-${captures[i].id}-2.png`),
                captures2[i].png,
              );
              throw new Error(
                `два захвата ${captures[i].id} различаются — проблема среды или документа`,
              );
            }
          }
          process.stdout.write('[double] два захвата идентичны\n');
        }
        const { dropDir, importOutput } = await writeDropAndImport({ outDir, corpus, fixture, captures });
        process.stdout.write(`[ok] ${fixture.id}: ${dropDir}\n${importOutput}\n`);
        done += 1;
      } catch (error) {
        process.stderr.write(`[fail] ${fixture.id}: ${error}\n`);
        failed += 1;
        const alive = await page.evaluate(() => 1).then(() => true).catch(() => false);
        if (page.isClosed() || !alive) {
          // Рендер-процесс Chromium может падать на OOPIF-скриншотах;
          // перезапускаем браузер и продолжаем со следующей фикстуры.
          process.stderr.write('[info] страница закрылась/упала — перезапускаю браузер\n');
          await context.close().catch(() => {});
          await killStaleProfileBrowsers();
          context = await launch();
          page = context.pages()[0] ?? (await context.newPage());
        }
      }
    }
    process.stdout.write(`готово: ${done} ok, ${failed} fail\n`);
    if (failed > 0) process.exitCode = 1;
  } finally {
    await context.close();
  }
}
