/**
 * Тесты препроцессоров OOXML. Архивы для интеграционных проверок собираются вручную
 * (тот же билдер, что в zip.spec.ts): это даёт контроль над каждой записью без
 * сторонних библиотек. Чистые XML-функции (unwrapRubyXml) тестируются напрямую.
 */
import { describe, expect, it } from 'vitest';

import {
  preprocessDocxRuby,
  preprocessPptx,
  preprocessXlsxLineBreaks,
  unwrapRubyXml,
} from '../src/preprocess.js';
import { listZipEntries, readZipEntry } from '../src/zip.js';

const LOCAL_SIGNATURE = 0x04034b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const EOCD_SIGNATURE = 0x06054b50;
const METHOD_STORED = 0;
const METHOD_DEFLATE = 8;

const utf8 = new TextEncoder();
const decoder = new TextDecoder('utf-8');

/** Таблица CRC32 — копия алгоритма из src/zip.ts. */
const CRC_TABLE = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let value = i;
  for (let bit = 0; bit < 8; bit++) {
    value = (value & 1) === 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  }
  CRC_TABLE[i] = value >>> 0;
}

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

async function deflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream('deflate-raw');
  const writer = stream.writable.getWriter();
  const chunks: Uint8Array[] = [];
  const collected = (async () => {
    const reader = stream.readable.getReader();
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

interface BuiltEntry {
  readonly name: string;
  /** Содержимое до сжатия. */
  readonly data: Uint8Array;
  /** Метод сжатия: 0, 8 или любой другой — чтобы проверить отказ по методу. */
  readonly method: number;
}

/** Собирает zip-архив вручную: local header'ы + данные + central directory + EOCD. */
async function buildZip(entries: readonly BuiltEntry[]): Promise<Uint8Array> {
  const localParts: Uint8Array[] = [];
  const centralRecords: Uint8Array[] = [];
  let localBytes = 0;
  for (const entry of entries) {
    const nameBytes = utf8.encode(entry.name);
    const payload = entry.method === METHOD_DEFLATE ? await deflateRaw(entry.data) : entry.data;
    const entryCrc = crc32(entry.data);
    const entryOffset = localBytes;

    const header = new Uint8Array(30 + nameBytes.length);
    const headerView = new DataView(header.buffer);
    headerView.setUint32(0, LOCAL_SIGNATURE, true);
    headerView.setUint16(4, 20, true);
    headerView.setUint16(8, entry.method, true);
    headerView.setUint32(14, entryCrc, true);
    headerView.setUint32(18, payload.length, true);
    headerView.setUint32(22, entry.data.length, true);
    headerView.setUint16(26, nameBytes.length, true);
    header.set(nameBytes, 30);

    localParts.push(header, payload);
    localBytes += header.length + payload.length;

    const record = new Uint8Array(46 + nameBytes.length);
    const recordView = new DataView(record.buffer);
    recordView.setUint32(0, CENTRAL_SIGNATURE, true);
    recordView.setUint16(4, 20, true);
    recordView.setUint16(6, 20, true);
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
  const eocd = new Uint8Array(22);
  const eocdView = new DataView(eocd.buffer);
  eocdView.setUint32(0, EOCD_SIGNATURE, true);
  eocdView.setUint16(8, entries.length, true);
  eocdView.setUint16(10, entries.length, true);
  eocdView.setUint32(12, centralDirectory.length, true);
  eocdView.setUint32(16, localBytes, true);
  return concat([concat(localParts), centralDirectory, eocd]);
}

/** Читает запись и декодирует её как UTF-8. */
async function readText(archive: Uint8Array, name: string): Promise<string> {
  const data = await readZipEntry(archive, name);
  if (data === null) {
    throw new Error(`entry ${name} not readable`);
  }
  return decoder.decode(data);
}

describe('unwrapRubyXml', () => {
  it('заменяет ruby базовыми run`ами, сохраняя текст rubyBase', () => {
    const xml =
      '<w:r><w:t>привет </w:t></w:r>' +
      '<w:ruby><w:rubyBase><w:r><w:t>мир</w:t></w:r></w:rubyBase>' +
      '<w:rt><w:r><w:t>world</w:t></w:r></w:rt></w:ruby>' +
      '<w:r><w:t>!</w:t></w:r>';
    const result = unwrapRubyXml(xml)!;
    expect(result).not.toContain('<w:ruby');
    expect(result).not.toContain('<w:rt>');
    // run закрыт, базовые run'ы вставлены, run снова открыт — без вложенных runs.
    expect(result).toContain('</w:r></w:r><w:r><w:t>мир</w:t></w:r><w:r><w:r>');
    expect(result).toContain('<w:t>привет </w:t>');
    expect(result).toContain('<w:t>!</w:t>');
  });

  it('ruby без rubyBase превращается в закрытие/открытие run без содержимого', () => {
    const xml = '<w:ruby><w:rt><w:r><w:t>x</w:t></w:r></w:rt></w:ruby>';
    expect(unwrapRubyXml(xml)).toBe('</w:r><w:r>');
  });

  it('разворачивает несколько ruby в одной строке', () => {
    const xml =
      '<w:ruby><w:rubyBase><w:r><w:t>а</w:t></w:r></w:rubyBase></w:ruby>' +
      '<w:r><w:t> </w:t></w:r>' +
      '<w:ruby><w:rubyBase><w:r><w:t>б</w:t></w:r></w:rubyBase></w:ruby>';
    const result = unwrapRubyXml(xml)!;
    expect(result).not.toContain('<w:ruby');
    expect(result).toContain('<w:t>а</w:t>');
    expect(result).toContain('<w:t>б</w:t>');
    expect(result).toContain('<w:t> </w:t>');
  });

  it('атрибуты ruby и rubyBase не мешают', () => {
    const xml =
      '<w:ruby w:some="1"><w:rubyBase w:lang="ja-JP"><w:r><w:t>字</w:t></w:r></w:rubyBase>' +
      '<w:rt w:x="2"><w:r><w:t>kana</w:t></w:r></w:rt></w:ruby>';
    const result = unwrapRubyXml(xml)!;
    expect(result).toContain('<w:r><w:t>字</w:t></w:r>');
    expect(result).not.toContain('<w:ruby');
    expect(result).not.toContain('kana');
  });

  it('без ruby возвращает null', () => {
    expect(unwrapRubyXml('<w:r><w:t>обычный текст</w:t></w:r>')).toBeNull();
    expect(unwrapRubyXml('')).toBeNull();
  });
});

describe('preprocessXlsxLineBreaks', () => {
  it('разворачивает двойное экранирование в xl/*.xml', async () => {
    const archive = await buildZip([
      {
        name: 'xl/sharedStrings.xml',
        data: bytes('<?xml version="1.0"?><sst><si><t>line1&amp;#10;line2</t></si></sst>'),
        method: METHOD_DEFLATE,
      },
      {
        name: 'xl/workbook.xml',
        data: bytes('<workbook><t>низ&amp;#10;низ</t></workbook>'),
        method: METHOD_DEFLATE,
      },
      { name: '[Content_Types].xml', data: bytes('<Types/>'), method: METHOD_DEFLATE },
      { name: 'docProps/core.xml', data: bytes('<cp:coreProperties/>'), method: METHOD_DEFLATE },
    ]);
    const result = await preprocessXlsxLineBreaks(archive);
    expect(result).not.toBe(archive);

    const sst = await readText(result, 'xl/sharedStrings.xml');
    expect(sst).toContain('line1&#10;line2');
    expect(sst).not.toContain('&amp;#10;');
    const workbook = await readText(result, 'xl/workbook.xml');
    expect(workbook).toContain('низ&#10;низ');
    expect(workbook).not.toContain('&amp;#10;');
    // Записи вне xl/ не трогаются вовсе.
    expect(await readZipEntry(result, '[Content_Types].xml')).toEqual(bytes('<Types/>'));
    expect(await readZipEntry(result, 'docProps/core.xml')).toEqual(bytes('<cp:coreProperties/>'));
  });

  it('без паттерна возвращает исходные байты без пересборки', async () => {
    const archive = await buildZip([
      {
        name: 'xl/sharedStrings.xml',
        data: bytes('<sst><si><t>чистый текст</t></si></sst>'),
        method: METHOD_DEFLATE,
      },
    ]);
    expect(await preprocessXlsxLineBreaks(archive)).toBe(archive);
  });

  it('паттерн вне xl/*.xml игнорируется — исходные байты', async () => {
    const archive = await buildZip([
      { name: 'word/document.xml', data: bytes('<w:p>&amp;#10;</w:p>'), method: METHOD_DEFLATE },
    ]);
    expect(await preprocessXlsxLineBreaks(archive)).toBe(archive);
  });

  it('нечитаемая запись xl/*.xml не роняет функцию', async () => {
    const archive = await buildZip([
      { name: 'xl/sharedStrings.xml', data: bytes('<sst>&amp;#10;</sst>'), method: 12 },
    ]);
    expect(await preprocessXlsxLineBreaks(archive)).toBe(archive);
  });
});

const OFFICE_DOCUMENT_TYPE =
  'http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument';

const PPTX_RELS =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
  `<Relationship Id="rId1" Type="${OFFICE_DOCUMENT_TYPE}" Target="ppt/presentation.xml"/>` +
  '</Relationships>';

const NOTES_SLIDE_WITH_ANIM =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:pic>' +
  '<p:nvPicPr><p:cNvPr id="1" name="1" showMasterPhAnim="1"/></p:nvPicPr>' +
  '</p:pic></p:spTree></p:cSld></p:notes>';

const NOTES_MASTER_WITH_ANIM =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<p:notesMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:sp>' +
  '<p:nvSpPr><p:cNvPr id="2" name="2" showMasterPhAnim="0"/></p:nvSpPr>' +
  '</p:sp></p:spTree></p:cSld></p:notesMaster>';

const EXPECTED_APP_XML =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">' +
  '<Application>Microsoft Office PowerPoint</Application></Properties>';

const EXPECTED_CORE_XML =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" ' +
  'xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" ' +
  'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title/><dc:creator/></cp:coreProperties>';

describe('preprocessPptx', () => {
  it('инжектирует docProps и отношения, убирает showMasterPhAnim из notes', async () => {
    const archive = await buildZip([
      { name: '[Content_Types].xml', data: bytes('<Types/>'), method: METHOD_DEFLATE },
      { name: '_rels/.rels', data: bytes(PPTX_RELS), method: METHOD_DEFLATE },
      { name: 'ppt/presentation.xml', data: bytes('<p:presentation/>'), method: METHOD_DEFLATE },
      { name: 'ppt/notesSlides/notesSlide1.xml', data: bytes(NOTES_SLIDE_WITH_ANIM), method: METHOD_DEFLATE },
      { name: 'ppt/notesMasters/notesMaster1.xml', data: bytes(NOTES_MASTER_WITH_ANIM), method: METHOD_DEFLATE },
    ]);
    const result = await preprocessPptx(archive);
    expect(result).not.toBe(archive);

    expect(await readText(result, 'docProps/app.xml')).toBe(EXPECTED_APP_XML);
    expect(await readText(result, 'docProps/core.xml')).toBe(EXPECTED_CORE_XML);

    const rels = await readText(result, '_rels/.rels');
    expect(rels).toContain(
      '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>',
    );
    expect(rels).toContain(
      '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>',
    );
    // Исходное отношение на месте, новые вставлены перед закрывающим тегом.
    expect(rels).toContain('<Relationship Id="rId1"');
    expect(rels.indexOf('<Relationship Id="rId2"')).toBeLessThan(rels.indexOf('</Relationships>'));
    expect(rels.indexOf('<Relationship Id="rId3"')).toBeLessThan(rels.indexOf('</Relationships>'));

    const slide = await readText(result, 'ppt/notesSlides/notesSlide1.xml');
    expect(slide).not.toContain('showMasterPhAnim');
    expect(slide).toContain('<p:cNvPr id="1" name="1"/>');
    const master = await readText(result, 'ppt/notesMasters/notesMaster1.xml');
    expect(master).not.toContain('showMasterPhAnim');
    expect(master).toContain('<p:cNvPr id="2" name="2"/>');
  });

  it('без rId в .rels нумерация начинается с rId1', async () => {
    const relsWithoutIds =
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
      '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>';
    const archive = await buildZip([
      { name: '_rels/.rels', data: bytes(relsWithoutIds), method: METHOD_DEFLATE },
      { name: 'ppt/presentation.xml', data: bytes('<p:presentation/>'), method: METHOD_DEFLATE },
    ]);
    const result = await preprocessPptx(archive);
    const rels = await readText(result, '_rels/.rels');
    expect(rels).toContain('<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties"');
    expect(rels).toContain('<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties"');
  });

  it('тип отношения уже упомянут — дубликат не вставляется', async () => {
    const relsWithAppMention =
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
      '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
      '<Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/custom.xml"/>' +
      '</Relationships>';
    const archive = await buildZip([
      { name: '_rels/.rels', data: bytes(relsWithAppMention), method: METHOD_DEFLATE },
      { name: 'ppt/presentation.xml', data: bytes('<p:presentation/>'), method: METHOD_DEFLATE },
    ]);
    const result = await preprocessPptx(archive);
    expect(await readText(result, 'docProps/app.xml')).toBe(EXPECTED_APP_XML);
    const rels = await readText(result, '_rels/.rels');
    // Упоминание extended-properties осталось одно — новое отношение не добавлено.
    expect(rels.match(/extended-properties/g)).toHaveLength(1);
    expect(rels).not.toContain('Target="docProps/app.xml"');
    // А core-properties, которого не было, — добавлено.
    expect(rels).toContain('Target="docProps/core.xml"');
  });

  it('только notes-фикс, когда docProps и отношения на месте', async () => {
    const relsComplete =
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
      '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
      `<Relationship Id="rId1" Type="${OFFICE_DOCUMENT_TYPE}" Target="ppt/presentation.xml"/>` +
      '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>' +
      '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>' +
      '</Relationships>';
    const archive = await buildZip([
      { name: '_rels/.rels', data: bytes(relsComplete), method: METHOD_DEFLATE },
      { name: 'ppt/notesSlides/notesSlide1.xml', data: bytes(NOTES_SLIDE_WITH_ANIM), method: METHOD_DEFLATE },
      { name: 'docProps/app.xml', data: bytes(EXPECTED_APP_XML), method: METHOD_DEFLATE },
      { name: 'docProps/core.xml', data: bytes(EXPECTED_CORE_XML), method: METHOD_DEFLATE },
    ]);
    const result = await preprocessPptx(archive);
    expect(result).not.toBe(archive);
    const slide = await readText(result, 'ppt/notesSlides/notesSlide1.xml');
    expect(slide).not.toContain('showMasterPhAnim');
    // docProps не задвоены.
    expect(listZipEntries(result).filter((entry) => entry.name === 'docProps/app.xml')).toHaveLength(1);
    expect(listZipEntries(result).filter((entry) => entry.name === 'docProps/core.xml')).toHaveLength(1);
  });

  it('всё на месте — исходные байты без пересборки', async () => {
    const relsComplete =
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
      '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
      `<Relationship Id="rId1" Type="${OFFICE_DOCUMENT_TYPE}" Target="ppt/presentation.xml"/>` +
      '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>' +
      '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>' +
      '</Relationships>';
    const cleanNotes =
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
      '<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:pic>' +
      '<p:nvPicPr><p:cNvPr id="1" name="1"/></p:nvPicPr>' +
      '</p:pic></p:spTree></p:cSld></p:notes>';
    const archive = await buildZip([
      { name: '[Content_Types].xml', data: bytes('<Types/>'), method: METHOD_DEFLATE },
      { name: '_rels/.rels', data: bytes(relsComplete), method: METHOD_DEFLATE },
      { name: 'ppt/presentation.xml', data: bytes('<p:presentation/>'), method: METHOD_DEFLATE },
      { name: 'ppt/notesSlides/notesSlide1.xml', data: bytes(cleanNotes), method: METHOD_DEFLATE },
      { name: 'docProps/app.xml', data: bytes(EXPECTED_APP_XML), method: METHOD_DEFLATE },
      { name: 'docProps/core.xml', data: bytes(EXPECTED_CORE_XML), method: METHOD_DEFLATE },
    ]);
    expect(await preprocessPptx(archive)).toBe(archive);
  });

  it('notes-фикс не трогает записи вне notesSlides/notesMasters', async () => {
    const slideWithAnim =
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
      '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:pic>' +
      '<p:nvPicPr><p:cNvPr id="3" name="3" showMasterPhAnim="1"/></p:nvPicPr>' +
      '</p:pic></p:spTree></p:cSld></p:sld>';
    const archive = await buildZip([
      { name: '_rels/.rels', data: bytes(PPTX_RELS), method: METHOD_DEFLATE },
      { name: 'ppt/slides/slide1.xml', data: bytes(slideWithAnim), method: METHOD_DEFLATE },
      { name: 'docProps/app.xml', data: bytes(EXPECTED_APP_XML), method: METHOD_DEFLATE },
      { name: 'docProps/core.xml', data: bytes(EXPECTED_CORE_XML), method: METHOD_DEFLATE },
    ]);
    // Атрибут у обычного слайда не под фильтром; docProps на месте — исходные байты.
    expect(await preprocessPptx(archive)).toBe(archive);
  });
});

const RUBY_DOCUMENT =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p>' +
  '<w:r><w:t>Привет </w:t></w:r>' +
  '<w:ruby><w:rubyBase><w:r><w:t>мир</w:t></w:r></w:rubyBase>' +
  '<w:rt><w:r><w:t>world</w:t></w:r></w:rt></w:ruby>' +
  '<w:r><w:t>!</w:t></w:r>' +
  '</w:p></w:body></w:document>';

describe('preprocessDocxRuby', () => {
  it('разворачивает ruby в word/*.xml, сохраняя базовый текст', async () => {
    const archive = await buildZip([
      { name: '[Content_Types].xml', data: bytes('<Types/>'), method: METHOD_DEFLATE },
      { name: 'word/document.xml', data: bytes(RUBY_DOCUMENT), method: METHOD_DEFLATE },
      { name: 'word/styles.xml', data: bytes('<w:styles><w:style/></w:styles>'), method: METHOD_DEFLATE },
    ]);
    const result = await preprocessDocxRuby(archive);
    expect(result).not.toBe(archive);

    const document = await readText(result, 'word/document.xml');
    expect(document).not.toContain('<w:ruby');
    expect(document).not.toContain('<w:rt>');
    expect(document).toContain('<w:t>мир</w:t>');
    expect(document).toContain('<w:t>Привет </w:t>');
    // styles.xml без ruby не менялась.
    expect(await readZipEntry(result, 'word/styles.xml')).toEqual(bytes('<w:styles><w:style/></w:styles>'));
  });

  it('ruby в другом word/*.xml тоже разворачивается, если он есть в document.xml', async () => {
    const archive = await buildZip([
      { name: 'word/document.xml', data: bytes(RUBY_DOCUMENT), method: METHOD_DEFLATE },
      {
        name: 'word/footnotes.xml',
        data: bytes('<w:footnotes><w:ruby><w:rubyBase><w:r><w:t>сноска</w:t></w:r></w:rubyBase></w:ruby></w:footnotes>'),
        method: METHOD_DEFLATE,
      },
    ]);
    const result = await preprocessDocxRuby(archive);
    const footnotes = await readText(result, 'word/footnotes.xml');
    expect(footnotes).not.toContain('<w:ruby');
    expect(footnotes).toContain('<w:t>сноска</w:t>');
  });

  it('без ruby в document.xml — исходные байты', async () => {
    const archive = await buildZip([
      { name: '[Content_Types].xml', data: bytes('<Types/>'), method: METHOD_DEFLATE },
      {
        name: 'word/document.xml',
        data: bytes('<w:document><w:body><w:p/></w:body></w:document>'),
        method: METHOD_DEFLATE,
      },
    ]);
    expect(await preprocessDocxRuby(archive)).toBe(archive);
  });

  it('нечитаемый document.xml — исходные байты', async () => {
    const archive = await buildZip([{ name: 'word/document.xml', data: bytes('<w:document/>'), method: 12 }]);
    expect(await preprocessDocxRuby(archive)).toBe(archive);
  });

  it('хранит записи вне word/*.xml нетронутыми', async () => {
    const archive = await buildZip([
      { name: 'word/document.xml', data: bytes(RUBY_DOCUMENT), method: METHOD_STORED },
      {
        name: 'customXml/item1.xml',
        data: bytes('<customXml><w:ruby/></customXml>'),
        method: METHOD_STORED,
      },
    ]);
    const result = await preprocessDocxRuby(archive);
    expect(await readZipEntry(result, 'customXml/item1.xml')).toEqual(bytes('<customXml><w:ruby/></customXml>'));
  });
});
