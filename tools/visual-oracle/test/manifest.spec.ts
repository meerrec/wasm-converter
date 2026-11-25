import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { sha256Hex } from '../src/hash.js';
import {
  validateManifestFile,
  validateManifestValue,
  type Manifest,
  type ManifestEnvironment,
} from '../src/manifest.js';
import { parsePngHeader } from '../src/png.js';
import { computeBaselineId } from '../src/reference/import.js';
import { readZipEntry } from '../src/zip.js';

const SCHEMA_PATH = 'docs/strict-viewer/manifest.schema.json';

const tempDirs: string[] = [];

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'visual-oracle-manifest-'));
  tempDirs.push(dir);
  return dir;
}

/** CRC-32 по таблице: нужен, чтобы собранный PNG был настоящим файлом. */
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

/** Собрать PNG с одним IHDR: пиксели валидатору не нужны. */
function buildPng(width: number, height: number, colorType: 2 | 6): Uint8Array {
  const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  const ihdr = new Uint8Array(13);
  const view = new DataView(ihdr.buffer);
  view.setUint32(0, width, false);
  view.setUint32(4, height, false);
  ihdr[8] = 8;
  ihdr[9] = colorType;
  const type = new TextEncoder().encode('IHDR');
  const body = new Uint8Array([...type, ...ihdr]);
  const crc = new Uint8Array(4);
  new DataView(crc.buffer).setUint32(0, crc32(body), false);
  const length = new Uint8Array(4);
  new DataView(length.buffer).setUint32(0, ihdr.byteLength, false);
  return new Uint8Array([...signature, ...length, ...body, ...crc]);
}

/** Собрать ZIP с одним entry без сжатия. */
function buildZip(name: string, content: string): Uint8Array {
  const nameBytes = new TextEncoder().encode(name);
  const data = new TextEncoder().encode(content);
  const crc = crc32(data);

  const local = new Uint8Array(30 + nameBytes.byteLength + data.byteLength);
  const localView = new DataView(local.buffer);
  localView.setUint32(0, 0x04034b50, true);
  localView.setUint16(4, 20, true);
  localView.setUint16(8, 0, true);
  localView.setUint32(14, crc, true);
  localView.setUint32(18, data.byteLength, true);
  localView.setUint32(22, data.byteLength, true);
  localView.setUint16(26, nameBytes.byteLength, true);
  local.set(nameBytes, 30);
  local.set(data, 30 + nameBytes.byteLength);

  const central = new Uint8Array(46 + nameBytes.byteLength);
  const centralView = new DataView(central.buffer);
  centralView.setUint32(0, 0x02014b50, true);
  centralView.setUint16(4, 20, true);
  centralView.setUint16(6, 20, true);
  centralView.setUint16(10, 0, true);
  centralView.setUint32(16, crc, true);
  centralView.setUint32(20, data.byteLength, true);
  centralView.setUint32(24, data.byteLength, true);
  centralView.setUint16(28, nameBytes.byteLength, true);
  centralView.setUint32(42, 0, true);
  central.set(nameBytes, 46);

  const eocd = new Uint8Array(22);
  const eocdView = new DataView(eocd.buffer);
  eocdView.setUint32(0, 0x06054b50, true);
  eocdView.setUint16(8, 1, true);
  eocdView.setUint16(10, 1, true);
  eocdView.setUint32(12, central.byteLength, true);
  eocdView.setUint32(16, local.byteLength, true);

  return new Uint8Array([...local, ...central, ...eocd]);
}

/** Среда экранного режима: viewport, DPR и colorScheme обязательны. */
function screenEnvironment(): ManifestEnvironment {
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

/** Собранный эталонный захват: файлы на диске плюс manifest в памяти. */
interface Fixture {
  fixturesRoot: string;
  storageRoot: string;
  manifestPath: string;
  manifest: Manifest;
}

/**
 * Разложить DOCX-захват на диске и вернуть manifest, который ему соответствует.
 *
 * `mutate` правит manifest после сборки — так тесты ломают ровно одно поле.
 */
async function docxFixture(
  mutate: (manifest: Manifest) => void = () => {},
  options: { mode?: 'print-layout' | 'pdf'; pages?: number[] } = {},
): Promise<Fixture> {
  const root = await tempDir();
  const fixturesRoot = join(root, 'test-fixtures');
  const storageRoot = join(root, 'test-fixtures', 'strict-viewer');
  const mode = options.mode ?? 'print-layout';
  const pages = options.pages ?? [1];

  const sourcePath = join(fixturesRoot, 'docx', 'tables', 'long_table.docx');
  await mkdir(join(sourcePath, '..'), { recursive: true });
  const sourceBytes = new TextEncoder().encode('fake docx bytes');
  await writeFile(sourcePath, sourceBytes);

  const modeDir = join(storageRoot, 'docx', 'docx-tables-long-table', mode);
  await mkdir(join(modeDir, 'reference'), { recursive: true });

  const captures = [];
  for (const page of pages) {
    const png = buildPng(100, 200, 6);
    const relative = `reference/page-${page}.png`;
    await writeFile(join(modeDir, relative), png);
    captures.push({
      id: `page-${page}`,
      kind: 'docx-page' as const,
      pageNumber: page,
      image: {
        uri: `docx/docx-tables-long-table/${mode}/${relative}`,
        sha256: sha256Hex(png),
        width: 100,
        height: 200,
        bytes: png.byteLength,
        colorType: 'RGBA8' as const,
      },
    });
  }

  const manifest: Manifest = {
    schemaVersion: '1.1.0',
    corpus: {
      id: 'strict-viewer-1.0.0',
      version: '1.0.0',
      matrixVersion: '1.0.0',
      baselineId: 'strict-viewer-1.0.0-1.0.0-00000000-000000000000',
      storage: { kind: 'git-lfs', rootUri: 'test-fixtures/strict-viewer' },
    },
    source: {
      path: 'docx/tables/long_table.docx',
      format: 'docx',
      sha256: sha256Hex(sourceBytes),
      bytes: sourceBytes.byteLength,
    },
    reference: {
      provider: 'office-online',
      application: 'word-online',
      capturedAt: '2026-11-20T10:00:00Z',
      captureMethod: 'manual',
      operator: 'qa',
    },
    environment: screenEnvironment(),
    view: { format: 'docx', mode, revisions: 'accepted', comments: 'hidden' },
    dynamicData: {
      dates: 'fixed-value',
      fields: 'cached-result',
      formulaCache: { policy: 'cached-values-only', dirty: false, fullCalcOnLoad: false, forceFullCalc: false },
      externalLinks: 'cached-values-no-network',
    },
    comparisonProfile: 'visual-v1',
    captures,
  };

  if (mode === 'pdf') {
    const pdf = new TextEncoder().encode('%PDF-1.7 fake');
    await writeFile(join(modeDir, 'reference', 'export.pdf'), pdf);
    manifest.reference.exportedPdf = {
      uri: 'docx/docx-tables-long-table/pdf/reference/export.pdf',
      sha256: sha256Hex(pdf),
    };
    manifest.environment = {
      ...screenEnvironment(),
      rasterizer: { name: 'pdftoppm', version: '24.02.0', dpi: 144 },
    };
    delete manifest.environment.viewportCss;
    delete manifest.environment.deviceScaleFactor;
    delete manifest.environment.colorScheme;
  }

  mutate(manifest);
  const manifestPath = join(modeDir, 'manifest.json');
  return { fixturesRoot, storageRoot, manifestPath, manifest };
}

async function validate(fixture: Fixture) {
  return validateManifestValue(fixture.manifest, fixture.manifestPath, {
    fixturesRoot: fixture.fixturesRoot,
    storageRoot: fixture.storageRoot,
    schemaPath: SCHEMA_PATH,
  });
}

describe('parsePngHeader', () => {
  it('читает размеры и тип цвета из IHDR', () => {
    expect(parsePngHeader(buildPng(640, 480, 6))).toMatchObject({
      width: 640,
      height: 480,
      bitDepth: 8,
      colorType: 6,
      colorTypeName: 'RGBA8',
    });
    expect(parsePngHeader(buildPng(10, 20, 2))?.colorTypeName).toBe('RGB8');
  });

  it('отвергает не-PNG и обрезанный файл', () => {
    expect(parsePngHeader(new TextEncoder().encode('not a png at all, really not'))).toBeNull();
    expect(parsePngHeader(buildPng(1, 1, 6).subarray(0, 20))).toBeNull();
  });

  it('не признаёт 16-битный цвет типом manifest', () => {
    const png = buildPng(1, 1, 6);
    png[24] = 16;
    expect(parsePngHeader(png)?.colorTypeName).toBeNull();
  });
});

describe('readZipEntry', () => {
  it('достаёт entry по имени', () => {
    const zip = buildZip('xl/workbook.xml', '<workbook/>');
    expect(new TextDecoder().decode(readZipEntry(zip, 'xl/workbook.xml') ?? new Uint8Array())).toBe(
      '<workbook/>',
    );
    expect(readZipEntry(zip, 'xl/missing.xml')).toBeNull();
  });
});

describe('validateManifestValue', () => {
  it('принимает корректный print-layout manifest', async () => {
    const fixture = await docxFixture();
    const result = await validate(fixture);
    expect(result.ok).toBe(true);
  });

  it('принимает корректный pdf manifest', async () => {
    const fixture = await docxFixture(() => {}, { mode: 'pdf' });
    const result = await validate(fixture);
    expect(result.ok).toBe(true);
  });

  it('отвергает неизвестное поле схемы', async () => {
    const fixture = await docxFixture((manifest) => {
      (manifest as unknown as Record<string, unknown>)['unexpectedField'] = true;
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /additional properties/.test(error.message))).toBe(true);
  });

  it('отвергает недопустимый colorType', async () => {
    const fixture = await docxFixture((manifest) => {
      const capture = manifest.captures[0];
      if (capture !== undefined) {
        (capture.image as unknown as Record<string, unknown>)['colorType'] = 'RGB16';
      }
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => error.path === 'captures[0].image.colorType')).toBe(true);
  });

  it('требует, чтобы image.uri лежал под reference/', async () => {
    const fixture = await docxFixture((manifest) => {
      const capture = manifest.captures[0];
      if (capture !== undefined) {
        capture.image.uri = 'docx/docx-tables-long-table/print-layout/actual/page-1.png';
      }
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => error.path === 'captures[0].image.uri')).toBe(true);
  });

  it('требует, чтобы exportedPdf.uri назывался reference/export.pdf', async () => {
    const fixture = await docxFixture((manifest) => {
      const exported = manifest.reference.exportedPdf;
      if (exported !== undefined) {
        exported.uri = 'docx/docx-tables-long-table/pdf/reference/page-1.pdf';
      }
    }, { mode: 'pdf' });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(
      result.errors.some(
        (error) =>
          error.path === 'reference.exportedPdf.uri' && /reference\/export\.pdf/.test(error.message),
      ),
    ).toBe(true);
  });

  it('ловит несовпадение SHA-256 изображения', async () => {
    const fixture = await docxFixture((manifest) => {
      const capture = manifest.captures[0];
      if (capture !== undefined) capture.image.sha256 = 'b'.repeat(64);
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => error.path === 'captures[0].image.sha256')).toBe(true);
  });

  it('ловит несовпадение размеров PNG', async () => {
    const fixture = await docxFixture((manifest) => {
      const capture = manifest.captures[0];
      if (capture !== undefined) capture.image.width = 101;
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => error.path === 'captures[0].image.width')).toBe(true);
  });

  it('требует непрерывные номера страниц DOCX', async () => {
    const fixture = await docxFixture(() => {}, { pages: [1, 3] });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /continuous from 1/.test(error.message))).toBe(true);
  });

  it('отвергает дубли capture id', async () => {
    const fixture = await docxFixture((manifest) => {
      const second = manifest.captures[1];
      if (second !== undefined) second.id = manifest.captures[0]?.id ?? 'page-1';
    }, { pages: [1, 2] });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /duplicate capture id/.test(error.message))).toBe(true);
  });

  it('отвергает маску во всё изображение', async () => {
    const fixture = await docxFixture((manifest) => {
      const capture = manifest.captures[0];
      if (capture !== undefined) {
        capture.masks = [
          {
            id: 'full',
            reason: 'dynamic-date',
            rect: { x: 0, y: 0, width: 100, height: 200 },
            textRule: 'date is fixed',
          },
        ];
      }
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /whole image/.test(error.message))).toBe(true);
  });

  it('отвергает маску за границей изображения', async () => {
    const fixture = await docxFixture((manifest) => {
      const capture = manifest.captures[0];
      if (capture !== undefined) {
        capture.masks = [
          {
            id: 'outside',
            reason: 'dynamic-date',
            rect: { x: 90, y: 0, width: 50, height: 10 },
            textRule: 'date is fixed',
          },
        ];
      }
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /exceeds image/.test(error.message))).toBe(true);
  });

  it('требует rasterizer и запрещает viewport в docx.pdf', async () => {
    const fixture = await docxFixture((manifest) => {
      delete manifest.environment.rasterizer;
      manifest.environment.viewportCss = { width: 1440, height: 900 };
    }, { mode: 'pdf' });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    const paths = result.errors.map((error) => error.path);
    expect(paths).toContain('environment.rasterizer');
    expect(paths).toContain('environment.viewportCss');
  });

  it('требует viewport и запрещает rasterizer вне docx.pdf', async () => {
    const fixture = await docxFixture((manifest) => {
      delete manifest.environment.viewportCss;
      manifest.environment.rasterizer = { name: 'pdftoppm', version: '24.02.0', dpi: 144 };
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    const paths = result.errors.map((error) => error.path);
    expect(paths).toContain('environment.viewportCss');
    expect(paths).toContain('environment.rasterizer');
  });

  it('принимает desktop Word-захват', async () => {
    const fixture = await docxFixture((manifest) => {
      manifest.reference = {
        provider: 'office-desktop',
        application: 'word-desktop',
        applicationVersion: '16.0.17928',
        capturedAt: '2026-11-20T10:00:00Z',
        captureMethod: 'automated',
        operator: 'qa',
      };
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(true);
  });

  it('отвергает office-desktop с online-приложением', async () => {
    const fixture = await docxFixture((manifest) => {
      manifest.reference.provider = 'office-desktop';
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors).toContainEqual({
      path: 'reference.application',
      message:
        'application "word-online" does not match provider "office-desktop": office-online requires a *-online application, office-desktop requires a *-desktop application',
    });
  });

  it('отвергает office-online с desktop-приложением', async () => {
    const fixture = await docxFixture((manifest) => {
      manifest.reference.application = 'word-desktop';
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(
      result.errors.some((error) => /does not match provider "office-online"/.test(error.message)),
    ).toBe(true);
  });

  it('отвергает provider вне enum схемы', async () => {
    const fixture = await docxFixture((manifest) => {
      (manifest.reference as unknown as Record<string, unknown>)['provider'] = 'libreoffice';
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => error.path === 'reference.provider')).toBe(true);
  });

  it('принимает полностью автоматический захват локального Word', async () => {
    // COM-автоматизация: провайдер и приложение — desktop-семейство, шаги
    // сделал скрипт. Схема обязана пропустить это без «переводов» значений.
    const fixture = await docxFixture((manifest) => {
      manifest.reference = {
        provider: 'office-desktop',
        application: 'word-desktop',
        applicationVersion: '16.0.17928',
        capturedAt: '2026-11-20T10:00:00Z',
        captureMethod: 'automated',
        operator: 'qa',
      };
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(true);
  });

  it('отвергает captureMethod вне enum схемы с путём поля', async () => {
    const fixture = await docxFixture((manifest) => {
      (manifest.reference as unknown as Record<string, unknown>)['captureMethod'] = 'full-auto';
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors).toContainEqual({
      path: 'reference.captureMethod',
      message:
        'must be equal to one of the allowed values ("manual", "semi-automated", "automated")',
    });
  });

  it('принимает пару office-online с online-приложением', async () => {
    const fixture = await docxFixture();
    const result = await validate(fixture);
    expect(result.ok).toBe(true);
  });

  it('требует путь manifest с режимом в конце', async () => {
    const fixture = await docxFixture();
    const result = await validateManifestValue(fixture.manifest, join(fixture.storageRoot, 'manifest.json'), {
      fixturesRoot: fixture.fixturesRoot,
      storageRoot: fixture.storageRoot,
      schemaPath: SCHEMA_PATH,
    });
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /must end with/.test(error.message))).toBe(true);
  });

  it('ловит отсутствующий исходный документ', async () => {
    const fixture = await docxFixture((manifest) => {
      manifest.source.path = 'docx/tables/missing.docx';
    });
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => error.path === 'source.path')).toBe(true);
  });

  it('не бросает исключение на мусор вместо manifest', async () => {
    const result = await validateManifestValue('not an object', 'x/print-layout/manifest.json', {
      schemaPath: SCHEMA_PATH,
    });
    expect(result.ok).toBe(false);
  });

  it('читает manifest с диска', async () => {
    const fixture = await docxFixture();
    await writeFile(fixture.manifestPath, JSON.stringify(fixture.manifest), 'utf8');
    const result = await validateManifestFile(fixture.manifestPath, {
      fixturesRoot: fixture.fixturesRoot,
      storageRoot: fixture.storageRoot,
      schemaPath: SCHEMA_PATH,
    });
    expect(result.ok).toBe(true);
  });
});

describe('validateManifestValue: XLSX sheets', () => {
  /** Разложить XLSX-захват с книгой из двух листов. */
  async function xlsxFixture(hiddenSecond: boolean): Promise<Fixture> {
    const root = await tempDir();
    const fixturesRoot = join(root, 'test-fixtures');
    const storageRoot = join(root, 'test-fixtures', 'strict-viewer');
    const sourcePath = join(fixturesRoot, 'xlsx', 'sheets-three.xlsx');
    await mkdir(join(sourcePath, '..'), { recursive: true });
    const workbook = `<workbook><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Extra" sheetId="2"${
      hiddenSecond ? ' state="hidden"' : ''
    } r:id="rId2"/></sheets></workbook>`;
    const sourceBytes = buildZip('xl/workbook.xml', workbook);
    await writeFile(sourcePath, sourceBytes);

    const modeDir = join(storageRoot, 'xlsx', 'xlsx-sheets-three', 'normal');
    await mkdir(join(modeDir, 'reference'), { recursive: true });
    const png = buildPng(300, 200, 2);
    await writeFile(join(modeDir, 'reference', 'screen.png'), png);

    const manifest: Manifest = {
      schemaVersion: '1.1.0',
      corpus: {
        id: 'strict-viewer-1.0.0',
        version: '1.0.0',
        matrixVersion: '1.0.0',
        baselineId: 'strict-viewer-1.0.0-1.0.0-00000000-000000000000',
        storage: { kind: 'git-lfs', rootUri: 'test-fixtures/strict-viewer' },
      },
      source: {
        path: 'xlsx/sheets-three.xlsx',
        format: 'xlsx',
        sha256: sha256Hex(sourceBytes),
        bytes: sourceBytes.byteLength,
      },
      reference: {
        provider: 'office-online',
        application: 'excel-online',
        capturedAt: '2026-11-20T10:00:00Z',
        captureMethod: 'manual',
        operator: 'qa',
      },
      environment: screenEnvironment(),
      view: {
        format: 'xlsx',
        mode: 'normal',
        activeSheet: { index: 0, name: 'Data' },
        hiddenSheets: 'exclude',
        showGridLines: true,
        showHeadings: true,
      },
      dynamicData: {
        dates: 'fixed-value',
        fields: 'cached-result',
        formulaCache: { policy: 'cached-values-only', dirty: false, fullCalcOnLoad: false, forceFullCalc: false },
        externalLinks: 'cached-values-no-network',
      },
      comparisonProfile: 'visual-v1',
      captures: [
        {
          id: 'screen-1',
          kind: 'xlsx-screen',
          sheet: { index: 0, name: 'Data' },
          visibleRange: 'A1:H20',
          scroll: { x: 0, y: 0 },
          activeCell: 'A1',
          image: {
            uri: 'xlsx/xlsx-sheets-three/normal/reference/screen.png',
            sha256: sha256Hex(png),
            width: 300,
            height: 200,
            bytes: png.byteLength,
            colorType: 'RGB8',
          },
        },
      ],
    };
    return { fixturesRoot, storageRoot, manifestPath: join(modeDir, 'manifest.json'), manifest };
  }

  it('принимает видимый лист', async () => {
    const fixture = await xlsxFixture(false);
    const result = await validate(fixture);
    expect(result.ok).toBe(true);
  });

  it('принимает desktop Excel-захват', async () => {
    const fixture = await xlsxFixture(false);
    fixture.manifest.reference = {
      provider: 'office-desktop',
      application: 'excel-desktop',
      applicationVersion: '16.0.17928',
      capturedAt: '2026-11-20T10:00:00Z',
      captureMethod: 'automated',
      operator: 'qa',
    };
    const result = await validate(fixture);
    expect(result.ok).toBe(true);
  });

  it('отвергает Word-приложение для XLSX', async () => {
    const fixture = await xlsxFixture(false);
    fixture.manifest.reference = {
      provider: 'office-desktop',
      application: 'word-desktop',
      capturedAt: '2026-11-20T10:00:00Z',
      captureMethod: 'automated',
      operator: 'qa',
    };
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(
      result.errors.some((error) =>
        /application "word-desktop" does not match source format "xlsx"/.test(error.message),
      ),
    ).toBe(true);
  });

  it('отвергает захват скрытого листа', async () => {
    const fixture = await xlsxFixture(true);
    fixture.manifest.view = {
      format: 'xlsx',
      mode: 'normal',
      activeSheet: { index: 1, name: 'Extra' },
      hiddenSheets: 'exclude',
      showGridLines: true,
      showHeadings: true,
    };
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /hidden sheets must not be captured/.test(error.message))).toBe(
      true,
    );
  });

  it('требует непрерывные печатные страницы в print-preview', async () => {
    const fixture = await xlsxFixture(false);
    const modeDir = join(fixture.storageRoot, 'xlsx', 'xlsx-sheets-three', 'print-preview');
    await mkdir(join(modeDir, 'reference'), { recursive: true });
    const pages = [1, 3];
    const captures = [];
    for (const page of pages) {
      const png = buildPng(300, 200, 2);
      await writeFile(join(modeDir, 'reference', `page-${page}.png`), png);
      captures.push({
        id: `page-${page}`,
        kind: 'xlsx-print-page' as const,
        pageNumber: page,
        sheet: { index: 0, name: 'Data' },
        image: {
          uri: `xlsx/xlsx-sheets-three/print-preview/reference/page-${page}.png`,
          sha256: sha256Hex(png),
          width: 300,
          height: 200,
          bytes: png.byteLength,
          colorType: 'RGB8' as const,
        },
      });
    }
    fixture.manifest.view = {
      format: 'xlsx',
      mode: 'print-preview',
      activeSheet: { index: 0, name: 'Data' },
      hiddenSheets: 'exclude',
      showGridLines: true,
      showHeadings: true,
    };
    fixture.manifest.captures = captures;
    const result = await validateManifestValue(
      fixture.manifest,
      join(modeDir, 'manifest.json'),
      {
        fixturesRoot: fixture.fixturesRoot,
        storageRoot: fixture.storageRoot,
        schemaPath: SCHEMA_PATH,
      },
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(
      result.errors.some((error) => /xlsx print-preview page numbers must be continuous/.test(error.message)),
    ).toBe(true);
  });

  it('отвергает несуществующий лист', async () => {
    const fixture = await xlsxFixture(false);
    fixture.manifest.view = {
      format: 'xlsx',
      mode: 'normal',
      activeSheet: { index: 5, name: 'Nope' },
      hiddenSheets: 'exclude',
      showGridLines: true,
      showHeadings: true,
    };
    const result = await validate(fixture);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /out of range/.test(error.message))).toBe(true);
  });
});

describe('computeBaselineId', () => {
  const environment = screenEnvironment();

  function input(imageSha256: string) {
    return {
      corpusVersion: '1.0.0',
      matrixVersion: '1.0.0',
      environment,
      sourcePath: 'docx/tables/long_table.docx',
      sourceSha256: 'c'.repeat(64),
      mode: 'print-layout' as const,
      captures: [
        {
          id: 'page-1',
          image: {
            uri: 'docx/docx-tables-long-table/print-layout/reference/page-1.png',
            sha256: imageSha256,
            width: 100,
            height: 200,
            bytes: 10,
            colorType: 'RGBA8' as const,
          },
        },
      ],
    };
  }

  it('стабилен для одинакового входа', () => {
    expect(computeBaselineId(input('d'.repeat(64)))).toBe(computeBaselineId(input('d'.repeat(64))));
  });

  it('меняется при другом хеше изображения', () => {
    expect(computeBaselineId(input('d'.repeat(64)))).not.toBe(computeBaselineId(input('e'.repeat(64))));
  });

  it('не зависит от порядка ключей среды', () => {
    const reordered = {
      fonts: environment.fonts,
      timezone: environment.timezone,
      locale: environment.locale,
      colorScheme: environment.colorScheme,
      browserZoomPercent: environment.browserZoomPercent,
      applicationZoomPercent: environment.applicationZoomPercent,
      deviceScaleFactor: environment.deviceScaleFactor,
      viewportCss: environment.viewportCss,
      browser: environment.browser,
      os: environment.os,
    } as ManifestEnvironment;
    const base = computeBaselineId(input('d'.repeat(64)));
    const other = computeBaselineId({ ...input('d'.repeat(64)), environment: reordered });
    expect(other).toBe(base);
  });

  it('имеет форму strict-viewer-<corpus>-<matrix>-<env>-<digest>', () => {
    expect(computeBaselineId(input('d'.repeat(64)))).toMatch(
      /^strict-viewer-1\.0\.0-1\.0\.0-[0-9a-f]{8}-[0-9a-f]{12}$/,
    );
  });
});
