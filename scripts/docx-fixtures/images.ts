// Фикстуры категории images: inline- и anchor-рисунки, типы обтекания, размеры.
//
// Картинки лежат в `word/media/`, rId выдаются по порядку ключей объекта
// `images`: `imageRelId(i)` = `rIdImg<i+1>`. Порядок ключей и порядок вставки
// рисунков в тело должны совпадать.

import {
    type FixtureSpec,
    anchoredImage,
    defaultSectPr,
    imageRelId,
    inlineImage,
    noisePng,
    packageParts,
    para,
    sectionProps,
} from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Абзац с одним рисунком в прогоне. */
const imageParagraph = (drawing: string): string => para('', `<w:r>${drawing}</w:r>`);

/** Абзац, где рисунок идёт вторым прогоном после текста. */
const textAndImageParagraph = (text: string, drawing: string): string =>
    para('', `<w:r><w:t xml:space="preserve">${text}</w:t></w:r><w:r>${drawing}</w:r>`);

export const imagesFixtures: FixtureSpec[] = [
    {
        name: 'images/inline',
        description: 'Inline-рисунок в абзаце',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            body: body(
                imageParagraph(
                    inlineImage(imageRelId(0), { id: 1, name: 'inline.png', cx: 914400, cy: 685800 }),
                ),
            ),
            images: { 'inline.png': noisePng(8, 0x1101) },
        }),
        content: {
            paragraphs: [{ text: '' }],
            images: [{ relId: imageRelId(0), file: 'inline.png', kind: 'inline', cx: 914400, cy: 685800 }],
        },
    },
    {
        name: 'images/anchor_wrap_square',
        description: 'Плавающий рисунок с обтеканием по квадрату и ненулевой позицией',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            body: body(
                imageParagraph(
                    anchoredImage(imageRelId(0), {
                        id: 1,
                        name: 'square.png',
                        cx: 1143000,
                        cy: 857250,
                        wrap: 'square',
                        hRelative: 'column',
                        posX: 457200,
                        posY: 228600,
                    }),
                ),
            ),
            images: { 'square.png': noisePng(12, 0x1102) },
        }),
        content: {
            paragraphs: [{ text: '' }],
            images: [
                {
                    relId: imageRelId(0),
                    file: 'square.png',
                    kind: 'anchor',
                    wrap: 'square',
                    hRelative: 'column',
                    posX: 457200,
                    posY: 228600,
                    behindDoc: false,
                    cx: 1143000,
                    cy: 857250,
                },
            ],
        },
    },
    {
        name: 'images/anchor_behind_text',
        description: 'Рисунок за текстом (behindDoc) без обтекания',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            body: body(
                textAndImageParagraph(
                    'Текст поверх рисунка',
                    anchoredImage(imageRelId(0), {
                        id: 1,
                        name: 'behind.png',
                        cx: 1371600,
                        cy: 914400,
                        wrap: 'none',
                        behindDoc: true,
                        hRelative: 'column',
                        posX: 0,
                        posY: 0,
                    }),
                ),
            ),
            images: { 'behind.png': noisePng(16, 0x1103) },
        }),
        content: {
            paragraphs: [{ text: 'Текст поверх рисунка' }],
            images: [
                {
                    relId: imageRelId(0),
                    file: 'behind.png',
                    kind: 'anchor',
                    wrap: 'none',
                    behindDoc: true,
                    cx: 1371600,
                    cy: 914400,
                },
            ],
        },
    },
    {
        name: 'images/anchor_wrap_tight',
        description: 'Плавающий рисунок с обтеканием по контуру',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            body: body(
                imageParagraph(
                    anchoredImage(imageRelId(0), {
                        id: 1,
                        name: 'tight.png',
                        cx: 685800,
                        cy: 685800,
                        wrap: 'tight',
                        hRelative: 'column',
                        posX: 228600,
                        posY: 114300,
                    }),
                ),
            ),
            images: { 'tight.png': noisePng(8, 0x1104) },
        }),
        content: {
            paragraphs: [{ text: '' }],
            images: [
                {
                    relId: imageRelId(0),
                    file: 'tight.png',
                    kind: 'anchor',
                    wrap: 'tight',
                    cx: 685800,
                    cy: 685800,
                },
            ],
        },
    },
    {
        name: 'images/multiple_sizes',
        description: 'Три рисунка разных размеров',
        expectedParagraphs: 3,
        expectedTables: 0,
        expectedImages: 3,
        parts: packageParts({
            body: body(
                imageParagraph(
                    inlineImage(imageRelId(0), { id: 1, name: 'small.png', cx: 457200, cy: 457200 }),
                ),
                imageParagraph(
                    inlineImage(imageRelId(1), { id: 2, name: 'wide.png', cx: 1371600, cy: 685800 }),
                ),
                imageParagraph(
                    inlineImage(imageRelId(2), { id: 3, name: 'tall.png', cx: 685800, cy: 1371600 }),
                ),
            ),
            images: {
                'small.png': noisePng(8, 0x1105),
                'wide.png': noisePng(24, 0x1106),
                'tall.png': noisePng(16, 0x1107),
            },
        }),
        content: {
            paragraphs: [{ text: '' }, { text: '' }, { text: '' }],
            images: [
                { relId: imageRelId(0), file: 'small.png', kind: 'inline', cx: 457200, cy: 457200 },
                { relId: imageRelId(1), file: 'wide.png', kind: 'inline', cx: 1371600, cy: 685800 },
                { relId: imageRelId(2), file: 'tall.png', kind: 'inline', cx: 685800, cy: 1371600 },
            ],
        },
    },
    {
        name: 'images/anchor_top_and_bottom',
        description: 'Плавающий рисунок с обтеканием сверху и снизу, привязка к странице',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 1,
        parts: packageParts({
            body: body(
                imageParagraph(
                    anchoredImage(imageRelId(0), {
                        id: 1,
                        name: 'topbottom.png',
                        cx: 2743200,
                        cy: 685800,
                        wrap: 'topAndBottom',
                        hRelative: 'page',
                        posX: 914400,
                        posY: 0,
                    }),
                ),
            ),
            images: { 'topbottom.png': noisePng(32, 0x1108) },
        }),
        content: {
            paragraphs: [{ text: '' }],
            images: [
                {
                    relId: imageRelId(0),
                    file: 'topbottom.png',
                    kind: 'anchor',
                    wrap: 'topAndBottom',
                    hRelative: 'page',
                    posX: 914400,
                    posY: 0,
                    cx: 2743200,
                    cy: 685800,
                },
            ],
        },
    },
];
