// Сноски и комментарии: `word/footnotes.xml` и `word/comments.xml` — отдельные
// части пакета, их абзацы не входят в `expectedParagraphs` (там только прямые
// дети `w:body`). Служебные сноски-разделители (`w:type="separator"`) не текст,
// а разметка, поэтому в `content.footnotes` их место — отдельный список.
//
// Автор, дата и текст фиксированы — иначе сайдкар недетерминирован.

import {
    type FixtureSpec,
    R_NS,
    W_NS,
    XML_DECL,
    defaultSectPr,
    escapeXml,
    packageParts,
    para,
    run,
    sectionProps,
    tr,
} from './kit.js';

const AUTHOR = 'Fixture Author';
const DATE = '2026-01-01T00:00:00Z';

/** Стили сносок и комментариев: в defaultStylesXml их нет, а ссылки на них — есть. */
const NOTE_STYLES =
    '<w:style w:type="paragraph" w:styleId="FootnoteText"><w:name w:val="footnote text"/>' +
    '<w:basedOn w:val="Normal"/><w:link w:val="FootnoteTextChar"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/>' +
    '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>' +
    '<w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>' +
    '<w:style w:type="character" w:customStyle="1" w:styleId="FootnoteTextChar">' +
    '<w:name w:val="Footnote Text Char"/><w:basedOn w:val="DefaultParagraphFont"/><w:link w:val="FootnoteText"/>' +
    '<w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>' +
    '<w:style w:type="character" w:styleId="FootnoteReference"><w:name w:val="footnote reference"/>' +
    '<w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/>' +
    '<w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>' +
    '<w:style w:type="paragraph" w:styleId="CommentText"><w:name w:val="annotation text"/>' +
    '<w:basedOn w:val="Normal"/><w:link w:val="CommentTextChar"/><w:semiHidden/><w:unhideWhenUsed/>' +
    '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>' +
    '<w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>' +
    '<w:style w:type="character" w:customStyle="1" w:styleId="CommentTextChar">' +
    '<w:name w:val="Comment Text Char"/><w:basedOn w:val="DefaultParagraphFont"/><w:link w:val="CommentText"/>' +
    '<w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>' +
    '<w:style w:type="character" w:styleId="CommentReference"><w:name w:val="annotation reference"/>' +
    '<w:semiHidden/><w:unhideWhenUsed/><w:rPr><w:sz w:val="16"/><w:szCs w:val="16"/></w:rPr></w:style>';

/** Тело из блоков + финальный `w:sectPr`. */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

const footnotesXml = (...notes: string[]): string =>
    XML_DECL + `<w:footnotes xmlns:w="${W_NS}" xmlns:r="${R_NS}">${notes.join('')}</w:footnotes>`;

/** Обычная сноска: номер-надстрочник и текст. */
const footnote = (id: number, text: string): string =>
    `<w:footnote w:id="${id}"><w:p><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr>` +
    '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/></w:r>' +
    `<w:r><w:t xml:space="preserve"> ${escapeXml(text)}</w:t></w:r></w:p></w:footnote>`;

/**
 * Служебная сноска-разделитель: id -1 и 0 зарезервированы под `w:separator`
 * и `w:continuationSeparator`.
 */
const separatorFootnote = (type: 'separator' | 'continuationSeparator', id: number, element: string): string =>
    `<w:footnote w:type="${type}" w:id="${id}"><w:p>` +
    '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>' +
    `<w:r><w:${element}/></w:r></w:p></w:footnote>`;

/** Прогон со ссылкой на сноску. */
const footnoteRef = (id: number): string =>
    `<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteReference w:id="${id}"/></w:r>`;

const commentsXml = (...comments: string[]): string =>
    XML_DECL + `<w:comments xmlns:w="${W_NS}" xmlns:r="${R_NS}">${comments.join('')}</w:comments>`;

const comment = (id: number, text: string): string =>
    `<w:comment w:id="${id}" w:author="${AUTHOR}" w:date="${DATE}" w:initials="FA">` +
    `<w:p><w:pPr><w:pStyle w:val="CommentText"/></w:pPr><w:r><w:t xml:space="preserve">${escapeXml(text)}</w:t></w:r></w:p>` +
    '</w:comment>';

/** Диапазон комментария вокруг прогонов + сам `w:commentReference`. */
const commentAnchor = (id: number, runs: string): string =>
    `<w:commentRangeStart w:id="${id}"/>${runs}<w:commentRangeEnd w:id="${id}"/>` +
    `<w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="${id}"/></w:r>`;

const footnoteBasic: FixtureSpec = {
    name: 'notes/footnote_basic',
    description: 'Сноска в конце абзаца (footnotes.xml + w:footnoteReference)',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(para('', run('Text with a footnote') + footnoteRef(1))),
        stylesExtra: NOTE_STYLES,
        footnotes: footnotesXml(footnote(1, 'Footnote text.')),
    }),
    content: {
        footnotes: [{ id: 1, text: 'Footnote text.' }],
    },
};

const footnoteSeparator: FixtureSpec = {
    name: 'notes/footnote_separator',
    description: 'Сноска вместе со служебными separator и continuationSeparator',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(para('', run('Text with a footnote') + footnoteRef(1))),
        stylesExtra: NOTE_STYLES,
        footnotes: footnotesXml(
            separatorFootnote('separator', -1, 'separator'),
            separatorFootnote('continuationSeparator', 0, 'continuationSeparator'),
            footnote(1, 'Footnote after separators.'),
        ),
    }),
    content: {
        footnotes: [{ id: 1, text: 'Footnote after separators.' }],
        separators: [
            { id: -1, type: 'separator' },
            { id: 0, type: 'continuationSeparator' },
        ],
    },
};

const footnoteInTable: FixtureSpec = {
    name: 'notes/footnote_in_table',
    description: 'Сноска внутри ячейки таблицы',
    expectedParagraphs: 0,
    expectedTables: 1,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        // table() собирает ячейки из текста, а со ссылкой нужен свой прогон — таблица руками.
        body: body(
            '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>' +
                ['top', 'left', 'bottom', 'right', 'insideH', 'insideV']
                    .map((side) => `<w:${side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>`)
                    .join('') +
                '</w:tblBorders></w:tblPr>' +
                tr(
                    '<w:tc><w:tcPr><w:tcW w:w="4675" w:type="dxa"/></w:tcPr>' +
                        para('', run('Cell with note') + footnoteRef(1)) +
                        '</w:tc>' +
                        '<w:tc><w:tcPr><w:tcW w:w="4675" w:type="dxa"/></w:tcPr>' +
                        para('', run('Plain cell')) +
                        '</w:tc>',
                ) +
                '</w:tbl>',
        ),
        stylesExtra: NOTE_STYLES,
        footnotes: footnotesXml(footnote(1, 'Footnote from a table cell.')),
    }),
    content: {
        tables: [{ rows: 1, cols: 2, cells: [['Cell with note', 'Plain cell']] }],
        footnotes: [{ id: 1, text: 'Footnote from a table cell.', inTable: true }],
    },
};

const commentsBasic: FixtureSpec = {
    name: 'notes/comments_basic',
    description: 'Один комментарий: commentRangeStart/End и commentReference',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(para('', commentAnchor(1, run('Annotated text')))),
        stylesExtra: NOTE_STYLES,
        comments: commentsXml(comment(1, 'Comment text.')),
    }),
    content: {
        comments: [{ id: 1, author: AUTHOR, date: DATE, text: 'Comment text.' }],
    },
};

const commentsMultiple: FixtureSpec = {
    name: 'notes/comments_multiple',
    description: 'Три комментария, один из них — в ячейке таблицы',
    expectedParagraphs: 2,
    expectedTables: 1,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            para('', commentAnchor(1, run('First annotated text'))),
            para('', commentAnchor(2, run('Second annotated text'))),
            '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>' +
                tr(
                    '<w:tc><w:tcPr><w:tcW w:w="9350" w:type="dxa"/></w:tcPr>' +
                        para('', commentAnchor(3, run('Annotated cell'))) +
                        '</w:tc>',
                ) +
                '</w:tbl>',
        ),
        stylesExtra: NOTE_STYLES,
        comments: commentsXml(
            comment(1, 'First comment.'),
            comment(2, 'Second comment.'),
            comment(3, 'Comment inside a table cell.'),
        ),
    }),
    content: {
        tables: [{ rows: 1, cols: 1, cells: [['Annotated cell']] }],
        comments: [
            { id: 1, author: AUTHOR, date: DATE, text: 'First comment.' },
            { id: 2, author: AUTHOR, date: DATE, text: 'Second comment.' },
            { id: 3, author: AUTHOR, date: DATE, text: 'Comment inside a table cell.', inTable: true },
        ],
    },
};

/** Пять фикстур категории `notes`. */
export const notesFixtures: FixtureSpec[] = [
    footnoteBasic,
    footnoteSeparator,
    footnoteInTable,
    commentsBasic,
    commentsMultiple,
];
