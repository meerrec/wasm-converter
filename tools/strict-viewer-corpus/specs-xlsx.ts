// Записи реестра для XLSX-фикстур из `scripts/generate_strict_viewer_xlsx.ts`.
//
// Эти книги закрывают пробелы матрицы, помеченные `требуется новая` или
// `planned`: печатная область, fit-to-page, колонтитулы печати, 1904,
// shrink-to-fit, внешние ссылки, pivot и slicer.

import type { FixtureInput } from './add.js';

const fixtures: FixtureInput[] = [
  {
    path: 'xlsx/print-area-titles.xlsx',
    title: 'Печатная область и повторяющиеся заголовки',
    tags: ['print', 'geometry'],
    matrixRows: ['xlsx.print-areas-breaks', 'xlsx.print-titles-scale'],
    expected:
      'Печатается только A1:D40 (`_xlnm.Print_Area`), строки 1–2 повторяются на каждой странице (`_xlnm.Print_Titles`); строки вне области в печать не попадают.',
    modes: ['print-preview'],
    notes: 'Пункты 13–14 приоритетного состава. Закрывает gaps `print-area-manual-breaks` и `repeating-titles-fit`.',
  },
  {
    path: 'xlsx/print-fit-to-page.xlsx',
    title: 'Fit-to-page: одна страница по ширине',
    tags: ['print', 'geometry'],
    matrixRows: ['xlsx.print-titles-scale'],
    expected:
      'Таблица 30×12 ужимается в одну страницу по ширине (`fitToWidth=1`, `fitToHeight=0`), число страниц по высоте не ограничено.',
    modes: ['print-preview'],
    notes: 'Пункт 14 приоритетного состава (fit-to-page).',
  },
  {
    path: 'xlsx/print-headers-footers.xlsx',
    title: 'Печатные колонтитулы',
    tags: ['print'],
    matrixRows: ['xlsx.print-headers-footers'],
    expected:
      'Верхний колонтитул: слева «Отчёт», по центру «Стр. N из M», справа дата; нижний: слева «Конфиденциально», справа номер страницы.',
    modes: ['print-preview'],
    notes: 'Пункт 15 приоритетного состава. Закрывает gap `headers-footers`.',
  },
  {
    path: 'xlsx/formats-date-1904.xlsx',
    title: 'Даты в системе 1904',
    tags: ['formats', 'locale'],
    matrixRows: ['xlsx.dates-locales'],
    expected:
      '`workbookPr date1904="1"`: серийные числа считаются от 01.01.1904, поэтому 02.01.1904 — серийное 1, а не 1 + смещение 1462.',
    modes: ['normal'],
    notes: 'Пункт 7 приоритетного состава. Закрывает gap `date1904`.',
  },
  {
    path: 'xlsx/layout-shrink-to-fit.xlsx',
    title: 'Shrink-to-fit в узкой ячейке',
    tags: ['geometry'],
    matrixRows: ['xlsx.wrap-overflow'],
    expected:
      'Длинный текст в ячейке шириной 12 знаков ужимается по горизонтали вместо переноса и вместо выхода за границу.',
    modes: ['normal'],
    notes: 'Пункт 4 приоритетного состава. Часть gap `shrink-rotation`.',
  },
  {
    path: 'xlsx/external-workbook-link.xlsx',
    title: 'Внешняя ссылка на другую книгу',
    tags: ['links', 'formulas'],
    matrixRows: ['xlsx.external-links'],
    expected:
      'Значение 41 пришло из `source.xlsx` через `externalLink`: сеть не трогается, показывается кэш, устаревание видно в отчёте.',
    modes: ['normal'],
    notes: 'Пункт 17 приоритетного состава. Закрывает gap `external-workbook-link`.',
  },
  {
    path: 'xlsx/pivot-basic.xlsx',
    title: 'Сводная таблица',
    tags: ['pivot', 'unsupported'],
    matrixRows: ['xlsx.pivot-slicers-controls'],
    expected:
      'Лист «Данные» и лист «Сводная» с pivotTable размером A3:B6; движок её не рисует, но обязан выдать `xlsx-pivot-slicer-control-excluded`.',
    modes: ['normal'],
    notes: 'Пункт 18 приоритетного состава. Строка матрицы — `excluded`.',
  },
  {
    path: 'xlsx/slicer-basic.xlsx',
    title: 'Slicer поверх фильтра',
    tags: ['controls', 'unsupported'],
    matrixRows: ['xlsx.pivot-slicers-controls'],
    expected:
      'Часть `xl/slicers/slicer1.xml` с named-фильтром «Категория»; движок её не рисует и выдаёт `xlsx-pivot-slicer-control-excluded`.',
    modes: ['normal'],
    notes: 'Пункт 19 приоритетного состава. Строка матрицы — `excluded`.',
  },
];

export default fixtures;
