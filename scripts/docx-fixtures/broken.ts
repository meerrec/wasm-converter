// Битые пакеты по ADR-0016. Фатальные случаи (ZIP, отсутствие document.xml,
// макросы) возвращают `Err` — в сайдкар кладётся `meta.fatal` с вариантом
// `ParseError`, модель не строится, счётчики нулевые. Восстановимые случаи дают
// warning и частичную модель, поэтому счётчики у них настоящие.
//
// `broken/truncated_zip` кладёт в `bytes` готовый обрезанный архив: ZIP без EOCD
// штатно не собирается, поэтому генератор пишет байты как есть.

import { buildZip } from '../docx-zip.js';
import { type FixtureSpec, CT_NS, XML_DECL, W_NS, defaultSectPr, omit, p, packageParts, sectionProps } from './kit.js';

/** Тело из блоков + финальный `w:sectPr`. */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

const intactZip = buildZip(packageParts({ body: body(p('Text that never gets parsed')) }));

/** Циклический basedOn: StyleA → StyleB → StyleA. */
const CYCLE_STYLES =
    '<w:style w:type="paragraph" w:styleId="StyleA"><w:name w:val="Style A"/><w:basedOn w:val="StyleB"/></w:style>' +
    '<w:style w:type="paragraph" w:styleId="StyleB"><w:name w:val="Style B"/><w:basedOn w:val="StyleA"/></w:style>';

/** Нумерация: рабочий abstractNum 0 и `w:num` 7, ссылающийся на несуществующий abstractNum 99. */
const BROKEN_NUMBERING =
    XML_DECL +
    `<w:numbering xmlns:w="${W_NS}">` +
    '<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>' +
    '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/>' +
    '<w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720"/></w:pPr></w:lvl></w:abstractNum>' +
    '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>' +
    '<w:num w:numId="7"><w:abstractNumId w:val="99"/></w:num>' +
    '</w:numbering>';

/** Content types макросного документа: main part — `macroEnabled.main+xml`. */
const MACRO_CONTENT_TYPES =
    XML_DECL +
    `<Types xmlns="${CT_NS}">` +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="png" ContentType="image/png"/>' +
    '<Override PartName="/word/document.xml" ContentType="application/vnd.ms-word.document.macroEnabled.main+xml"/>' +
    '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>' +
    '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>' +
    '<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>' +
    '</Types>';

const truncatedZip: FixtureSpec = {
    name: 'broken/truncated_zip',
    description: 'Обрезанный ZIP: последние 200 байт с EOCD отрезаны',
    expectedParagraphs: 0,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    meta: { fatal: 'Zip' },
    parts: [],
    bytes: intactZip.subarray(0, intactZip.length - 200),
};

const missingRootRels: FixtureSpec = {
    name: 'broken/missing_root_rels',
    description: 'Нет _rels/.rels: содержимое восстанавливается по стандартному пути',
    expectedParagraphs: 2,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: ['MissingRels'],
    parts: omit(packageParts({ body: body(p('First paragraph'), p('Second paragraph')) }), ['_rels/.rels']),
    content: { paragraphs: [{ text: 'First paragraph' }, { text: 'Second paragraph' }] },
};

const cyclicBasedOn: FixtureSpec = {
    name: 'broken/cyclic_based_on',
    description: 'Циклический basedOn: StyleA → StyleB → StyleA',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: ['CyclicBasedOn'],
    parts: packageParts({
        body: body(p('Paragraph with cyclic style', { style: 'StyleA' })),
        stylesExtra: CYCLE_STYLES,
    }),
    content: {
        paragraphs: [{ text: 'Paragraph with cyclic style', style: 'StyleA' }],
        styles: ['StyleA', 'StyleB'],
    },
};

const missingStyleAndAbstractNum: FixtureSpec = {
    name: 'broken/missing_style_and_abstract_num',
    description: 'Ссылка на несуществующий стиль и numId без abstractNum',
    expectedParagraphs: 2,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: ['MissingStyleRef', 'MissingAbstractNum'],
    parts: packageParts({
        body: body(
            p('Paragraph with missing style', { style: 'NoSuchStyle' }),
            p('Paragraph with broken numbering', { numId: 7 }),
        ),
        numbering: BROKEN_NUMBERING,
    }),
    content: {
        paragraphs: [
            { text: 'Paragraph with missing style', style: 'NoSuchStyle' },
            { text: 'Paragraph with broken numbering', numId: 7 },
        ],
    },
};

const noDocumentXml: FixtureSpec = {
    name: 'broken/no_document_xml',
    description: 'Нет word/document.xml — разбирать нечего',
    expectedParagraphs: 0,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    meta: { fatal: 'MissingDocumentXml' },
    parts: omit(packageParts({ body: body(p('Never parsed')) }), ['word/document.xml']),
};

const macroEnabled: FixtureSpec = {
    name: 'broken/macro_enabled',
    description: 'Макросный документ (.docm) с расширением .docx — non-goal v1',
    expectedParagraphs: 0,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    meta: { fatal: 'MacroEnabledDocument' },
    parts: packageParts({
        body: body(p('Macro-enabled document')),
        contentTypes: MACRO_CONTENT_TYPES,
    }),
};

/** Шесть фикстур категории `broken`. */
export const brokenFixtures: FixtureSpec[] = [
    truncatedZip,
    missingRootRels,
    cyclicBasedOn,
    missingStyleAndAbstractNum,
    noDocumentXml,
    macroEnabled,
];
