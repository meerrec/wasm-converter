// Фикстуры категории tables/: длинные таблицы, разрыв строк между страницами
// (`w:cantSplit`, `w:trHeight`), строка выше страницы, таблица шире полосы
// набора, поля ячеек (`w:tblCellMar`/`w:tcMar`) и вертикальное выравнивание.
//
// Отдельный файл, а не дополнение к `tables.ts`: эти фикстуры обслуживают
// постраничную раскладку (`crates/docx/benches/pagination.rs` и оракул
// пагинации), а не разбор свойств таблиц.

import { type FixtureSpec, defaultSectPr, p, packageParts, sectionProps } from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Ячейка с произвольными блоками: `tc()` из kit умеет только один абзац. */
const cell = (tcPr: string, blocks: string): string =>
    `<w:tc>${tcPr === '' ? '' : `<w:tcPr>${tcPr}</w:tcPr>`}${blocks}</w:tc>`;

/** Ячейка с `w:tcW` и одним абзацем. */
const cellW = (text: string, width: number, extraTcPr = ''): string =>
    cell(`<w:tcW w:w="${width}" w:type="dxa"/>${extraTcPr}`, p(text));

/** Строка `<w:tr>` из готовых ячеек. */
const row = (cells: string, trPr = ''): string =>
    `<w:tr>${trPr === '' ? '' : `<w:trPr>${trPr}</w:trPr>`}${cells}</w:tr>`;

/** Таблица: `w:tblGrid` идёт за `w:tblPr` — порядок детей в CT_Tbl жёсткий. */
const tbl = (tblPr: string, grid: number[], rows: string[]): string =>
    `<w:tbl><w:tblPr>${tblPr}</w:tblPr>` +
    `<w:tblGrid>${grid.map((w) => `<w:gridCol w:w="${w}"/>`).join('')}</w:tblGrid>` +
    rows.join('') +
    '</w:tbl>';

const BORDER_SIDES = ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'] as const;

const tblBorders = (sz: number): string =>
    '<w:tblBorders>' +
    BORDER_SIDES.map((side) => `<w:${side} w:val="single" w:sz="${sz}" w:space="0" w:color="auto"/>`).join('') +
    '</w:tblBorders>';

/** Полоса набора A4 с полями по умолчанию: 11906 − 2×1134 = 9638 twips. */
const CONTENT_WIDTH = 9638;

/** Три равных столбца; последний добирает остаток, чтобы сумма сошлась с полосой. */
const THIRDS = [3212, 3213, 3213];

const borderedTblPr = tblBorders(4) + '<w:tblW w:w="0" w:type="auto"/>';

/** Таблица из `rowsCount` строк по 3 колонки простого текста. */
const uniformTable = (
    rowsCount: number,
    columns: readonly string[],
    opts: { tblPr?: string } = {},
): string => {
    const rows: string[] = [];
    for (let i = 0; i < rowsCount; i++) {
        rows.push(
            row(THIRDS.map((width, column) => cellW(`${columns[column]!} ${i}`, width)).join('')),
        );
    }
    return tbl(opts.tblPr ?? borderedTblPr, THIRDS, rows);
};

const LONG_TABLE_ROWS = 60;
const LONG_TABLE_COLUMNS = ['Строка', 'Значение', 'Комментарий'] as const;
const LONG_TABLE = uniformTable(LONG_TABLE_ROWS, LONG_TABLE_COLUMNS);

const HEADER_REPEAT_ROWS = 40;
const HEADER_REPEAT_COLUMNS = ['Колонка A', 'Колонка B', 'Колонка C'] as const;
const HEADER_REPEAT_TEXT = ['Заголовок A', 'Заголовок B', 'Заголовок C'] as const;

/** 40 строк: первая помечена `w:tblHeader` и обязана повторяться на страницах. */
const headerRepeatTable = (() => {
    const rows: string[] = [];
    for (let i = 0; i < HEADER_REPEAT_ROWS; i++) {
        const isHeader = i === 0;
        rows.push(
            row(
                THIRDS.map((width, column) =>
                    cellW(
                        isHeader ? HEADER_REPEAT_TEXT[column]! : `${HEADER_REPEAT_COLUMNS[column]!} ${i}`,
                        width,
                    ),
                ).join(''),
                isHeader ? '<w:tblHeader/>' : '',
            ),
        );
    }
    return tbl(borderedTblPr, THIRDS, rows);
})();

/** Число абзацев в высокой строке `cant_split_rows`. */
const TALL_ROW_PARAGRAPHS = 12;

/** Строка, которая не влезает в остаток страницы: целиком уходит на следующую. */
const tallRow = row(
    cell(
        `<w:tcW w:w="4819" w:type="dxa"/>`,
        Array.from({ length: TALL_ROW_PARAGRAPHS }, (_, i) => p(`Абзац высокой строки ${i + 1}`)).join(''),
    ) + cellW('Соседняя ячейка высокой строки', 4819),
    '<w:cantSplit/><w:trHeight w:val="600" w:hRule="atLeast"/>',
);

/** 12 строк: все с `w:cantSplit`, одна — высокая (несколько абзацев в ячейке). */
const cantSplitTable = (() => {
    const rows: string[] = [];
    for (let i = 0; i < 12; i++) {
        if (i === 6) {
            rows.push(tallRow);
            continue;
        }
        rows.push(
            row(cellW(`Строка ${i + 1}`, 4819) + cellW(`Значение ${i + 1}`, 4819), '<w:cantSplit/>'),
        );
    }
    return tbl(borderedTblPr, [4819, 4819], rows);
})();

/**
 * Абзацы строки выше страницы A4: полоса набора по умолчанию — 971 px,
 * однострочный абзац занимает около 30 px, поэтому 60 абзацев заведомо
 * перекрывают страницу с запасом в полтора раза.
 */
const OVERFLOW_PARAGRAPHS = 60;

const rowTallerThanPage = [
    row(
        cell(
            '<w:tcW w:w="9638" w:type="dxa"/>',
            Array.from({ length: OVERFLOW_PARAGRAPHS }, (_, i) => p(`Абзац ${i + 1} высокой строки`)).join(''),
        ),
    ),
    row(cellW('Строка после высокой', 9638)),
].join('');

const rowTallerThanPageTable = tbl(borderedTblPr, [9638], [rowTallerThanPage]);

/** Ширины столбцов в `w:tcW`, сумма которых почти вдвое шире полосы набора. */
const OVERWIDE_COLUMN = 4800;
const overwideTable = tbl(
    '<w:tblW w:w="14400" w:type="dxa"/><w:tblLayout w:type="fixed"/>' + tblBorders(4),
    [OVERWIDE_COLUMN, OVERWIDE_COLUMN, OVERWIDE_COLUMN],
    [
        row(
            [0, 1, 2].map((column) => cellW(`Колонка ${'ABC'[column]!}`, OVERWIDE_COLUMN)).join(''),
        ),
        row(
            [0, 1, 2].map((column) => cellW(`Значение ${'ABC'[column]!}`, OVERWIDE_COLUMN)).join(''),
        ),
    ],
);

/** Ячейка с полями `w:tcMar` и одним длинным абзацем: `w:vAlign` виден на её высоте. */
const marginsCell = (text: string, valign: string, tcMar: string): string =>
    cell(
        `<w:tcW w:w="3212" w:type="dxa"/><w:tcMar>${tcMar}</w:tcMar><w:vAlign w:val="${valign}"/>`,
        p(text),
    );

const cellMarginsTable = tbl(
    '<w:tblW w:w="0" w:type="auto"/>' +
        // Умолчания таблицы: Word пишет top/bottom = 0, left/right = 108.
        '<w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>' +
        '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar>' +
        tblBorders(4),
    THIRDS,
    [
        row(
            cellW('Сверху', 3212, '<w:vAlign w:val="top"/>') +
                cellW('По центру', 3213, '<w:vAlign w:val="center"/>') +
                cellW('Снизу', 3213, '<w:vAlign w:val="bottom"/>'),
        ),
        row(
            marginsCell(
                'Большие поля сверху и снизу',
                'center',
                '<w:top w:w="480" w:type="dxa"/><w:bottom w:w="480" w:type="dxa"/>',
            ) +
                marginsCell(
                    'Широкие боковые поля',
                    'top',
                    '<w:left w:w="720" w:type="dxa"/><w:right w:w="720" w:type="dxa"/>',
                ) +
                cell(
                    '<w:tcW w:w="3213" w:type="dxa"/><w:vAlign w:val="bottom"/>' +
                        '<w:tcMar><w:top w:w="0" w:type="dxa"/><w:left w:w="0" w:type="dxa"/>' +
                        '<w:bottom w:w="240" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tcMar>',
                    p('Поля ячейки перекрывают таблицу') + p('Второй абзац для высоты'),
                ),
        ),
    ],
);

/**
 * Ровно ~100 страниц A4: строки однострочные, высота строки почти постоянна,
 * поэтому число страниц ≈ строки / строк-на-странице. Точное число строк
 * подобрано замером раскладки (`cargo bench -p doc-converter-docx --bench
 * pagination` печатает число страниц): 3300 строк дали 69 страниц,
 * 4800 строк ≈ 100. Фикстура нужна бенчмарку пагинации и оракулу.
 */
const HUNDRED_PAGES_ROWS = 4800;
/** Таблица на сотню страниц — без рамок: рисунок здесь не важен, важен объём. */
const hundredPagesTable = uniformTable(HUNDRED_PAGES_ROWS, ['Строка', 'Значение', 'Примечание'], {
    tblPr: '<w:tblW w:w="0" w:type="auto"/>',
});

/** Короткое описание больших таблиц: перечислять 3300 строк в сайдкаре незачем. */
const bigTableContent = (rows: number, sample: string[][]): Record<string, unknown> => ({
    rows,
    cols: 3,
    grid: THIRDS,
    sampleCells: sample,
});

export const tablesLayoutFixtures: FixtureSpec[] = [
    {
        name: 'tables/long_table',
        description: '60 строк по 3 колонки простого текста — таблица заведомо больше страницы',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(LONG_TABLE) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: LONG_TABLE_ROWS,
                    cols: 3,
                    grid: THIRDS,
                    borders: { val: 'single', sz: 4, color: 'auto', sides: 'all' },
                    cells: Array.from({ length: LONG_TABLE_ROWS }, (_, i) => [
                        `Строка ${i}`,
                        `Значение ${i}`,
                        `Комментарий ${i}`,
                    ]),
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/long_table_header_repeat',
        description: '40 строк, первая строка помечена w:tblHeader и повторяется на каждой странице',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(headerRepeatTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: HEADER_REPEAT_ROWS,
                    cols: 3,
                    grid: THIRDS,
                    headerRows: 1,
                    repeatHeader: true,
                    cells: Array.from({ length: HEADER_REPEAT_ROWS }, (_, i) =>
                        i === 0
                            ? [...HEADER_REPEAT_TEXT]
                            : HEADER_REPEAT_COLUMNS.map((name) => `${name} ${i}`),
                    ),
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/cant_split_rows',
        description: '12 строк с w:cantSplit; седьмая строка высокая (несколько абзацев и w:trHeight atLeast)',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(cantSplitTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 12,
                    cols: 2,
                    grid: [4819, 4819],
                    cantSplit: 'all',
                    rowHeights: [{ row: 6, val: 600, hRule: 'atLeast' }],
                    tallRow: { row: 6, paragraphs: TALL_ROW_PARAGRAPHS + 1 },
                    cells: [
                        ['Строка 1', 'Значение 1'],
                        ['Строка 2', 'Значение 2'],
                        ['Строка 3', 'Значение 3'],
                        ['Строка 4', 'Значение 4'],
                        ['Строка 5', 'Значение 5'],
                        ['Строка 6', 'Значение 6'],
                        ['Абзац высокой строки 1 Абзац высокой строки 12', 'Соседняя ячейка высокой строки'],
                        ['Строка 8', 'Значение 8'],
                        ['Строка 9', 'Значение 9'],
                        ['Строка 10', 'Значение 10'],
                        ['Строка 11', 'Значение 11'],
                        ['Строка 12', 'Значение 12'],
                    ],
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/row_taller_than_page',
        description: 'Строка с 60 абзацами в ячейке — выше полосы набора страницы A4',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(rowTallerThanPageTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 1,
                    grid: [9638],
                    tallerThanPage: { row: 0, paragraphs: OVERFLOW_PARAGRAPHS },
                    cells: [
                        ['Абзац 1 высокой строки Абзац 60 высокой строки'],
                        ['Строка после высокой'],
                    ],
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/table_wider_than_page',
        description: 'w:tblW и w:tcW дают 14400 twips — таблица шире полосы набора (9638 twips)',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(overwideTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 3,
                    tableWidth: { w: 14400, type: 'dxa' },
                    contentWidth: CONTENT_WIDTH,
                    layout: 'fixed',
                    grid: [OVERWIDE_COLUMN, OVERWIDE_COLUMN, OVERWIDE_COLUMN],
                    widths: [
                        [OVERWIDE_COLUMN, OVERWIDE_COLUMN, OVERWIDE_COLUMN],
                        [OVERWIDE_COLUMN, OVERWIDE_COLUMN, OVERWIDE_COLUMN],
                    ],
                    cells: [
                        ['Колонка A', 'Колонка B', 'Колонка C'],
                        ['Значение A', 'Значение B', 'Значение C'],
                    ],
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/cell_margins_valign',
        description: 'Поля ячеек w:tblCellMar и w:tcMar, вертикальное выравнивание w:vAlign top/center/bottom',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(cellMarginsTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 3,
                    grid: THIRDS,
                    cellMargins: { top: 0, left: 108, bottom: 0, right: 108 },
                    vAlign: [
                        ['top', 'center', 'bottom'],
                        ['center', 'top', 'bottom'],
                    ],
                    cellMarginsOverride: [
                        { row: 1, col: 0, top: 480, bottom: 480 },
                        { row: 1, col: 1, left: 720, right: 720 },
                        { row: 1, col: 2, bottom: 240 },
                    ],
                    cells: [
                        ['Сверху', 'По центру', 'Снизу'],
                        [
                            'Большие поля сверху и снизу',
                            'Широкие боковые поля',
                            'Поля ячейки перекрывают таблицу Второй абзац для высоты',
                        ],
                    ],
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/table_100_pages',
        description: `Таблица ${HUNDRED_PAGES_ROWS} строк по 3 колонки — около 100 страниц A4 при печати по умолчанию`,
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        meta: { expectedPages: 100 },
        parts: packageParts({ body: body(hundredPagesTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    ...bigTableContent(HUNDRED_PAGES_ROWS, [
                        ['Строка 0', 'Значение 0', 'Примечание 0'],
                        ['Строка 1', 'Значение 1', 'Примечание 1'],
                    ]),
                    textPattern: `Строка {i} / Значение {i} / Примечание {i}, i = 0..${HUNDRED_PAGES_ROWS - 1}`,
                },
            ],
            images: [],
        },
    },
];
