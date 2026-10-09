// Track changes: `w:ins`/`w:del`/`w:moveFrom`/`w:moveTo` и изменения свойств
// (`w:rPrChange`/`w:pPrChange`). Разметку ревизий парсер сохраняет как Unknown,
// поэтому `expectedWarnings` у всех фикстур пуст: неизвестное в известном
// контексте — не ошибка, а содержимое.
//
// Автор и дата ревизий фиксированы строками — иначе сайдкар перестанет быть
// детерминированным.

import { type FixtureSpec, defaultSectPr, p, packageParts, para, run, sectionProps, tc, tr } from './kit.js';

const AUTHOR = 'Fixture Author';
const DATE = '2026-01-01T00:00:00Z';

/** Элемент ревизии вокруг готового XML. */
const rev = (tag: 'ins' | 'del' | 'moveFrom' | 'moveTo', id: number, inner: string): string =>
    `<w:${tag} w:id="${id}" w:author="${AUTHOR}" w:date="${DATE}">${inner}</w:${tag}>`;

/** Изменение знака абзаца: `w:ins`/`w:del` внутри `w:pPr/w:rPr`. */
const markChange = (tag: 'ins' | 'del', id: number): string =>
    `<w:rPr><w:${tag} w:id="${id}" w:author="${AUTHOR}" w:date="${DATE}"/></w:rPr>`;

/** Тело из блоков + финальный `w:sectPr`. */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

const insertedRun: FixtureSpec = {
    name: 'track_changes/inserted_run',
    description: 'Вставленный прогон (w:ins)',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(para('', run('Before ') + rev('ins', 1, run('Inserted text')))),
    }),
    content: {
        revisions: [{ type: 'ins', author: AUTHOR, date: DATE, text: 'Inserted text' }],
    },
};

const deletedRun: FixtureSpec = {
    name: 'track_changes/deleted_run',
    description: 'Удалённый прогон (w:delText, не w:t)',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        // Внутри `w:del` текст лежит в `w:delText` — `w:t` там невалиден.
        body: body(
            para(
                '',
                run('Visible text ') +
                    rev('del', 2, `<w:r><w:delText xml:space="preserve">Deleted text</w:delText></w:r>`),
            ),
        ),
    }),
    content: {
        revisions: [{ type: 'del', author: AUTHOR, date: DATE, text: 'Deleted text' }],
    },
};

const paragraphMarkChange: FixtureSpec = {
    name: 'track_changes/paragraph_mark_change',
    description: 'Изменение знака абзаца (w:pPr/w:rPr/w:ins и w:del)',
    expectedParagraphs: 2,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            para(markChange('ins', 3), run('Paragraph with inserted mark')),
            para(markChange('del', 4), run('Paragraph with deleted mark')),
        ),
    }),
    content: {
        revisions: [
            { type: 'ins', author: AUTHOR, date: DATE, target: 'paragraphMark' },
            { type: 'del', author: AUTHOR, date: DATE, target: 'paragraphMark' },
        ],
    },
};

const moveAndFormatChange: FixtureSpec = {
    name: 'track_changes/move_and_format_change',
    description: 'Перемещение (w:moveFrom/w:moveTo) и изменения форматирования',
    expectedParagraphs: 3,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            // Пара маркеров диапазона обрамляет перенесённый фрагмент.
            para(
                `<w:moveFromRangeStart w:id="10" w:name="move1" w:author="${AUTHOR}" w:date="${DATE}"/>`,
                rev('moveFrom', 11, run('Moved away')) + '<w:moveFromRangeEnd w:id="10"/>',
            ),
            para(
                `<w:moveToRangeStart w:id="12" w:name="move1" w:author="${AUTHOR}" w:date="${DATE}"/>`,
                rev('moveTo', 11, run('Moved here')) + '<w:moveToRangeEnd w:id="12"/>',
            ),
            // Прежние свойства лежат внутри w:pPrChange/w:rPrChange, а не снаружи.
            para(
                `<w:pPrChange w:id="13" w:author="${AUTHOR}" w:date="${DATE}">` +
                    '<w:pPr><w:jc w:val="center"/></w:pPr></w:pPrChange>',
                `<w:r><w:rPr><w:i/><w:rPrChange w:id="14" w:author="${AUTHOR}" w:date="${DATE}">` +
                    '<w:rPr><w:b/></w:rPr></w:rPrChange></w:rPr>' +
                    '<w:t xml:space="preserve">Reformatted text</w:t></w:r>',
            ),
        ),
    }),
    content: {
        revisions: [
            { type: 'moveFrom', author: AUTHOR, date: DATE, text: 'Moved away' },
            { type: 'moveTo', author: AUTHOR, date: DATE, text: 'Moved here' },
            { type: 'pPrChange', author: AUTHOR, date: DATE },
            { type: 'rPrChange', author: AUTHOR, date: DATE },
        ],
    },
};

const tableRowChanges: FixtureSpec = {
    name: 'track_changes/table_row_changes',
    description: 'Вставка и удаление строки таблицы (w:ins/w:del в w:trPr, w:cellIns/w:cellDel)',
    expectedParagraphs: 0,
    expectedTables: 1,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>' +
                tr(
                    tc('Inserted row cell', `<w:cellIns w:id="20" w:author="${AUTHOR}" w:date="${DATE}"/>`),
                    `<w:ins w:id="21" w:author="${AUTHOR}" w:date="${DATE}"/>`,
                ) +
                tr(
                    tc('Deleted row cell', `<w:cellDel w:id="22" w:author="${AUTHOR}" w:date="${DATE}"/>`),
                    `<w:del w:id="23" w:author="${AUTHOR}" w:date="${DATE}"/>`,
                ) +
                '</w:tbl>',
        ),
    }),
    content: {
        tables: [{ rows: 2, cols: 1, cells: [['Inserted row cell'], ['Deleted row cell']] }],
        revisions: [
            { type: 'ins', author: AUTHOR, date: DATE, target: 'row' },
            { type: 'cellIns', author: AUTHOR, date: DATE, text: 'Inserted row cell' },
            { type: 'del', author: AUTHOR, date: DATE, target: 'row' },
            { type: 'cellDel', author: AUTHOR, date: DATE, text: 'Deleted row cell' },
        ],
    },
};

/** Пять фикстур категории `track_changes`. */
export const trackChangesFixtures: FixtureSpec[] = [
    insertedRun,
    deletedRun,
    paragraphMarkChange,
    moveAndFormatChange,
    tableRowChanges,
];
