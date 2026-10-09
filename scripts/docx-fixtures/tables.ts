// Фикстуры категории tables/: вложенные таблицы, объединения (gridSpan/vMerge),
// tblLook, рамки и заливка, повтор шапки на страницах, ширины и выравнивание.

import {
    type FixtureSpec,
    defaultSectPr,
    p,
    pageBreak,
    packageParts,
    sectionProps,
    table,
    tr,
} from './kit.js';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Ячейка с произвольными блоками: `tc()` из kit умеет только один абзац. */
const cell = (tcPr: string, blocks: string): string =>
    `<w:tc>${tcPr === '' ? '' : `<w:tcPr>${tcPr}</w:tcPr>`}${blocks}</w:tc>`;

/** Ячейка с `w:tcW` и одним абзацем. */
const cellW = (text: string, width: number, extraTcPr = ''): string =>
    cell(`<w:tcW w:w="${width}" w:type="dxa"/>${extraTcPr}`, p(text));

/**
 * Таблица с `w:tblGrid` там, где ему место по схеме — следующим за `w:tblPr`
 * (kit кладёт grid внутрь tblPr, для ручных таблиц это не годится).
 */
const tbl = (tblPr: string, grid: number[], rows: string[]): string =>
    `<w:tbl><w:tblPr>${tblPr}</w:tblPr>` +
    `<w:tblGrid>${grid.map((w) => `<w:gridCol w:w="${w}"/>`).join('')}</w:tblGrid>` +
    rows.join('') +
    '</w:tbl>';

const BORDER_SIDES = ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'] as const;

const tblBorders = (sides: readonly string[], val: string, sz: number, color: string): string =>
    '<w:tblBorders>' +
    sides.map((side) => `<w:${side} w:val="${val}" w:sz="${sz}" w:space="0" w:color="${color}"/>`).join('') +
    '</w:tblBorders>';

/** `w:type="table"` стиль с рамками — ссылка на него проверяет `w:tblLook`-фикстура. */
const TABLE_GRID_STYLE =
    '<w:style w:type="table" w:customStyle="1" w:styleId="TableGrid"><w:name w:val="Table Grid"/>' +
    '<w:basedOn w:val="TableNormal"/><w:uiPriority w:val="39"/><w:qFormat/>' +
    '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>' +
    '<w:tblPr>' +
    tblBorders(BORDER_SIDES, 'single', 4, 'auto') +
    '</w:tblPr></w:style>';

const MINIMAL_TBL_W = '<w:tblW w:w="0" w:type="auto"/>';

const tableStyles = [
    { id: 'TableNormal', type: 'table', default: true },
    { id: 'TableGrid', type: 'table', basedOn: 'TableNormal', borders: { val: 'single', sz: 4, color: 'auto' } },
];

/** Вложенная таблица 2×2, которая лежит в ячейке внешней таблицы. */
const nestedCells = [
    ['Внутренняя A1', 'Внутренняя B1'],
    ['Внутренняя A2', 'Внутренняя B2'],
];

/** Внешняя таблица 2×2; во второй строке первой ячейки — вложенная таблица. */
const nestedOuter = tbl(
    MINIMAL_TBL_W + tblBorders(BORDER_SIDES, 'single', 6, '2F5496'),
    [4500, 4500],
    [
        tr(
            cellW('Внешняя A1', 4500, '<w:vAlign w:val="center"/>') +
                cellW('Внешняя B1', 4500, '<w:vAlign w:val="center"/>'),
        ),
        tr(
            cell(
                '<w:tcW w:w="4500" w:type="dxa"/>',
                p('Текст до вложенной') + table(nestedCells, { borders: true }) + p('Текст после вложенной'),
            ) + cellW('Внешняя B2', 4500, '<w:vAlign w:val="center"/>'),
        ),
    ],
);

/** Таблица 3×3: первая строка объединена целиком, во второй две ячейки слиты. */
const gridSpanTable = tbl(
    MINIMAL_TBL_W + tblBorders(BORDER_SIDES, 'single', 4, 'auto'),
    [3000, 3000, 3000],
    [
        tr(cellW('Строка во всю ширину', 9000, '<w:gridSpan w:val="3"/>')),
        tr(
            cellW('Две ячейки', 6000, '<w:gridSpan w:val="2"/>') + cellW('C2', 3000),
        ),
        tr(cellW('A3', 3000) + cellW('B3', 3000) + cellW('C3', 3000)),
    ],
);

/** Таблица 3×2: первый столбец объединён по вертикали. */
const vMergeTable = tbl(
    MINIMAL_TBL_W + tblBorders(BORDER_SIDES, 'single', 4, 'auto'),
    [4500, 4500],
    [
        tr(cellW('Объединено по вертикали', 4500, '<w:vMerge w:val="restart"/>') + cellW('Строка 1', 4500)),
        tr(cellW('', 4500, '<w:vMerge/>') + cellW('Строка 2', 4500)),
        tr(cellW('', 4500, '<w:vMerge/>') + cellW('Строка 3', 4500)),
    ],
);

/** Флаги `w:tblLook` в том виде, в каком их пишет Word. */
const TBL_LOOK =
    '<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" ' +
    'w:noHBand="0" w:noVBand="1"/>';

const tblLookTable = tbl(
    '<w:tblStyle w:val="TableGrid"/>' + MINIMAL_TBL_W + TBL_LOOK,
    [4680, 4680],
    [
        tr(cellW('Заголовок A', 4680) + cellW('Заголовок B', 4680), '<w:tblHeader/>'),
        tr(cellW('A2', 4680) + cellW('B2', 4680)),
    ],
);

/** Крупные рамки таблицы + рамки ячейки + заливка `w:shd`. */
const shadedTable = tbl(
    MINIMAL_TBL_W + tblBorders(BORDER_SIDES, 'single', 8, '2F5496'),
    [4000, 4000],
    [
        tr(
            cell(
                '<w:tcW w:w="4000" w:type="dxa"/>' + '<w:shd w:val="clear" w:color="auto" w:fill="2F5496"/>',
                p('Шапка A', { bold: true, color: 'FFFFFF' }),
            ) +
                cell(
                    '<w:tcW w:w="4000" w:type="dxa"/>' + '<w:shd w:val="clear" w:color="auto" w:fill="2F5496"/>',
                    p('Шапка B', { bold: true, color: 'FFFFFF' }),
                ),
            '<w:tblHeader/>',
        ),
        tr(
            cell(
                '<w:tcW w:w="4000" w:type="dxa"/>' +
                    '<w:tcBorders><w:bottom w:val="double" w:sz="6" w:space="0" w:color="C00000"/></w:tcBorders>' +
                    '<w:shd w:val="clear" w:color="auto" w:fill="DEEAF6"/>',
                p('Ячейка A2'),
            ) +
                cell(
                    '<w:tcW w:w="4000" w:type="dxa"/>' +
                        '<w:shd w:val="clear" w:color="auto" w:fill="FFFFFF"/>',
                    p('Ячейка B2'),
                ),
        ),
    ],
);

/** 30 строк — таблица заведомо не влезает на одну страницу, шапка повторяется. */
const headerRepeatCells: string[][] = [
    ['Заголовок A', 'Заголовок B'],
    ...Array.from({ length: 29 }, (_, i) => [`Строка ${i + 1}`, `Значение ${i + 1}`]),
];

const headerRepeatTable = tbl(
    MINIMAL_TBL_W + tblBorders(BORDER_SIDES, 'single', 4, 'auto'),
    [4500, 4500],
    headerRepeatCells.map((row, index) =>
        tr(row.map((text) => cellW(text, 4500)).join(''), index === 0 ? '<w:tblHeader/>' : ''),
    ),
);

/** Ширины столбцов в `w:tblGrid`, `w:tcW` ячеек, выравнивание и вертикаль. */
const alignmentTable = tbl(
    MINIMAL_TBL_W + '<w:jc w:val="center"/>' + tblBorders(BORDER_SIDES, 'single', 4, 'auto'),
    [2000, 5000, 2000],
    [
        tr(
            cellW('Сверху', 2000, '<w:vAlign w:val="top"/>') +
                cellW('По центру', 5000, '<w:vAlign w:val="center"/>') +
                cellW('Снизу', 2000, '<w:vAlign w:val="bottom"/>'),
        ),
        tr(
            cellW('Шире сетки', 3000, '<w:vAlign w:val="top"/>') +
                cellW('Уже сетки', 4000, '<w:vAlign w:val="top"/>') +
                cellW('Ровно', 2000, '<w:vAlign w:val="top"/>'),
        ),
        tr(
            cell(
                '<w:tcW w:w="2000" w:type="dxa"/><w:vAlign w:val="bottom"/>',
                p('Первая строка', { jc: 'left' }) + p('Вторая строка', { jc: 'right' }),
            ) +
                cellW('По правому краю', 5000, '<w:vAlign w:val="bottom"/>') +
                cellW('По центру', 2000, '<w:vAlign w:val="bottom"/>'),
        ),
    ],
);

export const tablesFixtures: FixtureSpec[] = [
    {
        name: 'tables/nested',
        description: 'Таблица внутри ячейки внешней таблицы',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(nestedOuter) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 2,
                    cells: [
                        ['Внешняя A1', 'Внешняя B1'],
                        ['Текст до вложенной Текст после вложенной', 'Внешняя B2'],
                    ],
                    nested: { rows: 2, cols: 2, at: { row: 1, col: 0 }, cells: nestedCells },
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/grid_span',
        description: 'Горизонтальное объединение ячеек через w:gridSpan',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(gridSpanTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 3,
                    cols: 3,
                    gridSpan: [
                        { row: 0, col: 0, span: 3 },
                        { row: 1, col: 0, span: 2 },
                    ],
                    cells: [
                        ['Строка во всю ширину'],
                        ['Две ячейки', 'C2'],
                        ['A3', 'B3', 'C3'],
                    ],
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/v_merge',
        description: 'Вертикальное объединение: w:vMerge restart и продолжения без w:val',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(vMergeTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 3,
                    cols: 2,
                    vMerge: [
                        { row: 0, col: 0, val: 'restart' },
                        { row: 1, col: 0, val: 'continue' },
                        { row: 2, col: 0, val: 'continue' },
                    ],
                    cells: [
                        ['Объединено по вертикали', 'Строка 1'],
                        ['', 'Строка 2'],
                        ['', 'Строка 3'],
                    ],
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/tbl_look',
        description: 'w:tblLook с флагами первой строки, последней строки и первого столбца',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({
            body: body(tblLookTable),
            stylesExtra: TABLE_GRID_STYLE,
        }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 2,
                    style: 'TableGrid',
                    headerRows: 1,
                    tblLook: {
                        val: '04A0',
                        firstRow: true,
                        lastRow: false,
                        firstColumn: true,
                        lastColumn: false,
                        noHBand: false,
                        noVBand: true,
                    },
                    cells: [
                        ['Заголовок A', 'Заголовок B'],
                        ['A2', 'B2'],
                    ],
                },
            ],
            images: [],
            styles: tableStyles,
        },
    },
    {
        name: 'tables/borders_shading',
        description: 'Рамки таблицы (w:tblBorders), рамки ячейки (w:tcBorders) и заливка (w:shd)',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(shadedTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 2,
                    cols: 2,
                    borders: { val: 'single', sz: 8, color: '2F5496', sides: 'all' },
                    cells: [
                        ['Шапка A', 'Шапка B'],
                        ['Ячейка A2', 'Ячейка B2'],
                    ],
                    shading: [
                        { row: 0, col: 0, fill: '2F5496' },
                        { row: 0, col: 1, fill: '2F5496' },
                        { row: 1, col: 0, fill: 'DEEAF6' },
                        { row: 1, col: 1, fill: 'FFFFFF' },
                    ],
                    cellBorders: [{ row: 1, col: 0, bottom: { val: 'double', sz: 6, color: 'C00000' } }],
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/header_repeat',
        description: 'Шапка таблицы на 30 строк помечена w:tblHeader и повторяется на каждой странице',
        expectedParagraphs: 4,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({
            body: body(
                p('Текст до таблицы.'),
                pageBreak(),
                headerRepeatTable,
                pageBreak(),
                p('Текст после таблицы.'),
            ),
        }),
        content: {
            paragraphs: [
                { text: 'Текст до таблицы.' },
                { text: '', pageBreak: true },
                { text: '', pageBreak: true },
                { text: 'Текст после таблицы.' },
            ],
            tables: [
                {
                    rows: headerRepeatCells.length,
                    cols: 2,
                    headerRows: 1,
                    repeatHeader: true,
                    cells: headerRepeatCells,
                },
            ],
            images: [],
        },
    },
    {
        name: 'tables/alignment_widths',
        description: 'Явные ширины w:tblGrid и w:tcW, выравнивание w:jc и w:vAlign',
        expectedParagraphs: 0,
        expectedTables: 1,
        expectedImages: 0,
        parts: packageParts({ body: body(alignmentTable) }),
        content: {
            paragraphs: [],
            tables: [
                {
                    rows: 3,
                    cols: 3,
                    jc: 'center',
                    grid: [2000, 5000, 2000],
                    widths: [
                        [2000, 5000, 2000],
                        [3000, 4000, 2000],
                        [2000, 5000, 2000],
                    ],
                    vAlign: [
                        ['top', 'center', 'bottom'],
                        ['top', 'top', 'top'],
                        ['bottom', 'bottom', 'bottom'],
                    ],
                    cells: [
                        ['Сверху', 'По центру', 'Снизу'],
                        ['Шире сетки', 'Уже сетки', 'Ровно'],
                        ['Первая строка Вторая строка', 'По правому краю', 'По центру'],
                    ],
                },
            ],
            images: [],
        },
    },
];
