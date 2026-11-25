// Недостающие XLSX-фикстуры приоритетного состава, которых нет в
// `scripts/gen-fixtures.ts`: print area с повторяющимися заголовками,
// fit-to-page, печатные колонтитулы, внешние ссылки, date1904, shrint-to-fit,
// pivot и slicer.
//
// Почему отдельным скриптом, а не в общем генераторе: это части, которые
// Excel пишет «руками» и которые нельзя получить через exceljs — они либо
// отсутствуют в API, либо требуют порядка элементов внутри `worksheet`.
// Поэтому здесь строится XML целиком, а не патчится готовый файл.
//
// Детерминизм: свой ZIP-писатель с фиксированными таймстампами (общий
// `scripts/docx-zip.ts`), порядок записей фиксирован.

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { buildZip } from './docx-zip.js';

const ROOT = path.resolve(import.meta.dirname, '..');
const OUT_DIR = path.join(ROOT, 'test-fixtures', 'xlsx');

/**
 * Тема Office, взятая из существующей фикстуры.
 *
 * Без `theme1.xml` ссылка `color theme="1"` не разрешается ни в чёрный, ни в
 * что-либо ещё — тест `every_fixture_resolves_the_default_text_color_to_black`
 * это ловит. Копия живёт рядом генератором, а не извлекается из чужой книги
 * на лету: иначе правка той фикстуры молча меняла бы эти.
 */
const THEME = await readFile(path.join(import.meta.dirname, 'xlsx-office-theme.xml'), 'utf8');

const SML_NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main';
const R_NS = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships';

const XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n';

/** Одна книга: имя без расширения и части пакета. */
interface Book {
  /** Путь без расширения относительно `test-fixtures/xlsx/`. */
  name: string;
  /** Содержимое листов: `sheet1.xml`, `sheet2.xml`, … по порядку. */
  sheets: string[];
  /** Дополнительные части помимо стандартных. */
  extraParts?: Record<string, string>;
  /** Строки `xl/workbook.xml`: `<sheet name=… r:id=…>` по порядку. */
  sheetNames: string[];
  /** `xl/workbook.xml` целиком — если нужен `definedName` или `date1904`. */
  workbookOverride?: string;
  /** `[Content_Types].xml` целиком — если нужны Override для новых частей. */
  contentTypesOverride?: string;
  /**
   * `xl/styles.xml` целиком — только для книги, которой нужен собственный
   * `<alignment>`.
   *
   * Общая таблица стилей намеренно не расширяется: `cellXfs` в ней общий для
   * всех восьми книг, и добавленное выравнивание изменило бы байты остальных
   * семи. Поэтому переопределение точечное — у остальных книг стили остаются
   * прежними.
   */
  stylesOverride?: string;
}

/** Абзац-совместимая ячейка с строкой: `<c r="A1" t="inlineStr">`. */
const textCell = (ref: string, text: string): string =>
  `<c r="${ref}" t="inlineStr"><is><t>${text}</t></is></c>`;

/** Ячейка с числом. */
const numCell = (ref: string, value: string): string =>
  `<c r="${ref}"><v>${value}</v></c>`;

/** Строка листа. */
const row = (index: number, cells: string, attrs = ''): string =>
  `<row r="${index}"${attrs}>${cells}</row>`;

/** Лист из строк и дополнительных блоков (после `sheetData`). */
const worksheet = (rowsXml: string, after = '', before = '', extraNs = ''): string =>
  XML_DECL +
  `<worksheet xmlns="${SML_NS}" xmlns:r="${R_NS}"${extraNs}>` +
  before +
  `<sheetData>${rowsXml}</sheetData>` +
  after +
  '</worksheet>';

/** `cellXfs`-запись 1 по умолчанию: только денежный формат, без выравнивания. */
const CURRENCY_XF =
  '<xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>';

/**
 * `cellXfs`-запись 1 для книги shrink-to-fit: только
 * `<alignment shrinkToFit="1" wrapText="0"/>` — «ужать, но не переносить».
 *
 * Денежного `numFmt` здесь нет намеренно: ячейка — `inlineStr`, текстовый
 * формат на ней ничего не форматирует, зато намекал бы на числовую ячейку.
 */
const SHRINK_XF =
  '<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1">' +
  '<alignment shrinkToFit="1" wrapText="0"/></xf>';

/**
 * `xl/styles.xml` с заданной записью `cellXfs[1]`.
 *
 * `cellXfs[0]` — обычный формат, на него ссылается большинство ячеек; вторая
 * запись различается между книгами и приходит сюда параметром.
 */
function stylesXml(secondXf: string): string {
  return (
    XML_DECL +
    `<styleSheet xmlns="${SML_NS}">` +
    '<numFmts count="1"><numFmt numFmtId="164" formatCode="#,##0.00\\ &quot;₽&quot;"/></numFmts>' +
    '<fonts count="1"><font><sz val="11"/><color theme="1"/><name val="Calibri"/></font></fonts>' +
    '<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>' +
    '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>' +
    '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>' +
    `<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>${secondXf}</cellXfs>` +
    '</styleSheet>'
  );
}

/** Общая таблица стилей всех книг, кроме `layout-shrink-to-fit`. */
const SHARED_STYLES = stylesXml(CURRENCY_XF);

/** Стили книги shrink-to-fit: `s="1"` разрешается в `shrinkToFit="1"`. */
const SHRINK_STYLES = stylesXml(SHRINK_XF);

/** Стандартные части книги без листов. */
function standardParts(book: Book): Array<[string, string]> {
  const worksheets = book.sheets.map(
    (sheet, index) => `xl/worksheets/sheet${index + 1}.xml` as const,
  );
  const sheetEntries = book.sheetNames
    .map(
      (name, index) =>
        `<sheet name="${name}" sheetId="${index + 1}" r:id="rId${index + 1}"/>`,
    )
    .join('');
  const workbook =
    book.workbookOverride ??
    XML_DECL +
      `<workbook xmlns="${SML_NS}" xmlns:r="${R_NS}"><sheets>${sheetEntries}</sheets></workbook>`;

  const workbookRels =
    XML_DECL +
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    worksheets
      .map(
        (_, index) =>
          `<Relationship Id="rId${index + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet${index + 1}.xml"/>`,
      )
      .join('') +
    '<Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>' +
    '<Relationship Id="rIdTheme" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>' +
    '</Relationships>';

  const styles = book.stylesOverride ?? SHARED_STYLES;

  const contentTypes =
    book.contentTypesOverride ??
    XML_DECL +
      '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
      '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
      '<Default Extension="xml" ContentType="application/xml"/>' +
      '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
      worksheets
        .map(
          (name) =>
            `<Override PartName="/${name}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`,
        )
        .join('') +
      '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>' +
      '<Override PartName="/xl/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>' +
      '</Types>';

  const rootRels =
    XML_DECL +
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>' +
    '</Relationships>';

  const parts: Array<[string, string]> = [
    ['[Content_Types].xml', contentTypes],
    ['_rels/.rels', rootRels],
    ['xl/workbook.xml', workbook],
    ['xl/_rels/workbook.xml.rels', workbookRels],
    ['xl/styles.xml', styles],
    ['xl/theme/theme1.xml', THEME],
  ];
  worksheets.forEach((name, index) => {
    const sheet = book.sheets[index];
    if (sheet !== undefined) parts.push([name, sheet]);
  });
  for (const [name, content] of Object.entries(book.extraParts ?? {})) {
    parts.push([name, content]);
  }
  return parts;
}

// ==================== Фикстуры ====================

/** Print area + повторяющиеся заголовки: `definedName` в workbook. */
const printAreaTitles: Book = {
  name: 'print-area-titles',
  sheetNames: ['Отчёт'],
  workbookOverride:
    XML_DECL +
    `<workbook xmlns="${SML_NS}" xmlns:r="${R_NS}"><sheets>` +
    '<sheet name="Отчёт" sheetId="1" r:id="rId1"/>' +
    '</sheets><definedNames>' +
    '<definedName name="_xlnm.Print_Area" localSheetId="0">\'Отчёт\'!$A$1:$D$40</definedName>' +
    '<definedName name="_xlnm.Print_Titles" localSheetId="0">\'Отчёт\'!$1:$2</definedName>' +
    '</definedNames></workbook>',
  sheets: [
    worksheet(
      Array.from({ length: 40 }, (_, index) =>
        row(
          index + 1,
          textCell(`A${index + 1}`, index < 2 ? `Заголовок ${index + 1}` : `Строка ${index + 1}`) +
            numCell(`B${index + 1}`, String(index + 1)) +
            numCell(`C${index + 1}`, String((index + 1) * 2)) +
            numCell(`D${index + 1}`, String((index + 1) * 3)),
        ),
      ).join(''),
      '<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>',
    ),
  ],
};

/** Fit-to-page: одна страница по ширине, без ограничения по высоте. */
const fitToPage: Book = {
  name: 'print-fit-to-page',
  sheetNames: ['Fit'],
  sheets: [
    worksheet(
      Array.from({ length: 30 }, (_, index) =>
        row(
          index + 1,
          Array.from({ length: 12 }, (_, col) =>
            textCell(`${String.fromCharCode(65 + col)}${index + 1}`, `R${index + 1}C${col + 1}`),
          ).join(''),
        ),
      ).join(''),
      '<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>' +
        '<pageSetup fitToWidth="1" fitToHeight="0" orientation="landscape"/>',
      '<sheetPr><pageSetUpPr fitToPage="1"/></sheetPr>',
    ),
  ],
};

/** Печатные колонтитулы: часть `xl/headerFooter1.xml`, ссылка через rels. */
const printHeadersFooters: Book = {
  name: 'print-headers-footers',
  sheetNames: ['Колонтитулы'],
  extraParts: {
    'xl/headerFooter1.xml': XML_DECL + '<oddHeader>&amp;LОтчёт&amp;CСтр. &amp;P из &amp;N&amp;RДата &amp;D</oddHeader><oddFooter>&amp;LКонфиденциально&amp;RСтр. &amp;P</oddFooter>',
  },
  contentTypesOverride:
    XML_DECL +
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
    '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' +
    '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>' +
    '<Override PartName="/xl/headerFooter1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.printHeaderFooter+xml"/>' +
    '</Types>',
  sheets: [
    worksheet(
      row(1, textCell('A1', 'Лист с печатными колонтитулами')),
      '<headerFooter><oddHeader>&amp;LОтчёт&amp;CСтр. &amp;P из &amp;N&amp;RДата &amp;D</oddHeader>' +
        '<oddFooter>&amp;LКонфиденциально&amp;RСтр. &amp;P</oddFooter></headerFooter>' +
        '<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>',
    ),
  ],
};

/** Даты в системе 1904. */
const date1904: Book = {
  name: 'formats-date-1904',
  sheetNames: ['Даты1904'],
  workbookOverride:
    XML_DECL +
    `<workbook xmlns="${SML_NS}" xmlns:r="${R_NS}"><workbookPr date1904="1"/><sheets>` +
    '<sheet name="Даты1904" sheetId="1" r:id="rId1"/>' +
    '</sheets></workbook>',
  sheets: [
    worksheet(
      ['02.01.1904', '15.06.2026', '31.12.1999']
        .map((text, index) => row(index + 1, textCell(`A${index + 1}`, text)))
        .join(''),
    ),
  ],
};

/**
 * Shrink-to-fit: текст ужимается вместо переноса.
 *
 * Своя таблица стилей: `s="1"` — это `cellXfs[1]` (индексы с нуля), и только
 * в ней лежит `<alignment shrinkToFit="1" wrapText="0"/>`.
 */
const shrinkToFit: Book = {
  name: 'layout-shrink-to-fit',
  sheetNames: ['Ужать'],
  stylesOverride: SHRINK_STYLES,
  sheets: [
    worksheet(
      row(
        1,
        '<c r="A1" s="1" t="inlineStr"><is><t>Очень длинный текст, который должен ужаться в узкую ячейку шириной двенадцать знаков без переноса и без выхода за границу</t></is></c>',
        ' ht="20" customHeight="1"',
      ),
      '',
      '<cols><col min="1" max="1" width="12" customWidth="1"/></cols>',
    ),
  ],
};

/** Внешняя ссылка: `externalReference` и часть `xl/externalLinks/externalLink1.xml`. */
const externalLink: Book = {
  name: 'external-workbook-link',
  sheetNames: ['Ссылки'],
  extraParts: {
    'xl/externalLinks/externalLink1.xml':
      XML_DECL +
      `<externalLink xmlns="${SML_NS}" xmlns:r="${R_NS}">` +
      '<externalBook r:id="rId1"><sheetNames><sheetName val="Лист1"/></sheetNames>' +
      '<sheetDataSet><sheetData sheetId="0"><row r="1"><cell r="1"><v>41</v></cell></row></sheetData></sheetDataSet>' +
      '</externalBook></externalLink>',
    'xl/externalLinks/_rels/externalLink1.xml.rels':
      XML_DECL +
      '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
      '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath" Target="file:///C:/data/source.xlsx" TargetMode="External"/>' +
      '</Relationships>',
  },
  contentTypesOverride:
    XML_DECL +
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
    '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' +
    '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>' +
    '<Override PartName="/xl/externalLinks/externalLink1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml"/>' +
    '</Types>',
  sheets: [
    worksheet(
      row(1, textCell('A1', 'Значение из внешней книги:') + numCell('B1', '41')) +
        row(2, textCell('A2', 'Источник: source.xlsx')),
    ),
  ],
};

/** Сводная таблица: минимальный pivotCacheDefinition + pivotTable. */
const pivot: Book = {
  name: 'pivot-basic',
  sheetNames: ['Данные', 'Сводная'],
  extraParts: {
    'xl/pivotCache/pivotCacheDefinition1.xml':
      XML_DECL +
      `<pivotCacheDefinition xmlns="${SML_NS}" xmlns:r="${R_NS}" r:id="rId1" refreshOnLoad="1" recordCount="4">` +
      '<cacheSource type="worksheet"><worksheetSource ref="A1:B5" sheet="Данные"/></cacheSource>' +
      '<cacheFields count="2">' +
      '<cacheField name="Категория" numFmtId="0"><sharedItems count="2"><s v="A"/><s v="B"/></sharedItems></cacheField>' +
      '<cacheField name="Сумма" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0"><n v="10"/><n v="20"/></sharedItems></cacheField>' +
      '</cacheFields></pivotCacheDefinition>',
    'xl/pivotTables/pivotTable1.xml':
      XML_DECL +
      `<pivotTableDefinition xmlns="${SML_NS}" name="Сводная1" cacheId="1" applyNumberFormats="0" applyBorderFormats="0" dataCaption="Значения">` +
      '<location ref="A3:B6" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/>' +
      '<pivotFields count="2"><pivotField showAll="0"/><pivotField dataField="1" showAll="0"/></pivotFields>' +
      '<rowFields count="1"><field x="0"/></rowFields>' +
      '<dataFields count="1"><dataField name="Сумма по полю Сумма" fld="1" baseField="0" baseItem="0"/></dataFields>' +
      '</pivotTableDefinition>',
  },
  sheets: [
    worksheet(
      row(1, textCell('A1', 'Категория') + textCell('B1', 'Сумма')) +
        row(2, textCell('A2', 'A') + numCell('B2', '10')) +
        row(3, textCell('A3', 'B') + numCell('B3', '20')) +
        row(4, textCell('A4', 'A') + numCell('B4', '10')) +
        row(5, textCell('A5', 'B') + numCell('B5', '20')),
    ),
    worksheet(
      row(3, textCell('A3', 'Категория') + textCell('B3', 'Сумма')) +
        row(4, textCell('A4', 'A') + numCell('B4', '20')) +
        row(5, textCell('A5', 'B') + numCell('B5', '40')),
    ),
  ],
};

/** Slicer поверх сводной: часть `xl/slicers/slicer1.xml`. */
const slicer: Book = {
  name: 'slicer-basic',
  sheetNames: ['Данные'],
  extraParts: {
    'xl/slicers/slicer1.xml':
      XML_DECL +
      `<slicers xmlns="${SML_NS}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" ` +
      'xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" mc:Ignorable="x">' +
      '<slicer name="Категория" cache="Slicer_Категория" caption="Категория" rowHeight="241300" ' +
      'columnCount="1" startItem="0" x:style="SlicerStyleLight1"/></slicers>',
  },
  contentTypesOverride:
    XML_DECL +
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
    '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' +
    '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>' +
    '<Override PartName="/xl/slicers/slicer1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.slicer+xml"/>' +
    '</Types>',
  sheets: [
    worksheet(
      row(1, textCell('A1', 'Активный фильтр: категория A')) +
        row(2, textCell('A2', 'Значения скрыты slicer-ом')),
    ),
  ],
};

/** Все книги этого скрипта. */
const books: Book[] = [
  printAreaTitles,
  fitToPage,
  printHeadersFooters,
  date1904,
  shrinkToFit,
  externalLink,
  pivot,
  slicer,
];

/** Точка входа: собрать и записать книги. */
async function main(): Promise<void> {
  await mkdir(OUT_DIR, { recursive: true });
  for (const book of books) {
    const parts = standardParts(book);
    const bytes = buildZip(parts);
    const file = path.join(OUT_DIR, `${book.name}.xlsx`);
    await writeFile(file, bytes);
    process.stdout.write(`wrote ${path.relative(ROOT, file)}\n`);
  }
}

await main();
