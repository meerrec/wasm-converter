// Генератор фикстур XLSX и эталона к ним.
//
// Фикстуры пишет exceljs — сторонняя реализация SpreadsheetML, а не наш
// парсер. Смысл именно в этом: собранный своими руками XML проверяет то, о чём
// мы подумали, а чужой писатель — то, о чём нет.
//
// Тот же exceljs читает файлы обратно и складывает значения ячеек в
// `oracle.json`. Это дифференциальный тест: два независимых разбора одного
// файла должны сойтись.
//
//     node scripts/gen-fixtures.ts
//
// Даты пишутся числами с форматом даты, а не объектами `Date`. Так в эталон
// попадает ровно то, что лежит в файле. exceljs отдаёт такие ячейки обратно
// объектами `Date`, поэтому генератор переводит их назад в серийный номер по
// той же формуле, что и Excel.

import ExcelJS from 'exceljs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { crc32, deflateRawSync, deflateSync, inflateRawSync } from 'node:zlib';

const ROOT = path.resolve(import.meta.dirname, '..');
const OUT_DIR = path.join(ROOT, 'test-fixtures', 'xlsx');
const ORACLE = path.join(OUT_DIR, 'oracle.json');

type Build = (wb: ExcelJS.Workbook) => void;

interface Fixture {
  name: string;
  build: Build;
  /**
   * Правка уже записанного пакета: то, что exceljs не умеет записать сам.
   * Сейчас это единственный случай — `stopIfTrue` у правил условного
   * форматирования (см. `addStopIfTrue`).
   */
  after?: (file: string) => Promise<void>;
}

const argb = (hex: string) => ({ argb: `FF${hex.toUpperCase()}` });

/**
 * Начало отсчёта serial-дат Excel (1899-12-30) в миллисекундах Unix-времени.
 *
 * Считается именно разностью двух моментов, а не сдвигом на 25 569 суток:
 * сдвиг туда и обратно округляется дважды, и в эталон попадало бы
 * `1234.5678000000007` вместо `1234.5678`.
 */
const EPOCH_MS = Date.UTC(1899, 11, 30);

// ── Значения ────────────────────────────────────────────────

const numbers: Build = (wb) => {
  const ws = wb.addWorksheet('Числа');
  [0, 1, -1, 0.5, -0.5, 1e10, 1e-10, 1234567.891, 1 / 3, 2 ** 31].forEach((v, i) => {
    ws.getCell(i + 1, 1).value = v;
  });
};

const integers: Build = (wb) => {
  const ws = wb.addWorksheet('Целые');
  ws.addRow([0, 1, 42, -7, 1000000]);
  ws.addRow([Number.MAX_SAFE_INTEGER, 12, 999999, -100000, 3]);
};

const fractions: Build = (wb) => {
  const ws = wb.addWorksheet('Дроби');
  for (let i = 0; i < 20; i += 1) {
    ws.getCell(i + 1, 1).value = i / 7;
    ws.getCell(i + 1, 2).value = -(i / 3);
  }
};

const strings: Build = (wb) => {
  const ws = wb.addWorksheet('Строки');
  ['', 'a', 'Привет', 'Hello, world', 'смесь Latin и Кириллицы', '  leading', 'trailing  '].forEach(
    (v, i) => {
      ws.getCell(i + 1, 1).value = v;
    },
  );
};

const booleans: Build = (wb) => {
  const ws = wb.addWorksheet('Логические');
  ws.addRow([true, false]);
  ws.addRow([false, true]);
};

const errorValues = ['#NULL!', '#DIV/0!', '#VALUE!', '#REF!', '#NAME?', '#NUM!', '#N/A'];

const errors: Build = (wb) => {
  const ws = wb.addWorksheet('Ошибки');
  errorValues.forEach((error, i) => {
    ws.getCell(i + 1, 1).value = { error };
  });
};

const formulas: Build = (wb) => {
  const ws = wb.addWorksheet('Формулы');
  ws.getCell('A1').value = 1;
  ws.getCell('A2').value = 2;
  ws.getCell('B1').value = { formula: 'SUM(A1:A2)', result: 3 };
  ws.getCell('B2').value = { formula: 'A1+A2*2', result: 5 };
  ws.getCell('B3').value = { formula: 'CONCATENATE("a","b")', result: 'ab' };
};

const whitespace: Build = (wb) => {
  const ws = wb.addWorksheet('Пробелы');
  ws.getCell('A1').value = '  два пробела по краям  ';
  ws.getCell('A2').value = 'внутренние   пробелы';
  ws.getCell('A3').value = 'таб\tвнутри';
  ws.getCell('A4').value = 'перевод\nстроки';
};

const unicode: Build = (wb) => {
  const ws = wb.addWorksheet('Юникод');
  [
    '日本語のテキスト',
    '한국어 텍스트',
    'العربية',
    'עברית',
    '😀 эмодзи 🎉',
    'Ĉiuj homoj estas denaske liberaj',
    'Тире — и дефис - разные',
  ].forEach((v, i) => {
    ws.getCell(i + 1, 1).value = v;
  });
};

const longText: Build = (wb) => {
  const ws = wb.addWorksheet('Длинный текст');
  ws.getCell('A1').value = 'очень длинная строка '.repeat(200);
  ws.getCell('A2').value = 'я'.repeat(5000);
};

const markup: Build = (wb) => {
  const ws = wb.addWorksheet('Разметка');
  ['<tag>', 'a & b', '"quotes"', "it's", 'a < b && c > d', '</closing>'].forEach((v, i) => {
    ws.getCell(i + 1, 1).value = v;
  });
};

const richText: Build = (wb) => {
  const ws = wb.addWorksheet('Прогоны');
  ws.getCell('A1').value = {
    richText: [
      { text: 'жирный', font: { bold: true } },
      { text: ' обычный ' },
      { text: 'курсив', font: { italic: true } },
    ],
  };
  ws.getCell('A2').value = { richText: [{ text: 'только один прогон' }] };
};

const sparse: Build = (wb) => {
  const ws = wb.addWorksheet('Разреженный');
  ws.getCell('A1').value = 1;
  ws.getCell('A100').value = 2;
  ws.getCell('CV500').value = 3;
};

const dense: Build = (wb) => {
  const ws = wb.addWorksheet('Плотный');
  for (let r = 1; r <= 100; r += 1) {
    for (let c = 1; c <= 20; c += 1) {
      ws.getCell(r, c).value = r * 1000 + c;
    }
  }
};

/** Таблица 20×5: типичный отчёт. */
const table: Build = (wb) => {
  const ws = wb.addWorksheet('Таблица');
  ws.addRow(['Наименование', 'Количество', 'Цена', 'Сумма', 'Дата']);
  for (let r = 2; r <= 20; r += 1) {
    const price = 100 + r * 7.5;
    const cell = ws.getCell(r, 5);
    cell.value = 45000 + r;
    cell.numFmt = 'dd.mm.yyyy';
    ws.addRow([`Позиция ${r}`, r, price, { formula: `B${r}*C${r}`, result: r * price }, undefined]);
    ws.getCell(r, 5).value = 45000 + r;
    ws.getCell(r, 5).numFmt = 'dd.mm.yyyy';
  }
};

// ── Форматы чисел ───────────────────────────────────────────

const FORMATS: Array<[string, string]> = [
  ['general', 'General'],
  ['integer', '0'],
  ['two-decimals', '0.00'],
  ['thousands', '#,##0'],
  ['thousands-decimals', '#,##0.00'],
  ['percent', '0%'],
  ['percent-decimals', '0.00%'],
  ['scientific', '0.00E+00'],
  ['fraction', '# ?/?'],
  ['accounting', '#,##0 ;(#,##0)'],
  ['currency-ruble', '#,##0.00" ₽"'],
  ['currency-dollar', '"$"#,##0.00'],
  ['date-iso', 'yyyy-mm-dd'],
  ['date-ru', 'dd.mm.yyyy'],
  ['date-long', 'd mmmm yyyy'],
  ['time', 'hh:mm:ss'],
  ['time-elapsed', '[h]:mm:ss'],
  ['datetime', 'dd.mm.yyyy hh:mm'],
  ['text', '@'],
  ['phone', '000-00-00'],
];

/**
 * Значения для фикстуры с одним форматом.
 *
 * Форматы даты и времени получают серийные номера, а не произвольные дроби:
 * exceljs отдаёт такие ячейки объектами `Date` с миллисекундной точностью, и
 * serial вида `0.000123` (десять секунд) через них уже не проходит обратно.
 * Пары «дробное число + формат даты» разбираются в модульных тестах `numfmt`.
 */
const NUMBER_VALUES = [0, 1, -1, 0.5, -0.5, 1234.5678, -1234.5678, 0.000123, 1e15, 45000.5];

/** Целые и «человеческие» доли суток: ровно полночь, полдень, шесть утра. */
const DATE_VALUES = [0, 1, 59, 60, 61, 45000, 45000.5, 45001.25, 36526, 2.75];

const DATE_FORMATS = new Set([
  'date-iso',
  'date-ru',
  'date-long',
  'time',
  'time-elapsed',
  'datetime',
]);


// ── Стили ───────────────────────────────────────────────────

const fontWeight: Build = (wb) => {
  const ws = wb.addWorksheet('Жирный');
  for (let i = 0; i < 5; i += 1) {
    const cell = ws.getCell(i + 1, 1);
    cell.value = `строка ${i}`;
    cell.font = { bold: i % 2 === 0, italic: i % 3 === 0 };
  }
};

const fontSizes: Build = (wb) => {
  const ws = wb.addWorksheet('Кегли');
  [8, 10, 11, 14, 18, 24, 36].forEach((size, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = `${size} pt`;
    cell.font = { size };
  });
};

const fontNames: Build = (wb) => {
  const ws = wb.addWorksheet('Гарнитуры');
  ['Calibri', 'Arial', 'Times New Roman', 'Courier New', 'Georgia'].forEach((name, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = name;
    cell.font = { name };
  });
};

const fontDecorations: Build = (wb) => {
  const ws = wb.addWorksheet('Начертания');
  const decorations = [
    { underline: true },
    { strike: true },
    { underline: true, strike: true, bold: true },
  ] as const;
  decorations.forEach((font, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = 'текст';
    cell.font = { ...font };
  });
};

const fontColors: Build = (wb) => {
  const ws = wb.addWorksheet('Цвета текста');
  ['FF0000', '00AA00', '0000FF', '808080', 'FF00FF'].forEach((hex, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = `#${hex}`;
    cell.font = { color: argb(hex) };
  });
};

const solidFills: Build = (wb) => {
  const ws = wb.addWorksheet('Заливки');
  ['FFFF00', '00FFFF', 'FFCCCC', 'CCFFCC', 'EEEEEE'].forEach((hex, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = `#${hex}`;
    cell.fill = { type: 'pattern', pattern: 'solid', fgColor: argb(hex) };
  });
};

const patternFills: Build = (wb) => {
  const ws = wb.addWorksheet('Узоры');
  const patterns = ['darkGray', 'mediumGray', 'lightGray', 'gray125', 'lightUp'] as const;
  patterns.forEach((pattern, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = pattern;
    cell.fill = {
      type: 'pattern',
      pattern,
      fgColor: argb('000000'),
      bgColor: argb('FFFFFF'),
    };
  });
};

const borderStyles: Build = (wb) => {
  const ws = wb.addWorksheet('Рамки');
  const styles = ['thin', 'medium', 'thick', 'dashed', 'dotted', 'double', 'hair'] as const;
  styles.forEach((style, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = style;
    cell.border = {
      top: { style, color: argb('000000') },
      bottom: { style, color: argb('0000FF') },
    };
  });
};

const borderAll: Build = (wb) => {
  const ws = wb.addWorksheet('Рамка вокруг');
  for (let r = 1; r <= 5; r += 1) {
    for (let c = 1; c <= 5; c += 1) {
      const cell = ws.getCell(r, c);
      cell.value = r * 10 + c;
      cell.border = {
        top: { style: 'thin', color: argb('000000') },
        left: { style: 'thin', color: argb('000000') },
        bottom: { style: 'thin', color: argb('000000') },
        right: { style: 'thin', color: argb('000000') },
      };
    }
  }
};

const alignment: Build = (wb) => {
  const ws = wb.addWorksheet('Выравнивание');
  const modes = ['left', 'center', 'right', 'justify'] as const;
  modes.forEach((horizontal, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = `по ${horizontal}`;
    cell.alignment = { horizontal, vertical: 'middle', wrapText: true };
  });
};

// ── Раскладка ───────────────────────────────────────────────

const columnWidths: Build = (wb) => {
  const ws = wb.addWorksheet('Ширины');
  [4, 8.43, 12.5, 30, 60].forEach((width, i) => {
    ws.getColumn(i + 1).width = width;
    ws.getCell(1, i + 1).value = `ширина ${width}`;
  });
};

const hiddenColumns: Build = (wb) => {
  const ws = wb.addWorksheet('Скрытые столбцы');
  for (let c = 1; c <= 5; c += 1) {
    ws.getCell(1, c).value = `столбец ${c}`;
  }
  ws.getColumn(2).hidden = true;
  ws.getColumn(4).hidden = true;
};

const rowHeights: Build = (wb) => {
  const ws = wb.addWorksheet('Высоты');
  [10, 15, 20, 40, 80].forEach((height, i) => {
    ws.getRow(i + 1).height = height;
    ws.getCell(i + 1, 1).value = `высота ${height}`;
  });
};

const hiddenRows: Build = (wb) => {
  const ws = wb.addWorksheet('Скрытые строки');
  for (let r = 1; r <= 5; r += 1) {
    ws.getCell(r, 1).value = `строка ${r}`;
  }
  ws.getRow(2).hidden = true;
  ws.getRow(5).hidden = true;
};

const merged: Build = (wb) => {
  const ws = wb.addWorksheet('Объединения');
  ws.getCell('A1').value = 'заголовок на два столбца';
  ws.mergeCells('A1:B1');
  ws.getCell('A2').value = 'на два ряда';
  ws.mergeCells('A2:A3');
  ws.getCell('C1').value = 'блок 2×2';
  ws.mergeCells('C1:D2');
  ws.getCell('E1').value = 'обычная';
};

const frozenRows: Build = (wb) => {
  const ws = wb.addWorksheet('Закреплены строки');
  ws.views = [{ state: 'frozen', ySplit: 1, topLeftCell: 'A2', activeCell: 'A2' }];
  for (let r = 1; r <= 10; r += 1) {
    ws.getCell(r, 1).value = `строка ${r}`;
  }
};

const frozenColumns: Build = (wb) => {
  const ws = wb.addWorksheet('Закреплены столбцы');
  ws.views = [{ state: 'frozen', xSplit: 1, topLeftCell: 'B1', activeCell: 'B1' }];
  for (let c = 1; c <= 10; c += 1) {
    ws.getCell(1, c).value = `столбец ${c}`;
  }
};

const frozenBoth: Build = (wb) => {
  const ws = wb.addWorksheet('Закреплено всё');
  ws.views = [{ state: 'frozen', xSplit: 2, ySplit: 2, topLeftCell: 'C3', activeCell: 'C3' }];
  for (let r = 1; r <= 8; r += 1) {
    for (let c = 1; c <= 8; c += 1) {
      ws.getCell(r, c).value = `${String.fromCharCode(64 + c)}${r}`;
    }
  }
};

const noGrid: Build = (wb) => {
  const ws = wb.addWorksheet('Без сетки');
  ws.views = [{ showGridLines: false, zoomScale: 85 }];
  for (let r = 1; r <= 5; r += 1) {
    ws.getCell(r, 1).value = r;
  }
};

// ── Гиперссылки ─────────────────────────────────────────────

const hyperlinks: Build = (wb) => {
  const ws = wb.addWorksheet('Ссылки');
  ws.getCell('A1').value = { text: 'Example', hyperlink: 'https://example.com/' };
  ws.getCell('A2').value = {
    text: 'С параметрами',
    hyperlink: 'https://example.com/?a=1&b=2',
  };
  ws.getCell('A3').value = { text: 'Почта', hyperlink: 'mailto:test@example.com' };
  ws.getCell('A4').value = { text: 'На второй лист', hyperlink: '#Лист2!A1' };
  ws.getCell('B1').value = 'рядом';
};

// ── Условное форматирование ─────────────────────────────────

/**
 * Дифференциальный стиль правила (`dxf`): exceljs складывает его в отдельную
 * таблицу `styles.xml`, а в правиле остаётся ссылка `dxfId`. Набор свойств —
 * как у стиля ячейки, но применяется к ней фрагментарно.
 */
type Dxf = Partial<ExcelJS.Style>;

const dxfFill = (hex: string): Dxf => ({
  fill: { type: 'pattern', pattern: 'solid', fgColor: argb(hex) },
});

/** Столбец 1..12 значений: видно, на каком значении сработал порог. */
function cfNumbers(ws: ExcelJS.Worksheet, col: number, start: number, step: number): void {
  for (let r = 1; r <= 12; r += 1) {
    ws.getCell(r, col).value = start + r * step;
  }
}

/**
 * `cellIs`: операторы и пороги.
 *
 * В типах exceljs у `cellIs` объявлены четыре оператора, но писатель
 * подставляет в XML любой: ECMA-376 §18.18.15 разрешает ещё `notEqual`,
 * `lessThanOrEqual`, `greaterThanOrEqual` и `notBetween`.
 */
const cfCellIs: Build = (wb) => {
  const ws = wb.addWorksheet('Операторы');
  cfNumbers(ws, 1, 5, 5);
  cfNumbers(ws, 2, 100, -5);
  for (let r = 1; r <= 12; r += 1) {
    const cell = ws.getCell(r, 3);
    cell.value = r / 12;
    cell.numFmt = '0%';
  }

  ws.addConditionalFormatting({
    ref: 'A1:A12',
    rules: [
      {
        type: 'cellIs',
        operator: 'greaterThan',
        formulae: ['30'],
        priority: 1,
        style: dxfFill('FFC7CE'),
      },
      {
        type: 'cellIs',
        operator: 'lessThanOrEqual',
        formulae: ['20'],
        priority: 2,
        style: dxfFill('C6EFCE'),
      },
      {
        type: 'cellIs',
        operator: 'between',
        formulae: ['25', '45'],
        priority: 3,
        style: dxfFill('FFEB9C'),
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'C1:C12',
    rules: [
      {
        type: 'cellIs',
        operator: 'equal',
        formulae: ['0.5'],
        priority: 4,
        style: { font: { bold: true, color: { argb: 'FF9C0006' } } },
      },
      {
        type: 'cellIs',
        operator: 'notEqual',
        formulae: ['0.25'],
        priority: 5,
        style: { font: { italic: true } },
      },
      {
        type: 'cellIs',
        operator: 'greaterThanOrEqual',
        formulae: ['0.75'],
        priority: 6,
        style: { border: { top: { style: 'thin', color: argb('000000') } } },
      },
      {
        type: 'cellIs',
        operator: 'notBetween',
        formulae: ['0.3', '0.7'],
        priority: 7,
        style: dxfFill('DDDDDD'),
      },
    ],
  });
};

/** `colorScale`: две и три цвета, пороги `min`/`max`, проценты и процентили. */
const cfColorScale: Build = (wb) => {
  const ws = wb.addWorksheet('Цветовые шкалы');
  cfNumbers(ws, 1, 5, 5);
  cfNumbers(ws, 2, 100, -5);
  for (let r = 1; r <= 12; r += 1) {
    const cell = ws.getCell(r, 3);
    cell.value = r / 12;
    cell.numFmt = '0%';
  }

  ws.addConditionalFormatting({
    ref: 'A1:A12',
    rules: [
      {
        type: 'colorScale',
        priority: 1,
        cfvo: [{ type: 'min' }, { type: 'max' }],
        color: [{ argb: 'FFF8696B' }, { argb: 'FF63BE7B' }],
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'B1:B12',
    rules: [
      {
        type: 'colorScale',
        priority: 2,
        cfvo: [{ type: 'min' }, { type: 'percentile', value: 50 }, { type: 'max' }],
        color: [{ argb: 'FFF8696B' }, { argb: 'FFFFEB84' }, { argb: 'FF63BE7B' }],
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'C1:C12',
    rules: [
      {
        type: 'colorScale',
        priority: 3,
        cfvo: [
          { type: 'num', value: 0.1 },
          { type: 'percent', value: 50 },
          { type: 'num', value: 1 },
        ],
        color: [{ argb: 'FFFF0000' }, { argb: 'FFFFFFFF' }, { argb: 'FF0000FF' }],
      },
    ],
  });
};

/**
 * `dataBar`: гистограмма в ячейке.
 *
 * `gradient: true` оставляет правило в базовой схеме: без него exceljs уходит
 * в расширение `x14`, а туда он пишет случайный guid — эталон перестал бы
 * воспроизводиться. По той же причине `x14Id` задан явно: exceljs оставляет
 * в `extLst` пустой `<x14:id/>`, а так ссылка получается целой.
 */
const cfDataBar: Build = (wb) => {
  const ws = wb.addWorksheet('Гистограммы');
  cfNumbers(ws, 1, 5, 5);
  cfNumbers(ws, 2, -50, 10);

  ws.addConditionalFormatting({
    ref: 'A1:A12',
    rules: [
      {
        type: 'dataBar',
        priority: 1,
        gradient: true,
        cfvo: [{ type: 'min' }, { type: 'max' }],
        color: { argb: 'FF638EC6' },
        x14Id: '{00000000-0000-4000-8000-000000000001}',
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'B1:B12',
    rules: [
      {
        type: 'dataBar',
        priority: 2,
        gradient: true,
        cfvo: [
          { type: 'num', value: -50 },
          { type: 'num', value: 70 },
        ],
        color: { argb: 'FF63BE7B' },
        x14Id: '{00000000-0000-4000-8000-000000000002}',
      },
    ],
  });
};

/** `iconSet`: наборы на 3, 4 и 5 значков, с `reverse` и без значений. */
const cfIconSet: Build = (wb) => {
  const ws = wb.addWorksheet('Значки');
  cfNumbers(ws, 1, 5, 5);
  cfNumbers(ws, 2, 100, -5);
  cfNumbers(ws, 3, 0, 1);

  ws.addConditionalFormatting({
    ref: 'A1:A12',
    rules: [
      {
        type: 'iconSet',
        iconSet: '3TrafficLights1',
        priority: 1,
        cfvo: [
          { type: 'percent', value: 0 },
          { type: 'percent', value: 33 },
          { type: 'percent', value: 67 },
        ],
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'B1:B12',
    rules: [
      {
        type: 'iconSet',
        iconSet: '4Arrows',
        priority: 2,
        reverse: true,
        cfvo: [
          { type: 'percent', value: 0 },
          { type: 'percent', value: 25 },
          { type: 'percent', value: 50 },
          { type: 'percent', value: 75 },
        ],
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'C1:C12',
    rules: [
      {
        type: 'iconSet',
        iconSet: '5Quarters',
        priority: 3,
        showValue: false,
        cfvo: [
          { type: 'percent', value: 0 },
          { type: 'percent', value: 20 },
          { type: 'percent', value: 40 },
          { type: 'percent', value: 60 },
          { type: 'percent', value: 80 },
        ],
      },
    ],
  });
};

/** `expression`: правило-формула, в том числе поверх нескольких столбцов. */
const cfExpression: Build = (wb) => {
  const ws = wb.addWorksheet('Формулы');
  ws.getCell('A1').value = 'Позиция';
  ws.getCell('B1').value = 'Число';
  for (let r = 2; r <= 13; r += 1) {
    ws.getCell(r, 1).value = `Позиция ${r - 1}`;
    ws.getCell(r, 2).value = (r * 7) % 23;
  }

  ws.addConditionalFormatting({
    ref: 'B2:B13',
    rules: [
      {
        type: 'expression',
        formulae: ['MOD($B2,2)=0'],
        priority: 1,
        style: dxfFill('DDEBF7'),
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'A2:B13',
    rules: [
      {
        type: 'expression',
        formulae: ['$B2=0'],
        priority: 2,
        style: dxfFill('FFC7CE'),
      },
      {
        type: 'expression',
        formulae: ['LEN($A2)>10'],
        priority: 3,
        style: { font: { italic: true } },
      },
    ],
  });
};

/**
 * Приоритеты: несколько правил на одном диапазоне, порядок в файле не
 * совпадает с номерами приоритетов, у самого приоритетного — `stopIfTrue`
 * (дописывается `addStopIfTrue`: exceljs этот атрибут не пишет).
 */
const cfPriorities: Build = (wb) => {
  const ws = wb.addWorksheet('Приоритеты');
  cfNumbers(ws, 1, 5, 5);
  cfNumbers(ws, 3, 5, 5);

  ws.addConditionalFormatting({
    ref: 'A1:A12',
    rules: [
      {
        type: 'cellIs',
        operator: 'greaterThan',
        formulae: ['30'],
        priority: 9,
        style: dxfFill('C6EFCE'),
      },
      {
        type: 'cellIs',
        operator: 'greaterThan',
        formulae: ['45'],
        priority: 5,
        style: dxfFill('FFC7CE'),
      },
      {
        type: 'cellIs',
        operator: 'greaterThan',
        formulae: ['55'],
        priority: 7,
        style: { font: { bold: true } },
      },
    ],
  });
  // Второй блок на том же диапазоне: правила из разных блоков не сливаются.
  ws.addConditionalFormatting({
    ref: 'A1:A12',
    rules: [
      {
        type: 'expression',
        formulae: ['MOD($A1,2)=0'],
        priority: 12,
        style: dxfFill('FFEB9C'),
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'C1:C12',
    rules: [
      {
        type: 'cellIs',
        operator: 'lessThan',
        formulae: ['15'],
        priority: 3,
        style: dxfFill('D9D9D9'),
      },
    ],
  });
};

/** Одно правило на несколько несмежных диапазонов: `sqref` через пробел. */
const cfMultiRange: Build = (wb) => {
  const ws = wb.addWorksheet('Много диапазонов');
  for (let r = 1; r <= 10; r += 1) {
    for (let c = 1; c <= 6; c += 1) {
      ws.getCell(r, c).value = r * 10 + c;
    }
  }

  ws.addConditionalFormatting({
    ref: 'A1:A10 C1:C10 E1:E10',
    rules: [
      {
        type: 'cellIs',
        operator: 'greaterThan',
        formulae: ['55'],
        priority: 1,
        style: dxfFill('FFC7CE'),
      },
    ],
  });
  ws.addConditionalFormatting({
    ref: 'B1:B10 D1:D10',
    rules: [
      {
        type: 'expression',
        formulae: ['MOD(ROW(),2)=0'],
        priority: 2,
        style: dxfFill('DDEBF7'),
      },
      {
        type: 'colorScale',
        priority: 3,
        cfvo: [{ type: 'min' }, { type: 'max' }],
        color: [{ argb: 'FFFFFFFF' }, { argb: 'FF4472C4' }],
      },
    ],
  });
};

// ── Изображения ─────────────────────────────────────────────
//
// Байты картинок собираются здесь же: фикстура не должна тянуть бинарь из
// репозитория (и уж тем более из внешнего файла).

/** Цвет пикселя PNG — три канала по 8 бит. */
type Rgb = readonly [number, number, number];

/** Чанк PNG: длина, тип, данные и CRC от типа с данными. */
function pngChunk(type: string, data: Buffer): Buffer {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, 'latin1');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])));
  return Buffer.concat([head, data, crc]);
}

/** PNG с шахматкой из двух цветов: маленький, но настоящий файл. */
function pngBytes(size: number, first: Rgb, second: Rgb): Buffer {
  const stride = 1 + size * 3;
  const raw = Buffer.alloc(size * stride);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const color = (Math.floor(x / 4) + Math.floor(y / 4)) % 2 === 0 ? first : second;
      const at = y * stride + 1 + x * 3;
      raw[at] = color[0];
      raw[at + 1] = color[1];
      raw[at + 2] = color[2];
    }
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // бит на канал
  ihdr[9] = 2; // truecolor, без альфы
  // 10..12 остаются нулями: deflate, адаптивные фильтры, без интерлейса.

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', deflateSync(raw)),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

/**
 * Коды Хаффмана по длинам из `bits` и символам из `values`
 * (ITU T.81, Annex C): коды выдаются подряд, после каждой длины — сдвиг.
 */
function huffmanCodes(bits: number[], values: number[]): Map<number, [number, number]> {
  const codes = new Map<number, [number, number]>();
  let code = 0;
  let i = 0;
  for (let length = 1; length <= 16; length += 1) {
    for (let n = 0; n < bits[length - 1]; n += 1) {
      codes.set(values[i], [code, length]);
      code += 1;
      i += 1;
    }
    code <<= 1;
  }
  return codes;
}

/** Стандартная таблица Хаффмана для DC-коэффициентов яркости (T.81, K.3.1). */
const DC_BITS = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_VALUES = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/**
 * Минимальный baseline JPEG в оттенках серого: блоки 8×8 залиты ровным
 * цветом, поэтому кроме DC-коэффициента в блоке нет ничего и AC-таблица
 * сводится к одному символу EOB. Полноценный DCT-кодировщик для фикстуры —
 * лишний код.
 *
 * `brightness(x, y)` задаёт яркость 0..255; блок кодируется средним по своим
 * пикселям. Размеры кратны восьми.
 */
function jpegBytes(width: number, height: number, brightness: (x: number, y: number) => number): Buffer {
  const dc = huffmanCodes(DC_BITS, DC_VALUES);
  const eob: [number, number] = [0, 1];

  const out: number[] = [];
  let accumulator = 0;
  let length = 0;
  const writeBits = (value: number, count: number): void => {
    for (let i = count - 1; i >= 0; i -= 1) {
      accumulator = (accumulator << 1) | ((value >>> i) & 1);
      length += 1;
      if (length === 8) {
        out.push(accumulator);
        // Байт 0xFF в потоке данных экранируется нулём (T.81, B.1.1.5).
        if (accumulator === 0xff) out.push(0x00);
        accumulator = 0;
        length = 0;
      }
    }
  };

  let previous = 0;
  for (let by = 0; by < height; by += 8) {
    for (let bx = 0; bx < width; bx += 8) {
      let sum = 0;
      for (let y = by; y < by + 8; y += 1) {
        for (let x = bx; x < bx + 8; x += 1) {
          sum += brightness(x, y);
        }
      }
      // Квантователь DC равен 16, поэтому F(0,0)=8·(v−128) превращается в (v−128)/2.
      const level = Math.round((sum / 64 - 128) / 2);
      const diff = level - previous;
      previous = level;
      if (diff === 0) {
        writeBits(dc.get(0)![0], dc.get(0)![1]);
      } else {
        const size = Math.floor(Math.log2(Math.abs(diff))) + 1;
        if (size > 11) throw new Error(`яркость ${sum / 64} вне диапазона baseline JPEG`);
        writeBits(dc.get(size)![0], dc.get(size)![1]);
        // Отрицательные значения кодируются в дополнительном коде размерности size.
        writeBits(diff > 0 ? diff : diff + (1 << size) - 1, size);
      }
      writeBits(eob[0], eob[1]);
    }
  }
  if (length > 0) {
    writeBits((1 << (8 - length)) - 1, 8 - length);
  }

  const segment = (marker: number, payload: Buffer): Buffer => {
    const head = Buffer.alloc(4);
    head[0] = 0xff;
    head[1] = marker;
    head.writeUInt16BE(payload.length + 2, 2);
    return Buffer.concat([head, payload]);
  };

  const quant = Buffer.concat([Buffer.from([0x00]), Buffer.alloc(64, 16)]);

  const frame = Buffer.alloc(6);
  frame[0] = 8; // точность
  frame.writeUInt16BE(height, 1);
  frame.writeUInt16BE(width, 3);
  frame[5] = 1; // одна компонента
  const component = Buffer.from([0x01, 0x11, 0x00]);
  const scan = Buffer.from([0x01, 0x01, 0x00, 0x00, 0x3f, 0x00]);

  const tables = Buffer.concat([
    Buffer.from([0x00, ...DC_BITS, ...DC_VALUES]),
    Buffer.from([0x10, 1, ...Array(15).fill(0), 0x00]),
  ]);

  return Buffer.concat([
    Buffer.from([0xff, 0xd8]), // SOI
    segment(0xe0, Buffer.concat([Buffer.from('JFIF\0', 'latin1'), Buffer.from([1, 1, 0, 0, 1, 0, 1, 0, 0])])),
    segment(0xdb, quant),
    segment(0xc0, Buffer.concat([frame, component])),
    segment(0xc4, tables),
    segment(0xda, scan),
    Buffer.from(out),
    Buffer.from([0xff, 0xd9]), // EOI
  ]);
}

/** Шахматка 16×16 и полосатая яркость 16×16 — обе картинки крошечные. */
const IMAGE_PNG = pngBytes(16, [0x2f, 0x6b, 0x9a], [0xd9, 0xe2, 0xf3]);
const IMAGE_JPEG = jpegBytes(16, 16, (x) => (x < 8 ? 0xd0 : 0x40));

/** PNG: якорь на одну ячейку и якорь на диапазон. */
const imagesPng: Build = (wb) => {
  const ws = wb.addWorksheet('PNG');
  for (let r = 1; r <= 10; r += 1) {
    for (let c = 1; c <= 8; c += 1) {
      ws.getCell(r, c).value = r * 10 + c;
    }
  }
  const id = wb.addImage({ buffer: IMAGE_PNG, extension: 'png' });
  // One-cell: размер задан явно и больше ячейки-якоря — картинка выходит за её границы.
  ws.addImage(id, { tl: { col: 1, row: 1 }, ext: { width: 96, height: 72 } });
  // Two-cell: прямоугольник задан диапазоном E3:H8 и тянется по его границам.
  ws.addImage(id, 'E3:H8');
};

/** JPEG: те же два якоря, но их границы попадают в середину ячеек. */
const imagesJpeg: Build = (wb) => {
  const ws = wb.addWorksheet('JPEG');
  for (let r = 1; r <= 12; r += 1) {
    ws.getCell(r, 1).value = r;
  }
  const id = wb.addImage({ buffer: IMAGE_JPEG, extension: 'jpeg' });
  // Начало в середине ячейки D5.
  ws.addImage(id, { tl: { col: 2.5, row: 3.5 }, ext: { width: 120, height: 80 } });
  // Конец в середине ячейки, а не по её границе.
  ws.addImage(id, { tl: { col: 0, row: 0 }, br: { col: 3.5, row: 4 } });
};

/** Картинки поверх данных и у края листа — там, где нужно обрезать. */
const imagesOverData: Build = (wb) => {
  const ws = wb.addWorksheet('Поверх данных');
  ws.getRow(1).values = ['Товар', 'Цена', 'Количество', 'Сумма'];
  for (let r = 2; r <= 9; r += 1) {
    ws.getCell(r, 1).value = `Товар ${r - 1}`;
    ws.getCell(r, 2).value = r * 100;
    ws.getCell(r, 3).value = r;
    ws.getCell(r, 4).value = { formula: `B${r}*C${r}`, result: r * 100 * r };
  }

  const png = wb.addImage({ buffer: IMAGE_PNG, extension: 'png' });
  const jpeg = wb.addImage({ buffer: IMAGE_JPEG, extension: 'jpeg' });
  // Ровно по диапазону таблицы: под картинкой остаются ячейки с данными.
  ws.addImage(png, 'B2:D5');
  // Картинка шире своей ячейки-якоря: прямоугольник пересекает границы соседей.
  ws.addImage(jpeg, { tl: { col: 5, row: 1 }, ext: { width: 140, height: 100 } });
  // Прижата к правому краю листа: видимая часть обрезается границей листа.
  ws.addImage(png, { tl: { col: 16381, row: 4 }, ext: { width: 80, height: 60 } });
};

// ── PDF: текст и масштаб ────────────────────────────────────

/**
 * Кириллица и переносы: общий для канвы и PDF разбор строк (F1) обязан
 * ломать одни и те же слова в одних и тех же местах.
 */
const textCyrillicWrap: Build = (wb) => {
  const ws = wb.addWorksheet('Кириллица');
  ws.getColumn(1).width = 40;
  ws.getColumn(2).width = 4;
  // 9 — «ширина по умолчанию» самого exceljs: такую колонку он не пишет.
  ws.getColumn(3).width = 8;

  ws.getCell('A1').value = 'Привет, мир';
  ws.getCell('A2').value = 'Документ report final версия v2: смешанный текст';
  ws.getCell('A3').value = 'ё Ё Ђ ћ №5 — тире, дефис - и «ёлочки»';
  // Длинный текст: несколько строк переноса по словам.
  const long =
    'Перенос строки в ячейке проверяется по точкам разрыва: одинаковые слова должны ломаться одинаково и на канве, и в PDF. ';
  const wrapped = ws.getCell('A4');
  wrapped.value = long.repeat(2).trim();
  wrapped.alignment = { wrapText: true, vertical: 'top' };
  // Высота под четыре строки: иначе перенос обрезался бы уже на первой.
  ws.getRow(4).height = 60;
  // Явный перевод строки: жёсткий разрыв не зависит от ширины колонки.
  const manual = ws.getCell('A5');
  manual.value = 'первая строка\nвторая строка переноса';
  manual.alignment = { wrapText: true, vertical: 'top' };
  ws.getRow(5).height = 30;
  // Слово не влезает в колонку целиком — перенос обязан разорвать его.
  const word = ws.getCell('C1');
  word.value = 'гидроэлектростанция';
  word.alignment = { wrapText: true, vertical: 'top' };
  // Число в колонке шириной 4 не помещается: Excel рисует `#####`.
  const narrow = ws.getCell('B1');
  narrow.value = 1234567.89;
  narrow.numFmt = '#,##0.00';
};

/** Ровно 1000 непустых ячеек без стилей: бюджет размера PDF (< 200 КБ). */
const scale1000Cells: Build = (wb) => {
  const ws = wb.addWorksheet('1000 ячеек');
  for (let r = 1; r <= 50; r += 1) {
    ws.getCell(r, 1).value = `Строка ${r}`;
    for (let c = 2; c <= 20; c += 1) {
      ws.getCell(r, c).value = r * 100 + c;
    }
  }
};

/**
 * Высокий лист: на A4 даёт больше десяти страниц (бюджет «10 страниц < 300 мс»).
 *
 * Строки однострочные и без своей высоты (15 pt по умолчанию): число страниц
 * тогда зависит только от числа строк, а не от того, учитывает ли разбивку
 * `ht` из файла. 600 строк — около двенадцати страниц A4 при любой разумной
 * высоте полей, с запасом к порогу в десять.
 */
const scaleTenPages: Build = (wb) => {
  const ws = wb.addWorksheet('Отчёт');
  ws.addRow(['№', 'Наименование', 'Артикул', 'Количество', 'Цена', 'Сумма']);
  ws.getColumn(2).width = 40;
  for (let r = 1; r <= 600; r += 1) {
    ws.addRow([
      r,
      `Позиция ${r}: средний текст отчёта`,
      `АРТ-${String(r).padStart(5, '0')}`,
      (r % 17) + 1,
      // Деньги считаются в копейках и делятся один раз: иначе накопленная
      // погрешность double расходится с тем, что записано в файл.
      (((r * 137) % 9000) + 1000) / 100,
      (((r * 291) % 30000) + 10000) / 100,
    ]);
  }
};

// ── Правка готового пакета ──────────────────────────────────
//
// exceljs не умеет `stopIfTrue` (в 4.4.0 этого атрибута нет ни в одном
// xform), а фикстуре с приоритетами он нужен. Поэтому книга пишется exceljs,
// после чего атрибут дописывается в XML листа. Пакет пересобирается целиком:
// так не приходится пересчитывать смещения и контрольные суммы на месте, и
// содержимое остальных частей остаётся тем, что записал exceljs.

interface ZipEntry {
  name: string;
  data: Buffer;
}

/** Записи zip: имена и распакованное содержимое, в порядке файла. */
function unzip(zip: Buffer): ZipEntry[] {
  let end = zip.length - 22;
  while (end >= 0 && zip.readUInt32LE(end) !== 0x06_05_4b_50) {
    end -= 1;
  }
  if (end < 0) throw new Error('не zip-пакет');
  const count = zip.readUInt16LE(end + 10);
  const entries: ZipEntry[] = [];
  let at = zip.readUInt32LE(end + 16);

  for (let i = 0; i < count; i += 1) {
    if (zip.readUInt32LE(at) !== 0x02_01_4b_50) throw new Error('центральный каталог испорчен');
    const method = zip.readUInt16LE(at + 10);
    const packed = zip.readUInt32LE(at + 20);
    const nameLength = zip.readUInt16LE(at + 28);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.toString('utf8', at + 46, at + 46 + nameLength);
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const raw = zip.subarray(start, start + packed);
    entries.push({
      name,
      data: method === 0 ? Buffer.from(raw) : inflateRawSync(raw),
    });
    at += 46 + nameLength + zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32);
  }
  return entries;
}

/**
 * Собрать zip заново: без сжатых записей и zip64. Дата фиксирована — пакет
 * получается воспроизводимым, а не «сегодняшним».
 */
function zipEntries(entries: ZipEntry[]): Buffer {
  const dosDate = ((2026 - 1980) << 9) | (1 << 5) | 1;
  const parts: Buffer[] = [];
  const directory: Buffer[] = [];
  let offset = 0;

  for (const entry of entries) {
    const name = Buffer.from(entry.name, 'utf8');
    const packed = deflateRawSync(entry.data);
    const checksum = crc32(entry.data);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04_03_4b_50, 0);
    local.writeUInt16LE(20, 4); // версия распаковщика
    local.writeUInt16LE(8, 8); // deflate
    local.writeUInt16LE(dosDate, 12);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(entry.data.length, 22);
    local.writeUInt16LE(name.length, 26);
    parts.push(local, name, packed);

    const record = Buffer.alloc(46);
    record.writeUInt32LE(0x02_01_4b_50, 0);
    record.writeUInt16LE(20, 4);
    record.writeUInt16LE(20, 6);
    record.writeUInt16LE(8, 10);
    record.writeUInt16LE(dosDate, 14);
    record.writeUInt32LE(checksum, 16);
    record.writeUInt32LE(packed.length, 20);
    record.writeUInt32LE(entry.data.length, 24);
    record.writeUInt16LE(name.length, 28);
    record.writeUInt32LE(offset, 42);
    directory.push(record, name);

    offset += 30 + name.length + packed.length;
  }

  const central = Buffer.concat(directory);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06_05_4b_50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(central.length, 12);
  end.writeUInt32LE(offset, 16);

  return Buffer.concat([...parts, central, end]);
}

/** Заменить одну часть пакета текстом, который вернёт `patch`. */
function patchZipEntry(zip: Buffer, name: string, patch: (xml: string) => string): Buffer {
  const entries = unzip(zip);
  const entry = entries.find((e) => e.name === name);
  if (!entry) throw new Error(`в пакете нет части ${name}`);
  entry.data = Buffer.from(patch(entry.data.toString('utf8')), 'utf8');
  return zipEntries(entries);
}

/** Дописать `stopIfTrue="1"` правилу с приоритетом 5 в `cf-priorities`. */
async function addStopIfTrue(file: string): Promise<void> {
  const priority = 5;
  const rule = new RegExp(`<cfRule\\b[^>]*\\bpriority="${priority}"[^>]*>`);
  const patched = patchZipEntry(await readFile(file), 'xl/worksheets/sheet1.xml', (xml) => {
    if (xml.match(new RegExp(rule, 'g'))?.length !== 1) {
      throw new Error(`${file}: правило с priority="${priority}" не найдено однозначно`);
    }
    return xml.replace(rule, (tag) => `${tag.slice(0, -1)} stopIfTrue="1">`);
  });
  await writeFile(file, patched);
}

// ── Сборка списка ───────────────────────────────────────────

const FIXTURES: Fixture[] = [];

/** Простой лист с одной колонкой значений. */
function column(name: string, sheet: string, values: Array<string | number | boolean>): Fixture {
  return {
    name,
    build: (wb) => {
      const ws = wb.addWorksheet(sheet);
      values.forEach((v, i) => {
        ws.getCell(i + 1, 1).value = v;
      });
    },
  };
}

/** Книга из `count` листов с квадратной сеткой чисел. */
function grid(name: string, rows: number, cols: number, sheets = 1): Fixture {
  return {
    name,
    build: (wb) => {
      for (let s = 0; s < sheets; s += 1) {
        const ws = wb.addWorksheet(`Лист${s + 1}`);
        for (let r = 1; r <= rows; r += 1) {
          for (let c = 1; c <= cols; c += 1) {
            ws.getCell(r, c).value = r * 1000 + c;
          }
        }
      }
    },
  };
}

FIXTURES.push(
  { name: 'values-numbers', build: numbers },
  { name: 'values-integers', build: integers },
  { name: 'values-fractions', build: fractions },
  { name: 'values-strings', build: strings },
  { name: 'values-booleans', build: booleans },
  { name: 'values-errors', build: errors },
  { name: 'values-formulas', build: formulas },
  { name: 'values-whitespace', build: whitespace },
  { name: 'values-unicode', build: unicode },
  { name: 'values-long-text', build: longText },

  { name: 'content-markup', build: markup },
  { name: 'content-rich-text', build: richText },
  { name: 'content-sparse', build: sparse },
  { name: 'content-dense', build: dense },
  { name: 'content-table', build: table },
  { name: 'content-single-cell', build: singleCell },
  { name: 'content-empty-sheet', build: (wb) => void wb.addWorksheet('Пусто') },
  { name: 'content-styles-only', build: stylesOnly },
  { name: 'content-mixed-types', build: mixedTypes },
  { name: 'content-numbers-as-text', build: numbersAsText },

  { name: 'styles-bold-italic', build: fontWeight },
  { name: 'styles-font-sizes', build: fontSizes },
  { name: 'styles-font-names', build: fontNames },
  { name: 'styles-font-decorations', build: fontDecorations },
  { name: 'styles-font-colors', build: fontColors },
  { name: 'styles-solid-fills', build: solidFills },
  { name: 'styles-pattern-fills', build: patternFills },
  { name: 'styles-border-styles', build: borderStyles },
  { name: 'styles-border-box', build: borderAll },
  { name: 'styles-alignment', build: alignment },

  { name: 'layout-column-widths', build: columnWidths },
  { name: 'layout-hidden-columns', build: hiddenColumns },
  { name: 'layout-row-heights', build: rowHeights },
  { name: 'layout-hidden-rows', build: hiddenRows },
  { name: 'layout-merged', build: merged },
  { name: 'layout-frozen-rows', build: frozenRows },
  { name: 'layout-frozen-columns', build: frozenColumns },
  { name: 'layout-frozen-both', build: frozenBoth },
  { name: 'layout-no-grid', build: noGrid },
  { name: 'layout-links', build: hyperlinks },

  { name: 'sheets-two', build: twoSheets },
  { name: 'sheets-three', build: threeSheets },
  { name: 'sheets-hidden', build: hiddenSheet },
  { name: 'sheets-very-hidden', build: veryHiddenSheet },
  { name: 'sheets-unicode-names', build: unicodeSheetNames },
  { name: 'sheets-long-name', build: longSheetName },
  { name: 'sheets-different-shapes', build: differentShapes },
  { name: 'sheets-empty-second', build: emptySecondSheet },
  { name: 'sheets-same-data', build: sameDataSheets },
  { name: 'sheets-many', build: manySheets },

  { name: 'text-cyrillic-wrap', build: textCyrillicWrap },
  { name: 'scale-1000-cells', build: scale1000Cells },
  { name: 'scale-ten-pages', build: scaleTenPages },
);

// Условное форматирование: типы правил, операторы, пороги, приоритеты.
FIXTURES.push(
  { name: 'cf-cell-is', build: cfCellIs },
  { name: 'cf-color-scale', build: cfColorScale },
  { name: 'cf-data-bar', build: cfDataBar },
  { name: 'cf-icon-set', build: cfIconSet },
  { name: 'cf-expression', build: cfExpression },
  { name: 'cf-priorities', build: cfPriorities, after: addStopIfTrue },
  { name: 'cf-multi-range', build: cfMultiRange },
);

// Изображения: PNG и JPEG, якоря на ячейку и на диапазон.
FIXTURES.push(
  { name: 'images-png', build: imagesPng },
  { name: 'images-jpeg', build: imagesJpeg },
  { name: 'images-over-data', build: imagesOverData },
);

// Крайние случаи: границы листа, вырожденные размеры, длинные цепочки формул.
FIXTURES.push(
  { name: 'edge-max-cell', build: maxCell },
  { name: 'edge-first-row', build: (wb) => firstRow(wb) },
  { name: 'edge-first-column', build: (wb) => firstColumn(wb) },
  { name: 'edge-negative-zero', build: negativeZero },
  { name: 'edge-tiny-numbers', build: tinyNumbers },
  { name: 'edge-huge-numbers', build: hugeNumbers },
  { name: 'edge-zero-height-row', build: zeroHeightRow },
  { name: 'edge-zero-width-column', build: zeroWidthColumn },
  { name: 'edge-merged-large', build: mergedLarge },
  { name: 'edge-merged-styled', build: mergedStyled },
  { name: 'edge-many-rows', build: (wb) => manyRows(wb) },
  { name: 'edge-many-columns', build: (wb) => manyColumns(wb) },
  { name: 'edge-formula-chain', build: formulaChain },
  { name: 'edge-formula-error', build: formulaError },
  { name: 'edge-mixed-booleans', build: mixedBooleans },
  { name: 'edge-empty-strings', build: emptyStrings },
  { name: 'edge-duplicate-strings', build: duplicateStrings },
  { name: 'edge-column-style', build: columnStyle },
  { name: 'edge-row-style', build: rowStyle },
  { name: 'edge-distant-cells', build: distantCells },
);

// Форматы чисел: по фикстуре на код, значения подбираются под семейство.
for (const [label, code] of FORMATS) {
  const values = DATE_FORMATS.has(label) ? DATE_VALUES : NUMBER_VALUES;
  FIXTURES.push({ name: `formats-${label}`, build: singleFormat(code, values) });
}

// Размеры: от маленьких до крупных.
const SIZES: Array<[number, number, number]> = [
  [10, 10, 1],
  [50, 50, 1],
  [100, 20, 1],
  [200, 10, 1],
  [500, 5, 1],
  [1000, 2, 1],
  [2000, 1, 1],
  [5, 500, 1],
  [20, 100, 1],
  [30, 30, 2],
];
for (const [rows, cols, sheets] of SIZES) {
  FIXTURES.push(grid(`size-${rows}x${cols}x${sheets}`, rows, cols, sheets));
}

// ── Вспомогательные сборки ──────────────────────────────────

function singleCell(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('Одна ячейка').getCell('A1').value = 1;
}

function maxCell(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Последняя ячейка');
  // XFD1048576 — правый нижний угол листа.
  ws.getCell('XFD1048576').value = 'угол';
  ws.getCell('A1').value = 'начало';
}

function firstRow(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Первая строка');
  for (let c = 1; c <= 26; c += 1) {
    ws.getCell(1, c).value = c;
  }
}

function firstColumn(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Первый столбец');
  for (let r = 1; r <= 1000; r += 1) {
    ws.getCell(r, 1).value = r;
  }
}

function negativeZero(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Ноль');
  ws.getCell('A1').value = -0;
  ws.getCell('A2').value = 0;
}

function tinyNumbers(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Малые');
  [5e-324, 1e-300, 1e-100, Number.MIN_VALUE, 2.2250738585072014e-308].forEach((v, i) => {
    ws.getCell(i + 1, 1).value = v;
  });
}

function hugeNumbers(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Большие');
  [1e100, 1e300, Number.MAX_VALUE, 9007199254740993, 1e308].forEach((v, i) => {
    ws.getCell(i + 1, 1).value = v;
  });
}

function zeroHeightRow(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Нулевая высота');
  for (let r = 1; r <= 3; r += 1) {
    ws.getCell(r, 1).value = r;
  }
  ws.getRow(2).height = 0;
}

function zeroWidthColumn(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Нулевая ширина');
  for (let c = 1; c <= 3; c += 1) {
    ws.getCell(1, c).value = c;
  }
  ws.getColumn(2).width = 0;
}

function mergedLarge(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Большое объединение');
  ws.getCell('A1').value = 'во всю ширину';
  ws.mergeCells('A1:E10');
  ws.getCell('G2').value = 'рядом';
}

function mergedStyled(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Объединение со стилем');
  ws.getCell('A1').value = 'заголовок';
  ws.mergeCells('A1:C1');
  ws.getCell('A1').fill = { type: 'pattern', pattern: 'solid', fgColor: argb('CCCCCC') };
  ws.getCell('A1').border = { bottom: { style: 'medium', color: argb('000000') } };
  ws.getCell('A2').value = 'данные';
}

function manyRows(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Много строк');
  for (let r = 1; r <= 5000; r += 1) {
    ws.getCell(r, 1).value = `строка ${r}`;
  }
}

function manyColumns(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Много столбцов');
  for (let c = 1; c <= 200; c += 1) {
    ws.getCell(1, c).value = c;
  }
}

function formulaChain(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Цепочка');
  ws.getCell('A1').value = 1;
  for (let r = 2; r <= 20; r += 1) {
    ws.getCell(r, 1).value = { formula: `A${r - 1}+1`, result: r };
  }
}

function formulaError(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Формула с ошибкой');
  ws.getCell('A1').value = { formula: '1/0', result: { error: '#DIV/0!' } };
}

function mixedBooleans(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Логические вперемешку');
  for (let r = 1; r <= 10; r += 1) {
    ws.getCell(r, 1).value = r % 3 === 0 ? true : r;
    ws.getCell(r, 2).value = r % 2 === 0 ? false : `строка ${r}`;
  }
}

function emptyStrings(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Пустые строки');
  for (let r = 1; r <= 20; r += 1) {
    ws.getCell(r, 1).value = '';
    ws.getCell(r, 2).value = r % 2 === 0 ? '' : 'непусто';
  }
}

function duplicateStrings(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Повторы');
  for (let r = 1; r <= 500; r += 1) {
    ws.getCell(r, 1).value = 'Повторяющаяся строка';
    ws.getCell(r, 2).value = r;
  }
}

function columnStyle(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Стиль столбца');
  for (let c = 1; c <= 4; c += 1) {
    ws.getColumn(c).width = 15;
    ws.getColumn(c).font = { bold: true };
    for (let r = 1; r <= 5; r += 1) {
      ws.getCell(r, c).value = r * c;
    }
  }
}

function rowStyle(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Стиль строки');
  for (let r = 1; r <= 5; r += 1) {
    ws.getRow(r).height = 20;
    ws.getRow(r).font = { italic: true };
    for (let c = 1; c <= 4; c += 1) {
      ws.getCell(r, c).value = r * c;
    }
  }
}

function distantCells(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Далёкие ячейки');
  ws.getCell('A1').value = 'начало';
  ws.getCell('Z1').value = 'двадцать шестой';
  ws.getCell('AA1').value = 'двадцать седьмой';
  ws.getCell('A1000').value = 'тысячная строка';
  ws.getCell('XFD1').value = 'последний столбец';
}

function stylesOnly(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Только стили');
  for (let r = 1; r <= 5; r += 1) {
    const cell = ws.getCell(r, 1);
    cell.fill = { type: 'pattern', pattern: 'solid', fgColor: argb('DDDDDD') };
    cell.border = { bottom: { style: 'thin', color: argb('000000') } };
  }
  ws.getCell('A1').value = 'со стилем';
}

function mixedTypes(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Смесь');
  ws.getCell('A1').value = 42;
  ws.getCell('A2').value = 'строка';
  ws.getCell('A3').value = true;
  ws.getCell('A4').value = { error: '#N/A' };
  ws.getCell('A5').value = { formula: 'A1*2', result: 84 };
  ws.getCell('A6').value = { richText: [{ text: 'прогон' }] };
  ws.getCell('A7').value = { text: 'ссылка', hyperlink: 'https://example.com/' };
  ws.getCell('A8').value = null;
  ws.getCell('A9').value = '';
}

function numbersAsText(wb: ExcelJS.Workbook): void {
  const ws = wb.addWorksheet('Числа текстом');
  ['007', '1 234,56', '-0', '1e5', '0x10', ' 42 '].forEach((v, i) => {
    const cell = ws.getCell(i + 1, 1);
    cell.value = v;
    cell.numFmt = '@';
  });
}

function singleFormat(code: string, values: number[]): Build {
  return (wb) => {
    const ws = wb.addWorksheet('Формат');
    values.forEach((v, i) => {
      const cell = ws.getCell(i + 1, 1);
      cell.value = v;
      cell.numFmt = code;
    });
  };
}

function twoSheets(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('Первый').addRow(['a', 1, true]);
  wb.addWorksheet('Второй').addRow(['b', 2, false]);
}

function threeSheets(wb: ExcelJS.Workbook): void {
  for (let s = 1; s <= 3; s += 1) {
    wb.addWorksheet(`Лист${s}`).addRow([s, `значение ${s}`]);
  }
}

function hiddenSheet(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('Видимый').addRow([1, 2, 3]);
  const hidden = wb.addWorksheet('Скрытый');
  hidden.addRow(['секрет']);
  hidden.state = 'hidden';
}

function veryHiddenSheet(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('Видимый').addRow(['данные']);
  const hidden = wb.addWorksheet('Очень скрытый');
  hidden.addRow(['тайна']);
  hidden.state = 'veryHidden';
}

function unicodeSheetNames(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('Данные за 2024').addRow([1]);
  wb.addWorksheet('Продажи — итог').addRow([2]);
  wb.addWorksheet('日本語').addRow([3]);
  wb.addWorksheet('Ünïcödé').addRow([4]);
}

function longSheetName(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('Лист с очень длинным названием').addRow(['данные']);
  wb.addWorksheet('Ещё один длинный лист').addRow(['данные']);
}

function differentShapes(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('Одна ячейка').getCell('A1').value = 'одна';
  const wide = wb.addWorksheet('Широкая строка');
  for (let c = 1; c <= 50; c += 1) {
    wide.getCell(1, c).value = c;
  }
  const tall = wb.addWorksheet('Высокий столбец');
  for (let r = 1; r <= 200; r += 1) {
    tall.getCell(r, 1).value = r;
  }
}

function emptySecondSheet(wb: ExcelJS.Workbook): void {
  wb.addWorksheet('С данными').addRow(['a', 'b']);
  wb.addWorksheet('Пустой');
}

function sameDataSheets(wb: ExcelJS.Workbook): void {
  for (const name of ['Январь', 'Февраль', 'Март']) {
    const ws = wb.addWorksheet(name);
    ws.addRow(['Показатель', 'Значение']);
    ws.addRow(['Выручка', 1000]);
    ws.addRow(['Расход', 400]);
  }
}

function manySheets(wb: ExcelJS.Workbook): void {
  for (let s = 1; s <= 10; s += 1) {
    wb.addWorksheet(`Лист${s}`).addRow([s, s * s, s * s * s]);
  }
}

// ── Эталон ──────────────────────────────────────────────────

type Json = Record<string, unknown>;

/**
 * Ячейка эталона в компактной записи: `[строка, столбец, вид, значение]`.
 *
 * Виды: `n` — число, `s` — строка, `b` — логическое, `e` — ошибка,
 * `-` — значения нет. Пятым элементом идёт текст формулы, шестым — адрес
 * гиперссылки. Массивы вместо объектов: эталон хранится в репозитории, и
 * разница в размере тут семикратная.
 */
type CellRecord = [number, number, string, unknown?, string?, string?];

function describe(cell: ExcelJS.Cell, row: number, col: number): CellRecord {
  const at = [row - 1, col - 1] as const;
  const value: unknown = cell.value;

  if (value === null || value === undefined) {
    return [...at, '-'];
  }
  if (typeof value === 'number') {
    return [...at, 'n', value];
  }
  if (typeof value === 'string') {
    return [...at, 's', value];
  }
  if (typeof value === 'boolean') {
    return [...at, 'b', value];
  }
  if (value instanceof Date) {
    // Серийный номер — то, что реально лежит в файле.
    return [...at, 'n', (value.getTime() - EPOCH_MS) / 86_400_000];
  }
  if (typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    if (typeof obj.error === 'string') {
      return [...at, 'e', obj.error];
    }
    if (Array.isArray(obj.richText)) {
      const text = (obj.richText as Array<{ text: string }>).map((run) => run.text).join('');
      return [...at, 's', text];
    }
    if (typeof obj.hyperlink === 'string') {
      return [...at, 's', obj.text, undefined, obj.hyperlink];
    }
    if (typeof obj.formula === 'string' || typeof obj.sharedFormula === 'string') {
      const formula = (obj.formula ?? obj.sharedFormula) as string;
      const result = obj.result;
      if (typeof result === 'number') {
        return [...at, 'n', result, formula];
      }
      if (typeof result === 'string') {
        return [...at, 's', result, formula];
      }
      const error = (result as { error?: string } | undefined)?.error;
      if (typeof error === 'string') {
        return [...at, 'e', error, formula];
      }
      return [...at, '-', undefined, formula];
    }
  }
  throw new Error(`неизвестное значение в ${row}:${col}: ${JSON.stringify(value)}`);
}

async function oracleFor(file: string): Promise<Json> {
  const wb = new ExcelJS.Workbook();
  await wb.xlsx.readFile(file);
  const sheets: Json[] = [];
  wb.eachSheet((ws) => {
    const cells: CellRecord[] = [];
    ws.eachRow({ includeEmpty: false }, (row, rowNumber) => {
      row.eachCell({ includeEmpty: false }, (cell, colNumber) => {
        if (cell.value === null || cell.value === undefined) {
          return;
        }
        // exceljs размножает значение по всем ячейкам объединения, а в файле
        // оно лежит только в левой верхней. Эталон описывает файл.
        if (cell.isMerged && cell.master.address !== cell.address) {
          return;
        }
        cells.push(describe(cell, rowNumber, colNumber));
      });
    });
    sheets.push({ name: ws.name, state: ws.state ?? 'visible', cells });
  });
  return { sheets };
}

async function main(): Promise<void> {
  if (new Set(FIXTURES.map((f) => f.name)).size !== FIXTURES.length) {
    throw new Error('имена фикстур повторяются');
  }
  await mkdir(OUT_DIR, { recursive: true });

  const files: Json = {};
  for (const fixture of FIXTURES) {
    const wb = new ExcelJS.Workbook();
    wb.creator = 'doc-converter fixtures';
    wb.created = new Date(Date.UTC(2026, 0, 1));
    // `modified` exceljs иначе выставляет по часам — перегенерация даёт шум в core.xml.
    wb.modified = new Date(Date.UTC(2026, 0, 1));
    fixture.build(wb);

    const file = path.join(OUT_DIR, `${fixture.name}.xlsx`);
    await wb.xlsx.writeFile(file);
    // archiver ставит в заголовки zip момент записи — пересобираем пакет с
    // фиксированной датой (zipEntries), иначе байты плывут от запуска к запуску.
    await writeFile(file, zipEntries(unzip(await readFile(file))));
    await fixture.after?.(file);

    files[`${fixture.name}.xlsx`] = await oracleFor(file);
  }

  await writeFile(ORACLE, `${JSON.stringify({ version: 1, files })}\n`);
  console.log(`фикстур: ${FIXTURES.length}, эталон: ${path.relative(ROOT, ORACLE)}`);
}

await main();
