// Фикстуры категории rtl: арабский и иврит, смешанное направление, RTL-список
// и таблица с `w:bidiVisual`.
//
// RTL-оформление задаётся тремя уровнями: стиль абзаца/прогона (`stylesExtra`),
// сам абзац (`w:bidi`) и прогон (`w:rtl`) — фикстуры проверяют, что парсер
// видит все три.

import {
    type FixtureSpec,
    W_NS,
    XML_DECL,
    defaultSectPr,
    packageParts,
    p,
    para,
    run,
    sectionProps,
    tr,
} from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** RTL-стили поверх `defaultStylesXml` (вставляются перед `</w:styles>`). */
const rtlStyles = (): string =>
    '<w:style w:type="paragraph" w:styleId="RtlParagraph"><w:name w:val="RTL Paragraph"/>' +
    '<w:basedOn w:val="Normal"/><w:pPr><w:bidi/><w:jc w:val="right"/></w:pPr></w:style>' +
    '<w:style w:type="character" w:customStyle="1" w:styleId="RtlRun"><w:name w:val="RTL Run"/>' +
    '<w:basedOn w:val="DefaultParagraphFont"/><w:rPr><w:rtl/></w:rPr></w:style>';

/** Нумерация с RTL-уровнями: маркер справа, отступ справа. */
const numberingXml = (): string =>
    XML_DECL +
    `<w:numbering xmlns:w="${W_NS}">` +
    '<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>' +
    '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/>' +
    '<w:lvlJc w:val="right"/><w:pPr><w:bidi/><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>' +
    '<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2."/>' +
    '<w:lvlJc w:val="right"/><w:pPr><w:bidi/><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl>' +
    '</w:abstractNum>' +
    '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>' +
    '</w:numbering>';

/** Ячейка с RTL-абзацем. */
const rtlCell = (text: string): string =>
    '<w:tc><w:tcPr><w:tcW w:w="4675" w:type="dxa"/></w:tcPr>' +
    para('<w:bidi/><w:jc w:val="right"/>', run(text, { rtl: true })) +
    '</w:tc>';

/** Таблица 2×2 с направлением справа налево. */
const rtlTable = (cells: string[][]): string =>
    '<w:tbl><w:tblPr><w:bidiVisual/><w:tblW w:w="0" w:type="auto"/>' +
    '<w:tblBorders>' +
    ['top', 'left', 'bottom', 'right', 'insideH', 'insideV']
        .map((side) => `<w:${side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>`)
        .join('') +
    '</w:tblBorders></w:tblPr>' +
    '<w:tblGrid><w:gridCol w:w="4675"/><w:gridCol w:w="4675"/></w:tblGrid>' +
    cells.map((row) => tr(row.map(rtlCell).join(''))).join('') +
    '</w:tbl>';

export const rtlFixtures: FixtureSpec[] = [
    {
        name: 'rtl/arabic_basic',
        description: 'Арабский абзац: bidi и RTL-прогон',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(p('مرحبا بالعالم', { style: 'RtlParagraph', bidi: true, rtl: true })),
            stylesExtra: rtlStyles(),
        }),
        content: {
            direction: 'rtl',
            paragraphs: [{ text: 'مرحبا بالعالم', style: 'RtlParagraph', bidi: true, rtl: true }],
        },
    },
    {
        name: 'rtl/hebrew_basic',
        description: 'Иврит: bidi и RTL-прогон',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(p('שלום עולם', { style: 'RtlParagraph', bidi: true, rtl: true })),
            stylesExtra: rtlStyles(),
        }),
        content: {
            direction: 'rtl',
            paragraphs: [{ text: 'שלום עולם', style: 'RtlParagraph', bidi: true, rtl: true }],
        },
    },
    {
        name: 'rtl/mixed_direction',
        description: 'Смешанное направление: арабский и английский в одном абзаце',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '<w:bidi/><w:jc w:val="right"/>',
                    run('مرحبا', { rtl: true }) + run(' Hello World'),
                ),
            ),
            stylesExtra: rtlStyles(),
        }),
        content: {
            direction: 'rtl',
            paragraphs: [
                {
                    text: 'مرحبا Hello World',
                    bidi: true,
                    runs: [
                        { text: 'مرحبا', rtl: true },
                        { text: ' Hello World', rtl: false },
                    ],
                },
            ],
        },
    },
    {
        name: 'rtl/rtl_numbered_list',
        description: 'RTL-список с нумерацией: два уровня',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                p('العنصر الأول', { numId: 1, ilvl: 0, bidi: true, rtl: true }),
                p('العنصر الثاني', { numId: 1, ilvl: 0, bidi: true, rtl: true }),
                p('العنصر الفرعي', { numId: 1, ilvl: 1, bidi: true, rtl: true }),
            ),
            numbering: numberingXml(),
            stylesExtra: rtlStyles(),
        }),
        content: {
            direction: 'rtl',
            paragraphs: [
                { text: 'العنصر الأول', numId: 1, ilvl: 0, bidi: true },
                { text: 'العنصر الثاني', numId: 1, ilvl: 0, bidi: true },
                { text: 'العنصر الفرعي', numId: 1, ilvl: 1, bidi: true },
            ],
            numbering: { numId: 1, abstractNumId: 0, levels: 2 },
        },
    },
    {
        name: 'rtl/rtl_table',
        description: 'Таблица с bidiVisual и RTL-текстом в ячейках',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                rtlTable([
                    ['الخلية أ١', 'الخلية ب١'],
                    ['الخلية أ٢', 'الخلية ب٢'],
                ]),
            ),
            stylesExtra: rtlStyles(),
        }),
        content: {
            direction: 'rtl',
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 2,
                    bidiVisual: true,
                    cells: [
                        ['الخلية أ١', 'الخلية ب١'],
                        ['الخلية أ٢', 'الخلية ب٢'],
                    ],
                },
            ],
        },
    },
];
