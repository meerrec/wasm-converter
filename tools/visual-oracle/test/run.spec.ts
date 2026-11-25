// Тесты чистых частей прогона: код выхода и разбор `--override`.
//
// Сам прогон требует Chromium и dev-сервер, поэтому проверяется сквозным
// прогоном, а не здесь. Здесь — ровно то, что решает судьбу CI: таблица
// кодов выхода (baseline-policy §6) и отказ на неизвестной причине.

import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import type { CapturedImage } from '../src/capture/capture.js';
import type { CaptureSession } from '../src/capture/index.js';
import { encodePng } from '../src/diff/png.js';
import { sha256Hex } from '../src/hash.js';
import { REPO_ROOT } from '../src/paths.js';
import type { Manifest } from '../src/manifest.js';
import { CAUSES, draftGateFailure, exitCodeFor, parseOverrides, runOracle } from '../src/run.js';
import type { CaptureResult, ResultType, RunReport, RuntimeMetadata } from '../src/types.js';

/** Отчёт с заданными типами результатов и нулевыми остальными полями. */
function reportOf(...types: readonly ResultType[]): RunReport {
  const summary: RunReport['summary'] = {
    total: types.length,
    match: 0,
    'visual-diff': 0,
    'page-mismatch': 0,
    'missing-reference': 0,
    'capture-error': 0,
    'harness-error': 0,
  };
  for (const type of types) summary[type] += 1;
  return {
    schemaVersion: '1.0.0',
    generatedAt: '2026-01-01T00:00:00.000Z',
    engineRevision: 'deadbee',
    corpusId: 'strict-viewer',
    corpusVersion: '0.1.0',
    matrixVersion: '1.0.0',
    baselineId: null,
    summary,
    byCause: {
      parse: 0,
      model: 0,
      layout: 0,
      font: 0,
      render: 0,
      harness: 0,
      reference: 0,
    },
    byTag: {},
    captures: [],
  };
}

describe('exitCodeFor', () => {
  it('возвращает 0, когда все captures совпали', () => {
    expect(exitCodeFor(reportOf('match', 'match'))).toBe(0);
  });

  it('возвращает 0 на пустом отчёте', () => {
    expect(exitCodeFor(reportOf())).toBe(0);
  });

  it('возвращает 1 на visual-diff', () => {
    expect(exitCodeFor(reportOf('match', 'visual-diff'))).toBe(1);
  });

  it('возвращает 1 на page-mismatch', () => {
    expect(exitCodeFor(reportOf('page-mismatch'))).toBe(1);
  });

  it('возвращает 2 на missing-reference', () => {
    expect(exitCodeFor(reportOf('missing-reference'))).toBe(2);
  });

  it('возвращает 2 на capture-error', () => {
    expect(exitCodeFor(reportOf('capture-error'))).toBe(2);
  });

  it('возвращает 3 на harness-error', () => {
    expect(exitCodeFor(reportOf('harness-error'))).toBe(3);
  });

  it('harness-error перекрывает неполный захват', () => {
    expect(exitCodeFor(reportOf('missing-reference', 'harness-error'))).toBe(3);
  });

  it('harness-error перекрывает измеренное расхождение', () => {
    expect(exitCodeFor(reportOf('visual-diff', 'harness-error'))).toBe(3);
  });

  it('неполный захват перекрывает измеренное расхождение', () => {
    expect(exitCodeFor(reportOf('visual-diff', 'missing-reference'))).toBe(2);
    expect(exitCodeFor(reportOf('page-mismatch', 'capture-error'))).toBe(2);
  });
});

describe('parseOverrides', () => {
  it('разбирает переопределение фикстуры', () => {
    expect(parseOverrides(['docx-tables-long-table=font'])).toEqual({
      'docx-tables-long-table': 'font',
    });
  });

  it('разбирает переопределение одного capture', () => {
    expect(parseOverrides(['docx-tables-long-table/page-2=layout'])).toEqual({
      'docx-tables-long-table/page-2': 'layout',
    });
  });

  it('накапливает несколько записей', () => {
    expect(parseOverrides(['a=parse', 'b/page-1=render'])).toEqual({
      a: 'parse',
      'b/page-1': 'render',
    });
  });

  it('принимает все объявленные причины', () => {
    const entries = CAUSES.map((cause, index) => `fixture-${index}=${cause}`);
    expect(Object.values(parseOverrides(entries))).toEqual([...CAUSES]);
  });

  it('отвергает запись без причины', () => {
    expect(() => parseOverrides(['fixture='])).toThrow(/unknown cause/);
  });

  it('отвергает запись без разделителя', () => {
    expect(() => parseOverrides(['fixture'])).toThrow(/expected <fixtureId/);
  });

  it('отвергает пустой ключ', () => {
    expect(() => parseOverrides(['=font'])).toThrow(/expected <fixtureId/);
  });

  it('отвергает неизвестную причину', () => {
    expect(() => parseOverrides(['fixture=colour'])).toThrow(/unknown cause "colour"/);
  });
});

describe('CAUSES', () => {
  it('совпадает с Cause из types.ts', () => {
    // Список продублирован ради рантайм-проверки `--override`: тип стирается,
    // а опечатка в причине должна падать, а не попадать в отчёт.
    const fromTypes: readonly CaptureResult['cause'][] = [
      'parse',
      'model',
      'layout',
      'font',
      'render',
      'harness',
      'reference',
    ];
    expect([...CAUSES].sort()).toEqual([...fromTypes].sort());
  });
});

/** PNG одного цвета: содержимое важно только как различимый артефакт. */
function pngOf(red: number, green: number, blue: number): Uint8Array {
  return encodePng({
    width: 2,
    height: 2,
    data: new Uint8Array([
      red, green, blue, 255, red, green, blue, 255,
      red, green, blue, 255, red, green, blue, 255,
    ]),
  });
}

/** Manifest DOCX print-layout на две страницы с настоящими артефактами. */
function twoPageManifest(
  sourcePath: string,
  sourceBytes: Uint8Array,
  images: readonly { uri: string; bytes: Uint8Array }[],
): Manifest {
  return {
    schemaVersion: '1.1.0',
    corpus: {
      id: 'strict-viewer',
      version: '1.0.0',
      matrixVersion: '1.0.0',
      baselineId: 'strict-viewer-test',
      storage: { kind: 'git-lfs', rootUri: '.' },
    },
    source: {
      path: sourcePath,
      format: 'docx',
      sha256: sha256Hex(sourceBytes),
      bytes: sourceBytes.byteLength,
    },
    reference: {
      provider: 'office-online',
      application: 'word-online',
      capturedAt: '2026-01-01T00:00:00.000Z',
      captureMethod: 'manual',
      operator: 'test',
    },
    environment: {
      os: { name: 'macOS', version: '15' },
      browser: { name: 'chromium', version: '1243' },
      viewportCss: { width: 1440, height: 900 },
      deviceScaleFactor: 1,
      applicationZoomPercent: 100,
      browserZoomPercent: 100,
      locale: 'en-US',
      timezone: 'UTC',
      colorScheme: 'light',
      fonts: {
        package: 'carlito',
        version: '1.0',
        assets: [
          {
            family: 'Carlito',
            style: 'normal',
            weight: 400,
            sha256: '0'.repeat(64),
          },
        ],
      },
    },
    view: { format: 'docx', mode: 'print-layout', revisions: 'accepted', comments: 'hidden' },
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
    captures: images.map((image, index) => ({
      id: `page-${index + 1}`,
      kind: 'docx-page' as const,
      pageNumber: index + 1,
      image: {
        uri: image.uri,
        sha256: sha256Hex(image.bytes),
        width: 2,
        height: 2,
        bytes: image.bytes.byteLength,
        colorType: 'RGBA8' as const,
      },
    })),
  };
}

describe('runOracle: доказательство стабильности на строку отчёта', () => {
  it('берёт stableHashes и timeToStableMs у своего артефакта, а не у фикстуры', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'oracle-run-'));
    try {
      const fixtureId = 'docx-tables-long-table';
      const sourcePath = 'docx/tables/long_table.docx';
      const referencesRoot = path.join(root, 'refs');
      const outDir = path.join(root, 'out');
      const modeDir = path.join(referencesRoot, 'docx', fixtureId, 'print-layout');
      await mkdir(path.join(modeDir, 'reference'), { recursive: true });

      // `runOracle` разрешает `source.path` от настоящего `test-fixtures/`,
      // поэтому берём реальный файл корпуса, а не выдуманный.
      const sourceBytes = new Uint8Array(
        await readFile(path.join(REPO_ROOT, 'test-fixtures', sourcePath)),
      );

      const page1 = pngOf(10, 20, 30);
      const page2 = pngOf(200, 100, 50);
      await writeFile(path.join(modeDir, 'reference', 'page-1.png'), page1);
      await writeFile(path.join(modeDir, 'reference', 'page-2.png'), page2);

      const manifest = twoPageManifest(sourcePath, sourceBytes, [
        { uri: `docx/${fixtureId}/print-layout/reference/page-1.png`, bytes: page1 },
        { uri: `docx/${fixtureId}/print-layout/reference/page-2.png`, bytes: page2 },
      ]);
      await writeFile(path.join(modeDir, 'manifest.json'), JSON.stringify(manifest, null, 2));

      const corpusPath = path.join(root, 'corpus.json');
      await writeFile(
        corpusPath,
        JSON.stringify({
          schemaVersion: '1.0.0',
          corpus: {
            id: 'strict-viewer',
            version: '1.0.0',
            matrixVersion: '1.0.0',
            baselineId: null,
            storage: { kind: 'git-lfs', rootUri: 'refs' },
          },
          fixtures: [
            {
              id: fixtureId,
              format: 'docx',
              source: { path: sourcePath },
              title: 'Длинная таблица',
              tags: ['tables'],
              matrixRows: ['docx.tables'],
              expected: 'Таблица на две страницы.',
              modes: ['print-layout'],
            },
          ],
          gaps: [],
        }),
      );

      // Захват отдаёт те же пиксели, но с РАЗНЫМ доказательством на страницу:
      // именно это и должно попасть в строки отчёта.
      const runtime: RuntimeMetadata = {
        engineRevision: 'deadbee',
        transport: 'sab',
        renderMode: 'worker',
        viewport: { width: 1440, height: 900 },
        dpr: 1,
        zoom: 1,
        fonts: [],
        fontSubstitutions: [],
        featureSupport: null,
        timeToStableMs: 999,
        stableHashes: ['last', 'last'],
        captureBackend: 'canvas',
      };
      const captured: CapturedImage[] = [
        {
          captureId: 'page-1',
          kind: 'docx-page',
          path: path.join(outDir, fixtureId, 'print-layout', 'page-1.png'),
          sha256: sha256Hex(page1),
          width: 2,
          height: 2,
          frame: { cssWidth: 2, cssHeight: 2 },
          pageNumber: 1,
          stableHashes: ['aaa1', 'aaa1'],
          timeToStableMs: 11,
        },
        {
          captureId: 'page-2',
          kind: 'docx-page',
          path: path.join(outDir, fixtureId, 'print-layout', 'page-2.png'),
          sha256: sha256Hex(page2),
          width: 2,
          height: 2,
          frame: { cssWidth: 2, cssHeight: 2 },
          pageNumber: 2,
          stableHashes: ['bbb2', 'bbb2'],
          timeToStableMs: 22,
        },
      ];
      await mkdir(path.join(outDir, fixtureId, 'print-layout'), { recursive: true });
      await writeFile(captured[0]!.path, page1);
      await writeFile(captured[1]!.path, page2);

      const session: CaptureSession = {
        async capture() {
          return { ok: true, captures: captured, runtime, structure: { unit: 'pages', count: 2 } };
        },
        async close() {
          // Сессия поддельная: закрывать нечего.
        },
      };

      const summary = await runOracle({
        corpusPath,
        referencesRoot,
        outDir,
        captureSession: session,
        now: () => new Date('2026-01-01T00:00:00.000Z'),
      });

      const rows = summary.report.captures;
      expect(rows.map((row) => row.resultType)).toEqual(['match', 'match']);
      expect(rows).toHaveLength(2);
      const row1 = rows.find((row) => row.captureId === 'page-1');
      const row2 = rows.find((row) => row.captureId === 'page-2');
      expect(row1?.runtime?.stableHashes).toEqual(['aaa1', 'aaa1']);
      expect(row1?.runtime?.timeToStableMs).toBe(11);
      expect(row2?.runtime?.stableHashes).toEqual(['bbb2', 'bbb2']);
      expect(row2?.runtime?.timeToStableMs).toBe(22);
      // Остальные поля runtime — общие для фикстуры.
      expect(row1?.runtime?.engineRevision).toBe('deadbee');
      expect(row1?.runtime?.captureBackend).toBe('canvas');

      // И то же самое в записанном report.json.
      const written = JSON.parse(await readFile(summary.reportJsonPath, 'utf8')) as RunReport;
      const writtenRow = written.captures.find((row) => row.captureId === 'page-1');
      expect(writtenRow?.runtime?.stableHashes).toEqual(['aaa1', 'aaa1']);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});

describe('draftGateFailure', () => {
  it('допускает черновой прогон, где нет ничего, кроме missing-reference', () => {
    expect(draftGateFailure(reportOf('match', 'missing-reference'), 2)).toBeNull();
  });

  it('допускает черновой прогон, где все существующие эталоны совпали', () => {
    expect(draftGateFailure(reportOf('match', 'match'), 0)).toBeNull();
  });

  // Ровно тот случай, который пропускала прежняя проверка кода 2: эталон есть
  // у одной пары, и именно он разъехался.
  it('отказывает при visual-diff, спрятанном за кодом 2 из-за missing-reference', () => {
    const report = reportOf('visual-diff', ...Array.from({ length: 83 }, () => 'missing-reference' as const));
    expect(exitCodeFor(report)).toBe(2);
    const failure = draftGateFailure(report, 2);
    expect(failure).toContain('visual-diff=1');
  });

  it('отказывает при page-mismatch в черновом прогоне', () => {
    const failure = draftGateFailure(reportOf('page-mismatch', 'missing-reference'), 2);
    expect(failure).toContain('page-mismatch=1');
  });

  it('отказывает при capture-error', () => {
    expect(draftGateFailure(reportOf('capture-error', 'missing-reference'), 2)).toContain(
      'capture-error=1',
    );
  });

  it('отказывает при harness-error', () => {
    expect(draftGateFailure(reportOf('harness-error'), 3)).toContain('exited with code 3');
  });

  it('отказывает при коде 1, даже если счётчики нулевые', () => {
    expect(draftGateFailure(reportOf('match'), 1)).toContain('exited with code 1');
  });

  it('отказывает, когда счётчика нет: отчёт без числа не судит', () => {
    const report = reportOf('match');
    const summary: Partial<RunReport['summary']> = { ...report.summary };
    delete summary['visual-diff'];
    const broken = { ...report, summary: summary as RunReport['summary'] };
    expect(draftGateFailure(broken, 0)).toContain('visual-diff is not a number');
  });
});
