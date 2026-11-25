// Тесты чистых частей `capture`: без браузера, без dev-сервера и без записи.
//
// Проверяется ровно то, что решает судьбу прогона до запуска Chromium: slug
// каталога вывода, формат по расширению, отказ на режимах, которые требует
// байтовая матрица (pdf/print-preview), запрет записи в корень эталонов и
// форма синтетического manifest. Сам кадр снимает `createCaptureSession`, его
// проверяет сквозной прогон, а не этот файл.

import { describe, expect, it } from 'vitest';

import { parseArgs } from '../src/cli.js';
import {
  assertOutDirAllowed,
  buildSyntheticManifest,
  CaptureOnlyUsageError,
  detectDocFormat,
  docFixtureSlug,
  resolveCaptureMode,
  type SyntheticManifestInput,
} from '../src/capture_only.js';
import { validateManifestValue } from '../src/manifest.js';

/**
 * Вход manifest: поля, которые тест не меняет.
 *
 * `pageCount: undefined` — законный способ сказать «это не DOCX» в вариантах
 * `it.each`: точный тип запрещает неявный `undefined`, поэтому входной тип
 * шире самого `SyntheticManifestInput`.
 */
function input(
  overrides: Partial<Record<keyof SyntheticManifestInput, unknown>>,
): SyntheticManifestInput {
  const defined = Object.fromEntries(
    Object.entries(overrides).filter(([, value]) => value !== undefined),
  ) as Partial<SyntheticManifestInput>;
  return {
    docPath: 'docx/simple/one_paragraph.docx',
    format: 'docx',
    mode: 'print-layout',
    pageCount: 1,
    outDir: '/tmp/visual-oracle-out',
    sourceSha256: 'b'.repeat(64),
    sourceBytes: 1234,
    operator: 'tester',
    capturedAt: '2026-11-25T00:00:00.000Z',
    ...defined,
  };
}

describe('docFixtureSlug', () => {
  it('берёт имя файла без расширения в нижнем регистре', () => {
    expect(docFixtureSlug('test-fixtures/docx/simple/One_Paragraph.docx')).toBe('one-paragraph');
  });

  it('схлопывает не-буквы и не-цифры в одиночный дефис', () => {
    expect(docFixtureSlug('a b  (c) -- d.xlsx')).toBe('a-b-c-d');
  });

  it('сохраняет не-латинские буквы', () => {
    expect(docFixtureSlug('Отчёт за 2026 год.docx')).toBe('отчёт-за-2026-год');
  });

  it('не оставляет пустого slug', () => {
    expect(docFixtureSlug('___.docx')).toBe('document');
  });
});

describe('detectDocFormat', () => {
  it.each([
    ['docx', 'report.docx'],
    ['docx', 'REPORT.DOCX'],
    ['xlsx', 'book.xlsx'],
    ['xlsx', 'book.XLSX'],
  ] as const)('распознаёт %s по пути %s', (format, path) => {
    expect(detectDocFormat(path)).toBe(format);
  });

  it('отвергает неизвестное расширение', () => {
    expect(() => detectDocFormat('slides.pptx')).toThrow(CaptureOnlyUsageError);
    expect(() => detectDocFormat('no-extension')).toThrow(/unsupported document extension/);
  });
});

describe('resolveCaptureMode', () => {
  it('по умолчанию даёт print-layout для docx и normal для xlsx', () => {
    expect(resolveCaptureMode('docx', null)).toBe('print-layout');
    expect(resolveCaptureMode('xlsx', null)).toBe('normal');
  });

  it('принимает режим своего формата', () => {
    expect(resolveCaptureMode('docx', 'print-layout')).toBe('print-layout');
    expect(resolveCaptureMode('xlsx', 'normal')).toBe('normal');
  });

  it.each(['pdf', 'print-preview'])('отвергает процессный режим %s', (mode) => {
    expect(() => resolveCaptureMode('docx', mode)).toThrow(CaptureOnlyUsageError);
    expect(() => resolveCaptureMode('xlsx', mode)).toThrow(/needs PDF export and rasterization/);
  });

  it('отвергает режим чужого формата', () => {
    expect(() => resolveCaptureMode('docx', 'normal')).toThrow(CaptureOnlyUsageError);
    expect(() => resolveCaptureMode('xlsx', 'print-layout')).toThrow(CaptureOnlyUsageError);
  });
});

describe('assertOutDirAllowed', () => {
  it('отвергает сам корень эталонов и вложенные каталоги', () => {
    expect(() => assertOutDirAllowed('test-fixtures/strict-viewer')).toThrow(CaptureOnlyUsageError);
    expect(() => assertOutDirAllowed('test-fixtures/strict-viewer/docx/x/print-layout')).toThrow(
      /refusing to write/,
    );
  });

  it('разрешает каталог рядом с корнем эталонов', () => {
    expect(() => assertOutDirAllowed('oracle-out/capture')).not.toThrow();
    expect(() => assertOutDirAllowed('test-fixtures/strict-viewer-copy')).not.toThrow();
  });
});

describe('buildSyntheticManifest', () => {
  it('делает по кадру на страницу DOCX', () => {
    const manifest = buildSyntheticManifest(input({ pageCount: 3 }));

    expect(manifest.schemaVersion).toBe('1.1.0');
    expect(manifest.source).toEqual({
      path: 'docx/simple/one_paragraph.docx',
      format: 'docx',
      sha256: 'b'.repeat(64),
      bytes: 1234,
    });
    expect(manifest.corpus.storage.rootUri).toBe('/tmp/visual-oracle-out');
    expect(manifest.view).toEqual({
      format: 'docx',
      mode: 'print-layout',
      revisions: 'accepted',
      comments: 'hidden',
    });
    expect(manifest.reference).toMatchObject({
      provider: 'office-desktop',
      application: 'word-desktop',
      captureMethod: 'automated',
      operator: 'tester',
      capturedAt: '2026-11-25T00:00:00.000Z',
    });
    expect(manifest.captures.map((capture) => capture.kind)).toEqual([
      'docx-page',
      'docx-page',
      'docx-page',
    ]);
    expect(manifest.captures.map((capture) => capture.pageNumber)).toEqual([1, 2, 3]);
    expect(manifest.captures.map((capture) => capture.id)).toEqual(['page-1', 'page-2', 'page-3']);
  });

  it('не добавляет XLSX-полей в DOCX-кадры', () => {
    const [first] = buildSyntheticManifest(input({ pageCount: 1 })).captures;
    expect(first).toBeDefined();
    expect(first?.sheet).toBeUndefined();
    expect(first?.visibleRange).toBeUndefined();
  });

  it('делает по экрану на лист XLSX', () => {
    const manifest = buildSyntheticManifest(
      input({
        docPath: 'xlsx/book.xlsx',
        format: 'xlsx',
        mode: 'normal',
        pageCount: undefined,
        sheetNames: ['Данные', 'Итоги'],
      }),
    );

    expect(manifest.source.format).toBe('xlsx');
    expect(manifest.reference.application).toBe('excel-desktop');
    expect(manifest.view).toEqual({
      format: 'xlsx',
      mode: 'normal',
      activeSheet: { index: 0, name: 'Данные' },
      hiddenSheets: 'exclude',
      showGridLines: true,
      showHeadings: true,
    });
    expect(manifest.captures).toHaveLength(2);
    expect(manifest.captures.map((capture) => capture.kind)).toEqual([
      'xlsx-screen',
      'xlsx-screen',
    ]);
    expect(manifest.captures.map((capture) => capture.sheet)).toEqual([
      { index: 0, name: 'Данные' },
      { index: 1, name: 'Итоги' },
    ]);
    for (const capture of manifest.captures) {
      expect(capture.visibleRange).toBe('A1:H20');
      expect(capture.scroll).toEqual({ x: 0, y: 0 });
      expect(capture.activeCell).toBe('A1');
      expect(capture.pageNumber).toBeUndefined();
    }
  });

  it('отвергает пустой список листов', () => {
    expect(() =>
      buildSyntheticManifest(input({ format: 'xlsx', mode: 'normal', pageCount: undefined, sheetNames: [] })),
    ).toThrow(CaptureOnlyUsageError);
  });

  it('отвергает нулевое число страниц', () => {
    expect(() => buildSyntheticManifest(input({ pageCount: 0 }))).toThrow(/positive page count/);
  });
});

describe('parseArgs capture', () => {
  it('разбирает обязательные флаги и умолчания', () => {
    const command = parseArgs([
      'capture',
      '--doc',
      'test-fixtures/docx/simple/one_paragraph.docx',
      '--out',
      'oracle-out/capture',
    ]);
    expect(command).toEqual({
      kind: 'capture',
      options: {
        doc: 'test-fixtures/docx/simple/one_paragraph.docx',
        out: 'oracle-out/capture',
        mode: null,
        port: null,
        headed: false,
      },
    });
  });

  it('разбирает необязательные флаги', () => {
    const command = parseArgs([
      'capture',
      '--doc',
      'book.xlsx',
      '--out',
      'out',
      '--mode',
      'normal',
      '--port',
      '5199',
      '--headed',
    ]);
    expect(command).toEqual({
      kind: 'capture',
      options: { doc: 'book.xlsx', out: 'out', mode: 'normal', port: 5199, headed: true },
    });
  });

  it('печатает справку по --help', () => {
    const command = parseArgs(['capture', '--help']);
    expect(command.kind).toBe('help');
    if (command.kind === 'help') expect(command.text).toContain('visual-oracle capture');
  });

  it('требует --doc и --out', () => {
    expect(() => parseArgs(['capture', '--out', 'out'])).toThrow(/missing required flag --doc/);
    expect(() => parseArgs(['capture', '--doc', 'a.docx'])).toThrow(/missing required flag --out/);
  });

  it('отвергает нечисловой порт и неизвестный флаг', () => {
    expect(() => parseArgs(['capture', '--doc', 'a.docx', '--out', 'o', '--port', 'x'])).toThrow(
      /flag --port expects a number/,
    );
    expect(() => parseArgs(['capture', '--doc', 'a.docx', '--out', 'o', '--bogus'])).toThrow(
      /unknown argument/,
    );
  });
});

describe('синтетический manifest и словарь захвата', () => {
  // Синтетический manifest заведомо не проходит полную валидацию (нет
  // исходников и PNG на диске), поэтому проверяется то, что решает словарь
  // ADR-0056: provider/application обязаны быть одного семейства, иначе
  // `checkReferenceIdentity` отвергает верные значения.
  it.each([
    ['docx', 'word-desktop'],
    ['xlsx', 'excel-desktop'],
  ])('даёт согласованную пару provider/application для %s', async (format, application) => {
    const manifest = buildSyntheticManifest(
      input({ format, mode: format === 'docx' ? 'print-layout' : 'normal', pageCount: undefined, sheetNames: ['Лист'] }),
    );
    expect(manifest.reference.provider).toBe('office-desktop');
    expect(manifest.reference.application).toBe(application);

    const result = await validateManifestValue(
      manifest,
      `/tmp/capture-only/${format}-fixture/manifest.json`,
      { schemaPath: 'docs/strict-viewer/manifest.schema.json' },
    );
    // Ожидаемые ошибки — только про отсутствующие артефакты и путь кадра,
    // а не про словарь эталона.
    const errors = result.ok ? [] : result.errors;
    const vocabulary = errors.filter((error) => error.path.startsWith('reference'));
    expect(vocabulary, JSON.stringify(errors)).toEqual([]);
  });
});
