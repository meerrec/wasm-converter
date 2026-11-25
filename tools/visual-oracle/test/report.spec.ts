import { describe, expect, it } from 'vitest';

import type { Corpus } from '../src/corpus.js';
import { buildReport, serializeReport } from '../src/report/json.js';
import { renderHtml } from '../src/report/html.js';
import type { CaptureResult, ResultType } from '../src/types.js';

/** Минимальный corpus index: две фикстуры с разными тегами. */
function corpus(): Corpus {
  return {
    schemaVersion: '1.0.0',
    corpus: {
      id: 'strict-viewer',
      version: '1.0.0',
      matrixVersion: '1.0.0',
      baselineId: 'strict-viewer-1.0.0-1.0.0-abcdef12-0123456789ab',
      storage: { kind: 'git-lfs', rootUri: 'test-fixtures/strict-viewer' },
    },
    fixtures: [
      {
        id: 'docx-tables-long-table',
        format: 'docx',
        source: { path: 'docx/tables/long_table.docx' },
        title: 'Long table',
        tags: ['table', 'pagination'],
        matrixRows: ['docx.tables-pagination'],
        expected: 'Header repeats.',
        modes: ['print-layout'],
      },
      {
        id: 'xlsx-sheets-basic',
        format: 'xlsx',
        source: { path: 'xlsx/sheets/basic.xlsx' },
        title: 'Basic sheets',
        tags: ['sheet'],
        matrixRows: ['xlsx.sheets'],
        expected: 'Two sheets.',
        modes: ['normal'],
      },
    ],
    gaps: [],
  };
}

/** Минимальный capture: остальные тесты меняют одно поле. */
function capture(overrides: Partial<CaptureResult> = {}): CaptureResult {
  return {
    fixtureId: 'docx-tables-long-table',
    format: 'docx',
    mode: 'print-layout',
    captureId: 'page-1',
    resultType: 'match',
    cause: 'render',
    causeSource: 'rule',
    causeEvidence: '0 diff pixels outside masks; ssim=1.0000',
    reference: { uri: 'ref/page-1.png', sha256: 'a'.repeat(64), width: 100, height: 100 },
    actual: { uri: 'actual/page-1.png', sha256: 'b'.repeat(64), width: 100, height: 100 },
    diffImage: null,
    metrics: { diffPixels: 0, diffRatio: 0, maxChannelDelta: 0, ssim: 1 },
    regions: [],
    runtime: null,
    error: null,
    ...overrides,
  };
}

const FIXED_NOW = (): Date => new Date('2026-11-25T10:00:00.000Z');

describe('buildReport', () => {
  it('заполняет все ключи сводки нулями', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [capture()],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    const resultTypes: ResultType[] = [
      'match',
      'visual-diff',
      'page-mismatch',
      'missing-reference',
      'capture-error',
      'harness-error',
    ];
    for (const resultType of resultTypes) expect(report.summary[resultType]).toBeTypeOf('number');
    expect(report.summary.total).toBe(1);
    expect(report.summary.match).toBe(1);
    expect(report.summary['visual-diff']).toBe(0);
    expect(Object.keys(report.byCause).sort()).toEqual(
      ['font', 'harness', 'layout', 'model', 'parse', 'reference', 'render'].sort(),
    );
    expect(report.byCause.render).toBe(1);
  });

  it('сортирует captures по fixtureId и captureId', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [
        capture({ fixtureId: 'xlsx-sheets-basic', captureId: 'sheet-2', format: 'xlsx', mode: 'normal' }),
        capture({ fixtureId: 'docx-tables-long-table', captureId: 'page-2' }),
        capture({ fixtureId: 'docx-tables-long-table', captureId: 'page-1' }),
      ],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    expect(report.captures.map((entry) => `${entry.fixtureId}/${entry.captureId}`)).toEqual([
      'docx-tables-long-table/page-1',
      'docx-tables-long-table/page-2',
      'xlsx-sheets-basic/sheet-2',
    ]);
  });

  it('считает byTag по тегам фикстур, включая теги без captures', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [capture()],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    expect(report.byTag.table).toEqual({ total: 1, match: 1 });
    expect(report.byTag.pagination).toEqual({ total: 1, match: 1 });
    expect(report.byTag.sheet).toEqual({ total: 0, match: 0 });
  });

  it('берёт версии из corpus и baselineId из аргумента', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [],
      engineRevision: 'abc1234',
      baselineId: 'strict-viewer-1.0.0-1.0.0-abcdef12-0123456789ab',
      now: FIXED_NOW,
    });
    expect(report.corpusId).toBe('strict-viewer');
    expect(report.corpusVersion).toBe('1.0.0');
    expect(report.matrixVersion).toBe('1.0.0');
    expect(report.baselineId).toBe('strict-viewer-1.0.0-1.0.0-abcdef12-0123456789ab');
    expect(report.generatedAt).toBe('2026-11-25T10:00:00.000Z');
    expect(report.schemaVersion).toBe('1.0.0');
  });

  it('детерминирован при одинаковом входе', () => {
    const input = {
      corpus: corpus(),
      captures: [capture(), capture({ captureId: 'page-2', resultType: 'visual-diff' as const, cause: 'layout' as const })],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    };
    expect(serializeReport(buildReport(input))).toBe(serializeReport(buildReport(input)));
  });
});

describe('serializeReport', () => {
  it('два пробела отступа и завершающий перевод строки', () => {
    const text = serializeReport(
      buildReport({
        corpus: corpus(),
        captures: [capture()],
        engineRevision: 'abc1234',
        baselineId: null,
        now: FIXED_NOW,
      }),
    );
    expect(text.endsWith('\n')).toBe(true);
    expect(text).toContain('\n  "schemaVersion": "1.0.0"');
    expect(JSON.parse(text)).toMatchObject({ schemaVersion: '1.0.0' });
  });
});

describe('renderHtml', () => {
  it('самодостаточен: без скриптов и внешних ресурсов', () => {
    const html = renderHtml(
      buildReport({
        corpus: corpus(),
        captures: [capture()],
        engineRevision: 'abc1234',
        baselineId: null,
        now: FIXED_NOW,
      }),
    );
    expect(html.startsWith('<!DOCTYPE html>')).toBe(true);
    expect(html).not.toContain('<script');
    expect(html).not.toContain('http://');
    expect(html).not.toContain('https://');
    expect(html).toContain('<style>');
  });

  it('экранирует подставленный текст', () => {
    const html = renderHtml(
      buildReport({
        corpus: corpus(),
        captures: [
          capture({
            fixtureId: '<script>alert(1)</script>',
            causeEvidence: '"><img src=x onerror=alert(1)>',
          }),
        ],
        engineRevision: 'abc1234',
        baselineId: null,
        now: FIXED_NOW,
      }),
      { title: '<b>Report</b>' },
    );
    expect(html).not.toContain('<script>alert(1)</script>');
    expect(html).not.toContain('<img src=x');
    expect(html).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
    expect(html).toContain('&lt;b&gt;Report&lt;/b&gt;');
  });

  it('баннер «не дефект движка» только у capture/harness-error', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [
        capture({ captureId: 'page-1', resultType: 'match' }),
        capture({ captureId: 'page-2', resultType: 'capture-error', cause: 'harness' }),
        capture({ captureId: 'page-3', resultType: 'harness-error', cause: 'harness' }),
      ],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    const html = renderHtml(report);
    const banners = html.split('Не дефект движка').length - 1;
    expect(banners).toBe(2);
  });

  it('сортирует карточки худшими сверху', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [
        capture({ captureId: 'a-match', resultType: 'match' }),
        capture({ captureId: 'b-diff', resultType: 'visual-diff', cause: 'render' }),
        capture({ captureId: 'c-harness', resultType: 'harness-error', cause: 'harness' }),
      ],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    const html = renderHtml(report);
    const harness = html.indexOf('c-harness');
    const diff = html.indexOf('b-diff');
    const match = html.indexOf('a-match');
    expect(harness).toBeGreaterThan(-1);
    expect(harness).toBeLessThan(diff);
    expect(diff).toBeLessThan(match);
  });

  it('внутри группы visual-diff сортирует по доле расхождения', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [
        capture({
          captureId: 'small',
          resultType: 'visual-diff',
          cause: 'render',
          metrics: { diffPixels: 1, diffRatio: 0.01, maxChannelDelta: 5, ssim: 0.99 },
        }),
        capture({
          captureId: 'large',
          resultType: 'visual-diff',
          cause: 'render',
          metrics: { diffPixels: 50, diffRatio: 0.5, maxChannelDelta: 255, ssim: 0.5 },
        }),
      ],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    const html = renderHtml(report);
    expect(html.indexOf('large')).toBeLessThan(html.indexOf('small'));
  });

  it('ссылки на картинки строятся от каталога артефактов', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [
        capture({
          diffImage: 'diff/page-1.png',
          reference: { uri: '/abs/ref/page-1.png', sha256: 'a'.repeat(64), width: 100, height: 100 },
        }),
      ],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    const html = renderHtml(report, { artifactsDir: 'artifacts' });
    expect(html).toContain('src="artifacts/page-1.png"');
    // Относительный путь сохраняет структуру каталогов артефактов.
    expect(html).toContain('src="artifacts/diff/page-1.png"');
    expect(html).not.toContain('/abs/ref');
  });

  it('показывает сведения о фикстуре и ссылку на manifest', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [capture()],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    const html = renderHtml(report, {
      fixtures: [
        {
          fixtureId: 'docx-tables-long-table',
          title: 'Long table',
          sourcePath: 'test-fixtures/docx/tables/long_table.docx',
          manifestPath: 'test-fixtures/strict-viewer/docx/tables/long_table/print-layout/manifest.json',
        },
      ],
    });
    expect(html).toContain('Long table');
    expect(html).toContain('test-fixtures/docx/tables/long_table.docx');
    expect(html).toContain('manifest.json');
  });

  it('показывает ошибку захвата в карточке', () => {
    const report = buildReport({
      corpus: corpus(),
      captures: [
        capture({
          resultType: 'capture-error',
          cause: 'harness',
          error: { code: 'open-failed', message: 'document did not open' },
          metrics: null,
          reference: null,
          actual: null,
        }),
      ],
      engineRevision: 'abc1234',
      baselineId: null,
      now: FIXED_NOW,
    });
    const html = renderHtml(report);
    expect(html).toContain('open-failed');
    expect(html).toContain('document did not open');
  });
});
