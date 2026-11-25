// Человекочитаемый отчёт прогона (`report.html`).
//
// Файл самодостаточен: стили встроены, скриптов и внешних ресурсов нет —
// отчёт открывается из файловой системы без сервера и без сети. Картинки
// лежат рядом с отчётом, поэтому ссылки на них относительные.
//
// Порядок карточек — «худшие сверху»: сначала сломанный контур, затем
// отсутствующий эталон, затем расхождение страниц, затем визуальные
// расхождения по убыванию доли расходящихся пикселей, и только в конце
// совпадения. Внутри группы сортировка по `diffRatio` по убыванию, затем по
// `fixtureId`/`captureId` — чтобы отчёт был детерминирован.
//
// Карточки `capture-error` и `harness-error` помечены баннером «не дефект
// движка»: такие результаты не сертифицируют движок ни в какую сторону, и
// оператор не должен принять их за регрессию рендера.

import type { CaptureResult, Cause, ResultType, RunReport } from '../types.js';

/** Сведения о фикстуре, которых нет в самом capture. */
export interface HtmlFixtureInfo {
  fixtureId: string;
  title: string;
  /** Путь исходного документа от корня репозитория. */
  sourcePath: string;
  /**
   * Путь manifest эталона относительно самого отчёта либо `null`, если
   * эталона нет. Абсолютный путь сделал бы отчёт непереносимым.
   */
  manifestPath: string | null;
}

/** Настройки рендера. */
export interface RenderHtmlOptions {
  /**
   * Каталог артефактов относительно самого отчёта. По умолчанию `.` —
   * картинки лежат рядом с `report.html`.
   */
  artifactsDir?: string;
  /** Заголовок страницы. По умолчанию — «Visual oracle report». */
  title?: string;
  /** Сведения о фикстурах: заголовок, путь источника, путь manifest. */
  fixtures?: readonly HtmlFixtureInfo[];
}

/** Вес типа результата: меньше — хуже. */
const SEVERITY: Readonly<Record<ResultType, number>> = {
  'harness-error': 0,
  'capture-error': 1,
  'missing-reference': 2,
  'page-mismatch': 3,
  'visual-diff': 4,
  match: 5,
};

/** Подпись типа результата для человека. */
const RESULT_LABEL: Readonly<Record<ResultType, string>> = {
  match: 'match',
  'visual-diff': 'visual diff',
  'page-mismatch': 'page mismatch',
  'missing-reference': 'missing reference',
  'capture-error': 'capture error',
  'harness-error': 'harness error',
};

/** Цвет акцента по типу результата: задаётся CSS-переменной карточки. */
const ACCENT: Readonly<Record<ResultType, string>> = {
  match: '#1a7f37',
  'visual-diff': '#b7791f',
  'page-mismatch': '#c05621',
  'missing-reference': '#6b7280',
  'capture-error': '#4a5568',
  'harness-error': '#4a5568',
};

/** Типы, которые не являются дефектом движка. */
const NOT_ENGINE_DEFECT: Readonly<Record<ResultType, boolean>> = {
  match: false,
  'visual-diff': false,
  'page-mismatch': false,
  'missing-reference': false,
  'capture-error': true,
  'harness-error': true,
};

/**
 * Отрисовать отчёт в одну HTML-строку.
 *
 * Чистая функция: одинаковый отчёт и настройки дают одинаковый HTML.
 */
export function renderHtml(report: RunReport, options: RenderHtmlOptions = {}): string {
  const artifactsDir = options.artifactsDir ?? '.';
  const title = options.title ?? 'Visual oracle report';
  const fixtures = new Map<string, HtmlFixtureInfo>();
  for (const fixture of options.fixtures ?? []) fixtures.set(fixture.fixtureId, fixture);

  const captures = [...report.captures].sort(compareCaptures);
  const cards = captures.map((capture) =>
    renderCard(capture, fixtures.get(capture.fixtureId), artifactsDir),
  );

  return [
    '<!DOCTYPE html>',
    '<html lang="ru">',
    '<head>',
    '<meta charset="utf-8">',
    '<meta name="viewport" content="width=device-width, initial-scale=1">',
    `<title>${escapeHtml(title)}</title>`,
    `<style>${STYLES}</style>`,
    '</head>',
    '<body>',
    '<main class="page">',
    `<h1>${escapeHtml(title)}</h1>`,
    renderMeta(report),
    renderSummary(report),
    renderByCause(report),
    renderByTag(report),
    `<section class="cards">${cards.join('')}</section>`,
    '</main>',
    '</body>',
    '</html>',
    '',
  ].join('\n');
}

/** Шапка: версии, ревизия движка, время генерации. */
function renderMeta(report: RunReport): string {
  const rows: [string, string][] = [
    ['Generated at', report.generatedAt],
    ['Engine revision', report.engineRevision],
    ['Corpus', `${report.corpusId} ${report.corpusVersion}`],
    ['Matrix version', report.matrixVersion],
    ['Baseline', report.baselineId ?? 'not accepted'],
  ];
  const body = rows
    .map(([key, value]) => `<tr><th>${escapeHtml(key)}</th><td>${escapeHtml(value)}</td></tr>`)
    .join('');
  return `<table class="meta"><tbody>${body}</tbody></table>`;
}

/** Сводка по типам результата. */
function renderSummary(report: RunReport): string {
  const order: ResultType[] = [
    'match',
    'visual-diff',
    'page-mismatch',
    'missing-reference',
    'capture-error',
    'harness-error',
  ];
  const cells = order
    .map(
      (resultType) =>
        `<td class="count count-${resultType}">${report.summary[resultType]}</td>`,
    )
    .join('');
  const heads = order.map((resultType) => `<th>${escapeHtml(RESULT_LABEL[resultType])}</th>`).join('');
  return [
    '<section class="summary">',
    '<h2>Summary</h2>',
    `<p class="total">Total captures: <strong>${report.summary.total}</strong></p>`,
    `<table class="counts"><thead><tr>${heads}</tr></thead><tbody><tr>${cells}</tr></tbody></table>`,
    '</section>',
  ].join('');
}

/** Сводка по причинам. */
function renderByCause(report: RunReport): string {
  const order: Cause[] = ['parse', 'model', 'layout', 'font', 'render', 'harness', 'reference'];
  const rows = order
    .map(
      (cause) =>
        `<tr><th>${escapeHtml(cause)}</th><td>${report.byCause[cause]}</td></tr>`,
    )
    .join('');
  return `<section class="by-cause"><h2>By cause</h2><table><tbody>${rows}</tbody></table></section>`;
}

/** Сводка по тегам корпуса. */
function renderByTag(report: RunReport): string {
  const tags = Object.keys(report.byTag);
  if (tags.length === 0) return '';
  const rows = tags
    .map((tag) => {
      const entry = report.byTag[tag];
      if (entry === undefined) return '';
      return `<tr><th>${escapeHtml(tag)}</th><td>${entry.match} / ${entry.total}</td></tr>`;
    })
    .join('');
  return `<section class="by-tag"><h2>By tag (match / total)</h2><table><tbody>${rows}</tbody></table></section>`;
}

/** Одна карточка capture. */
function renderCard(
  capture: CaptureResult,
  fixture: HtmlFixtureInfo | undefined,
  artifactsDir: string,
): string {
  const accent = ACCENT[capture.resultType];
  const banner = NOT_ENGINE_DEFECT[capture.resultType]
    ? '<p class="banner">Не дефект движка: результат не сертифицирует рендер.</p>'
    : '';

  const heading = fixture?.title ?? capture.fixtureId;
  const links: string[] = [];
  if (fixture !== undefined) {
    links.push(`<span class="source">${escapeHtml(fixture.sourcePath)}</span>`);
    if (fixture.manifestPath !== null) {
      links.push(`<a href="${escapeHtml(fixture.manifestPath)}">manifest</a>`);
    }
  }

  const facts: [string, string][] = [
    ['Fixture', capture.fixtureId],
    ['Capture', capture.captureId],
    ['Format', capture.format],
    ['Mode', capture.mode],
    ['Result', RESULT_LABEL[capture.resultType]],
    ['Cause', `${capture.cause} (${capture.causeSource})`],
    ['Evidence', capture.causeEvidence],
  ];
  if (capture.metrics !== null) {
    facts.push(
      ['Diff pixels', String(capture.metrics.diffPixels)],
      ['Diff ratio', `${(capture.metrics.diffRatio * 100).toFixed(2)}%`],
      ['Max channel delta', String(capture.metrics.maxChannelDelta)],
      ['SSIM', capture.metrics.ssim === null ? 'n/a' : capture.metrics.ssim.toFixed(4)],
    );
  }
  if (capture.error !== null) {
    facts.push(['Error', `${capture.error.code}: ${capture.error.message}`]);
  }
  if (capture.regions.length > 0) {
    facts.push(['Regions', capture.regions.map((rect) => `${rect.x},${rect.y} ${rect.width}×${rect.height}`).join(', ')]);
  }

  const factRows = facts
    .map(([key, value]) => `<tr><th>${escapeHtml(key)}</th><td>${escapeHtml(value)}</td></tr>`)
    .join('');

  return [
    `<article class="card" style="--accent: ${accent}">`,
    `<header><h3>${escapeHtml(heading)}</h3><span class="badge">${escapeHtml(RESULT_LABEL[capture.resultType])}</span></header>`,
    banner,
    links.length === 0 ? '' : `<p class="links">${links.join(' ')}</p>`,
    `<table class="facts"><tbody>${factRows}</tbody></table>`,
    renderImages(capture, artifactsDir),
    '</article>',
  ].join('');
}

/** Три картинки: эталон, захват, diff. */
function renderImages(capture: CaptureResult, artifactsDir: string): string {
  const figures: string[] = [];
  if (capture.reference !== null) {
    figures.push(
      renderFigure('reference', capture.reference.uri, artifactsDir, capture.reference.width, capture.reference.height),
    );
  }
  if (capture.actual !== null) {
    figures.push(
      renderFigure('actual', capture.actual.uri, artifactsDir, capture.actual.width, capture.actual.height),
    );
  }
  if (capture.diffImage !== null) {
    figures.push(renderFigure('diff', capture.diffImage, artifactsDir, null, null));
  }
  if (figures.length === 0) return '';
  return `<div class="images">${figures.join('')}</div>`;
}

function renderFigure(
  label: string,
  uri: string,
  artifactsDir: string,
  width: number | null,
  height: number | null,
): string {
  const size = width !== null && height !== null ? ` <span class="size">${width}×${height}</span>` : '';
  const href = escapeHtml(artifactHref(uri, artifactsDir));
  return [
    '<figure>',
    `<a href="${href}">`,
    `<img src="${href}" alt="${escapeHtml(label)}" loading="lazy">`,
    '</a>',
    `<figcaption>${escapeHtml(label)}${size}</figcaption>`,
    '</figure>',
  ].join('');
}

/** Порядок карточек: худшие сверху, затем по доле расхождения. */
function compareCaptures(a: CaptureResult, b: CaptureResult): number {
  const severity = SEVERITY[a.resultType] - SEVERITY[b.resultType];
  if (severity !== 0) return severity;
  const ratio = (b.metrics?.diffRatio ?? 0) - (a.metrics?.diffRatio ?? 0);
  if (ratio !== 0) return ratio;
  return a.fixtureId.localeCompare(b.fixtureId) || a.captureId.localeCompare(b.captureId);
}

/**
 * Относительная ссылка на артефакт.
 *
 * `uri` из capture — путь к файлу артефакта. Если он относительный, он и есть
 * путь внутри каталога артефактов: структура каталогов сохраняется, иначе
 * `reference/p1.png` и `actual/p1.png` схлопнулись бы в одну ссылку. Если
 * путь абсолютный, снимается префикс `artifactsDir` (когда он совпадает),
 * иначе остаётся только имя файла — абсолютный путь машины в отчёт попадать
 * не должен.
 */
function artifactHref(uri: string, artifactsDir: string): string {
  const withoutQuery = (uri.split(/[?#]/, 1)[0] ?? uri).replace(/\\/g, '/');
  const isAbsolute = withoutQuery.startsWith('/') || /^[A-Za-z]:\//.test(withoutQuery);
  const root = artifactsDir.replace(/\\/g, '/').replace(/\/+$/, '');

  let relative: string;
  if (!isAbsolute) {
    relative = withoutQuery.replace(/^\.\//, '');
  } else if (root !== '' && root !== '.' && withoutQuery.startsWith(`${root}/`)) {
    relative = withoutQuery.slice(root.length + 1);
  } else {
    const segments = withoutQuery.split('/');
    relative = segments[segments.length - 1] ?? withoutQuery;
  }

  if (root === '' || root === '.') return relative;
  return `${root}/${relative}`;
}

/** Экранирование текста: всё, что попало в HTML, проходит через него. */
function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Встроенные стили: без внешних ресурсов и без скриптов. */
const STYLES = `
:root { color-scheme: light; }
* { box-sizing: border-box; }
body { margin: 0; background: #f6f7f9; color: #1f2328;
  font: 14px/1.5 -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif; }
.page { max-width: 1100px; margin: 0 auto; padding: 32px 20px 64px; }
h1 { font-size: 24px; margin: 0 0 16px; }
h2 { font-size: 16px; margin: 24px 0 8px; text-transform: uppercase; letter-spacing: .04em; color: #57606a; }
h3 { font-size: 15px; margin: 0; }
table { border-collapse: collapse; width: 100%; background: #fff; }
th, td { text-align: left; padding: 6px 10px; border-bottom: 1px solid #e5e7eb; vertical-align: top; }
th { font-weight: 600; color: #57606a; width: 180px; }
.meta th, .by-cause th, .by-tag th { width: 220px; }
.counts th { width: auto; text-align: center; font-size: 12px; }
.counts td { text-align: center; font-size: 18px; font-weight: 600; }
.count-match { color: #1a7f37; }
.count-visual-diff { color: #b7791f; }
.count-page-mismatch { color: #c05621; }
.count-missing-reference { color: #6b7280; }
.count-capture-error, .count-harness-error { color: #4a5568; }
.total { margin: 0 0 8px; }
.cards { display: grid; gap: 16px; margin-top: 24px; }
.card { background: #fff; border: 1px solid #e5e7eb; border-left: 4px solid var(--accent);
  border-radius: 6px; padding: 16px; }
.card header { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
.badge { font-size: 12px; font-weight: 600; color: #fff; background: var(--accent);
  border-radius: 999px; padding: 2px 10px; white-space: nowrap; }
.banner { margin: 12px 0 0; padding: 8px 12px; background: #eef2f7; border-radius: 4px;
  color: #334155; font-weight: 600; }
.links { margin: 8px 0 0; color: #57606a; font-size: 13px; }
.links a { color: #0969da; }
.facts { margin-top: 12px; }
.images { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 12px; margin-top: 12px; }
figure { margin: 0; }
figure img { width: 100%; height: auto; border: 1px solid #e5e7eb; background: #fff; }
figcaption { font-size: 12px; color: #57606a; margin-top: 4px; }
.size { color: #8c959f; }
`;
