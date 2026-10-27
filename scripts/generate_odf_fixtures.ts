// Генератор ODF-фикстур (ODT/ODS/ODP) для будущего крейта `crates/odf`.
//
// Парсера ещё нет, поэтому фикстуры — единственный контракт на вход: они
// покрывают то, что предстоит разбирать (текст, таблицы, страницы, слайды),
// и собираются здесь, а не руками в LibreOffice, чтобы каждая мелочь была
// видна в диффе.
//
// Архив пишется своим детерминированным writer'ом: фиксированные DOS-времена
// (1980-01-01 00:00), порядок записей как в массиве, своя таблица CRC32.
// `mimetype` — всегда первая запись и без сжатия (требование ODF: читатель
// узнаёт формат, не распаковывая архив); остальные части сжимаются deflate,
// если это реально уменьшает байты, иначе кладутся как есть (PNG не растёт).
// Два прогона подряд дают побайтово те же файлы.
//
// Запуск:
//   npx tsx scripts/generate_odf_fixtures.ts
//
// README.md в каталоге фикстур генератору не принадлежит и не удаляется:
// чистятся только каталоги форматов (odt/, ods/, odp/).

import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { deflateRawSync } from 'node:zlib';

const ROOT = path.resolve(import.meta.dirname, '..');
const OUT_DIR = path.join(ROOT, 'test-fixtures', 'odf');

// ── Детерминированный ZIP ───────────────────────────────────

/** Часть пакета: путь внутри архива и содержимое (строка кодируется в UTF-8). */
type Part = readonly [name: string, data: string | Uint8Array];

/** Запись архива; `store` выключает deflate — нужно для `mimetype`. */
interface ZipEntry {
  readonly name: string;
  readonly data: string | Uint8Array;
  readonly store?: boolean;
}

/** CRC32 (IEEE 802.3), таблица строится один раз. */
const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) {
      c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

const crc32 = (data: Uint8Array): number => {
  let crc = 0xffffffff;
  for (let i = 0; i < data.length; i++) {
    crc = CRC_TABLE[(crc ^ data[i]!) & 0xff]! ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
};

// Дата 1980-01-01 (day=1, month=1, year=0). Именно 0x0021, а не 0: нулевой
// день/месяц формально невалиден и некоторые читатели на нём ругаются.
const DOS_DATE = 0x0021;
const DOS_TIME = 0x0000;

const VERSION = 20;
const METHOD_DEFLATE = 8;
const METHOD_STORE = 0;

const SIG_LOCAL = 0x04034b50;
const SIG_CENTRAL = 0x02014b50;
const SIG_EOCD = 0x06054b50;

/**
 * Собирает ZIP в памяти. Порядок записей сохраняется как передан.
 *
 * @param entries записи архива; `mimetype` обязана быть первой и со `store`
 * @returns байты архива
 */
const buildZip = (entries: readonly ZipEntry[]): Buffer => {
  if (entries.length === 0) throw new Error('empty archive');

  const localChunks: Buffer[] = [];
  const centralChunks: Buffer[] = [];
  let offset = 0;

  for (const entry of entries) {
    const name = Buffer.from(entry.name, 'utf8');
    const data =
      typeof entry.data === 'string' ? Buffer.from(entry.data, 'utf8') : Buffer.from(entry.data);
    const crc = crc32(data);
    const deflated = entry.store === true ? null : deflateRawSync(data, { level: 9 });
    // Как `zip -9`: несжимаемое (PNG) кладём как есть, иначе оно растёт.
    const useDeflate = deflated !== null && deflated.length < data.length;
    const payload = useDeflate && deflated !== null ? deflated : data;
    const method = useDeflate ? METHOD_DEFLATE : METHOD_STORE;

    const local = Buffer.alloc(30);
    local.writeUInt32LE(SIG_LOCAL, 0);
    local.writeUInt16LE(VERSION, 4);
    local.writeUInt16LE(0, 6); // flags: без UTF-8-флага (имена ASCII), без data descriptor
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(DOS_TIME, 10);
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(payload.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28); // extraLen

    localChunks.push(local, name, payload);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(SIG_CENTRAL, 0);
    central.writeUInt16LE(VERSION, 4);
    central.writeUInt16LE(VERSION, 6);
    central.writeUInt16LE(0, 8); // flags
    central.writeUInt16LE(method, 10);
    central.writeUInt16LE(DOS_TIME, 12);
    central.writeUInt16LE(DOS_DATE, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(payload.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt16LE(0, 30); // extraLen
    central.writeUInt16LE(0, 32); // commentLen
    central.writeUInt16LE(0, 34); // diskStart
    central.writeUInt16LE(0, 36); // internalAttrs
    central.writeUInt32LE(0, 38); // externalAttrs
    central.writeUInt32LE(offset, 42);
    centralChunks.push(central, name);

    offset += local.length + name.length + payload.length;
  }

  const centralSize = centralChunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(SIG_EOCD, 0);
  eocd.writeUInt16LE(0, 4); // diskNum
  eocd.writeUInt16LE(0, 6); // cdStartDisk
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(centralSize, 12);
  eocd.writeUInt32LE(offset, 16);
  eocd.writeUInt16LE(0, 20); // commentLen

  return Buffer.concat([...localChunks, ...centralChunks, eocd] as Uint8Array[]);
};

// ── Пространства имён и XML-помощники ───────────────────────

const NS = {
  office: 'urn:oasis:names:tc:opendocument:xmlns:office:1.0',
  style: 'urn:oasis:names:tc:opendocument:xmlns:style:1.0',
  text: 'urn:oasis:names:tc:opendocument:xmlns:text:1.0',
  table: 'urn:oasis:names:tc:opendocument:xmlns:table:1.0',
  draw: 'urn:oasis:names:tc:opendocument:xmlns:drawing:1.0',
  presentation: 'urn:oasis:names:tc:opendocument:xmlns:presentation:1.0',
  of: 'urn:oasis:names:tc:opendocument:xmlns:of:1.2',
  fo: 'urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0',
  svg: 'urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0',
  xlink: 'http://www.w3.org/1999/xlink',
  dc: 'http://purl.org/dc/elements/1.1/',
  meta: 'urn:oasis:names:tc:opendocument:xmlns:meta:1.0',
  number: 'urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0',
  manifest: 'urn:oasis:names:tc:opendocument:xmlns:manifest:1.0',
  config: 'urn:oasis:names:tc:opendocument:xmlns:config:1.0',
} as const;

type NsName = keyof typeof NS;

// Объявляем надмножество: неиспользуемый префикс в XML легален, а список
// частей у ODT/ODS/ODP почти один и тот же — так его не приходится дробить.
const CONTENT_NS: readonly NsName[] = [
  'office',
  'style',
  'text',
  'table',
  'draw',
  'presentation',
  'of',
  'fo',
  'svg',
  'xlink',
  'number',
];
const STYLES_NS: readonly NsName[] = CONTENT_NS;
const META_NS: readonly NsName[] = ['office', 'meta', 'dc'];
const MANIFEST_NS: readonly NsName[] = ['manifest'];
const SETTINGS_NS: readonly NsName[] = ['office', 'config'];

const XML_DECL = '<?xml version="1.0" encoding="UTF-8"?>\n';

const nsDecls = (namespaces: readonly NsName[]): string =>
  namespaces.map((name) => `xmlns:${name}="${NS[name]}"`).join(' ');

/** Экранирование текста и значений атрибутов; кавычка важна для атрибутов. */
const xmlEscape = (text: string): string =>
  text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');

const wrap = (name: string, content: string): string =>
  content === '' ? `<${name}/>` : `<${name}>${content}</${name}>`;

// ── Сборка пакета ───────────────────────────────────────────

/** Формат ODF: MIME-тип, корень `office:body` и расширение файла. */
interface OdfKind {
  readonly mimetype: string;
  readonly root: 'text' | 'spreadsheet' | 'presentation';
  readonly extension: 'odt' | 'ods' | 'odp';
}

const ODT: OdfKind = {
  mimetype: 'application/vnd.oasis.opendocument.text',
  root: 'text',
  extension: 'odt',
};
const ODS: OdfKind = {
  mimetype: 'application/vnd.oasis.opendocument.spreadsheet',
  root: 'spreadsheet',
  extension: 'ods',
};
const ODP: OdfKind = {
  mimetype: 'application/vnd.oasis.opendocument.presentation',
  root: 'presentation',
  extension: 'odp',
};

/** Содержимое пакета: тело и разложенные по частям стили. */
interface OdfPackage {
  /** `dc:title` в meta.xml — фиксированный, без времени генерации. */
  readonly title: string;
  /** Дети `office:body/office:<root>`. */
  readonly body: string;
  /** `office:automatic-styles` в content.xml. */
  readonly automaticStyles?: string;
  /** `office:styles` в styles.xml. */
  readonly styles?: string;
  /** `office:automatic-styles` в styles.xml (page-layout и т.п.). */
  readonly stylesAutomatic?: string;
  /** `office:master-styles` в styles.xml (master-page, колонтитулы). */
  readonly masterStyles?: string;
  /** Дополнительные части пакета: Pictures/… */
  readonly extraParts?: readonly Part[];
}

const contentXml = (kind: OdfKind, spec: OdfPackage): string =>
  XML_DECL +
  `<office:document-content ${nsDecls(CONTENT_NS)} office:version="1.2" ` +
  `office:mimetype="${kind.mimetype}">` +
  wrap('office:automatic-styles', spec.automaticStyles ?? '') +
  `<office:body><office:${kind.root}>${spec.body}</office:${kind.root}></office:body>` +
  '</office:document-content>';

const stylesXml = (spec: OdfPackage): string =>
  XML_DECL +
  `<office:document-styles ${nsDecls(STYLES_NS)} office:version="1.2">` +
  wrap('office:styles', spec.styles ?? '') +
  wrap('office:automatic-styles', spec.stylesAutomatic ?? '') +
  wrap('office:master-styles', spec.masterStyles ?? '') +
  '</office:document-styles>';

// Метаданные фиксированы: Date.now() сделал бы пакеты невоспроизводимыми,
// а для разбора meta.xml важны сами поля, а не текущая дата.
const META_DATE = '2024-01-01T00:00:00';

const metaXml = (title: string): string =>
  XML_DECL +
  `<office:document-meta ${nsDecls(META_NS)} office:version="1.2">` +
  '<office:meta>' +
  `<dc:title>${xmlEscape(title)}</dc:title>` +
  '<meta:generator>doc-converter fixture generator</meta:generator>' +
  `<meta:creation-date>${META_DATE}</meta:creation-date>` +
  `<dc:date>${META_DATE}</dc:date>` +
  '</office:meta></office:document-meta>';

const settingsXml = (): string =>
  XML_DECL +
  `<office:document-settings ${nsDecls(SETTINGS_NS)} office:version="1.2">` +
  '<office:settings><config:config-item-set config:name="ooo:view-settings">' +
  '<config:config-item config:name="ViewAreaTop" config:type="long">0</config:config-item>' +
  '<config:config-item config:name="ViewAreaLeft" config:type="long">0</config:config-item>' +
  '</config:config-item-set></office:settings></office:document-settings>';

const mediaType = (partName: string): string => {
  if (partName.endsWith('.xml') === true) return 'text/xml';
  if (partName.endsWith('.png') === true) return 'image/png';
  throw new Error(`unknown media type for package part: ${partName}`);
};

const manifestXml = (kind: OdfKind, parts: readonly Part[]): string =>
  XML_DECL +
  `<manifest:manifest ${nsDecls(MANIFEST_NS)} manifest:version="1.2">` +
  '<manifest:file-entry manifest:full-path="/" manifest:version="1.2" ' +
  `manifest:media-type="${kind.mimetype}"/>` +
  parts
    .map(
      ([name]) =>
        `<manifest:file-entry manifest:full-path="${xmlEscape(name)}" ` +
        `manifest:media-type="${mediaType(name)}"/>`,
    )
    .join('') +
  '</manifest:manifest>';

/**
 * Собирает пакет ODF: mimetype, манифест, затем content/styles/meta/settings
 * и дополнительные части. Manifest строится из фактического списка частей,
 * поэтому забыть объявить картинку невозможно.
 */
const writePackage = (kind: OdfKind, spec: OdfPackage): Buffer => {
  const parts: Part[] = [
    ['content.xml', contentXml(kind, spec)],
    ['styles.xml', stylesXml(spec)],
    ['meta.xml', metaXml(spec.title)],
    ['settings.xml', settingsXml()],
    ...(spec.extraParts ?? []),
  ];
  return buildZip([
    // ODF 1.2 §3.2: mimetype лежит первым, без сжатия и без extra-поля —
    // по нему читатель определяет формат до распаковки.
    { name: 'mimetype', data: kind.mimetype, store: true },
    { name: 'META-INF/manifest.xml', data: manifestXml(kind, parts) },
    ...parts.map(([name, data]) => ({ name, data })),
  ]);
};

/** Собирает ODT. */
const writeOdt = (spec: OdfPackage): Buffer => writePackage(ODT, spec);

/** Собирает ODS. */
const writeOds = (spec: OdfPackage): Buffer => writePackage(ODS, spec);

/** Собирает ODP. */
const writeOdp = (spec: OdfPackage): Buffer => writePackage(ODP, spec);

// ── Общие XML-фрагменты ─────────────────────────────────────

/**
 * 1×1 PNG (RGBA, пиксель #4472C4) фиксированными байтами: deflate внутри PNG
 * зависит от версии zlib, поэтому готовый файл вшит, а не собирается на лету.
 */
const PIXEL_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNwKTryHwAE8gJ6dFBxlQAAAABJRU5ErkJggg==',
  'base64',
);

const textStyle = (name: string, properties: string): string =>
  `<style:style style:name="${name}" style:family="text">` +
  `<style:text-properties ${properties}/></style:style>`;

const paragraphStyle = (name: string, properties: string, textProperties = ''): string =>
  `<style:style style:name="${name}" style:family="paragraph">` +
  `<style:paragraph-properties ${properties}/>` +
  (textProperties === '' ? '' : `<style:text-properties ${textProperties}/>`) +
  '</style:style>';

const span = (styleName: string, text: string): string =>
  `<text:span text:style-name="${styleName}">${text}</text:span>`;

const paragraph = (text: string, styleName = 'P_body', attributes = ''): string =>
  `<text:p text:style-name="${styleName}"${attributes === '' ? '' : ` ${attributes}`}>` +
  `${text}</text:p>`;

const heading = (level: 1 | 2, text: string, styleName: string): string =>
  `<text:h text:style-name="${styleName}" text:outline-level="${level}">${text}</text:h>`;

const hyperlink = (href: string, styleName: string, text: string): string =>
  `<text:a xlink:type="simple" xlink:href="${href}" text:style-name="${styleName}">${text}</text:a>`;

const list = (styleName: string, items: readonly string[]): string =>
  `<text:list text:style-name="${styleName}">` +
  items.map((item) => `<text:list-item>${item}</text:list-item>`).join('') +
  '</text:list>';

const frame = (styleName: string, geometry: string, body: string): string =>
  `<draw:frame draw:style-name="${styleName}" ${geometry}>${body}</draw:frame>`;

const image = (href: string): string =>
  `<draw:image xlink:href="${href}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>`;

const textBox = (paragraphs: string): string => `<draw:text-box>${paragraphs}</draw:text-box>`;

const slide = (name: string, shapes: string): string =>
  `<draw:page draw:name="${name}" draw:master-page-name="Standard">${shapes}</draw:page>`;

// Табличные помощники общие для ODT и ODS: элементы у них одни и те же.

const column = (styleName: string, repeat = 1): string =>
  `<table:table-column${repeat > 1 ? ` table:number-columns-repeated="${repeat}"` : ''} ` +
  `table:style-name="${styleName}"/>`;

const emptyCell = (repeat = 1): string =>
  `<table:table-cell${repeat > 1 ? ` table:number-columns-repeated="${repeat}"` : ''}/>`;

const coveredCells = (repeat = 1): string =>
  `<table:covered-table-cell${repeat > 1 ? ` table:number-columns-repeated="${repeat}"` : ''}/>`;

const row = (
  cells: string,
  options: { readonly style?: string; readonly repeat?: number } = {},
): string => {
  const attributes =
    (options.style === undefined ? '' : ` table:style-name="${options.style}"`) +
    (options.repeat !== undefined && options.repeat > 1
      ? ` table:number-rows-repeated="${options.repeat}"`
      : '');
  return `<table:table-row${attributes}>${cells}</table:table-row>`;
};

const cell = (
  valueAttributes: string,
  text: string,
  options: {
    readonly style?: string;
    readonly formula?: string;
    readonly columnsSpanned?: number;
    readonly rowsSpanned?: number;
  } = {},
): string => {
  let attributes =
    options.style === undefined ? '' : ` table:style-name="${options.style}"`;
  if (options.columnsSpanned !== undefined) {
    attributes += ` table:number-columns-spanned="${options.columnsSpanned}"`;
  }
  if (options.rowsSpanned !== undefined) {
    attributes += ` table:number-rows-spanned="${options.rowsSpanned}"`;
  }
  if (options.formula !== undefined) {
    attributes += ` table:formula="${xmlEscape(options.formula)}"`;
  }
  if (valueAttributes !== '') attributes += ` ${valueAttributes}`;
  const body = text === '' ? '' : `<text:p>${text}</text:p>`;
  return `<table:table-cell${attributes}>${body}</table:table-cell>`;
};

const table = (name: string, columns: string, rows: string, styleName = 'ta1'): string =>
  `<table:table table:name="${name}" table:style-name="${styleName}">` +
  `${columns}${rows}</table:table>`;

// ── Общие стили ─────────────────────────────────────────────

const LIST_STYLE_BULLET =
  '<text:list-style style:name="L_bullet">' +
  '<text:list-level-style-bullet text:level="1" text:bullet-char="•">' +
  '<style:list-level-properties text:space-before="0.5cm" text:min-label-width="0.5cm"/>' +
  '</text:list-level-style-bullet>' +
  '<text:list-level-style-bullet text:level="2" text:bullet-char="–">' +
  '<style:list-level-properties text:space-before="1cm" text:min-label-width="0.5cm"/>' +
  '</text:list-level-style-bullet>' +
  '</text:list-style>';

const ODT_TEXT_STYLES =
  textStyle('T_bold', 'fo:font-weight="bold"') +
  textStyle('T_italic', 'fo:font-style="italic"') +
  textStyle(
    'T_link',
    'fo:color="#0563C1" style:text-underline-style="solid" ' +
      'style:text-underline-color="font-color"',
  );

const ODT_PARAGRAPH_STYLES =
  paragraphStyle('P_body', 'fo:text-align="justify"', 'fo:font-size="11pt"') +
  paragraphStyle(
    'P_h1',
    'fo:margin-top="0.4cm" fo:margin-bottom="0.2cm"',
    'fo:font-size="18pt" fo:font-weight="bold" fo:color="#1F3864"',
  ) +
  paragraphStyle(
    'P_h2',
    'fo:margin-top="0.3cm" fo:margin-bottom="0.2cm"',
    'fo:font-size="14pt" fo:font-weight="bold"',
  );

// ── ODT: simple.odt ─────────────────────────────────────────

/** Заголовки, прогоны bold/italic, список с вложенностью, ссылка, пробелы. */
const simpleOdt = (): Buffer =>
  writeOdt({
    title: 'ODF text fixture',
    automaticStyles: ODT_TEXT_STYLES + ODT_PARAGRAPH_STYLES,
    styles: LIST_STYLE_BULLET,
    body:
      heading(1, 'ODF text fixture', 'P_h1') +
      paragraph(
        'Plain paragraph with a ' +
          span('T_bold', 'bold') +
          ' word and an ' +
          span('T_italic', 'italic') +
          ' word.',
      ) +
      paragraph(
        'Two  spaces  survive, and so does the trailing one. ',
        'P_body',
        'xml:space="preserve"',
      ) +
      paragraph('Tab<text:tab/>and<text:s text:c="3"/>three spaces.') +
      paragraph(
        'Visit ' +
          hyperlink('https://example.com/odf', 'T_link', 'example.com/odf') +
          ' for details.',
      ) +
      paragraph('Items:') +
      list('L_bullet', [
        paragraph('First item'),
        paragraph('Second item with ' + span('T_bold', 'emphasis')) +
          list('L_bullet', [paragraph('Nested item')]),
      ]) +
      heading(2, 'Second heading', 'P_h2') +
      paragraph('Tail paragraph.'),
  });

// ── ODT: table_image.odt ────────────────────────────────────

const ODT_TABLE_STYLES =
  '<style:style style:name="Tbl1" style:family="table">' +
  '<style:table-properties style:width="14cm" table:align="margins"/></style:style>' +
  '<style:style style:name="Tbl1.A" style:family="table-column">' +
  '<style:table-column-properties style:column-width="7cm"/></style:style>' +
  '<style:style style:name="Tbl1.Row" style:family="table-row">' +
  '<style:table-row-properties style:row-height="0.9cm"/></style:style>' +
  '<style:style style:name="Tbl1.Head" style:family="table-cell">' +
  '<style:table-cell-properties fo:background-color="#DCE6F1" fo:padding="0.1cm" ' +
  'fo:border="0.5pt solid #808080"/>' +
  '<style:paragraph-properties fo:text-align="center"/>' +
  '<style:text-properties fo:font-weight="bold"/>' +
  '</style:style>' +
  '<style:style style:name="Tbl1.Cell" style:family="table-cell">' +
  '<style:table-cell-properties fo:padding="0.1cm" fo:border="0.5pt solid #808080"/>' +
  '</style:style>';

const ODT_INLINE_GRAPHIC_STYLE =
  '<style:style style:name="fr1" style:family="graphic">' +
  '<style:graphic-properties style:vertical-pos="top" style:vertical-rel="baseline"/>' +
  '</style:style>';

/** Таблица 2×2 с объединённой шапкой и встроенная картинка 1×1. */
const tableImageOdt = (): Buffer =>
  writeOdt({
    title: 'ODF table and image fixture',
    automaticStyles: ODT_PARAGRAPH_STYLES + ODT_TABLE_STYLES + ODT_INLINE_GRAPHIC_STYLE,
    extraParts: [['Pictures/pixel.png', PIXEL_PNG]],
    body:
      heading(1, 'Table and image', 'P_h1') +
      paragraph('Table with a merged header cell:') +
      table(
        'Table1',
        column('Tbl1.A') + column('Tbl1.A'),
        row(
          cell('office:value-type="string"', 'Merged header', {
            style: 'Tbl1.Head',
            columnsSpanned: 2,
          }) + coveredCells(),
          { style: 'Tbl1.Row' },
        ) +
          row(
            cell('office:value-type="string"', 'Left', { style: 'Tbl1.Cell' }) +
              cell('office:value-type="string"', 'Right', { style: 'Tbl1.Cell' }),
            { style: 'Tbl1.Row' },
          ),
        'Tbl1',
      ) +
      paragraph(
        'Inline image ' +
          frame(
            'fr1',
            'text:anchor-type="as-char" svg:width="1cm" svg:height="1cm"',
            image('Pictures/pixel.png'),
          ) +
          ' after the picture.',
      ),
  });

// ── ODT: page_styles.odt ────────────────────────────────────

const ODT_PAGE_TEXT_STYLES =
  '<style:style style:name="Header" style:family="paragraph">' +
  '<style:paragraph-properties fo:text-align="end"/>' +
  '<style:text-properties fo:font-size="9pt" fo:color="#666666"/></style:style>' +
  '<style:style style:name="Footer" style:family="paragraph">' +
  '<style:paragraph-properties fo:text-align="center"/>' +
  '<style:text-properties fo:font-size="9pt" fo:color="#666666"/></style:style>';

const ODT_PAGE_LAYOUTS =
  '<style:page-layout style:name="pm1">' +
  '<style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" ' +
  'style:print-orientation="portrait" fo:margin-top="2cm" fo:margin-bottom="2cm" ' +
  'fo:margin-left="2.5cm" fo:margin-right="2.5cm"/>' +
  '<style:header-style>' +
  '<style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="0.5cm"/>' +
  '</style:header-style>' +
  '<style:footer-style>' +
  '<style:header-footer-properties fo:min-height="1cm" fo:margin-top="0.5cm"/>' +
  '</style:footer-style>' +
  '</style:page-layout>' +
  '<style:page-layout style:name="pm_landscape">' +
  '<style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" ' +
  'style:print-orientation="landscape" fo:margin-top="1.5cm" fo:margin-bottom="1.5cm" ' +
  'fo:margin-left="2cm" fo:margin-right="2cm"/>' +
  '</style:page-layout>';

const ODT_MASTER_PAGES =
  '<style:master-page style:name="Standard" style:page-layout-name="pm1">' +
  '<style:header><text:p text:style-name="Header">Header on every page</text:p></style:header>' +
  '<style:footer><text:p text:style-name="Footer">Footer on every page</text:p></style:footer>' +
  '</style:master-page>' +
  '<style:master-page style:name="Landscape" style:page-layout-name="pm_landscape">' +
  '<style:header><text:p text:style-name="Header">Landscape header</text:p></style:header>' +
  '</style:master-page>';

const ODT_PAGE_BREAK_STYLES =
  '<style:style style:name="P_break" style:family="paragraph">' +
  '<style:paragraph-properties fo:break-before="page"/></style:style>' +
  // style:master-page-name — атрибут самого style:style (так его пишет и
  // LibreOffice), а не paragraph-properties; внутри свойств он игнорируется.
  '<style:style style:name="P_landscape" style:family="paragraph" ' +
  'style:master-page-name="Landscape">' +
  '<style:paragraph-properties fo:break-before="page"/></style:style>';

/** Размеры страницы и поля, колонтитулы, разрыв страницы, смена ориентации. */
const pageStylesOdt = (): Buffer =>
  writeOdt({
    title: 'ODF page styles fixture',
    automaticStyles: ODT_PARAGRAPH_STYLES + ODT_PAGE_BREAK_STYLES,
    styles: ODT_PAGE_TEXT_STYLES,
    stylesAutomatic: ODT_PAGE_LAYOUTS,
    masterStyles: ODT_MASTER_PAGES,
    body:
      heading(1, 'Page styles', 'P_h1') +
      paragraph('First page paragraph.') +
      paragraph('The next paragraph starts a new page.', 'P_break') +
      paragraph('Body of the second page.') +
      paragraph('This paragraph switches to the landscape master page.', 'P_landscape') +
      paragraph('Landscape body text.'),
  });

// ── ODS: simple.ods ─────────────────────────────────────────

const ODS_TABLE_STYLES =
  '<style:style style:name="ta1" style:family="table">' +
  '<style:table-properties table:display="true"/></style:style>' +
  '<style:style style:name="co1" style:family="table-column">' +
  '<style:table-column-properties style:column-width="4cm"/></style:style>' +
  '<style:style style:name="co2" style:family="table-column">' +
  '<style:table-column-properties style:column-width="3cm"/></style:style>' +
  '<style:style style:name="co3" style:family="table-column">' +
  '<style:table-column-properties style:column-width="2cm"/></style:style>' +
  '<style:style style:name="co4" style:family="table-column">' +
  '<style:table-column-properties style:column-width="3cm"/></style:style>' +
  '<style:style style:name="co5" style:family="table-column">' +
  '<style:table-column-properties style:column-width="2.5cm"/></style:style>' +
  '<style:style style:name="ro1" style:family="table-row">' +
  '<style:table-row-properties style:row-height="0.8cm" ' +
  'style:use-optimal-row-height="false"/></style:style>';

const ODS_CELL_STYLES =
  '<style:style style:name="ce_head" style:family="table-cell">' +
  '<style:table-cell-properties fo:background-color="#DCE6F1" ' +
  'fo:border="0.5pt solid #808080" style:vertical-align="middle"/>' +
  '<style:paragraph-properties fo:text-align="center"/>' +
  '<style:text-properties fo:font-weight="bold"/>' +
  '</style:style>' +
  '<style:style style:name="ce_text" style:family="table-cell">' +
  '<style:table-cell-properties fo:border="0.5pt solid #808080"/></style:style>' +
  '<style:style style:name="ce_number" style:family="table-cell" ' +
  'style:data-style-name="N_decimal">' +
  '<style:table-cell-properties fo:border="0.5pt solid #808080"/>' +
  '<style:paragraph-properties fo:text-align="end"/></style:style>' +
  '<style:style style:name="ce_percent" style:family="table-cell" ' +
  'style:data-style-name="N_percent">' +
  '<style:table-cell-properties fo:border="0.5pt solid #808080"/>' +
  '<style:paragraph-properties fo:text-align="end"/></style:style>' +
  '<style:style style:name="ce_date" style:family="table-cell" ' +
  'style:data-style-name="N_date">' +
  '<style:table-cell-properties fo:border="0.5pt solid #808080"/></style:style>' +
  '<style:style style:name="ce_merged" style:family="table-cell">' +
  '<style:table-cell-properties fo:background-color="#FFF2CC" ' +
  'fo:border="0.5pt solid #808080" style:vertical-align="middle"/></style:style>';

const ODS_DATA_STYLES =
  '<number:number-style style:name="N_decimal">' +
  '<number:number number:decimal-places="2" number:min-integer-digits="1"/>' +
  '</number:number-style>' +
  '<number:percentage-style style:name="N_percent">' +
  '<number:number number:decimal-places="0" number:min-integer-digits="1"/>' +
  '<number:text>%</number:text></number:percentage-style>' +
  '<number:date-style style:name="N_date">' +
  '<number:day number:style="long"/><number:text>.</number:text>' +
  '<number:month number:style="long"/><number:text>.</number:text>' +
  '<number:year number:style="long"/></number:date-style>';

/** Два листа: строки/числа/boolean/даты, формулы, объединение, форматы. */
const simpleOds = (): Buffer =>
  writeOds({
    title: 'ODF spreadsheet fixture',
    automaticStyles: ODS_TABLE_STYLES + ODS_CELL_STYLES,
    styles: ODS_DATA_STYLES,
    body:
      table(
        'Data',
        column('co1') +
          column('co2') +
          column('co3') +
          column('co4') +
          column('co5'),
        row(
          cell('office:value-type="string"', 'Item', { style: 'ce_head' }) +
            cell('office:value-type="string"', 'Amount', { style: 'ce_head' }) +
            cell('office:value-type="string"', 'Active', { style: 'ce_head' }) +
            cell('office:value-type="string"', 'When', { style: 'ce_head' }) +
            cell('office:value-type="string"', 'Total', { style: 'ce_head' }),
          { style: 'ro1' },
        ) +
          row(
            cell('office:value-type="string"', 'Widget', { style: 'ce_text' }) +
              cell('office:value-type="float" office:value="12.5"', '12.50', {
                style: 'ce_number',
              }) +
              cell('office:value-type="boolean" office:boolean-value="true"', 'TRUE', {
                style: 'ce_text',
              }) +
              cell('office:value-type="date" office:date-value="2024-01-15"', '15.01.2024', {
                style: 'ce_date',
              }) +
              cell('office:value-type="float" office:value="20"', '20.00', {
                style: 'ce_number',
                formula: 'of:=SUM([.B2:.B3])',
              }),
            { style: 'ro1' },
          ) +
          row(
            cell('office:value-type="string"', 'Gadget', { style: 'ce_text' }) +
              cell('office:value-type="float" office:value="7.5"', '7.50', {
                style: 'ce_number',
              }) +
              cell('office:value-type="boolean" office:boolean-value="false"', 'FALSE', {
                style: 'ce_text',
              }) +
              cell('office:value-type="date" office:date-value="2024-02-20"', '20.02.2024', {
                style: 'ce_date',
              }) +
              cell('office:value-type="float" office:value="20"', '20.00', {
                style: 'ce_number',
                formula: 'of:=SUM([.B2:.B3])',
              }),
            { style: 'ro1' },
          ) +
          row(
            cell('office:value-type="string"', 'Merged A4:B4', {
              style: 'ce_merged',
              columnsSpanned: 2,
            }) +
              coveredCells() +
              cell('office:value-type="float" office:value="0.25"', '25%', {
                style: 'ce_percent',
              }) +
              cell('office:value-type="string"', 'note', { style: 'ce_text' }) +
              cell('office:value-type="float" office:value="0"', '0.00', {
                style: 'ce_number',
              }),
            { style: 'ro1' },
          ),
      ) +
      table(
        'Summary',
        column('co1') + column('co2'),
        row(
          cell('office:value-type="string"', 'Label', { style: 'ce_head' }) +
            cell('office:value-type="string"', 'Value', { style: 'ce_head' }),
          { style: 'ro1' },
        ) +
          row(
            cell('office:value-type="string"', 'Total', { style: 'ce_text' }) +
              cell('office:value-type="float" office:value="20"', '20.00', {
                style: 'ce_number',
                formula: 'of:=SUM([Data.B2:Data.B3])',
              }),
            { style: 'ro1' },
          ),
      ),
  });

// ── ODS: repeated.ods ───────────────────────────────────────

const REPEATED_TABLE_STYLES =
  '<style:style style:name="ta1" style:family="table">' +
  '<style:table-properties table:display="true"/></style:style>' +
  '<style:style style:name="co1" style:family="table-column">' +
  '<style:table-column-properties style:column-width="3cm"/></style:style>' +
  '<style:style style:name="co2" style:family="table-column">' +
  '<style:table-column-properties style:column-width="6cm"/></style:style>' +
  '<style:style style:name="ro1" style:family="table-row">' +
  '<style:table-row-properties style:row-height="0.7cm"/></style:style>' +
  '<style:style style:name="ce_head" style:family="table-cell">' +
  '<style:table-cell-properties fo:background-color="#E2EFDA" ' +
  'fo:border="0.5pt solid #808080"/>' +
  '<style:text-properties fo:font-weight="bold"/></style:style>';

/** Повторяющиеся строки/столбцы и объединения на 2 столбца и 2 строки. */
const repeatedOds = (): Buffer =>
  writeOds({
    title: 'ODF repeated cells fixture',
    automaticStyles: REPEATED_TABLE_STYLES,
    body: table(
      'Repeated',
      column('co1', 3) + column('co2'),
      row(
        cell('office:value-type="string"', 'A', { style: 'ce_head' }) +
          cell('office:value-type="string"', 'B', { style: 'ce_head' }) +
          cell('office:value-type="string"', 'C', { style: 'ce_head' }) +
          cell('office:value-type="string"', 'D', { style: 'ce_head' }),
        { style: 'ro1' },
      ) +
        row(
          cell('office:value-type="string"', 'same', { style: 'ce_head' }) +
            cell('office:value-type="float" office:value="1"', '1') +
            emptyCell(2),
          { style: 'ro1', repeat: 4 },
        ) +
        row(
          cell('office:value-type="string"', 'Span 2x2', {
            columnsSpanned: 2,
            rowsSpanned: 2,
          }) +
            coveredCells() +
            cell('office:value-type="string"', 'C6') +
            cell('office:value-type="string"', 'D6'),
          { style: 'ro1' },
        ) +
        row(
          coveredCells(2) +
            cell('office:value-type="string"', 'C7') +
            cell('office:value-type="string"', 'D7'),
          { style: 'ro1' },
        ) +
        row(emptyCell(3) + cell('office:value-type="string"', 'D8')),
    ),
  });

// ── ODP: simple.odp ─────────────────────────────────────────

const ODP_GRAPHIC_STYLES =
  '<style:style style:name="gr_title" style:family="graphic">' +
  '<style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>' +
  '<style:style style:name="gr_body" style:family="graphic">' +
  '<style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>' +
  '<style:style style:name="gr_rect" style:family="graphic">' +
  '<style:graphic-properties draw:stroke="solid" svg:stroke-color="#1F3864" ' +
  'svg:stroke-width="0.03cm" draw:fill="solid" draw:fill-color="#4472C4"/></style:style>' +
  '<style:style style:name="gr_pic" style:family="graphic">' +
  '<style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>';

const ODP_PARAGRAPH_STYLES =
  paragraphStyle('P_title', 'fo:text-align="center"') +
  paragraphStyle('P_body', 'fo:text-align="start"', 'fo:font-size="18pt"');

const ODP_TEXT_STYLES =
  textStyle('T_title', 'fo:font-size="32pt" fo:font-weight="bold" fo:color="#1F3864"') +
  textStyle('T_bold', 'fo:font-weight="bold"') +
  textStyle('T_italic', 'fo:font-style="italic"');

const ODP_PAGE_LAYOUT =
  '<style:page-layout style:name="pm1">' +
  '<style:page-layout-properties fo:page-width="28cm" fo:page-height="15.75cm" ' +
  'style:print-orientation="landscape" fo:margin-top="1cm" fo:margin-bottom="1cm" ' +
  'fo:margin-left="1cm" fo:margin-right="1cm"/></style:page-layout>';

const ODP_MASTER_PAGE = '<style:master-page style:name="Standard" style:page-layout-name="pm1"/>';

const TITLE_FRAME_GEOMETRY =
  'draw:layer="layout" svg:x="2cm" svg:y="1.1cm" svg:width="24cm" svg:height="2cm"';
const BODY_FRAME_GEOMETRY =
  'draw:layer="layout" svg:x="2cm" svg:y="3.8cm" svg:width="14cm" svg:height="4cm"';
const BULLET_FRAME_GEOMETRY =
  'draw:layer="layout" svg:x="2cm" svg:y="3.8cm" svg:width="24cm" svg:height="6cm"';

/** Два слайда: текст-бокс с прогонами, прямоугольник с заливкой и картинка. */
const simpleOdp = (): Buffer =>
  writeOdp({
    title: 'ODF presentation fixture',
    automaticStyles: ODP_GRAPHIC_STYLES + ODP_TEXT_STYLES + ODP_PARAGRAPH_STYLES,
    styles: LIST_STYLE_BULLET,
    stylesAutomatic: ODP_PAGE_LAYOUT,
    masterStyles: ODP_MASTER_PAGE,
    extraParts: [['Pictures/pixel.png', PIXEL_PNG]],
    body:
      slide(
        'Slide 1',
        frame(
          'gr_title',
          TITLE_FRAME_GEOMETRY,
          textBox(paragraph(span('T_title', 'Slides in ODF'), 'P_title')),
        ) +
          frame(
            'gr_body',
            BODY_FRAME_GEOMETRY,
            textBox(
              paragraph(
                'Text box with ' +
                  span('T_bold', 'bold') +
                  ' and ' +
                  span('T_italic', 'italic') +
                  ' runs.',
              ) +
                paragraph(
                  'Second  paragraph  keeps  spaces.',
                  'P_body',
                  'xml:space="preserve"',
                ),
            ),
          ) +
          '<draw:rect draw:style-name="gr_rect" draw:layer="layout" ' +
          'svg:x="17cm" svg:y="3.8cm" svg:width="9cm" svg:height="4cm"/>' +
          frame(
            'gr_pic',
            'draw:layer="layout" svg:x="17cm" svg:y="9cm" svg:width="3cm" svg:height="3cm"',
            image('Pictures/pixel.png'),
          ),
      ) +
      slide(
        'Slide 2',
        frame(
          'gr_title',
          TITLE_FRAME_GEOMETRY,
          textBox(paragraph(span('T_title', 'Slide two'), 'P_title')),
        ) +
          frame(
            'gr_body',
            BULLET_FRAME_GEOMETRY,
            textBox(
              list('L_bullet', [
                paragraph('Bullet one'),
                paragraph('Bullet two'),
              ]),
            ),
          ),
      ),
  });

// ── ODP: text_styles.odp ────────────────────────────────────

const TEXT_STYLES_ODP =
  textStyle('T_title', 'fo:font-size="40pt" fo:font-weight="bold" fo:color="#1F3864"') +
  textStyle('T_sub', 'fo:font-size="24pt" fo:font-style="italic" fo:color="#C00000"') +
  textStyle('T_big', 'fo:font-size="18pt" fo:font-weight="bold" fo:color="#2E75B6"') +
  textStyle('T_small', 'fo:font-size="11pt" fo:color="#595959"') +
  textStyle('T_accent', 'fo:font-size="14pt" fo:color="#C55A11"') +
  textStyle('T_bold', 'fo:font-weight="bold"');

/** Заголовок и абзацы разных кеглей/цветов, список с вложенным уровнем. */
const textStylesOdp = (): Buffer =>
  writeOdp({
    title: 'ODF text styles fixture',
    automaticStyles: ODP_GRAPHIC_STYLES + TEXT_STYLES_ODP + ODP_PARAGRAPH_STYLES,
    styles: LIST_STYLE_BULLET,
    stylesAutomatic: ODP_PAGE_LAYOUT,
    masterStyles: ODP_MASTER_PAGE,
    body: slide(
      'Text styles',
      frame(
        'gr_title',
        TITLE_FRAME_GEOMETRY,
        textBox(paragraph(span('T_title', 'Text styles'), 'P_title')),
      ) +
        frame(
          'gr_body',
          'draw:layer="layout" svg:x="2cm" svg:y="4.2cm" svg:width="24cm" svg:height="9cm"',
          textBox(
            paragraph(span('T_sub', 'Subtitle 24 pt')) +
              paragraph(span('T_big', 'Heading 18 pt bold')) +
              paragraph(span('T_small', 'Small 11 pt grey text.')) +
              paragraph(
                'Mixed ' +
                  span('T_bold', 'bold') +
                  ' and ' +
                  span('T_accent', 'accent 14 pt') +
                  ' runs.',
              ) +
              list('L_bullet', [
                paragraph(span('T_big', 'Level one item')),
                paragraph(span('T_small', 'Small level one item')) +
                  list('L_bullet', [paragraph(span('T_accent', 'Nested accent item'))]),
              ]),
          ),
        ),
    ),
  });

// ── Запуск ──────────────────────────────────────────────────

interface Fixture {
  readonly name: string;
  readonly kind: OdfKind;
  readonly build: () => Buffer;
}

const FIXTURES: readonly Fixture[] = [
  { name: 'odt/simple.odt', kind: ODT, build: simpleOdt },
  { name: 'odt/table_image.odt', kind: ODT, build: tableImageOdt },
  { name: 'odt/page_styles.odt', kind: ODT, build: pageStylesOdt },
  { name: 'ods/simple.ods', kind: ODS, build: simpleOds },
  { name: 'ods/repeated.ods', kind: ODS, build: repeatedOds },
  { name: 'odp/simple.odp', kind: ODP, build: simpleOdp },
  { name: 'odp/text_styles.odp', kind: ODP, build: textStylesOdp },
];

const validate = (fixtures: readonly Fixture[]): void => {
  const seen = new Set<string>();
  for (const fixture of fixtures) {
    if (seen.has(fixture.name)) throw new Error(`duplicate fixture name: ${fixture.name}`);
    if (!fixture.name.includes('/')) throw new Error(`fixture without category prefix: ${fixture.name}`);
    const extension = path.extname(fixture.name);
    if (extension !== `.${fixture.kind.extension}`) {
      throw new Error(`${fixture.name}: expected extension .${fixture.kind.extension}`);
    }
    seen.add(fixture.name);
  }
};

/**
 * Проверяет инвариант ODF: `mimetype` — первая запись, без сжатия, без
 * extra-поля и без data descriptor. Некоторые читатели ищут его по смещению
 * 30 и не терпят ни сжатия, ни лишних полей.
 */
const assertMimetypeFirst = (bytes: Buffer, expected: string): void => {
  if (bytes.readUInt32LE(0) !== SIG_LOCAL) {
    throw new Error('archive does not start with a local file header');
  }
  const flags = bytes.readUInt16LE(6);
  const method = bytes.readUInt16LE(8);
  const compressedSize = bytes.readUInt32LE(18);
  const nameLength = bytes.readUInt16LE(26);
  const extraLength = bytes.readUInt16LE(28);
  const name = bytes.subarray(30, 30 + nameLength).toString('utf8');
  if (name !== 'mimetype') throw new Error(`first entry is ${name}, expected mimetype`);
  if (method !== METHOD_STORE) throw new Error('mimetype entry is compressed');
  if (flags !== 0) throw new Error('mimetype entry uses general-purpose flags');
  if (extraLength !== 0) throw new Error('mimetype entry has an extra field');
  const payload = bytes
    .subarray(30 + nameLength, 30 + nameLength + compressedSize)
    .toString('utf8');
  if (payload !== expected) throw new Error(`mimetype is ${payload}, expected ${expected}`);
};

const main = async (): Promise<void> => {
  validate(FIXTURES);
  // README.md лежит в корне каталога и генератору не принадлежит: чистим
  // только каталоги форматов, иначе документация исчезала бы при каждом прогоне.
  for (const directory of ['odt', 'ods', 'odp']) {
    await rm(path.join(OUT_DIR, directory), { recursive: true, force: true });
  }
  await mkdir(OUT_DIR, { recursive: true });
  for (const fixture of FIXTURES) {
    const bytes = fixture.build();
    assertMimetypeFirst(bytes, fixture.kind.mimetype);
    const file = path.join(OUT_DIR, fixture.name);
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, bytes);
    console.log(`  ${fixture.name.padEnd(24)} ${bytes.length} B`);
  }
  console.log(`OK: ${FIXTURES.length} фикстур ODF -> test-fixtures/odf/`);
};

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
