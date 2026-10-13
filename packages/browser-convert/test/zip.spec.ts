/**
 * Тесты zip-механики. Архивы строятся руками прямо в тестах: local header'ы + данные
 * (+ data descriptor для флага 3) + central directory + EOCD. Это даёт контроль над
 * каждым байтом — нули в local header, чужой метод сжатия, комментарий EOCD с ложной
 * сигнатурой внутри — которого не дала бы никакая библиотека; их здесь и нет.
 */
import { describe, expect, it } from 'vitest';

import { listZipEntries, readZipEntry, rewriteZip, zipHasEntry } from '../src/index.js';

const LOCAL_SIGNATURE = 0x04034b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const EOCD_SIGNATURE = 0x06054b50;
const DESCRIPTOR_SIGNATURE = 0x08074b50;
const METHOD_STORED = 0;
const METHOD_DEFLATE = 8;
const FLAG_DESCRIPTOR = 0x0008;

const utf8 = new TextEncoder();

/** Таблица CRC32 — копия алгоритма из src/zip.ts; её сходимость проверяется известным вектором ниже. */
const CRC_TABLE = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let value = i;
  for (let bit = 0; bit < 8; bit++) {
    value = (value & 1) === 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  }
  CRC_TABLE[i] = value >>> 0;
}

/** CRC32 (IEEE 802.3), как в zip. */
function crc32(data: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of data) {
    crc = CRC_TABLE[(crc ^ byte) & 0xff]! ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function bytes(text: string): Uint8Array {
  return utf8.encode(text);
}

function concat(parts: readonly Uint8Array[]): Uint8Array {
  let total = 0;
  for (const part of parts) {
    total += part.length;
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

/** Сжимает raw-deflate — как это делает запись zip с методом 8. */
async function deflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream('deflate-raw');
  const writer = stream.writable.getWriter();
  const reader = stream.readable.getReader();
  const chunks: Uint8Array[] = [];
  const collected = (async () => {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      chunks.push(value);
    }
  })();
  await writer.write(data as BufferSource);
  await writer.close();
  await collected;
  return concat(chunks);
}

/** Одна запись для ручного билдера архива. */
interface BuiltEntry {
  readonly name: string;
  /** Содержимое до сжатия. */
  readonly data: Uint8Array;
  /** Метод сжатия: 0, 8 или любой другой — чтобы проверить отказ по методу. */
  readonly method: number;
  /** Флаг 3: в local header нули, настоящие значения — в data descriptor (потоковый писатель). */
  readonly withDescriptor?: boolean;
}

interface BuildOptions {
  /** Комментарий EOCD; внутрь можно класть что угодно, включая байты сигнатуры EOCD. */
  readonly comment?: string;
}

/** Собирает zip-архив вручную: local header'ы + данные (+ descriptor'ы) + central directory + EOCD. */
async function buildZip(entries: readonly BuiltEntry[], options: BuildOptions = {}): Promise<Uint8Array> {
  const localParts: Uint8Array[] = [];
  const centralRecords: Uint8Array[] = [];
  let localBytes = 0;
  for (const entry of entries) {
    const nameBytes = utf8.encode(entry.name);
    const payload = entry.method === METHOD_DEFLATE ? await deflateRaw(entry.data) : entry.data;
    const entryCrc = crc32(entry.data);
    const withDescriptor = entry.withDescriptor === true;
    const entryOffset = localBytes;

    const header = new Uint8Array(30 + nameBytes.length);
    const headerView = new DataView(header.buffer);
    headerView.setUint32(0, LOCAL_SIGNATURE, true);
    headerView.setUint16(4, 20, true);
    headerView.setUint16(6, withDescriptor ? FLAG_DESCRIPTOR : 0, true);
    headerView.setUint16(8, entry.method, true);
    if (!withDescriptor) {
      headerView.setUint32(14, entryCrc, true);
      headerView.setUint32(18, payload.length, true);
      headerView.setUint32(22, entry.data.length, true);
    }
    headerView.setUint16(26, nameBytes.length, true);
    header.set(nameBytes, 30);

    localParts.push(header, payload);
    localBytes += header.length + payload.length;
    if (withDescriptor) {
      const descriptor = new Uint8Array(16);
      const descriptorView = new DataView(descriptor.buffer);
      descriptorView.setUint32(0, DESCRIPTOR_SIGNATURE, true);
      descriptorView.setUint32(4, entryCrc, true);
      descriptorView.setUint32(8, payload.length, true);
      descriptorView.setUint32(12, entry.data.length, true);
      localParts.push(descriptor);
      localBytes += descriptor.length;
    }

    const record = new Uint8Array(46 + nameBytes.length);
    const recordView = new DataView(record.buffer);
    recordView.setUint32(0, CENTRAL_SIGNATURE, true);
    recordView.setUint16(4, 20, true);
    recordView.setUint16(6, 20, true);
    recordView.setUint16(8, withDescriptor ? FLAG_DESCRIPTOR : 0, true);
    recordView.setUint16(10, entry.method, true);
    recordView.setUint32(16, entryCrc, true);
    recordView.setUint32(20, payload.length, true);
    recordView.setUint32(24, entry.data.length, true);
    recordView.setUint16(28, nameBytes.length, true);
    recordView.setUint32(42, entryOffset, true);
    record.set(nameBytes, 46);
    centralRecords.push(record);
  }

  const centralDirectory = concat(centralRecords);
  const commentBytes = options.comment === undefined ? new Uint8Array(0) : utf8.encode(options.comment);
  const eocd = new Uint8Array(22 + commentBytes.length);
  const eocdView = new DataView(eocd.buffer);
  eocdView.setUint32(0, EOCD_SIGNATURE, true);
  eocdView.setUint16(8, entries.length, true);
  eocdView.setUint16(10, entries.length, true);
  eocdView.setUint32(12, centralDirectory.length, true);
  eocdView.setUint32(16, localBytes, true);
  eocdView.setUint16(20, commentBytes.length, true);
  eocd.set(commentBytes, 22);
  return concat([concat(localParts), centralDirectory, eocd]);
}

/** Извлекает из архива сырой кусок записи: local header + данные (+ descriptor, если был). */
function entryRegion(archive: Uint8Array, entryOffset: number, length: number): Uint8Array {
  return archive.slice(entryOffset, entryOffset + length);
}

describe('crc32 (тестовый хелпер)', () => {
  it('совпадает с известным вектором', () => {
    expect(crc32(bytes('123456789'))).toBe(0xcbf43926);
  });
});

describe('listZipEntries', () => {
  it('читает имена, размеры, CRC и смещения из central directory', async () => {
    const longText = bytes('повторяющийся текст для сжатия. '.repeat(40));
    const archive = await buildZip([
      { name: 'dir/', data: new Uint8Array(0), method: METHOD_STORED },
      { name: 'hello.txt', data: bytes('hello'), method: METHOD_STORED },
      { name: 'text.bin', data: longText, method: METHOD_DEFLATE },
    ]);
    const entries = listZipEntries(archive);
    expect(entries.map((entry) => entry.name)).toEqual(['dir/', 'hello.txt', 'text.bin']);
    expect(entries[0]?.isDir).toBe(true);
    expect(entries[1]?.isDir).toBe(false);

    const hello = entries[1]!;
    expect(hello.method).toBe(METHOD_STORED);
    expect(hello.size).toBe(5);
    expect(hello.compressedSize).toBe(5);
    expect(hello.crc32).toBe(crc32(bytes('hello')));
    expect(new DataView(archive.buffer).getUint32(hello.localOffset, true)).toBe(LOCAL_SIGNATURE);

    const text = entries[2]!;
    expect(text.method).toBe(METHOD_DEFLATE);
    expect(text.size).toBe(longText.length);
    expect(text.crc32).toBe(crc32(longText));
    expect(text.compressedSize).toBeLessThan(text.size);
  });

  it('декодирует имена как UTF-8', async () => {
    const archive = await buildZip([{ name: 'документ/содержимое.xml', data: bytes('<root/>'), method: METHOD_STORED }]);
    expect(listZipEntries(archive)[0]!.name).toBe('документ/содержимое.xml');
  });
});

describe('readZipEntry', () => {
  it('читает stored-запись без распаковки', async () => {
    const archive = await buildZip([{ name: 'hello.txt', data: bytes('hello'), method: METHOD_STORED }]);
    expect(await readZipEntry(archive, 'hello.txt')).toEqual(bytes('hello'));
  });

  it('разжимает deflate-запись', async () => {
    const payload = bytes('дефлейт-данные. '.repeat(200));
    const archive = await buildZip([{ name: 'data.bin', data: payload, method: METHOD_DEFLATE }]);
    const entry = listZipEntries(archive)[0]!;
    expect(await readZipEntry(archive, entry)).toEqual(payload);
  });

  it('возвращает null для неподдерживаемого метода сжатия', async () => {
    const archive = await buildZip([{ name: 'data.bin', data: bytes('bzip2 сюда не положим'), method: 12 }]);
    const entry = listZipEntries(archive)[0]!;
    expect(entry.method).toBe(12);
    expect(await readZipEntry(archive, entry)).toBeNull();
  });

  it('возвращает null для отсутствующей записи', async () => {
    const archive = await buildZip([{ name: 'a.txt', data: bytes('a'), method: METHOD_STORED }]);
    expect(await readZipEntry(archive, 'nope.txt')).toBeNull();
  });
});

describe('data descriptor (флаг 3)', () => {
  it('читает запись, у которой в local header нули, а значения в data descriptor', async () => {
    const payload = bytes('потоковые данные. '.repeat(100));
    const archive = await buildZip([
      { name: 'stream.txt', data: payload, method: METHOD_DEFLATE, withDescriptor: true },
    ]);
    const entry = listZipEntries(archive)[0]!;
    const view = new DataView(archive.buffer);
    // Потоковый писатель оставил в local header нули…
    expect(view.getUint32(entry.localOffset + 14, true)).toBe(0);
    expect(view.getUint32(entry.localOffset + 18, true)).toBe(0);
    expect(view.getUint32(entry.localOffset + 22, true)).toBe(0);
    // …а central directory заполнена — и читать надо именно её.
    expect(entry.size).toBe(payload.length);
    expect(entry.crc32).toBe(crc32(payload));
    expect(await readZipEntry(archive, entry)).toEqual(payload);
  });
});

describe('структурная порча', () => {
  it('бросает на не-zip данных', () => {
    expect(() => listZipEntries(new Uint8Array(64))).toThrow(/not a ZIP/);
    expect(() => listZipEntries(bytes('это не архив'))).toThrow(/not a ZIP/);
  });

  it('бросает на zip64-маркере в EOCD', async () => {
    const archive = await buildZip([{ name: 'a.txt', data: bytes('a'), method: METHOD_STORED }]);
    const tampered = archive.slice();
    const eocdOffset = tampered.length - 22;
    new DataView(tampered.buffer).setUint32(eocdOffset + 12, 0xffffffff, true);
    expect(() => listZipEntries(tampered)).toThrow(/zip64/);
  });

  it('бросает на битой сигнатуре central directory', async () => {
    const archive = await buildZip([{ name: 'a.txt', data: bytes('a'), method: METHOD_STORED }]);
    const tampered = archive.slice();
    const eocdOffset = tampered.length - 22;
    const cdOffset = new DataView(tampered.buffer).getUint32(eocdOffset + 16, true);
    tampered[cdOffset] = 0;
    expect(() => listZipEntries(tampered)).toThrow(/central directory signature/);
  });
});

describe('EOCD с комментарием', () => {
  it('ищет EOCD с конца, даже если в комментарии лежит ложная сигнатура', async () => {
    const archive = await buildZip(
      [{ name: 'a.txt', data: bytes('a'), method: METHOD_STORED }],
      { comment: 'комментарий PK\u0005\u0006 со встроенной сигнатурой' },
    );
    expect(listZipEntries(archive).map((entry) => entry.name)).toEqual(['a.txt']);
  });
});

describe('zipHasEntry', () => {
  it('находит записи по имени', async () => {
    const archive = await buildZip([
      { name: 'word/document.xml', data: bytes('<w:document/>'), method: METHOD_STORED },
    ]);
    expect(zipHasEntry(archive, 'word/document.xml')).toBe(true);
    expect(zipHasEntry(archive, 'word/styles.xml')).toBe(false);
  });
});

describe('rewriteZip', () => {
  const SAMPLE_ENTRIES = [
    { name: 'keep.txt', data: bytes('keep me verbatim'), method: METHOD_STORED },
    { name: 'change.txt', data: bytes('ORIGINAL'), method: METHOD_STORED },
    { name: 'other.bin', data: bytes('other entry. '.repeat(500)), method: METHOD_DEFLATE },
  ] as const;

  async function buildSample(): Promise<Uint8Array> {
    return buildZip(SAMPLE_ENTRIES);
  }

  /** Длина сырого куска записи в исходном архиве: local header + данные (+ descriptor). */
  function regionLength(name: string, compressedSize: number): number {
    return 30 + utf8.encode(name).length + compressedSize;
  }

  it('transform меняет запись, остальные копируются байт-в-байт', async () => {
    const archive = await buildSample();
    const before = listZipEntries(archive);
    const rewritten = await rewriteZip(archive, {
      filter: (entry) => entry.name === 'change.txt',
      transform: () => 'REPLACED',
    });
    const after = listZipEntries(rewritten);
    expect(after.map((entry) => entry.name)).toEqual(['keep.txt', 'change.txt', 'other.bin']);

    // verbatim: сырые куски keep.txt и other.bin не изменились ни на байт.
    for (const name of ['keep.txt', 'other.bin']) {
      const oldEntry = before.find((entry) => entry.name === name)!;
      const newEntry = after.find((entry) => entry.name === name)!;
      const length = regionLength(name, oldEntry.compressedSize);
      expect(entryRegion(rewritten, newEntry.localOffset, length)).toEqual(entryRegion(archive, oldEntry.localOffset, length));
    }

    // изменённая запись пережата deflate, размеры и CRC сходятся.
    const changed = after[1]!;
    expect(changed.method).toBe(METHOD_DEFLATE);
    expect(changed.size).toBe(bytes('REPLACED').length);
    expect(changed.crc32).toBe(crc32(bytes('REPLACED')));
    expect(await readZipEntry(rewritten, 'change.txt')).toEqual(bytes('REPLACED'));
  });

  it('transform, вернувший null, оставляет запись verbatim', async () => {
    const archive = await buildSample();
    const before = listZipEntries(archive);
    const rewritten = await rewriteZip(archive, {
      filter: (entry) => entry.name === 'change.txt',
      transform: () => null,
    });
    const oldEntry = before.find((entry) => entry.name === 'change.txt')!;
    const newEntry = listZipEntries(rewritten).find((entry) => entry.name === 'change.txt')!;
    const length = regionLength('change.txt', oldEntry.compressedSize);
    expect(entryRegion(rewritten, newEntry.localOffset, length)).toEqual(entryRegion(archive, oldEntry.localOffset, length));
    expect(await readZipEntry(rewritten, 'change.txt')).toEqual(bytes('ORIGINAL'));
  });

  it('inject добавляет записи (deflate)', async () => {
    const archive = await buildZip([{ name: 'a.txt', data: bytes('a'), method: METHOD_STORED }]);
    const injected = bytes('injected entry. '.repeat(200));
    const rewritten = await rewriteZip(archive, { inject: [{ name: 'new/entry.txt', data: injected }] });
    const entries = listZipEntries(rewritten);
    expect(entries.map((entry) => entry.name)).toEqual(['a.txt', 'new/entry.txt']);
    const added = entries[1]!;
    expect(added.method).toBe(METHOD_DEFLATE);
    expect(await readZipEntry(rewritten, added)).toEqual(injected);
    expect(await readZipEntry(rewritten, 'new/entry.txt')).toEqual(injected);
  });

  it('verbatim-копия записи с флагом 3 сохраняет data descriptor', async () => {
    const payload = bytes('streamed. '.repeat(100));
    const archive = await buildZip([
      { name: 'stream.txt', data: payload, method: METHOD_DEFLATE, withDescriptor: true },
      { name: 'x.txt', data: bytes('x'), method: METHOD_STORED },
    ]);
    const before = listZipEntries(archive);
    const oldStream = before[0]!;
    // header (30) + имя + сжатые данные + descriptor с подписью (16).
    const length = 30 + utf8.encode('stream.txt').length + oldStream.compressedSize + 16;
    const originalRegion = entryRegion(archive, oldStream.localOffset, length);

    const rewritten = await rewriteZip(archive, {
      filter: (entry) => entry.name === 'x.txt',
      transform: () => 'y',
    });
    const newStream = listZipEntries(rewritten)[0]!;
    expect(entryRegion(rewritten, newStream.localOffset, length)).toEqual(originalRegion);
    expect(await readZipEntry(rewritten, 'stream.txt')).toEqual(payload);
  });

  it('самопроверка: пересобранный архив читается, данные и CRC сходятся', async () => {
    const archive = await buildZip([
      { name: 'one.txt', data: bytes('one'), method: METHOD_STORED },
      { name: 'two.txt', data: bytes('two '.repeat(100)), method: METHOD_DEFLATE },
    ]);
    const rewritten = await rewriteZip(archive, {
      filter: (entry) => entry.name === 'two.txt',
      transform: (data) => data, // содержимое то же, но запись пережимается заново
      inject: [{ name: 'three.txt', data: bytes('three') }],
    });
    const entries = listZipEntries(rewritten);
    const byName = new Map(entries.map((entry) => [entry.name, entry] as const));
    expect(await readZipEntry(rewritten, byName.get('one.txt')!)).toEqual(bytes('one'));
    expect(await readZipEntry(rewritten, byName.get('two.txt')!)).toEqual(bytes('two '.repeat(100)));
    expect(await readZipEntry(rewritten, 'three.txt')).toEqual(bytes('three'));
    for (const entry of entries) {
      const data = await readZipEntry(rewritten, entry);
      expect(data).not.toBeNull();
      expect(crc32(data!)).toBe(entry.crc32);
    }
  });

  it('сохраняет комментарий EOCD и пересобирает пустой архив', async () => {
    const comment = 'архив с комментарием';
    const archive = await buildZip([], { comment });
    expect(listZipEntries(archive)).toEqual([]);
    const rewritten = await rewriteZip(archive, { inject: [{ name: 'only.txt', data: bytes('only') }] });
    expect(listZipEntries(rewritten).map((entry) => entry.name)).toEqual(['only.txt']);
    expect(await readZipEntry(rewritten, 'only.txt')).toEqual(bytes('only'));
    const commentBytes = utf8.encode(comment);
    expect(rewritten.slice(-commentBytes.length)).toEqual(commentBytes);
  });
});
