// Фикстуры категории cjk: китайский, японский, корейский, смешанный набор и
// вертикальное письмо.
//
// CJK-шрифт обязан стоять в `w:eastAsia` — слота `w:ascii`/`w:hAnsi` для
// иероглифов недостаточно, поэтому шрифты задаются через `w:rFonts` целиком.

import {
    type FixtureSpec,
    defaultSettingsXml,
    defaultSectPr,
    packageParts,
    para,
    run,
    sectionProps,
} from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** `w:rFonts` со всеми четырьмя слотами: CJK-шрифт должен быть и в `eastAsia`. */
const cjkFonts = (font: string): string =>
    `<w:rFonts w:ascii="${font}" w:hAnsi="${font}" w:eastAsia="${font}" w:cs="${font}"/>`;

/** Прогон с CJK-шрифтом и, если задан, языком восточноазиатского текста. */
const cjkRun = (text: string, font: string, lang?: string): string =>
    run(text, {
        rPr: cjkFonts(font) + (lang === undefined ? '' : `<w:lang w:val="${lang}" w:eastAsia="${lang}"/>`),
    });

/** Прогон латиницы: `eastAsia` не задан, иероглифы возьмут шрифт из docDefaults. */
const latinRun = (text: string, font = 'Calibri'): string =>
    run(text, { rPr: `<w:rFonts w:ascii="${font}" w:hAnsi="${font}"/>` });

/** `sectPr` вертикального письма: `w:textDirection` идёт после `w:cols`, до `w:docGrid`. */
const verticalSectPr = (): string =>
    '<w:sectPr>' +
    '<w:pgSz w:w="11906" w:h="16838"/>' +
    '<w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="709" w:footer="709" w:gutter="0"/>' +
    '<w:cols w:space="708"/><w:textDirection w:val="tbRl"/><w:docGrid w:linePitch="360"/>' +
    '</w:sectPr>';

export const cjkFixtures: FixtureSpec[] = [
    {
        name: 'cjk/chinese_simplified',
        description: 'Упрощённый китайский: eastAsia-шрифт SimSun и полноширинные знаки',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(para('', cjkRun('简体中文测试：汉字、标点，句号。', 'SimSun', 'zh-CN'))),
        }),
        content: {
            paragraphs: [
                {
                    text: '简体中文测试：汉字、标点，句号。',
                    runs: [{ text: '简体中文测试：汉字、标点，句号。', font: 'SimSun', eastAsia: 'zh-CN' }],
                },
            ],
        },
    },
    {
        name: 'cjk/japanese',
        description: 'Японский: lang eastAsia="ja-JP" и правила кинсоку в настройках',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(para('', cjkRun('日本語のテスト。漢字とかなの混在。', 'MS Mincho', 'ja-JP'))),
            settings: defaultSettingsXml('<w:kinsoku/><w:overflowPunct/><w:autoSpaceDE/><w:autoSpaceDN/>'),
        }),
        content: {
            paragraphs: [
                {
                    text: '日本語のテスト。漢字とかなの混在。',
                    runs: [{ text: '日本語のテスト。漢字とかなの混在。', font: 'MS Mincho', eastAsia: 'ja-JP' }],
                },
            ],
            settings: { kinsoku: true, overflowPunct: true },
        },
    },
    {
        name: 'cjk/korean',
        description: 'Корейский хангыль: eastAsia-шрифт Malgun Gothic',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(para('', cjkRun('한국어 테스트입니다. 한글 자모 조합.', 'Malgun Gothic', 'ko-KR'))),
        }),
        content: {
            paragraphs: [
                {
                    text: '한국어 테스트입니다. 한글 자모 조합.',
                    runs: [{ text: '한국어 테스트입니다. 한글 자모 조합.', font: 'Malgun Gothic', eastAsia: 'ko-KR' }],
                },
            ],
        },
    },
    {
        name: 'cjk/mixed_latin',
        description: 'Смешанный абзац: латиница и CJK в разных прогонах',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                para(
                    '',
                    latinRun('Hello ') +
                        run('中文', { rPr: `<w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="SimSun"/>` }) +
                        latinRun(' world'),
                ),
            ),
        }),
        content: {
            paragraphs: [
                {
                    text: 'Hello 中文 world',
                    runs: [
                        { text: 'Hello ', font: 'Calibri', eastAsia: null },
                        { text: '中文', font: 'Calibri', eastAsia: 'SimSun' },
                        { text: ' world', font: 'Calibri', eastAsia: null },
                    ],
                },
            ],
        },
    },
    {
        name: 'cjk/vertical_text',
        description: 'Вертикальное письмо: textDirection tbRl в sectPr',
        expectedParagraphs: 1,
        expectedTables: 0,
        expectedImages: 0,
        parts: packageParts({
            body: para('', cjkRun('縦書きのテキストです。', 'MS Mincho', 'ja-JP')) + verticalSectPr(),
        }),
        content: {
            paragraphs: [{ text: '縦書きのテキストです。', textDirection: 'tbRl' }],
            section: { textDirection: 'tbRl' },
        },
    },
];
