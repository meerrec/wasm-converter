import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { sha256Hex } from '../src/hash.js';
import { ImportError, importReference } from '../src/reference/import.js';

const SCHEMA_PATH = 'docs/strict-viewer/manifest.schema.json';

const tempDirs: string[] = [];

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'visual-oracle-import-'));
  tempDirs.push(dir);
  return dir;
}

/** CRC-32: собранный PNG должен быть настоящим файлом, а не только заголовком. */
function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** PNG с одним IHDR: импортёр читает только заголовок. */
function buildPng(width: number, height: number, colorType: 2 | 6, bitDepth = 8): Uint8Array {
  const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  const ihdr = new Uint8Array(13);
  const view = new DataView(ihdr.buffer);
  view.setUint32(0, width, false);
  view.setUint32(4, height, false);
  ihdr[8] = bitDepth;
  ihdr[9] = colorType;
  const body = new Uint8Array([...new TextEncoder().encode('IHDR'), ...ihdr]);
  const crc = new Uint8Array(4);
  new DataView(crc.buffer).setUint32(0, crc32(body), false);
  const length = new Uint8Array(4);
  new DataView(length.buffer).setUint32(0, ihdr.byteLength, false);
  return new Uint8Array([...signature, ...length, ...body, ...crc]);
}

/** Окружение экранного режима: viewport, DPR и colorScheme обязательны. */
function environment(): Record<string, unknown> {
  return {
    os: { name: 'Windows 11', version: '10.0.22631' },
    browser: { name: 'chromium', version: '131.0.6778.86' },
    viewportCss: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    applicationZoomPercent: 100,
    browserZoomPercent: 100,
    locale: 'ru-RU',
    timezone: 'UTC',
    colorScheme: 'light',
    fonts: {
      package: 'strict-viewer-fonts',
      version: '1.0.0',
      assets: [{ family: 'Carlito', style: 'normal', weight: 400, sha256: 'a'.repeat(64) }],
    },
  };
}

/** Corpus index с одной DOCX-фикстурой в двух режимах. */
function corpusValue(): Record<string, unknown> {
  return {
    schemaVersion: '1.0.0',
    corpus: {
      id: 'strict-viewer-1.0.0',
      version: '1.0.0',
      matrixVersion: '1.0.0',
      baselineId: null,
      storage: { kind: 'git-lfs', rootUri: 'test-fixtures/strict-viewer' },
    },
    fixtures: [
      {
        id: 'docx-tables-long-table',
        format: 'docx',
        source: { path: 'docx/tables/long_table.docx' },
        title: 'Long table',
        tags: ['table'],
        matrixRows: ['docx.tables-pagination'],
        expected: 'Header repeats.',
        modes: ['print-layout', 'pdf'],
      },
    ],
    gaps: [],
  };
}

/** Разложенный операторский drop плюс корни, с которыми его импортируют. */
interface Drop {
  root: string;
  corpusPath: string;
  fixturesRoot: string;
  referencesRoot: string;
  from: string;
  captureJson: Record<string, unknown>;
}

/**
 * Собрать drop `<format>/<fixture-id>/<mode>` с `capture.json` и PNG.
 *
 * `mutate` правит `capture.json` до записи — так тесты ломают ровно одно поле.
 */
async function makeDrop(
  mutate: (capture: Record<string, unknown>) => void = () => {},
  options: { mode?: 'print-layout' | 'pdf'; colorType?: 2 | 6; bitDepth?: number } = {},
): Promise<Drop> {
  const root = await tempDir();
  const fixturesRoot = join(root, 'test-fixtures');
  const referencesRoot = join(root, 'test-fixtures', 'strict-viewer');
  const mode = options.mode ?? 'print-layout';
  const from = join(referencesRoot, 'docx', 'docx-tables-long-table', mode);

  const sourcePath = join(fixturesRoot, 'docx', 'tables', 'long_table.docx');
  await mkdir(join(sourcePath, '..'), { recursive: true });
  await writeFile(sourcePath, new TextEncoder().encode('fake docx bytes'));

  await mkdir(join(from, 'reference'), { recursive: true });
  const png = buildPng(100, 200, options.colorType ?? 6, options.bitDepth ?? 8);
  await writeFile(join(from, 'reference', 'page-1.png'), png);

  const captureJson: Record<string, unknown> = {
    corpus: { version: '1.0.0', matrixVersion: '1.0.0' },
    source: { path: 'docx/tables/long_table.docx' },
    reference: {
      provider: 'office-online',
      application: 'word-online',
      capturedAt: '2026-11-20T10:00:00Z',
      captureMethod: 'manual',
      operator: 'qa',
    },
    environment: environment(),
    view: { format: 'docx', mode, revisions: 'accepted', comments: 'hidden' },
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
    captures: [{ id: 'page-1', kind: 'docx-page', pageNumber: 1, image: 'reference/page-1.png' }],
  };

  if (mode === 'pdf') {
    const pdf = new TextEncoder().encode('%PDF-1.7 fake');
    await writeFile(join(from, 'reference', 'export.pdf'), pdf);
    (captureJson['reference'] as Record<string, unknown>)['exportedPdf'] = {
      uri: 'reference/export.pdf',
    };
    const env = captureJson['environment'] as Record<string, unknown>;
    delete env['viewportCss'];
    delete env['deviceScaleFactor'];
    delete env['colorScheme'];
    env['rasterizer'] = { name: 'pdftoppm', version: '24.02.0', dpi: 144 };
  }

  mutate(captureJson);
  await writeFile(join(from, 'capture.json'), JSON.stringify(captureJson, null, 2), 'utf8');

  const corpusPath = join(root, 'corpus.json');
  await writeFile(corpusPath, JSON.stringify(corpusValue(), null, 2), 'utf8');

  return { root, corpusPath, fixturesRoot, referencesRoot, from, captureJson };
}

function importOptions(drop: Drop, extra: { force?: boolean } = {}) {
  return {
    from: drop.from,
    corpusPath: drop.corpusPath,
    rootUri: drop.referencesRoot,
    fixturesRoot: drop.fixturesRoot,
    schemaPath: SCHEMA_PATH,
    ...extra,
  };
}

describe('importReference', () => {
  it('пишет schema-valid manifest рядом с drop', async () => {
    const drop = await makeDrop();
    const result = await importReference(importOptions(drop));

    expect(result.manifestPath).toBe(join(drop.from, 'manifest.json'));
    expect(result.captureCount).toBe(1);
    expect(result.previousBaselineId).toBeNull();
    expect(result.baselineId).toMatch(
      /^strict-viewer-1\.0\.0-1\.0\.0-[0-9a-f]{8}-[0-9a-f]{12}$/,
    );

    const manifest = JSON.parse(await readFile(result.manifestPath, 'utf8')) as Record<
      string,
      unknown
    >;
    expect(manifest['schemaVersion']).toBe('1.1.0');
    expect((manifest['corpus'] as Record<string, unknown>)['baselineId']).toBe(result.baselineId);
    expect((manifest['source'] as Record<string, unknown>)['bytes']).toBe(15);

    const capture = (manifest['captures'] as Array<Record<string, unknown>>)[0];
    expect(capture?.['image']).toMatchObject({
      uri: 'docx/docx-tables-long-table/print-layout/reference/page-1.png',
      width: 100,
      height: 200,
      colorType: 'RGBA8',
    });
  });

  it('импортирует pdf-режим с exportedPdf', async () => {
    const drop = await makeDrop(() => {}, { mode: 'pdf' });
    const result = await importReference(importOptions(drop));
    const manifest = JSON.parse(await readFile(result.manifestPath, 'utf8')) as Record<
      string,
      unknown
    >;
    expect((manifest['reference'] as Record<string, unknown>)['exportedPdf']).toMatchObject({
      uri: 'docx/docx-tables-long-table/pdf/reference/export.pdf',
    });
  });

  it('отказывается перезаписывать manifest без force', async () => {
    const drop = await makeDrop();
    await importReference(importOptions(drop));
    await expect(importReference(importOptions(drop))).rejects.toBeInstanceOf(ImportError);
  });

  it('перезаписывает manifest с force и сообщает прежний baselineId', async () => {
    const drop = await makeDrop();
    const first = await importReference(importOptions(drop));
    const second = await importReference(importOptions(drop, { force: true }));
    expect(second.previousBaselineId).toBe(first.baselineId);
    expect(second.baselineId).toBe(first.baselineId);
  });

  it('требует каталог <format>/<fixture-id>/<mode>', async () => {
    const drop = await makeDrop();
    // Тот же drop, но каталог назван иначе: путь — часть контракта, а не
    // удобство, иначе manifest уедет не в ту фикстуру.
    const wrong = join(drop.referencesRoot, 'docx', 'docx-tables-long-table', 'normal');
    await mkdir(wrong, { recursive: true });
    await writeFile(join(wrong, 'capture.json'), JSON.stringify(drop.captureJson), 'utf8');
    await expect(importReference({ ...importOptions(drop), from: wrong })).rejects.toThrow(
      /must end with docx\/docx-tables-long-table\/print-layout/,
    );
  });

  it('требует capture.json', async () => {
    const drop = await makeDrop();
    await rm(join(drop.from, 'capture.json'));
    await expect(importReference(importOptions(drop))).rejects.toThrow(/cannot read .*capture\.json/);
  });

  it('требует фикстуру в corpus index', async () => {
    const drop = await makeDrop();
    const corpus = corpusValue();
    corpus['fixtures'] = [];
    await writeFile(drop.corpusPath, JSON.stringify(corpus), 'utf8');
    await expect(importReference(importOptions(drop))).rejects.toThrow(/is not listed in/);
  });

  it('требует режим, объявленный в corpus index', async () => {
    const drop = await makeDrop();
    const corpus = corpusValue();
    (corpus['fixtures'] as Array<Record<string, unknown>>)[0]!['modes'] = ['pdf'];
    await writeFile(drop.corpusPath, JSON.stringify(corpus), 'utf8');
    await expect(importReference(importOptions(drop))).rejects.toThrow(/is not listed for fixture/);
  });

  it('ловит расхождение source.path с corpus index', async () => {
    const drop = await makeDrop();
    // Тот же slug, другой путь: id совпадает, а документ — нет.
    const corpus = corpusValue();
    (corpus['fixtures'] as Array<Record<string, unknown>>)[0]!['source'] = {
      path: 'docx/tables/long-table.docx',
    };
    await writeFile(drop.corpusPath, JSON.stringify(corpus), 'utf8');
    await expect(importReference(importOptions(drop))).rejects.toThrow(/does not match corpus index/);
  });

  it('ловит устаревшую версию корпуса в capture.json', async () => {
    const drop = await makeDrop((capture) => {
      capture['corpus'] = { version: '0.9.0', matrixVersion: '1.0.0' };
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(/corpus version 0\.9\.0/);
  });

  it('отвергает изображение, которое не PNG', async () => {
    const drop = await makeDrop();
    await writeFile(join(drop.from, 'reference', 'page-1.png'), 'not a png at all, really not');
    await expect(importReference(importOptions(drop))).rejects.toThrow(/not a PNG/);
  });

  it('отвергает 16-битный PNG', async () => {
    const drop = await makeDrop(() => {}, { bitDepth: 16 });
    await expect(importReference(importOptions(drop))).rejects.toThrow(/unsupported color type/);
  });

  it('отвергает путь изображения вне drop', async () => {
    const drop = await makeDrop((capture) => {
      (capture['captures'] as Array<Record<string, unknown>>)[0]!['image'] = '../escape.png';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(/does not resolve inside/);
  });

  it('отвергает изображение не под reference/', async () => {
    // Drop содержит только reference/, поэтому image без префикса указывал бы
    // на несуществующий файл в manifest.
    const drop = await makeDrop((capture) => {
      (capture['captures'] as Array<Record<string, unknown>>)[0]!['image'] = 'page-1.png';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /captures\[0\]\.image "page-1\.png" must live under reference\//,
    );
    await expect(readFile(join(drop.from, 'manifest.json'), 'utf8')).rejects.toThrow();
  });

  it('отвергает изображение из actual/', async () => {
    const drop = await makeDrop((capture) => {
      (capture['captures'] as Array<Record<string, unknown>>)[0]!['image'] = 'actual/page-1.png';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /captures\[0\]\.image "actual\/page-1\.png" must live under reference\//,
    );
  });

  it('отвергает вложенный путь под reference/', async () => {
    const drop = await makeDrop((capture) => {
      (capture['captures'] as Array<Record<string, unknown>>)[0]!['image'] =
        'reference/nested/page-1.png';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /must be a file directly under reference\//,
    );
  });

  it('отвергает не-PNG под reference/', async () => {
    const drop = await makeDrop((capture) => {
      (capture['captures'] as Array<Record<string, unknown>>)[0]!['image'] = 'reference/page-1.jpg';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(/must be a \.png file/);
  });

  it('принимает reference/page-1.png и пишет его в manifest', async () => {
    const drop = await makeDrop();
    const result = await importReference(importOptions(drop));
    const manifest = JSON.parse(await readFile(result.manifestPath, 'utf8')) as Record<
      string,
      unknown
    >;
    const capture = (manifest['captures'] as Array<Record<string, unknown>>)[0];
    expect((capture?.['image'] as Record<string, unknown>)['uri']).toBe(
      'docx/docx-tables-long-table/print-layout/reference/page-1.png',
    );
  });

  it('отвергает exportedPdf вне reference/export.pdf', async () => {
    const drop = await makeDrop(
      (capture) => {
        (capture['reference'] as Record<string, unknown>)['exportedPdf'] = { uri: 'export.pdf' };
      },
      { mode: 'pdf' },
    );
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /reference\.exportedPdf\.uri "export\.pdf" must live under reference\//,
    );
  });

  it('отвергает неизвестный kind захвата', async () => {
    const drop = await makeDrop((capture) => {
      (capture['captures'] as Array<Record<string, unknown>>)[0]!['kind'] = 'docx-screenshot';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(/kind must be/);
  });

  it('не пишет manifest, если итог не проходит валидацию', async () => {
    const drop = await makeDrop((capture) => {
      (capture['captures'] as Array<Record<string, unknown>>)[0]!['pageNumber'] = 2;
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(/generated manifest is invalid/);
    await expect(readFile(join(drop.from, 'manifest.json'), 'utf8')).rejects.toThrow();
  });

  it('принимает desktop-захват Word', async () => {
    const drop = await makeDrop((capture) => {
      capture['reference'] = {
        provider: 'office-desktop',
        application: 'word-desktop',
        applicationVersion: '16.0.17928',
        capturedAt: '2026-11-20T10:00:00Z',
        captureMethod: 'automated',
        operator: 'qa',
      };
    });
    const result = await importReference(importOptions(drop));
    const manifest = JSON.parse(await readFile(result.manifestPath, 'utf8')) as Record<
      string,
      unknown
    >;
    // Три поля способа съёмки копируются дословно: импорт не переписывает
    // захват и не выбирает значение по умолчанию.
    expect(manifest['reference']).toMatchObject({
      provider: 'office-desktop',
      application: 'word-desktop',
      applicationVersion: '16.0.17928',
      captureMethod: 'automated',
    });
  });

  it('отвергает office-desktop с online-приложением', async () => {
    // Пара provider/application противоречива: desktop-захват нельзя записать
    // как online, и наоборот — иначе manifest врёт о среде эталона.
    const drop = await makeDrop((capture) => {
      capture['reference'] = {
        provider: 'office-desktop',
        application: 'word-online',
        capturedAt: '2026-11-20T10:00:00Z',
        captureMethod: 'automated',
        operator: 'qa',
      };
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /generated manifest is invalid.*application "word-online" does not match provider "office-desktop"/,
    );
  });

  it('отвергает приложение другого формата', async () => {
    const drop = await makeDrop((capture) => {
      capture['reference'] = {
        provider: 'office-desktop',
        application: 'excel-desktop',
        capturedAt: '2026-11-20T10:00:00Z',
        captureMethod: 'automated',
        operator: 'qa',
      };
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /application "excel-desktop" does not match source format "docx"/,
    );
  });

  it('отвергает неизвестный provider', async () => {
    // Список значений — свойство схемы, а не импортёра: сообщение обязано
    // назвать путь поля и допустимые значения.
    const drop = await makeDrop((capture) => {
      (capture['reference'] as Record<string, unknown>)['provider'] = 'libreoffice';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /generated manifest is invalid: reference\.provider: must be equal to one of the allowed values \("office-online", "office-desktop"\)/,
    );
  });

  it('отвергает неизвестный captureMethod', async () => {
    const drop = await makeDrop((capture) => {
      (capture['reference'] as Record<string, unknown>)['captureMethod'] = 'full-auto';
    });
    await expect(importReference(importOptions(drop))).rejects.toThrow(
      /generated manifest is invalid: reference\.captureMethod: must be equal to one of the allowed values \("manual", "semi-automated", "automated"\)/,
    );
  });

  it('считает source.sha256 от реальных байтов', async () => {
    const drop = await makeDrop();
    const result = await importReference(importOptions(drop));
    const manifest = JSON.parse(await readFile(result.manifestPath, 'utf8')) as Record<
      string,
      unknown
    >;
    const expected = sha256Hex(new TextEncoder().encode('fake docx bytes'));
    expect((manifest['source'] as Record<string, unknown>)['sha256']).toBe(expected);
  });
});
