// Базовые фикстуры тела документа: форматирование прогонов, пустое тело,
// значимые пробелы, разрывы и табуляции, интервалы, выравнивание, отступы,
// астральная Unicode, абзац из сотни прогонов и две секции.
//
// Счётчики считаются по построению: `expectedParagraphs` — только прямые дети
// `w:body`. `w:sectPr` абзацем не считается ни в теле, ни в `w:pPr`;
// `pageBreak()` из kit — считается, потому что это отдельный `<w:p>`.

import {
    type FixtureSpec,
    defaultSectPr,
    p,
    pageBreak,
    packageParts,
    para,
    run,
    sectionProps,
} from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Прогон с готовым содержимым — когда одного `<w:t>` мало (`w:br`, `w:tab`). */
const rawRun = (inner: string): string => `<w:r>${inner}</w:r>`;

/** Текст прогона со значимыми пробелами. */
const t = (value: string): string => `<w:t xml:space="preserve">${value}</w:t>`;

/** Характерный стиль прогона из `stylesExtra` — на него ссылается `rStyle`. */
const EMPHASIS_STYLE =
    '<w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/>' +
    '<w:basedOn w:val="DefaultParagraphFont"/><w:uiPriority w:val="20"/><w:qFormat/>' +
    '<w:rPr><w:i/><w:color w:val="CC0000"/></w:rPr></w:style>';

/** Портретная A4 для промежуточного `w:sectPr`. */
const PORTRAIT_SECT =
    '<w:pgSz w:w="11906" w:h="16838"/>' +
    '<w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="709" w:footer="709" w:gutter="0"/>';

/** Ландшафтная A4 для финального `w:sectPr`. */
const LANDSCAPE_SECT =
    '<w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/>' +
    '<w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="709" w:footer="709" w:gutter="0"/>';

/** Слова для абзаца из сотни прогонов. */
const MANY_RUN_WORDS = Array.from({ length: 100 }, (_, i) => `run${i + 1}`);

export const basicFixtures: FixtureSpec[] = [
    {
        name: 'basic/multiple_runs',
        description: 'Несколько прогонов с разным форматированием в одном абзаце',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '',
                    run('Plain ') +
                        run('bold ', { bold: true }) +
                        run('italic ', { italic: true }) +
                        run('underlined ', { underline: true }) +
                        run('colored ', { color: 'FF0000' }) +
                        run('sized', { size: 32 }) +
                        run(' emphasized', { style: 'Emphasis' }),
                ),
            ),
            stylesExtra: EMPHASIS_STYLE,
        }),
        content: {
            paragraphs: [
                {
                    text: 'Plain bold italic underlined colored sized emphasized',
                    runs: [
                        { text: 'Plain ' },
                        { text: 'bold ', bold: true },
                        { text: 'italic ', italic: true },
                        { text: 'underlined ', underline: true },
                        { text: 'colored ', color: 'FF0000' },
                        { text: 'sized', size: 32 },
                        { text: ' emphasized', style: 'Emphasis' },
                    ],
                },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/empty_body',
        description: 'Пустое тело: только sectPr, абзацев нет',
        expectedParagraphs: 0,
        expectedTables: 0,
        expectedImages: 0,
        // `body` задан явно: без него kit подставил бы пустой абзац.
        parts: packageParts({ body: sectionProps(defaultSectPr) }),
        content: { paragraphs: [], tables: [], images: [] },
    },
    {
        name: 'basic/whitespace_preserve',
        description: 'Значимые пробелы: ведущие, замыкающие и двойные внутри',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(p('   leading and trailing   '), p('double  spaces   between  words')),
        }),
        content: {
            paragraphs: [
                { text: '   leading and trailing   ' },
                { text: 'double  spaces   between  words' },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/breaks_and_tabs',
        description: 'Разрывы строки и страницы, табуляция внутри прогонов',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                // Обычный `w:br` — разрыв строки внутри одного абзаца.
                para('', rawRun(t('First line') + '<w:br/>' + t('second line'))),
                // `w:tab` — позиция табуляции по умолчанию (708 twips).
                para('', rawRun(t('Col1') + '<w:tab/>' + t('Col2'))),
                // Разрыв страницы kit отдаёт отдельным абзацем — он тоже в счёте.
                pageBreak(),
                p('After the page break'),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'First line\nsecond line', breaks: ['line'] },
                { text: 'Col1\tCol2', tabs: 1 },
                { text: '', breaks: ['page'] },
                { text: 'After the page break' },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/paragraph_spacing',
        description: 'Интервалы абзацев: before/after, line и lineRule',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                p('Before 240, after 120', { pPr: '<w:spacing w:before="240" w:after="120"/>' }),
                p('Line 360, lineRule auto', { pPr: '<w:spacing w:line="360" w:lineRule="auto"/>' }),
                p('Line 240, lineRule exact', {
                    pPr: '<w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/>',
                }),
                p('Line 720, lineRule atLeast', {
                    pPr: '<w:spacing w:before="360" w:after="360" w:line="720" w:lineRule="atLeast"/>',
                }),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Before 240, after 120', spacing: { before: 240, after: 120 } },
                { text: 'Line 360, lineRule auto', spacing: { line: 360, lineRule: 'auto' } },
                {
                    text: 'Line 240, lineRule exact',
                    spacing: { before: 0, after: 0, line: 240, lineRule: 'exact' },
                },
                {
                    text: 'Line 720, lineRule atLeast',
                    spacing: { before: 360, after: 360, line: 720, lineRule: 'atLeast' },
                },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/paragraph_alignment',
        description: 'Выравнивание абзацев: left, center, right, both',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                p('Aligned left', { jc: 'left' }),
                p('Aligned center', { jc: 'center' }),
                p('Aligned right', { jc: 'right' }),
                p('Aligned both', { jc: 'both' }),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Aligned left', jc: 'left' },
                { text: 'Aligned center', jc: 'center' },
                { text: 'Aligned right', jc: 'right' },
                { text: 'Aligned both', jc: 'both' },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/indentation',
        description: 'Отступы абзацев: left, right, firstLine, hanging',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                p('Left indent 720', { pPr: '<w:ind w:left="720"/>' }),
                p('Right indent 720', { pPr: '<w:ind w:right="720"/>' }),
                p('First line indent 720', { pPr: '<w:ind w:firstLine="720"/>' }),
                p('Left 1440 with hanging 360', { pPr: '<w:ind w:left="1440" w:hanging="360"/>' }),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Left indent 720', ind: { left: 720 } },
                { text: 'Right indent 720', ind: { right: 720 } },
                { text: 'First line indent 720', ind: { firstLine: 720 } },
                { text: 'Left 1440 with hanging 360', ind: { left: 1440, hanging: 360 } },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/astral_unicode',
        description: 'Символы вне BMP: эмодзи и музыкальные знаки вперемешку с текстом',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                p('Emoji: 😀 🎉 🚀'),
                p('Astral symbols: 𝄞 𝄢 𝕏'),
                p('Mixed: before 😀 middle 𝄞 after'),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Emoji: 😀 🎉 🚀' },
                { text: 'Astral symbols: 𝄞 𝄢 𝕏' },
                { text: 'Mixed: before 😀 middle 𝄞 after' },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/many_runs_paragraph',
        description: 'Один абзац из сотни прогонов',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '',
                    MANY_RUN_WORDS.map((word, i) =>
                        run(
                            // Пробел-разделитель живёт в конце прогона: `xml:space`
                            // сохраняет его, а лишнего пробела в хвосте нет.
                            i === MANY_RUN_WORDS.length - 1 ? word : `${word} `,
                            i % 10 === 9 ? { bold: true } : {},
                        ),
                    ).join(''),
                ),
            ),
        }),
        content: {
            paragraphs: [
                {
                    text: MANY_RUN_WORDS.join(' '),
                    runCount: MANY_RUN_WORDS.length,
                    boldEvery: 10,
                },
            ],
            tables: [],
            images: [],
        },
    },
    {
        name: 'basic/sections',
        description: 'Две секции: портрет A4 и ландшафт A4',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 0,
        // Промежуточный `sectPr` закрывает первую секцию и лежит в `w:pPr`;
        // финальный — прямо в теле и закрывает вторую.
        parts: packageParts({
            body:
                para(
                    sectionProps(`<w:type w:val="nextPage"/>${PORTRAIT_SECT}`),
                    run('First section: portrait A4'),
                ) +
                p('Second section: landscape A4') +
                sectionProps(LANDSCAPE_SECT),
        }),
        meta: {
            sections: [
                { orientation: 'portrait', widthTwips: 11906, heightTwips: 16838 },
                { orientation: 'landscape', widthTwips: 16838, heightTwips: 11906 },
            ],
        },
        content: {
            paragraphs: [
                { text: 'First section: portrait A4' },
                { text: 'Second section: landscape A4' },
            ],
            tables: [],
            images: [],
        },
    },
];
