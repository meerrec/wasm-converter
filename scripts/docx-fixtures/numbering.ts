// Фикстуры категории numbering/: определения списков (abstractNum/num),
// уровни, bullet-символы, lvlOverride/startOverride и форматы нумерации.

import { type FixtureSpec, W_NS, XML_DECL, defaultSectPr, p, packageParts, sectionProps } from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Абзацы списка ссылаются на List Paragraph — как это делает Word. */
const LIST_PARAGRAPH_STYLE =
    '<w:style w:type="paragraph" w:customStyle="1" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/>' +
    '<w:basedOn w:val="Normal"/><w:uiPriority w:val="34"/><w:qFormat/>' +
    '<w:pPr><w:contextualSpacing/><w:ind w:left="720"/></w:pPr></w:style>';

interface LevelSpec {
    ilvl: number;
    numFmt: string;
    lvlText: string;
    start?: number;
    /** `w:rFonts` маркера-символа: Symbol/Wingdings для bullet-уровней. */
    font?: { ascii: string; hAnsi: string; hint?: string };
    indent?: { left: number; hanging: number };
    /** `w:lvlRestart w:val` — 0 означает «не перезапускать после старшего уровня». */
    lvlRestart?: number;
}

const level = (spec: LevelSpec): string => {
    const font = spec.font === undefined
        ? ''
        : `<w:rFonts w:ascii="${spec.font.ascii}" w:hAnsi="${spec.font.hAnsi}"` +
          `${spec.font.hint === undefined ? '' : ` w:hint="${spec.font.hint}"`}/>`;
    return (
        `<w:lvl w:ilvl="${spec.ilvl}">` +
        `<w:start w:val="${spec.start ?? 1}"/><w:numFmt w:val="${spec.numFmt}"/>` +
        (spec.lvlRestart === undefined ? '' : `<w:lvlRestart w:val="${spec.lvlRestart}"/>`) +
        `<w:lvlText w:val="${spec.lvlText}"/><w:lvlJc w:val="left"/>` +
        `<w:pPr><w:ind w:left="${spec.indent?.left ?? 720}" w:hanging="${spec.indent?.hanging ?? 360}"/></w:pPr>` +
        (font === '' ? '' : `<w:rPr>${font}</w:rPr>`) +
        '</w:lvl>'
    );
};

/** Детерминированный nsid — Word пишет его в каждый abstractNum. */
const nsid = (id: number): string => (0x1a2b3c4d + id * 0x00010001).toString(16).toUpperCase().padStart(8, '0');

const abstractNum = (abstractNumId: number, levels: LevelSpec[], multiLevelType: string): string =>
    `<w:abstractNum w:abstractNumId="${abstractNumId}"><w:nsid w:val="${nsid(abstractNumId)}"/>` +
    `<w:multiLevelType w:val="${multiLevelType}"/>` +
    levels.map(level).join('') +
    '</w:abstractNum>';

/** `w:num` со ссылкой на abstractNum и необязательными `lvlOverride`/`startOverride`. */
const num = (
    numId: number,
    abstractNumId: number,
    overrides: Array<{ ilvl: number; start: number }> = [],
): string =>
    `<w:num w:numId="${numId}"><w:abstractNumId w:val="${abstractNumId}"/>` +
    overrides
        .map(
            (override) =>
                `<w:lvlOverride w:ilvl="${override.ilvl}"><w:startOverride w:val="${override.start}"/></w:lvlOverride>`,
        )
        .join('') +
    '</w:num>';

const numberingXml = (...entries: string[]): string =>
    XML_DECL + `<w:numbering xmlns:w="${W_NS}">${entries.join('')}</w:numbering>`;

/** Абзац-пункт списка с явными numId/ilvl. */
const item = (text: string, numId: number, ilvl: number): string =>
    p(text, { style: 'ListParagraph', numId, ilvl });

/** Общий низ сайдкара для всех фикстур категории. */
const listStyles = [{ id: 'ListParagraph', type: 'paragraph', basedOn: 'Normal' }];

export const numberingFixtures: FixtureSpec[] = [
    {
        name: 'numbering/decimal_basic',
        description: 'Один список: abstractNum + num, уровень 0 в формате decimal',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                item('Первый пункт', 1, 0),
                item('Второй пункт', 1, 0),
                item('Третий пункт', 1, 0),
            ),
            numbering: numberingXml(
                abstractNum(
                    0,
                    [{ ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1, indent: { left: 720, hanging: 360 } }],
                    'singleLevel',
                ),
                num(1, 0),
            ),
            stylesExtra: LIST_PARAGRAPH_STYLE,
        }),
        content: {
            paragraphs: [
                { text: 'Первый пункт', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Второй пункт', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Третий пункт', numId: 1, ilvl: 0, style: 'ListParagraph' },
            ],
            tables: [],
            images: [],
            numbering: [
                {
                    numId: 1,
                    abstractNumId: 0,
                    levels: [{ ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1 }],
                },
            ],
            styles: listStyles,
        },
    },
    {
        name: 'numbering/multilevel',
        description: 'Три уровня одного списка с составным lvlText %1.%2.%3',
        expectedParagraphs: 5,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                item('Раздел первый', 1, 0),
                item('Подраздел 1.1', 1, 1),
                item('Пункт 1.1.1', 1, 2),
                item('Подраздел 1.2', 1, 1),
                item('Раздел второй', 1, 0),
            ),
            numbering: numberingXml(
                abstractNum(
                    0,
                    [
                        { ilvl: 0, numFmt: 'decimal', lvlText: '%1.', indent: { left: 720, hanging: 360 } },
                        { ilvl: 1, numFmt: 'decimal', lvlText: '%1.%2.', indent: { left: 1440, hanging: 360 } },
                        { ilvl: 2, numFmt: 'decimal', lvlText: '%1.%2.%3.', indent: { left: 2160, hanging: 360 } },
                    ],
                    'multilevel',
                ),
                num(1, 0),
            ),
            stylesExtra: LIST_PARAGRAPH_STYLE,
        }),
        content: {
            paragraphs: [
                { text: 'Раздел первый', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Подраздел 1.1', numId: 1, ilvl: 1, style: 'ListParagraph' },
                { text: 'Пункт 1.1.1', numId: 1, ilvl: 2, style: 'ListParagraph' },
                { text: 'Подраздел 1.2', numId: 1, ilvl: 1, style: 'ListParagraph' },
                { text: 'Раздел второй', numId: 1, ilvl: 0, style: 'ListParagraph' },
            ],
            tables: [],
            images: [],
            numbering: [
                {
                    numId: 1,
                    abstractNumId: 0,
                    levels: [
                        { ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1 },
                        { ilvl: 1, numFmt: 'decimal', lvlText: '%1.%2.', start: 1 },
                        { ilvl: 2, numFmt: 'decimal', lvlText: '%1.%2.%3.', start: 1 },
                    ],
                },
            ],
            styles: listStyles,
        },
    },
    {
        name: 'numbering/start_override',
        description: 'lvlOverride со startOverride сдвигает начало списка на 5',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                item('Пункт с номером 5', 1, 0),
                item('Пункт с номером 6', 1, 0),
                item('Пункт с номером 7', 1, 0),
            ),
            numbering: numberingXml(
                abstractNum(
                    0,
                    [{ ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1, indent: { left: 720, hanging: 360 } }],
                    'singleLevel',
                ),
                num(1, 0, [{ ilvl: 0, start: 5 }]),
            ),
            stylesExtra: LIST_PARAGRAPH_STYLE,
        }),
        content: {
            paragraphs: [
                { text: 'Пункт с номером 5', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Пункт с номером 6', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Пункт с номером 7', numId: 1, ilvl: 0, style: 'ListParagraph' },
            ],
            tables: [],
            images: [],
            numbering: [
                {
                    numId: 1,
                    abstractNumId: 0,
                    levels: [{ ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1, startOverride: 5 }],
                },
            ],
            styles: listStyles,
        },
    },
    {
        name: 'numbering/bullet_symbols',
        description: 'Bullet-уровни с маркерами в шрифтах Symbol, Courier New и Wingdings',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                item('Первый маркер', 1, 0),
                item('Вложенный маркер', 1, 1),
                item('Третий уровень', 1, 2),
                item('Снова второй уровень', 1, 1),
            ),
            numbering: numberingXml(
                abstractNum(
                    0,
                    [
                        {
                            ilvl: 0,
                            numFmt: 'bullet',
                            lvlText: '&#xF0B7;',
                            font: { ascii: 'Symbol', hAnsi: 'Symbol', hint: 'default' },
                            indent: { left: 720, hanging: 360 },
                        },
                        {
                            ilvl: 1,
                            numFmt: 'bullet',
                            lvlText: 'o',
                            font: { ascii: 'Courier New', hAnsi: 'Courier New', hint: 'default' },
                            indent: { left: 1440, hanging: 360 },
                        },
                        {
                            ilvl: 2,
                            numFmt: 'bullet',
                            lvlText: '&#xF0A7;',
                            font: { ascii: 'Wingdings', hAnsi: 'Wingdings', hint: 'default' },
                            indent: { left: 2160, hanging: 360 },
                        },
                    ],
                    'hybridMultilevel',
                ),
                num(1, 0),
            ),
            stylesExtra: LIST_PARAGRAPH_STYLE,
        }),
        content: {
            paragraphs: [
                { text: 'Первый маркер', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Вложенный маркер', numId: 1, ilvl: 1, style: 'ListParagraph' },
                { text: 'Третий уровень', numId: 1, ilvl: 2, style: 'ListParagraph' },
                { text: 'Снова второй уровень', numId: 1, ilvl: 1, style: 'ListParagraph' },
            ],
            tables: [],
            images: [],
            numbering: [
                {
                    numId: 1,
                    abstractNumId: 0,
                    levels: [
                        { ilvl: 0, numFmt: 'bullet', lvlText: '', start: 1, font: 'Symbol' },
                        { ilvl: 1, numFmt: 'bullet', lvlText: 'o', start: 1, font: 'Courier New' },
                        { ilvl: 2, numFmt: 'bullet', lvlText: '', start: 1, font: 'Wingdings' },
                    ],
                },
            ],
            styles: listStyles,
        },
    },
    {
        name: 'numbering/restart_numbering',
        description: 'Два num на одном abstractNum: второй перезапускает нумерацию через startOverride',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                item('Первый список, пункт 1', 1, 0),
                item('Первый список, пункт 2', 1, 0),
                item('Второй список, пункт 1', 2, 0),
                item('Второй список, пункт 2', 2, 0),
            ),
            numbering: numberingXml(
                abstractNum(
                    0,
                    [{ ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1, indent: { left: 720, hanging: 360 } }],
                    'singleLevel',
                ),
                num(1, 0),
                num(2, 0, [{ ilvl: 0, start: 1 }]),
            ),
            stylesExtra: LIST_PARAGRAPH_STYLE,
        }),
        content: {
            paragraphs: [
                { text: 'Первый список, пункт 1', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Первый список, пункт 2', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Второй список, пункт 1', numId: 2, ilvl: 0, style: 'ListParagraph' },
                { text: 'Второй список, пункт 2', numId: 2, ilvl: 0, style: 'ListParagraph' },
            ],
            tables: [],
            images: [],
            numbering: [
                { numId: 1, abstractNumId: 0, levels: [{ ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1 }] },
                {
                    numId: 2,
                    abstractNumId: 0,
                    levels: [{ ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1, startOverride: 1 }],
                },
            ],
            styles: listStyles,
        },
    },
    {
        name: 'numbering/nested_levels',
        description: 'Абзацы на ilvl 0, 1 и 2 с lvlRestart на вложенных уровнях',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                item('Верхний уровень', 1, 0),
                item('Вложенный уровень 1', 1, 1),
                item('Вложенный уровень 2', 1, 2),
                item('Возврат на уровень 1', 1, 1),
            ),
            numbering: numberingXml(
                abstractNum(
                    0,
                    [
                        { ilvl: 0, numFmt: 'decimal', lvlText: '%1.', indent: { left: 720, hanging: 360 } },
                        {
                            ilvl: 1,
                            numFmt: 'decimal',
                            lvlText: '%2.',
                            lvlRestart: 0,
                            indent: { left: 1440, hanging: 360 },
                        },
                        { ilvl: 2, numFmt: 'decimal', lvlText: '%3.', indent: { left: 2160, hanging: 360 } },
                    ],
                    'multilevel',
                ),
                num(1, 0),
            ),
            stylesExtra: LIST_PARAGRAPH_STYLE,
        }),
        content: {
            paragraphs: [
                { text: 'Верхний уровень', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Вложенный уровень 1', numId: 1, ilvl: 1, style: 'ListParagraph' },
                { text: 'Вложенный уровень 2', numId: 1, ilvl: 2, style: 'ListParagraph' },
                { text: 'Возврат на уровень 1', numId: 1, ilvl: 1, style: 'ListParagraph' },
            ],
            tables: [],
            images: [],
            numbering: [
                {
                    numId: 1,
                    abstractNumId: 0,
                    levels: [
                        { ilvl: 0, numFmt: 'decimal', lvlText: '%1.', start: 1 },
                        { ilvl: 1, numFmt: 'decimal', lvlText: '%2.', start: 1, lvlRestart: 0 },
                        { ilvl: 2, numFmt: 'decimal', lvlText: '%3.', start: 1 },
                    ],
                },
            ],
            styles: listStyles,
        },
    },
    {
        name: 'numbering/custom_format',
        description: 'Нестандартные форматы: lowerLetter на первом уровне и upperRoman на втором',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                item('Первый пункт', 1, 0),
                item('Первый подпункт', 1, 1),
                item('Второй пункт', 1, 0),
            ),
            numbering: numberingXml(
                abstractNum(
                    0,
                    [
                        { ilvl: 0, numFmt: 'lowerLetter', lvlText: '%1)', indent: { left: 720, hanging: 360 } },
                        { ilvl: 1, numFmt: 'upperRoman', lvlText: '%2.', indent: { left: 1440, hanging: 360 } },
                    ],
                    'multilevel',
                ),
                num(1, 0),
            ),
            stylesExtra: LIST_PARAGRAPH_STYLE,
        }),
        content: {
            paragraphs: [
                { text: 'Первый пункт', numId: 1, ilvl: 0, style: 'ListParagraph' },
                { text: 'Первый подпункт', numId: 1, ilvl: 1, style: 'ListParagraph' },
                { text: 'Второй пункт', numId: 1, ilvl: 0, style: 'ListParagraph' },
            ],
            tables: [],
            images: [],
            numbering: [
                {
                    numId: 1,
                    abstractNumId: 0,
                    levels: [
                        { ilvl: 0, numFmt: 'lowerLetter', lvlText: '%1)', start: 1 },
                        { ilvl: 1, numFmt: 'upperRoman', lvlText: '%2.', start: 1 },
                    ],
                },
            ],
            styles: listStyles,
        },
    },
];
