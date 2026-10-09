// Первые 16 фикстур: те же пути и то же содержимое, что и раньше, но с полным
// OOXML-пакетом (styles.xml с docDefaults, таблицами стилей, sectPr) вместо
// минимального набора частей.

import { type FixtureSpec, defaultSectPr, h, p, packageParts, sectionProps, table } from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

export const legacyFixtures: FixtureSpec[] = [
    // ==================== simple ====================
    {
        name: 'simple/empty',
        description: 'Пустой документ',
        expectedParagraphs: 0,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({ body: body() }),
        content: { paragraphs: [], tables: [], images: [] },
    },
    {
        name: 'simple/one_paragraph',
        description: 'Один абзац',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({ body: body(p('Hello, World!')) }),
        content: { paragraphs: [{ text: 'Hello, World!' }], tables: [], images: [] },
    },
    ...Array.from({ length: 3 }, (_, i): FixtureSpec => {
        const texts = Array.from({ length: 5 }, (_, j) => `Paragraph ${j + 1}`);
        return {
            name: `simple/multiple_paragraphs_${i}`,
            description: '5 абзацев',
            expectedParagraphs: 5,
            expectedTables: 0,
            expectedImages: 0,
            parts: packageParts({ body: body(...texts.map((t) => p(t))) }),
            content: { paragraphs: texts.map((text) => ({ text })), tables: [], images: [] },
        };
    }),

    // ==================== formatting ====================
    {
        name: 'formatting/bold',
        description: 'Жирный текст',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({ body: body(p('Bold text', { bold: true })) }),
        content: { paragraphs: [{ text: 'Bold text', bold: true }], tables: [], images: [] },
    },
    {
        name: 'formatting/italic',
        description: 'Курсив',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({ body: body(p('Italic text', { italic: true })) }),
        content: { paragraphs: [{ text: 'Italic text', italic: true }], tables: [], images: [] },
    },
    ...([1, 2, 3] as const).map(
        (level): FixtureSpec => ({
            name: `formatting/heading_${level}`,
            description: `Заголовок уровня ${level}`,
            expectedParagraphs: 1,
            expectedTables: 0,
            expectedImages: 0,
            parts: packageParts({ body: body(h(`Heading ${level}`, level)) }),
            content: { paragraphs: [{ text: `Heading ${level}`, style: `Heading${level}` }], tables: [], images: [] },
        }),
    ),

    // ==================== tables ====================
    {
        name: 'tables/simple_2x2',
        description: 'Таблица 2x2',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(table([['A1', 'A2'], ['B1', 'B2']], { borders: true })) }),
        content: {
            paragraphs: [],
            tables: [{ rows: 2, cols: 2, cells: [['A1', 'A2'], ['B1', 'B2']] }],
            images: [],
        },
    },
    {
        name: 'tables/simple_3x3',
        description: 'Таблица 3x3',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({
            body: body(table([['A1', 'A2', 'A3'], ['B1', 'B2', 'B3'], ['C1', 'C2', 'C3']], { borders: true })),
        }),
        content: {
            paragraphs: [],
            tables: [
                { rows: 3, cols: 3, cells: [['A1', 'A2', 'A3'], ['B1', 'B2', 'B3'], ['C1', 'C2', 'C3']] },
            ],
            images: [],
        },
    },

    // ==================== complex ====================
    {
        name: 'complex/text_and_table',
        description: 'Текст + таблица + текст',
        expectedParagraphs: 2,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({
            body: body(p('Text before'), table([['H1', 'H2'], ['C1', 'C2']], { borders: true }), p('Text after')),
        }),
        content: {
            paragraphs: [{ text: 'Text before' }, { text: 'Text after' }],
            tables: [{ rows: 2, cols: 2, cells: [['H1', 'H2'], ['C1', 'C2']] }],
            images: [],
        },
    },

    // ==================== edge_cases ====================
    {
        name: 'edge_cases/empty_paragraphs',
        description: 'Пустые абзацы',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({ body: body(p(''), p('Non-empty'), p('')) }),
        content: {
            paragraphs: [{ text: '' }, { text: 'Non-empty' }, { text: '' }],
            tables: [],
            images: [],
        },
    },
    {
        name: 'edge_cases/special_chars',
        description: 'Специальные символы',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({ body: body(p('<>&"\'Test&\'"<>')) }),
        content: { paragraphs: [{ text: '<>&"\'Test&\'"<>' }], tables: [], images: [] },
    },
    {
        name: 'edge_cases/multilingual',
        description: 'Разные языки',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({ body: body(p('English'), p('Русский'), p('中文')) }),
        content: {
            paragraphs: [{ text: 'English' }, { text: 'Русский' }, { text: '中文' }],
            tables: [],
            images: [],
        },
    },
];
