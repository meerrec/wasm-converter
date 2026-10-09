// Коды полей Word: `w:fldSimple`, `w:fldChar` + `w:instrText`, кэшированный
// результат между `separate` и `end`, вложенные поля, DATE с форматом и
// HYPERLINK с внешним relationship.
//
// Кэш результата — то, что Word показывал до последнего пересчёта; парсер
// обязан его сохранить, а не выбрасывать: пересчитывать поля он не умеет.

import {
    HYPERLINK_REL_TYPE,
    type FixtureSpec,
    defaultSectPr,
    escapeXml,
    packageParts,
    para,
    run,
    sectionProps,
} from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Прогон с готовым содержимым: `w:fldChar`, `w:instrText`, `w:t`. */
const rawRun = (inner: string, rPr = ''): string =>
    `<w:r>${rPr === '' ? '' : `<w:rPr>${rPr}</w:rPr>`}${inner}</w:r>`;

/** `w:fldChar` — граница поля (begin/separate/end). */
const fldChar = (type: 'begin' | 'separate' | 'end'): string =>
    rawRun(`<w:fldChar w:fldCharType="${type}"/>`);

/** `w:instrText` — инструкция поля; окружающие пробелы значимы. */
const instrText = (instruction: string): string =>
    rawRun(`<w:instrText xml:space="preserve">${escapeXml(instruction)}</w:instrText>`);

/** Кэшированный результат поля. */
const result = (value: string, rPr = ''): string =>
    rawRun(`<w:t xml:space="preserve">${escapeXml(value)}</w:t>`, rPr);

/** Поле в форме fldChar с кэшем: begin → instrText → separate → результат → end. */
const cachedField = (instruction: string, value: string): string =>
    fldChar('begin') + instrText(instruction) + fldChar('separate') + result(value) + fldChar('end');

/** `w:fldSimple` — поле, у которого нет ни `w:fldChar`, ни `w:instrText`. */
const simpleField = (instruction: string, value: string): string =>
    `<w:fldSimple w:instr="${escapeXml(instruction)}">${run(value)}</w:fldSimple>`;

/** Цель гиперссылки и её внешний relationship (styles занимает rId1, значит свободен rId2). */
const HYPERLINK_TARGET = 'https://example.com/docs';
const HYPERLINK_REL = `<Relationship Id="rId2" Type="${HYPERLINK_REL_TYPE}" Target="${HYPERLINK_TARGET}" TargetMode="External"/>`;

/** Стиль `Hyperlink` — на него ссылается `rStyle` прогонов внутри ссылки. */
const HYPERLINK_STYLE =
    '<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/>' +
    '<w:basedOn w:val="DefaultParagraphFont"/><w:uiPriority w:val="99"/><w:unhideWhenUsed/>' +
    '<w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>';

export const fieldsFixtures: FixtureSpec[] = [
    {
        name: 'fields/fld_simple',
        description: 'Поля через w:fldSimple с кэшированным результатом',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '',
                    run('Page ') +
                        simpleField(' PAGE ', '1') +
                        run(' of ') +
                        simpleField(' NUMPAGES ', '12'),
                ),
                // Вторая форма того же поля в том же документе: парсер обязан
                // получить одинаковую модель независимо от кодировки.
                para('', run('Page (fldChar): ') + cachedField(' PAGE ', '1')),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Page 1 of 12' },
                { text: 'Page (fldChar): 1' },
            ],
            fields: [
                { instruction: ' PAGE ', type: 'page', cached: '1', form: 'fldSimple' },
                { instruction: ' NUMPAGES ', type: 'numpages', cached: '12', form: 'fldSimple' },
                { instruction: ' PAGE ', type: 'page', cached: '1', form: 'fldChar' },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'fields/instr_text',
        description: 'Поля через fldChar begin/end и w:instrText без результата',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                // Между begin и end нет separate, значит результата у поля нет.
                para('', run('Page ') + fldChar('begin') + instrText(' PAGE ') + fldChar('end')),
                para('', run('Today is ') + fldChar('begin') + instrText(' DATE ') + fldChar('end')),
            ),
        }),
        content: {
            paragraphs: [{ text: 'Page ' }, { text: 'Today is ' }],
            fields: [
                { instruction: ' PAGE ', type: 'page', form: 'fldChar' },
                { instruction: ' DATE ', type: 'date', form: 'fldChar' },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'fields/fld_char_with_separate',
        description: 'fldChar с separate: кэшированный результат разбит по прогонам',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para('', run('Page ') + cachedField(' PAGE ', '7') + run(' of 10')),
                para(
                    '',
                    run('Bold result: ') +
                        fldChar('begin') +
                        instrText(' DATE ') +
                        fldChar('separate') +
                        result('01.01.2026', '<w:b/>') +
                        fldChar('end'),
                ),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Page 7 of 10' },
                { text: 'Bold result: 01.01.2026' },
            ],
            fields: [
                { instruction: ' PAGE ', type: 'page', cached: '7', form: 'fldChar' },
                {
                    instruction: ' DATE ',
                    type: 'date',
                    cached: '01.01.2026',
                    cachedBold: true,
                    form: 'fldChar',
                },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'fields/nested_fields',
        description: 'Вложенное поле: PAGE внутри инструкции IF',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        // Инструкция внешнего IF разорвана вложенным полем на два `w:instrText`;
        // собирать её нужно по порядку прогонов, а не по одному тексту.
        parts: packageParts({
            body: body(
                para(
                    '',
                    fldChar('begin') +
                        instrText(' IF ') +
                        fldChar('begin') +
                        instrText(' PAGE ') +
                        fldChar('separate') +
                        result('1') +
                        fldChar('end') +
                        instrText(' > 0 "yes" "no" ') +
                        fldChar('separate') +
                        result('yes') +
                        fldChar('end'),
                ),
            ),
        }),
        content: {
            paragraphs: [{ text: 'yes' }],
            fields: [
                {
                    instruction: ' IF 1 > 0 "yes" "no" ',
                    type: 'if',
                    cached: 'yes',
                    form: 'fldChar',
                    children: [
                        { instruction: ' PAGE ', type: 'page', cached: '1', form: 'fldChar' },
                    ],
                },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'fields/date_field',
        description: 'Поле DATE с форматом вывода и кэшированным результатом',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para('', run('Date: ') + cachedField(' DATE \\@ "dd.MM.yyyy" ', '01.01.2026')),
                // У `fldSimple` кавычки в инструкции — это ещё и атрибутные
                // сущности: `&quot;` парсер обязан раскодировать.
                para('', run('Date (fldSimple): ') + simpleField(' DATE \\@ "dd.MM.yyyy" ', '01.01.2026')),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Date: 01.01.2026' },
                { text: 'Date (fldSimple): 01.01.2026' },
            ],
            fields: [
                {
                    instruction: ' DATE \\@ "dd.MM.yyyy" ',
                    type: 'date',
                    format: 'dd.MM.yyyy',
                    cached: '01.01.2026',
                    form: 'fldChar',
                },
                {
                    instruction: ' DATE \\@ "dd.MM.yyyy" ',
                    type: 'date',
                    format: 'dd.MM.yyyy',
                    cached: '01.01.2026',
                    form: 'fldSimple',
                },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'fields/hyperlink_field',
        description: 'Поля HYPERLINK: элемент w:hyperlink и форма fldChar с внешним rel',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '',
                    `<w:hyperlink r:id="rId2" w:history="1">${run('example.com/docs', { style: 'Hyperlink' })}</w:hyperlink>`,
                ),
                para(
                    '',
                    run('Link: ') +
                        fldChar('begin') +
                        instrText(` HYPERLINK "${HYPERLINK_TARGET}" `) +
                        fldChar('separate') +
                        `<w:hyperlink r:id="rId2" w:history="1">${run('example.com/docs', { style: 'Hyperlink' })}</w:hyperlink>` +
                        fldChar('end'),
                ),
            ),
            extraRels: HYPERLINK_REL,
            stylesExtra: HYPERLINK_STYLE,
        }),
        content: {
            paragraphs: [
                {
                    text: 'example.com/docs',
                    hyperlink: HYPERLINK_TARGET,
                    relId: 'rId2',
                },
                {
                    text: 'Link: example.com/docs',
                    hyperlink: HYPERLINK_TARGET,
                    relId: 'rId2',
                },
            ],
            fields: [
                {
                    instruction: ` HYPERLINK "${HYPERLINK_TARGET}" `,
                    type: 'hyperlink',
                    target: HYPERLINK_TARGET,
                    form: 'fldChar',
                },
            ],
            rels: [
                {
                    id: 'rId2',
                    type: HYPERLINK_REL_TYPE,
                    target: HYPERLINK_TARGET,
                    external: true,
                },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'fields/unknown_field',
        description: 'Поля с нестандартными инструкциями: MERGEFIELD и неизвестное имя',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        // Незнакомая инструкция — это сохранённое как Unknown поле, а не
        // нарушение формата: `unknown_element` в WarningKind про XML, не про коды
        // полей, поэтому предупреждений тут не ждём.
        expectedWarnings: [],
        parts: packageParts({
            body: body(
                para(
                    '',
                    run('Hello ') +
                        cachedField(' MERGEFIELD FirstName \\* MERGEFORMAT ', 'Иван') +
                        run('!'),
                ),
                para('', fldChar('begin') + instrText(' FOOBAR "arg" ') + fldChar('end')),
            ),
        }),
        content: {
            paragraphs: [{ text: 'Hello Иван!' }, { text: '' }],
            fields: [
                {
                    instruction: ' MERGEFIELD FirstName \\* MERGEFORMAT ',
                    type: 'unknown',
                    cached: 'Иван',
                    form: 'fldChar',
                },
                { instruction: ' FOOBAR "arg" ', type: 'unknown', form: 'fldChar' },
            ],
            tables: [],
            images: [],
        },
    },
];
