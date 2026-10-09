// Фикстуры категории alternate_content: `mc:AlternateContent` с DrawingML в
// `mc:Choice` и VML в `mc:Fallback`.
//
// Политика парсера (ADR-0014): берётся первая поддерживаемая `mc:Choice`,
// `mc:Fallback` не разбирается никогда. Поэтому `expectedImages` считает
// `w:drawing` в ветках Choice, а VML (`w:pict`) в счёт не идёт.

import {
    type FixtureSpec,
    V_NS,
    WPG_NS,
    WPS_NS,
    defaultSectPr,
    documentXml,
    p,
    packageParts,
    para,
    sectionProps,
} from './kit.js';

/** Тело + `w:sectPr`; `xmlns:v` объявлен на корне — VML живёт только в Fallback. */
const doc = (bodyXml: string): string =>
    documentXml(bodyXml + sectionProps(defaultSectPr), ` xmlns:v="${V_NS}"`);

const choice = (requires: string, runsXml: string): string =>
    `<mc:Choice Requires="${requires}">${runsXml}</mc:Choice>`;

const fallback = (runsXml: string): string => `<mc:Fallback>${runsXml}</mc:Fallback>`;

const alternate = (...blocks: string[]): string => `<mc:AlternateContent>${blocks.join('')}</mc:AlternateContent>`;

const runOf = (drawing: string): string => `<w:r>${drawing}</w:r>`;

/** `wp:inline` вокруг фигуры DrawingML: `uri` — пространство имён содержимого. */
const inlineGraphic = (
    uri: string,
    shapeXml: string,
    cx: number,
    cy: number,
    id: number,
    name: string,
): string =>
    '<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">' +
    `<wp:extent cx="${cx}" cy="${cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/>` +
    `<wp:docPr id="${id}" name="${name}"/><wp:cNvGraphicFramePr/>` +
    `<a:graphic><a:graphicData uri="${uri}">${shapeXml}</a:graphicData></a:graphic>` +
    '</wp:inline></w:drawing>';

/** `wps:txbx` — текстовое поле WordprocessingShape. */
const wpsTextBox = (text: string, cx: number, cy: number): string =>
    '<wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr>' +
    `<a:xfrm><a:off x="0" y="0"/><a:ext cx="${cx}" cy="${cy}"/></a:xfrm>` +
    '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>' +
    '<a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill>' +
    '<a:ln w="9525"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln>' +
    '</wps:spPr>' +
    `<wps:txbx><w:txbxContent>${p(text)}</w:txbxContent></wps:txbx>` +
    '<wps:bodyPr rot="0" wrap="square" lIns="91440" tIns="45720" rIns="91440" bIns="45720"><a:noAutofit/></wps:bodyPr>' +
    '</wps:wsp>';

/** Фигура без текста: прямоугольник, овал и т.п. */
const wpsShape = (prst: string, color: string, cx: number, cy: number): string =>
    '<wps:wsp><wps:cNvSpPr/><wps:spPr>' +
    `<a:xfrm><a:off x="0" y="0"/><a:ext cx="${cx}" cy="${cy}"/></a:xfrm>` +
    `<a:prstGeom prst="${prst}"><a:avLst/></a:prstGeom>` +
    `<a:solidFill><a:srgbClr val="${color}"/></a:solidFill>` +
    '</wps:spPr><wps:bodyPr/></wps:wsp>';

/** `wpg:wgp` — группа фигур; координаты общие на всю группу. */
const wpsGroup = (shapes: string, cx: number, cy: number): string =>
    '<wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr>' +
    `<a:xfrm><a:off x="0" y="0"/><a:ext cx="${cx}" cy="${cy}"/>` +
    `<a:chOff x="0" y="0"/><a:chExt cx="${cx}" cy="${cy}"/></a:xfrm>` +
    `</wpg:grpSpPr>${shapes}</wpg:wgp>`;

/** VML-надпись: legacy-представление `wps:txbx`. */
const vmlTextBox = (text: string): string =>
    '<w:pict><v:shape style="width:144pt;height:72pt" filled="f" stroked="t" strokecolor="#000000">' +
    `<v:textbox><w:txbxContent>${p(text)}</w:txbxContent></v:textbox>` +
    '</v:shape></w:pict>';

const vmlOval = (): string =>
    '<w:pict><v:oval style="width:72pt;height:36pt" fillcolor="#4472c4"/></w:pict>';

const vmlGroup = (): string =>
    '<w:pict><v:group style="width:216pt;height:72pt" coordorigin="0,0" coordsize="4320,1440">' +
    '<v:rect style="width:144pt;height:72pt" fillcolor="#4472c4"/>' +
    '<v:oval style="left:144pt;width:72pt;height:72pt" fillcolor="#ed7d31"/>' +
    '</v:group></w:pict>';

const textRun = (text: string): string => `<w:r><w:t xml:space="preserve">${text}</w:t></w:r>`;

export const alternateContentFixtures: FixtureSpec[] = [
    {
        name: 'alternate_content/choice_fallback_textbox',
        description: 'Текстовое поле: DrawingML в Choice, VML в Fallback',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            document: doc(
                para(
                    '',
                    alternate(
                        choice(
                            'wps',
                            runOf(
                                inlineGraphic(
                                    WPS_NS,
                                    wpsTextBox('Текст в надписи', 1828800, 914400),
                                    1828800,
                                    914400,
                                    1,
                                    'TextBox 1',
                                ),
                            ),
                        ),
                        fallback(runOf(vmlTextBox('Текст в надписи'))),
                    ),
                ),
            ),
        }),
        content: {
            paragraphs: [{ text: '' }],
            alternateContent: [{ choices: ['wps'], fallback: 'vml' }],
            notes: 'Choice разрешается в текстовое поле wps:txbx, Fallback не разбирается',
        },
    },
    {
        name: 'alternate_content/choice_shape',
        description: 'Фигура: прямоугольник DrawingML в Choice, овал VML в Fallback',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            document: doc(
                para(
                    '',
                    alternate(
                        choice(
                            'wps',
                            runOf(
                                inlineGraphic(
                                    WPS_NS,
                                    wpsShape('rect', '4472C4', 1371600, 685800),
                                    1371600,
                                    685800,
                                    1,
                                    'Rectangle 1',
                                ),
                            ),
                        ),
                        fallback(runOf(vmlOval())),
                    ),
                ),
            ),
        }),
        content: {
            paragraphs: [{ text: '' }],
            alternateContent: [{ choices: ['wps'], fallback: 'vml' }],
            notes: 'Choice — примитив prstGeom rect без текста',
        },
    },
    {
        name: 'alternate_content/fallback_only',
        description: 'AlternateContent без Choice: только Fallback',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        // Имя варианта — как в ADR-0016 (`UnknownElement`), не в snake_case.
        expectedWarnings: ['UnknownElement'],
        parts: packageParts({
            document: doc(
                para('', alternate(fallback(runOf(vmlTextBox('Только fallback'))))),
            ),
        }),
        content: {
            paragraphs: [{ text: '' }],
            alternateContent: [{ choices: [], fallback: 'vml' }],
            notes: 'Ни одной поддерживаемой ветки Choice нет — блок сохраняется как Unknown',
        },
    },
    {
        name: 'alternate_content/group_shape',
        description: 'Группа фигур: wpg:wgp из двух фигур в Choice, VML-группа в Fallback',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            document: doc(
                para(
                    '',
                    alternate(
                        choice(
                            'wpg',
                            runOf(
                                inlineGraphic(
                                    WPG_NS,
                                    wpsGroup(
                                        wpsShape('rect', '4472C4', 1371600, 685800) +
                                            wpsShape('ellipse', 'ED7D31', 685800, 685800),
                                        2743200,
                                        914400,
                                    ),
                                    2743200,
                                    914400,
                                    1,
                                    'Group 1',
                                ),
                            ),
                        ),
                        fallback(runOf(vmlGroup())),
                    ),
                ),
            ),
        }),
        content: {
            paragraphs: [{ text: '' }],
            alternateContent: [{ choices: ['wpg'], fallback: 'vml' }],
            notes: 'Choice требует wpg; в группе прямоугольник и овал',
        },
    },
    {
        name: 'alternate_content/nested_alternate',
        description: 'Вложенный AlternateContent внутри Choice',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            document: doc(
                para(
                    '',
                    alternate(
                        choice(
                            'wps',
                            alternate(
                                choice(
                                    'wpg',
                                    runOf(
                                        inlineGraphic(
                                            WPG_NS,
                                            wpsGroup(
                                                wpsShape('rect', '4472C4', 685800, 685800) +
                                                    wpsShape('rect', '70AD47', 685800, 685800),
                                                1371600,
                                                685800,
                                            ),
                                            1371600,
                                            685800,
                                            1,
                                            'Nested Group 1',
                                        ),
                                    ),
                                ),
                                fallback(textRun('вложенный fallback')),
                            ),
                        ),
                        fallback(textRun('внешний fallback')),
                    ),
                ),
            ),
        }),
        content: {
            paragraphs: [{ text: '' }],
            alternateContent: [
                { choices: ['wps'], fallback: 'text' },
                { choices: ['wpg'], fallback: 'text' },
            ],
            notes: 'Первый Choice поддерживается и содержит второй AlternateContent',
        },
    },
];
