/**
 * Тесты медиа-экстракции на реальных закоммиченных фикстурах
 * test-fixtures/docx/images/*.docx (anchor_wrap_tight — 1 рисунок,
 * multiple_sizes — 3 рисунка). URL.createObjectURL/revokeObjectURL мокаются:
 * так видно каждый Blob, и тест не создаёт настоящих object URLs.
 */
import { readFileSync } from 'node:fs';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { extractDocxMediaUrls } from '../src/media.js';
import { listZipEntries, readZipEntry, rewriteZip } from '../src/zip.js';

const FIXTURES_ROOT = new URL('../../../test-fixtures/docx/images/', import.meta.url);

function readFixture(name: string): Uint8Array {
  return new Uint8Array(readFileSync(new URL(name, FIXTURES_ROOT)));
}

const utf8 = new TextEncoder();
function bytes(text: string): Uint8Array {
  return utf8.encode(text);
}

let capturedBlobs: Blob[];
let revokeCalls: string[];

beforeEach(() => {
  capturedBlobs = [];
  revokeCalls = [];
  vi.spyOn(URL, 'createObjectURL').mockImplementation((object: Blob | MediaSource) => {
    const blob = object as Blob;
    capturedBlobs.push(blob);
    return `blob:mock:${capturedBlobs.length}`;
  });
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation((url: string) => {
    revokeCalls.push(url);
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('extractDocxMediaUrls', () => {
  it('извлекает единственный рисунок из anchor_wrap_tight.docx', async () => {
    const archive = readFixture('anchor_wrap_tight.docx');
    const urls = await extractDocxMediaUrls(archive);

    expect(Object.keys(urls)).toEqual(['media/tight.png']);
    const expected = await readZipEntry(archive, 'word/media/tight.png');

    const blob = capturedBlobs[0]!;
    expect(blob.type).toBe('image/png');
    expect(blob.size).toBe(expected!.length);
    expect(new Uint8Array(await blob.arrayBuffer())).toEqual(expected);
    expect(urls['media/tight.png']).toBe('blob:mock:1');
    // Ревокать обязан вызывающий — экстрактор URL не отзывает.
    expect(revokeCalls).toEqual([]);
  });

  it('извлекает все три рисунка из multiple_sizes.docx', async () => {
    const archive = readFixture('multiple_sizes.docx');
    const urls = await extractDocxMediaUrls(archive);

    const mediaEntries = listZipEntries(archive).filter(
      (entry) => entry.name.startsWith('word/media/') && !entry.isDir,
    );
    expect(mediaEntries.map((entry) => entry.name)).toEqual([
      'word/media/small.png',
      'word/media/wide.png',
      'word/media/tall.png',
    ]);
    expect(Object.keys(urls)).toEqual(['media/small.png', 'media/wide.png', 'media/tall.png']);

    // Порядок URL повторяет порядок записей архива.
    for (const [index, entry] of mediaEntries.entries()) {
      const data = await readZipEntry(archive, entry);
      const blob = capturedBlobs[index]!;
      expect(blob.type).toBe('image/png');
      expect(blob.size).toBe(data!.length);
      expect(new Uint8Array(await blob.arrayBuffer())).toEqual(data);
      const key = `media/${entry.name.slice('word/media/'.length)}`;
      expect(urls[key]).toBe(`blob:mock:${index + 1}`);
    }
  });

  it('классифицирует MIME по расширению и пропускает не-медиа записи', async () => {
    const base = readFixture('anchor_wrap_tight.docx');
    const archive = await rewriteZip(base, {
      inject: [
        { name: 'word/media/photo.JPG', data: bytes('jpeg') },
        { name: 'xl/media/chart.emf', data: bytes('emf') },
        { name: 'ppt/media/clip.mp4', data: bytes('mp4') },
        { name: 'word/media/music.mp3', data: bytes('mp3') },
        { name: 'word/media/noext', data: bytes('noext') },
        // Вложенный путь и запись вне media-каталогов не извлекаются.
        { name: 'word/media/nested/logo.png', data: bytes('nested') },
        { name: 'docProps/fake.png', data: bytes('fake') },
      ],
    });
    const urls = await extractDocxMediaUrls(archive);

    expect(Object.keys(urls).sort()).toEqual([
      'media/chart.emf',
      'media/clip.mp4',
      'media/music.mp3',
      'media/noext',
      'media/photo.JPG',
      'media/tight.png',
    ]);

    const blobWithType = (type: string): Blob => {
      const blob = capturedBlobs.find((candidate) => candidate.type === type);
      if (blob === undefined) {
        throw new Error(`no blob with type ${type}`);
      }
      return blob;
    };
    // Расширение нижним регистром, содержимое Blob — байты записи.
    expect(blobWithType('image/jpeg').size).toBe(4);
    expect(blobWithType('image/x-emf').size).toBe(3);
    expect(blobWithType('video/mp4').size).toBe(3);
    expect(blobWithType('audio/mpeg').size).toBe(3);
    expect(blobWithType('application/octet-stream').size).toBe(5);
    expect(blobWithType('image/png').size).toBe(268);
  });

  it('URL ревокается вызывающим', async () => {
    const archive = readFixture('anchor_wrap_tight.docx');
    const urls = await extractDocxMediaUrls(archive);
    expect(urls['media/tight.png']).toBe('blob:mock:1');

    for (const url of Object.values(urls)) {
      URL.revokeObjectURL(url);
    }
    expect(revokeCalls).toEqual(['blob:mock:1']);
  });
});
