// Фикстуры категории styles/: разрешение стилей абзацев и прогонов —
// docDefaults, w:default, цепочки basedOn, linked-стили, qFormat/latentStyles,
// табличные и character-стили, прямое форматирование поверх стиля.

import {
    type FixtureSpec,
    R_NS,
    W_NS,
    XML_DECL,
    defaultSectPr,
    p,
    packageParts,
    para,
    run,
    sectionProps,
    table,
} from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** docDefaults, который пишет Word: Calibri 11 pt, интервал 1.08 строки. */
const WORD_DOC_DEFAULTS =
    '<w:docDefaults>' +
    '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="SimSun" w:cs="Arial"/>' +
    '<w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="en-US" w:eastAsia="zh-CN" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>' +
    '<w:pPrDefault><w:pPr><w:spacing w:before="0" w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault>' +
    '</w:docDefaults>';

/**
 * styles.xml целиком: docDefaults, необязательный latentStyles (по схеме —
 * сразу за docDefaults) и дефолтные Normal/DefaultParagraphFont/TableNormal.
 */
const stylesDocument = (
    styles: string,
    opts: { docDefaults?: string; latentStyles?: string } = {},
): string =>
    XML_DECL +
    `<w:styles xmlns:w="${W_NS}" xmlns:r="${R_NS}">` +
    (opts.docDefaults ?? WORD_DOC_DEFAULTS) +
    (opts.latentStyles ?? '') +
    '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>' +
    '<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont">' +
    '<w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style>' +
    '<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/>' +
    '<w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/></w:style>' +
    styles +
    '</w:styles>';

/** Абзацный стиль без `w:default` — проверяем, что без pStyle побеждает Normal. */
const BODY_TEXT_STYLE =
    '<w:style w:type="paragraph" w:customStyle="1" w:styleId="BodyText"><w:name w:val="Body Text"/>' +
    '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="1"/><w:qFormat/>' +
    '<w:pPr><w:spacing w:after="120"/></w:pPr>' +
    '<w:rPr><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia"/><w:sz w:val="24"/></w:rPr></w:style>';

/** Табличный стиль с рамками: применяется абзацем-таблицей через `w:tblStyle`. */
const BANDED_GRID_STYLE =
    '<w:style w:type="table" w:customStyle="1" w:styleId="BandedGrid"><w:name w:val="Banded Grid"/>' +
    '<w:basedOn w:val="TableNormal"/><w:uiPriority w:val="50"/><w:qFormat/>' +
    '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>' +
    '<w:tblPr><w:tblBorders>' +
    ['top', 'left', 'bottom', 'right', 'insideH', 'insideV']
        .map((side) => `<w:${side} w:val="single" w:sz="4" w:space="0" w:color="7F7F7F"/>`)
        .join('') +
    '</w:tblBorders><w:tblCellMar><w:top w:w="57" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>' +
    '<w:bottom w:w="57" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr>' +
    '<w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr></w:tblStylePr></w:style>';

/** Character-стиль и унаследованный от него: применяются через `w:rStyle`. */
const STRONG_RED_STYLE =
    '<w:style w:type="character" w:customStyle="1" w:styleId="StrongRed"><w:name w:val="Strong Red"/>' +
    '<w:basedOn w:val="DefaultParagraphFont"/><w:uiPriority w:val="22"/><w:qFormat/>' +
    '<w:rPr><w:b/><w:color w:val="C00000"/><w:sz w:val="26"/></w:rPr></w:style>' +
    '<w:style w:type="character" w:customStyle="1" w:styleId="StrongRedUnderline">' +
    '<w:name w:val="Strong Red Underline"/><w:basedOn w:val="StrongRed"/><w:uiPriority w:val="22"/><w:qFormat/>' +
    '<w:rPr><w:u w:val="single"/></w:rPr></w:style>';

/** Абзацный стиль с курсивом и цветом — поверх него ложится прямое форматирование. */
const QUOTE_STYLE =
    '<w:style w:type="paragraph" w:customStyle="1" w:styleId="Quote"><w:name w:val="Quote"/>' +
    '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="29"/><w:qFormat/>' +
    '<w:pPr><w:spacing w:before="200" w:after="200"/><w:ind w:left="720" w:right="720"/></w:pPr>' +
    '<w:rPr><w:i/><w:color w:val="595959"/></w:rPr></w:style>';

export const stylesFixtures: FixtureSpec[] = [
    {
        name: 'styles/default_paragraph',
        description: 'Абзац без pStyle резолвится в стиль Normal с w:default="1"',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(p('Абзац без pStyle наследует Normal')),
            stylesExtra: BODY_TEXT_STYLE,
        }),
        content: {
            paragraphs: [{ text: 'Абзац без pStyle наследует Normal', style: 'Normal' }],
            tables: [],
            images: [],
            styles: [
                { id: 'Normal', type: 'paragraph', default: true, qFormat: true },
                { id: 'DefaultParagraphFont', type: 'character', default: true },
                { id: 'BodyText', type: 'paragraph', default: false, basedOn: 'Normal' },
            ],
        },
    },
    {
        name: 'styles/based_on_chain',
        description: 'Цепочка basedOn из трёх уровней: Heading3 → Heading2 → Heading1 → Normal',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(p('Заголовок конца цепочки', { style: 'Heading3' })),
            styles: stylesDocument(
                '<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/>' +
                    '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/>' +
                    '<w:pPr><w:keepNext/><w:spacing w:before="240" w:after="60"/><w:outlineLvl w:val="0"/></w:pPr>' +
                    '<w:rPr><w:b/><w:color w:val="2F5496"/><w:sz w:val="32"/></w:rPr></w:style>' +
                    '<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/>' +
                    '<w:basedOn w:val="Heading1"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/>' +
                    '<w:pPr><w:outlineLvl w:val="1"/></w:pPr>' +
                    '<w:rPr><w:color w:val="2F5496"/><w:sz w:val="26"/></w:rPr></w:style>' +
                    '<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/>' +
                    '<w:basedOn w:val="Heading2"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/>' +
                    '<w:pPr><w:outlineLvl w:val="2"/></w:pPr>' +
                    '<w:rPr><w:color w:val="1F3763"/><w:sz w:val="24"/><w:i/></w:rPr></w:style>',
            ),
        }),
        content: {
            paragraphs: [{ text: 'Заголовок конца цепочки', style: 'Heading3' }],
            tables: [],
            images: [],
            styles: [
                { id: 'Normal', type: 'paragraph', default: true },
                { id: 'Heading1', type: 'paragraph', basedOn: 'Normal', bold: true, color: '2F5496' },
                { id: 'Heading2', type: 'paragraph', basedOn: 'Heading1', color: '2F5496' },
                { id: 'Heading3', type: 'paragraph', basedOn: 'Heading2', italic: true },
            ],
        },
    },
    {
        name: 'styles/linked_character',
        description: 'Параграфный стиль с w:link на character-стиль; абзац и прогон со связанными стилями',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '<w:pStyle w:val="Heading1"/>',
                    run('Заголовок с ') + run('linked-прогоном', { style: 'Heading1Char' }),
                ),
            ),
        }),
        content: {
            paragraphs: [
                {
                    text: 'Заголовок с linked-прогоном',
                    style: 'Heading1',
                    runs: [
                        { text: 'Заголовок с ' },
                        { text: 'linked-прогоном', style: 'Heading1Char' },
                    ],
                },
            ],
            tables: [],
            images: [],
            styles: [
                { id: 'Heading1', type: 'paragraph', link: 'Heading1Char', bold: true, color: '2F5496' },
                { id: 'Heading1Char', type: 'character', link: 'Heading1', basedOn: 'DefaultParagraphFont' },
            ],
        },
    },
    {
        name: 'styles/doc_defaults',
        description: 'docDefaults задаёт шрифт, размер и интервалы; абзац без прямого форматирования их наследует',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(p('Абзац наследует docDefaults')),
            styles: stylesDocument('', {
                docDefaults:
                    '<w:docDefaults>' +
                    '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia" w:eastAsia="MS Mincho" w:cs="Georgia"/>' +
                    '<w:sz w:val="26"/><w:szCs w:val="26"/><w:color w:val="1A1A1A"/>' +
                    '<w:lang w:val="ru-RU" w:eastAsia="ja-JP" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>' +
                    '<w:pPrDefault><w:pPr><w:spacing w:before="0" w:after="240" w:line="360" w:lineRule="auto"/>' +
                    '<w:jc w:val="both"/></w:pPr></w:pPrDefault>' +
                    '</w:docDefaults>',
            }),
        }),
        content: {
            paragraphs: [{ text: 'Абзац наследует docDefaults', style: 'Normal' }],
            tables: [],
            images: [],
            docDefaults: {
                font: 'Georgia',
                sizeHalfPoints: 26,
                color: '1A1A1A',
                spacingAfterTwips: 240,
                lineTwips: 360,
                justification: 'both',
            },
            styles: [{ id: 'Normal', type: 'paragraph', default: true }],
        },
    },
    {
        name: 'styles/table_style',
        description: 'Табличный стиль с tblPr (рамки) применяется таблицей через w:tblStyle',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                table(
                    [
                        ['Заголовок 1', 'Заголовок 2'],
                        ['Ячейка 1', 'Ячейка 2'],
                    ],
                    { styleId: 'BandedGrid' },
                ),
            ),
            stylesExtra: BANDED_GRID_STYLE,
        }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 2,
                    style: 'BandedGrid',
                    cells: [
                        ['Заголовок 1', 'Заголовок 2'],
                        ['Ячейка 1', 'Ячейка 2'],
                    ],
                },
            ],
            images: [],
            styles: [
                { id: 'TableNormal', type: 'table', default: true },
                {
                    id: 'BandedGrid',
                    type: 'table',
                    basedOn: 'TableNormal',
                    borders: { val: 'single', sz: 4, color: '7F7F7F', sides: 'all' },
                },
            ],
        },
    },
    {
        name: 'styles/qformat_latent',
        description: 'w:latentStyles и флаги qFormat/uiPriority/semiHidden на стилях',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                p('Абзац со стилем из qFormat', { style: 'FancyQuote' }),
                p('Абзац со скрытым стилем', { style: 'HiddenNote' }),
            ),
            styles: stylesDocument(
                '<w:style w:type="paragraph" w:customStyle="1" w:styleId="FancyQuote"><w:name w:val="Fancy Quote"/>' +
                    '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="31"/><w:qFormat/>' +
                    '<w:pPr><w:ind w:left="720" w:right="720"/></w:pPr>' +
                    '<w:rPr><w:i/><w:color w:val="4472C4"/></w:rPr></w:style>' +
                    '<w:style w:type="paragraph" w:customStyle="1" w:styleId="HiddenNote"><w:name w:val="Hidden Note"/>' +
                    '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="99"/>' +
                    '<w:semiHidden/><w:unhideWhenUsed/>' +
                    '<w:rPr><w:sz w:val="18"/></w:rPr></w:style>',
                {
                    latentStyles:
                        '<w:latentStyles w:defLockedState="0" w:defUIPriority="99" w:defSemiHidden="0" ' +
                        'w:defUnhideWhenUsed="0" w:defQFormat="0" w:count="376">' +
                        '<w:lsdException w:name="Normal" w:uiPriority="0" w:qFormat="1"/>' +
                        '<w:lsdException w:name="heading 1" w:uiPriority="9" w:qFormat="1"/>' +
                        '<w:lsdException w:name="Subtitle" w:uiPriority="11" w:semiHidden="1" w:unhideWhenUsed="1"/>' +
                        '</w:latentStyles>',
                },
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Абзац со стилем из qFormat', style: 'FancyQuote' },
                { text: 'Абзац со скрытым стилем', style: 'HiddenNote' },
            ],
            tables: [],
            images: [],
            latentStyles: {
                count: 376,
                exceptions: [
                    { name: 'Normal', qFormat: true, uiPriority: 0 },
                    { name: 'heading 1', qFormat: true, uiPriority: 9 },
                    { name: 'Subtitle', semiHidden: true, unhideWhenUsed: true, uiPriority: 11 },
                ],
            },
            styles: [
                { id: 'FancyQuote', type: 'paragraph', qFormat: true, uiPriority: 31, italic: true },
                { id: 'HiddenNote', type: 'paragraph', semiHidden: true, unhideWhenUsed: true, uiPriority: 99 },
            ],
        },
    },
    {
        name: 'styles/character_style_run',
        description: 'Кастомный character-стиль применяется к прогону через w:rStyle',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '',
                    run('Обычный текст, ') +
                        run('жирный красный', { style: 'StrongRed' }) +
                        run(' и подчёркнутый по наследству', { style: 'StrongRedUnderline' }),
                ),
            ),
            stylesExtra: STRONG_RED_STYLE,
        }),
        content: {
            paragraphs: [
                {
                    text: 'Обычный текст, жирный красный и подчёркнутый по наследству',
                    runs: [
                        { text: 'Обычный текст, ' },
                        { text: 'жирный красный', style: 'StrongRed' },
                        { text: ' и подчёркнутый по наследству', style: 'StrongRedUnderline' },
                    ],
                },
            ],
            tables: [],
            images: [],
            styles: [
                {
                    id: 'StrongRed',
                    type: 'character',
                    basedOn: 'DefaultParagraphFont',
                    bold: true,
                    color: 'C00000',
                    sizeHalfPoints: 26,
                },
                {
                    id: 'StrongRedUnderline',
                    type: 'character',
                    basedOn: 'StrongRed',
                    underline: true,
                },
            ],
        },
    },
    {
        name: 'styles/direct_formatting_override',
        description: 'Прямое форматирование прогона (w:b, w:sz, w:color) перекрывает стиль',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '<w:pStyle w:val="Quote"/>',
                    run('Прямое форматирование', { style: 'Quote', bold: true, size: 36, color: '008000' }) +
                        run(' остальное из стиля', { style: 'Quote' }),
                ),
            ),
            stylesExtra: QUOTE_STYLE,
        }),
        content: {
            paragraphs: [
                {
                    text: 'Прямое форматирование остальное из стиля',
                    style: 'Quote',
                    runs: [
                        {
                            text: 'Прямое форматирование',
                            style: 'Quote',
                            bold: true,
                            sizeHalfPoints: 36,
                            color: '008000',
                        },
                        { text: ' остальное из стиля', style: 'Quote' },
                    ],
                },
            ],
            tables: [],
            images: [],
            styles: [
                {
                    id: 'Quote',
                    type: 'paragraph',
                    basedOn: 'Normal',
                    italic: true,
                    color: '595959',
                },
            ],
        },
    },
];
