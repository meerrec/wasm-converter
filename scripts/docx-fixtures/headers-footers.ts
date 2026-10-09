// Колонтитулы: default/first/even, `w:titlePg`, поля PAGE/NUMPAGES и картинка
// в колонтитуле. Абзацы колонтитулов живут в отдельных частях (`word/headerN.xml`),
// поэтому в `expectedParagraphs` не входят — там только прямые дети `w:body`.
// Изображение в колонтитуле по той же причине не попадает в `expectedImages`:
// счётчик описывает тело документа.
//
// rId колонтитулов предсказуемы: `headerRelId(i)`/`footerRelId(i)` совпадают с
// relationship'ами, которые packageParts заводит по порядку ключей `headers`/`footers`.

import {
    type FixtureSpec,
    A_NS,
    PIC_NS,
    R_NS,
    REL_NS,
    W_NS,
    WP_NS,
    XML_DECL,
    defaultSettingsXml,
    footerRelId,
    headerRelId,
    imageRelId,
    inlineImage,
    noisePng,
    p,
    packageParts,
    para,
    run,
} from './kit.js';

/** Тип relationship картинки (в kit.ts это внутренняя константа). */
const IMAGE_REL_TYPE = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/image';

/** Параметры страницы; `w:docGrid` добавляется в `section()` последним (порядок CT_SectPr). */
const PAGE_SETUP =
    '<w:pgSz w:w="11906" w:h="16838"/>' +
    '<w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="709" w:footer="709" w:gutter="0"/>' +
    '<w:cols w:space="708"/>';

/**
 * `w:sectPr` с ссылками на колонтитулы.
 *
 * @param refs `w:headerReference`/`w:footerReference` — по схеме идут первыми
 * @param extra то, что в CT_SectPr стоит после `w:cols` (`w:titlePg`)
 */
const section = (refs: string, extra = ''): string =>
    `<w:sectPr>${refs}${PAGE_SETUP}${extra}<w:docGrid w:linePitch="360"/></w:sectPr>`;

const headerXml = (inner: string): string =>
    XML_DECL +
    `<w:hdr xmlns:w="${W_NS}" xmlns:r="${R_NS}" xmlns:wp="${WP_NS}" xmlns:a="${A_NS}" xmlns:pic="${PIC_NS}">` +
    `${inner}</w:hdr>`;

const footerXml = (inner: string): string =>
    XML_DECL +
    `<w:ftr xmlns:w="${W_NS}" xmlns:r="${R_NS}" xmlns:wp="${WP_NS}" xmlns:a="${A_NS}" xmlns:pic="${PIC_NS}">` +
    `${inner}</w:ftr>`;

/** Поле через fldChar begin/instrText/separate/result/end. */
const field = (instruction: string, result: string): string =>
    '<w:r><w:fldChar w:fldCharType="begin"/></w:r>' +
    `<w:r><w:instrText xml:space="preserve"> ${instruction} </w:instrText></w:r>` +
    '<w:r><w:fldChar w:fldCharType="separate"/></w:r>' +
    run(result) +
    '<w:r><w:fldChar w:fldCharType="end"/></w:r>';

const defaultHeaderFooter: FixtureSpec = {
    name: 'headers_footers/default_header_footer',
    description: 'Колонтитулы default: header1.xml и footer1.xml',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body:
            p('Body text') +
            section(
                `<w:headerReference w:type="default" r:id="${headerRelId(0)}"/>` +
                    `<w:footerReference w:type="default" r:id="${footerRelId(0)}"/>`,
            ),
        headers: { 'header1.xml': headerXml(para('', run('Default header'))) },
        footers: { 'footer1.xml': footerXml(para('', run('Default footer'))) },
    }),
    content: {
        headers: [{ type: 'default', part: 'word/header1.xml', text: 'Default header' }],
        footers: [{ type: 'default', part: 'word/footer1.xml', text: 'Default footer' }],
    },
};

const titlePgFirst: FixtureSpec = {
    name: 'headers_footers/title_pg_first',
    description: 'Отдельный колонтитул первой страницы (w:titlePg + w:type="first")',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body:
            p('First page body') +
            section(
                `<w:headerReference w:type="default" r:id="${headerRelId(0)}"/>` +
                    `<w:headerReference w:type="first" r:id="${headerRelId(1)}"/>`,
                '<w:titlePg/>',
            ),
        headers: {
            'header1.xml': headerXml(para('', run('Default header'))),
            'header2.xml': headerXml(para('', run('First page header'))),
        },
    }),
    content: {
        headers: [
            { type: 'default', part: 'word/header1.xml', text: 'Default header' },
            { type: 'first', part: 'word/header2.xml', text: 'First page header' },
        ],
        titlePg: true,
    },
};

const evenOdd: FixtureSpec = {
    name: 'headers_footers/even_odd',
    description: 'Разные колонтитулы чётных и нечётных страниц (w:evenAndOddHeaders)',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body:
            p('Body text') +
            section(
                `<w:headerReference w:type="default" r:id="${headerRelId(0)}"/>` +
                    `<w:headerReference w:type="first" r:id="${headerRelId(1)}"/>` +
                    `<w:headerReference w:type="even" r:id="${headerRelId(2)}"/>`,
                '<w:titlePg/>',
            ),
        // Без w:evenAndOddHeaders ссылку type="even" парсер вправе игнорировать.
        settings: defaultSettingsXml('<w:evenAndOddHeaders/>'),
        headers: {
            'header1.xml': headerXml(para('', run('Default header'))),
            'header2.xml': headerXml(para('', run('First page header'))),
            'header3.xml': headerXml(para('', run('Even page header'))),
        },
    }),
    content: {
        headers: [
            { type: 'default', part: 'word/header1.xml', text: 'Default header' },
            { type: 'first', part: 'word/header2.xml', text: 'First page header' },
            { type: 'even', part: 'word/header3.xml', text: 'Even page header' },
        ],
        evenAndOddHeaders: true,
    },
};

const pageNumberField: FixtureSpec = {
    name: 'headers_footers/page_number_field',
    description: 'Поля PAGE и NUMPAGES в колонтитулах (fldChar + instrText)',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body:
            p('Body text') +
            section(
                `<w:headerReference w:type="default" r:id="${headerRelId(0)}"/>` +
                    `<w:footerReference w:type="default" r:id="${footerRelId(0)}"/>`,
            ),
        headers: { 'header1.xml': headerXml(para('', run('Page ') + field('PAGE', '1'))) },
        footers: { 'footer1.xml': footerXml(para('', run('of ') + field('NUMPAGES', '3'))) },
    }),
    content: {
        fields: [
            { instruction: 'PAGE', part: 'word/header1.xml', result: '1' },
            { instruction: 'NUMPAGES', part: 'word/footer1.xml', result: '3' },
        ],
    },
};

const headerWithImage: FixtureSpec = {
    name: 'headers_footers/header_with_image',
    description: 'Картинка в колонтитуле: свои rels у header1.xml',
    expectedParagraphs: 1,
    expectedTables: 0,
    // Картинка лежит в word/media, но ссылается на неё колонтитул, а не тело.
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: p('Body text') + section(`<w:headerReference w:type="default" r:id="${headerRelId(0)}"/>`),
        headers: {
            'header1.xml': headerXml(
                para(
                    '',
                    '<w:r>' +
                        inlineImage(imageRelId(0), { cx: 190500, cy: 166688, name: 'header-logo.png', id: 1 }) +
                        '</w:r>',
                ),
            ),
        },
        images: { 'image1.png': noisePng(8, 7) },
        // Ссылка на media/ резолвится относительно word/, поэтому Target без ведущего слэша.
        extraParts: {
            'word/_rels/header1.xml.rels':
                XML_DECL +
                `<Relationships xmlns="${REL_NS}">` +
                `<Relationship Id="${imageRelId(0)}" Type="${IMAGE_REL_TYPE}" Target="media/image1.png"/>` +
                '</Relationships>',
        },
    }),
    content: {
        headers: [{ type: 'default', part: 'word/header1.xml', imageRel: imageRelId(0) }],
        media: [{ part: 'word/media/image1.png', rel: imageRelId(0), inPart: 'word/header1.xml' }],
    },
};

/** Пять фикстур категории `headers_footers`. */
export const headersFootersFixtures: FixtureSpec[] = [
    defaultHeaderFooter,
    titlePgFirst,
    evenOdd,
    pageNumberField,
    headerWithImage,
];
