// Починка XLSX-фикстур, у которых матрица объявляет содержимое, а книги его не
// содержат. Оба файла — минимальные фикстуры строк матрицы, поэтому дефект
// блокирует сертификацию соответствующих возможностей.
//
// Что чинится и почему:
// - `sheets-different-shapes.xlsx` — матрица (`xlsx.shapes`) ссылается на неё
//   как на минимальную фикстуру фигур, а `xdr:sp` в книге не было вовсе;
//   добавляется `xl/drawings/drawing1.xml` с тремя фигурами разных видов.
// - `layout-broken-link.xlsx` — матрица (`xlsx.external-links`) ждёт внешнюю
//   ссылку, а в книге были только гиперссылки; добавляется
//   `xl/externalLinks/externalLink1.xml` с битым путём (название фикстуры —
//   «broken link») и `externalReference` в workbook.
//
// Оба патча — правка ZIP существующей книги: пересобирать её целиком нельзя,
// иначе поедут десятки частей, которые генерирует exceljs.

import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { inflateRawSync, deflateRawSync } from 'node:zlib';

const ROOT = path.resolve(import.meta.dirname, '..');
const XLSX_DIR = path.join(ROOT, 'test-fixtures', 'xlsx');

const SML_NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main';
const R_NS = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships';
const XDR_NS = 'http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing';
const A_NS = 'http://schemas.openxmlformats.org/drawingml/2006/main';
const REL_NS = 'http://schemas.openxmlformats.org/package/2006/relationships';
const XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n';

/** Запись ZIP, которую читает и пересобирает этот скрипт. */
interface Entry {
  name: string;
  data: Buffer;
}

/**
 * Прочитать ZIP в список записей.
 *
 * Свой минимальный ридер, а не зависимость: нужен только разбор центрального
 * каталога без извлечения на диск, а книги маленькие (десятки килобайт).
 */
function readZip(bytes: Buffer): Entry[] {
  const entries: Entry[] = [];
  let offset = 0;
  while (offset + 30 <= bytes.length) {
    const signature = bytes.readUInt32LE(offset);
    if (signature !== 0x04034b50) break;
    const method = bytes.readUInt16LE(offset + 8);
    const compressedSize = bytes.readUInt32LE(offset + 18);
    const nameLength = bytes.readUInt16LE(offset + 26);
    const extraLength = bytes.readUInt16LE(offset + 28);
    const nameStart = offset + 30;
    const name = bytes.subarray(nameStart, nameStart + nameLength).toString('utf8');
    const dataStart = nameStart + nameLength + extraLength;
    const raw = bytes.subarray(dataStart, dataStart + compressedSize);
    const data = method === 0 ? Buffer.from(raw) : inflateRawSync(raw);
    if (!name.endsWith('/')) entries.push({ name, data });
    offset = dataStart + compressedSize;
  }
  return entries;
}

/** Собрать ZIP обратно: те же имена, deflate, фиксированный таймстамп. */
function writeZip(entries: readonly Entry[]): Buffer {
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const entry of entries) {
    const name = Buffer.from(entry.name, 'utf8');
    const compressed = deflateRawSync(entry.data);
    const crc = crc32(entry.data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0, 6);
    local.writeUInt16LE(8, 8);
    local.writeUInt16LE(0, 10);
    local.writeUInt16LE(0x21, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(compressed.length, 18);
    local.writeUInt32LE(entry.data.length, 22);
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28);
    locals.push(local, name, compressed);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0, 8);
    central.writeUInt16LE(8, 10);
    central.writeUInt16LE(0, 12);
    central.writeUInt16LE(0x21, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(compressed.length, 20);
    central.writeUInt32LE(entry.data.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt32LE(offset, 42);
    centrals.push(central, name);

    offset += local.length + name.length + compressed.length;
  }
  const centralSize = centrals.reduce((sum, chunk) => sum + chunk.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralSize, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, ...centrals, end]);
}

/** CRC-32 таблицей, как в zip-спецификации. */
function crc32(data: Buffer): number {
  let crc = 0xffffffff;
  for (const byte of data) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** Фигуры для `xdr`-part: прямоугольник, скруглённый прямоугольник и овал. */
const DRAWING_XML =
  XML_DECL +
  `<xdr:wsDr xmlns:xdr="${XDR_NS}" xmlns:a="${A_NS}">` +
  [
    { id: 2, name: 'Прямоугольник', prst: 'rect', from: [1, 1], to: [3, 4], fill: 'C00000' },
    { id: 3, name: 'Скруглённый', prst: 'roundRect', from: [4, 1], to: [7, 4], fill: '00B050' },
    { id: 4, name: 'Овал', prst: 'ellipse', from: [8, 1], to: [11, 4], fill: '0070C0' },
  ]
    .map(
      (shape) =>
        `<xdr:twoCellAnchor editAs="oneCell">` +
        `<xdr:from><xdr:col>${shape.from[0]}</xdr:col><xdr:colOff>0</xdr:colOff>` +
        `<xdr:row>${shape.from[1]}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>` +
        `<xdr:to><xdr:col>${shape.to[0]}</xdr:col><xdr:colOff>0</xdr:colOff>` +
        `<xdr:row>${shape.to[1]}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>` +
        `<xdr:sp macro="" textlink=""><xdr:nvSpPr>` +
        `<xdr:cNvPr id="${shape.id}" name="${shape.name}"/><xdr:cNvSpPr/>` +
        '</xdr:nvSpPr><xdr:spPr>' +
        `<a:xfrm><a:off x="0" y="0"/><a:ext cx="1200000" cy="600000"/></a:xfrm>` +
        `<a:prstGeom prst="${shape.prst}"><a:avLst/></a:prstGeom>` +
        `<a:solidFill><a:srgbClr val="${shape.fill}"/></a:solidFill>` +
        '<a:ln w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln>' +
        '</xdr:spPr>' +
        `<xdr:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>${shape.name}</a:t></a:r></a:p></xdr:txBody>` +
        '</xdr:sp></xdr:twoCellAnchor>',
    )
    .join('') +
  '</xdr:wsDr>';

/** Внешняя ссылка с битым путём. */
const EXTERNAL_LINK_XML =
  XML_DECL +
  `<externalLink xmlns="${SML_NS}" xmlns:r="${R_NS}">` +
  '<externalBook r:id="rId1"><sheetNames><sheetName val="Лист1"/></sheetNames>' +
  '<sheetDataSet><sheetData sheetId="0"><row r="1"><cell r="1"><v>7</v></cell></row></sheetData></sheetDataSet>' +
  '</externalBook></externalLink>';

const EXTERNAL_LINK_RELS =
  XML_DECL +
  `<Relationships xmlns="${REL_NS}">` +
  '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath" ' +
  'Target="file:///C:/missing/source.xlsx" TargetMode="External"/>' +
  '</Relationships>';

/** Заменить или добавить часть. */
function upsert(entries: Entry[], name: string, content: string): void {
  const data = Buffer.from(content, 'utf8');
  const existing = entries.find((entry) => entry.name === name);
  if (existing === undefined) entries.push({ name, data });
  else existing.data = data;
}

/** Добавить `Override` в `[Content_Types].xml`, если его ещё нет. */
function addContentType(entries: Entry[], partName: string, contentType: string): void {
  const file = entries.find((entry) => entry.name === '[Content_Types].xml');
  if (file === undefined) throw new Error('[Content_Types].xml отсутствует');
  let xml = file.data.toString('utf8');
  if (xml.includes(`PartName="/${partName}"`)) return;
  xml = xml.replace(
    '</Types>',
    `<Override PartName="/${partName}" ContentType="${contentType}"/></Types>`,
  );
  file.data = Buffer.from(xml, 'utf8');
}

/** Дать листу связь с drawing и поставить `xdr` в конец `worksheet`. */
function attachDrawing(entries: Entry[], sheetPart: string, target: string): void {
  const sheet = entries.find((entry) => entry.name === sheetPart);
  if (sheet === undefined) throw new Error(`${sheetPart} отсутствует`);
  let xml = sheet.data.toString('utf8');
  xml = xml.replace('</worksheet>', '<drawing r:id="rIdDraw1"/></worksheet>');
  sheet.data = Buffer.from(xml, 'utf8');

  const relsName = `xl/worksheets/_rels/${path.posix.basename(sheetPart)}.rels`;
  const relsFile = entries.find((entry) => entry.name === relsName);
  const relationship =
    `<Relationship Id="rIdDraw1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/${target}"/>`;
  if (relsFile === undefined) {
    entries.push({
      name: relsName,
      data: Buffer.from(XML_DECL + `<Relationships xmlns="${REL_NS}">${relationship}</Relationships>`, 'utf8'),
    });
  } else {
    const xml2 = relsFile.data
      .toString('utf8')
      .replace('</Relationships>', `${relationship}</Relationships>`);
    relsFile.data = Buffer.from(xml2, 'utf8');
  }
}

/** Добавить `externalReference` в workbook и связь к нему. */
function attachExternalLink(entries: Entry[]): void {
  const workbook = entries.find((entry) => entry.name === 'xl/workbook.xml');
  const rels = entries.find((entry) => entry.name === 'xl/_rels/workbook.xml.rels');
  if (workbook === undefined || rels === undefined) throw new Error('части workbook отсутствуют');

  let workbookXml = workbook.data.toString('utf8');
  if (!workbookXml.includes('<externalReferences>')) {
    workbookXml = workbookXml.replace(
      '</sheets>',
      '</sheets><externalReferences><externalReference r:id="rIdExtLink1"/></externalReferences>',
    );
    workbook.data = Buffer.from(workbookXml, 'utf8');
  }
  let relsXml = rels.data.toString('utf8');
  if (!relsXml.includes('externalLink1.xml')) {
    relsXml = relsXml.replace(
      '</Relationships>',
      '<Relationship Id="rIdExtLink1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink" Target="externalLinks/externalLink1.xml"/></Relationships>',
    );
    rels.data = Buffer.from(relsXml, 'utf8');
  }
}

/** Точка входа. */
async function main(): Promise<void> {
  const shapesPath = path.join(XLSX_DIR, 'sheets-different-shapes.xlsx');
  const shapes = readZip(await readFile(shapesPath));
  upsert(shapes, 'xl/drawings/drawing1.xml', DRAWING_XML);
  upsert(
    shapes,
    'xl/drawings/_rels/drawing1.xml.rels',
    XML_DECL + `<Relationships xmlns="${REL_NS}"></Relationships>`,
  );
  addContentType(
    shapes,
    'xl/drawings/drawing1.xml',
    'application/vnd.openxmlformats-officedocument.drawing+xml',
  );
  attachDrawing(shapes, 'xl/worksheets/sheet1.xml', 'drawing1.xml');
  await writeFile(shapesPath, writeZip(shapes));
  process.stdout.write(`patched ${path.relative(ROOT, shapesPath)}\n`);

  const linkPath = path.join(XLSX_DIR, 'layout-broken-link.xlsx');
  const link = readZip(await readFile(linkPath));
  upsert(link, 'xl/externalLinks/externalLink1.xml', EXTERNAL_LINK_XML);
  upsert(link, 'xl/externalLinks/_rels/externalLink1.xml.rels', EXTERNAL_LINK_RELS);
  addContentType(
    link,
    'xl/externalLinks/externalLink1.xml',
    'application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml',
  );
  attachExternalLink(link);
  await writeFile(linkPath, writeZip(link));
  process.stdout.write(`patched ${path.relative(ROOT, linkPath)}\n`);
}

await main();
