// Записи реестра для фикстур, сгенерированных в
// `scripts/docx-fixtures/strict-viewer.ts` (пагинация, колонки, endnotes,
// Math, SmartArt/OLE, fallback шрифтов).
//
// `expectedWarnings` этих фикстур (`docx-math-unsupported`,
// `docx-smartart-excluded`, `docx-ole-excluded`) живут в сайдкаре и
// проверяются тестами крейта; здесь фиксируется только то, что нужно реестру.

import type { FixtureInput } from './add.js';

const fixtures: FixtureInput[] = [
  {
    path: 'docx/columns/two_equal.docx',
    title: 'Две равные колонки с разрывом колонки',
    tags: ['sections', 'pagination'],
    matrixRows: ['docx.sections-columns'],
    expected:
      'Две равные колонки с разделителем: текст заполняет первую, явный разрыв колонки переводит его во вторую; разделитель виден между колонками.',
    modes: ['print-layout'],
    notes: 'Пункт 5 приоритетного состава (колонки). Было помечено `planned`.',
  },
  {
    path: 'docx/columns/three_unequal.docx',
    title: 'Три колонки разной ширины',
    tags: ['sections', 'pagination'],
    matrixRows: ['docx.sections-columns'],
    expected:
      'Три колонки с ширинами 2551/5102/2551 twips: центральная вдвое шире боковых, `w:equalWidth="0"` соблюдается.',
    modes: ['print-layout'],
    notes: 'Пункт 5 приоритетного состава (неравные колонки).',
  },
  {
    path: 'docx/pagination/keep_next_chain.docx',
    title: 'Цепочка абзацев с keepNext',
    tags: ['pagination', 'text'],
    matrixRows: ['docx.paragraph-runs'],
    expected:
      'Заголовок с `keepNext` и `keepLines` переезжает на следующую страницу вместе с абзацем цепочки; обычные абзацы разрываются свободно.',
    modes: ['print-layout'],
    notes: 'Пункт 2 приоритетного состава (keepNext).',
  },
  {
    path: 'docx/pagination/widow_orphan.docx',
    title: 'Widow/orphan на границе страниц',
    tags: ['pagination'],
    matrixRows: ['docx.tables-pagination'],
    expected:
      'Первый абзац с `w:widowControl` переносится целиком или не оставляет одну строку; второй с `w:val="0"` оставляет вдову намеренно.',
    modes: ['print-layout'],
    notes:
      'Пункт 3 приоритетного состава. Отдельной строки матрицы для widow/orphan нет — нужна новая строка либо привязка к пагинации таблиц.',
  },
  {
    path: 'docx/notes/endnote_basic.docx',
    title: 'Концевые сноски',
    tags: ['notes'],
    matrixRows: ['docx.footnotes-endnotes'],
    expected:
      'Ссылка `w:endnoteReference` в тексте, текст в `word/endnotes.xml`; служебные `separator` и `continuationSeparator` не считаются содержимым.',
    modes: ['print-layout'],
    notes: 'Пункт 14 приоритетного состава. Закрывает gap `endnotes`.',
  },
  {
    path: 'docx/math/omml_simple.docx',
    title: 'Office Math: дробь в m:oMathPara',
    tags: ['math', 'unsupported'],
    matrixRows: ['docx.math'],
    expected:
      'Формула `x = (a + b) / 2` в `m:oMathPara`; движок её не рисует, но обязан выдать `docx-math-unsupported` и не терять соседний текст.',
    modes: ['print-layout'],
    notes: 'Пункт 16 приоритетного состава. Закрывает gap `OMML`.',
  },
  {
    path: 'docx/embedded/smartart_basic.docx',
    title: 'SmartArt: ссылка на diagrams/dataModel',
    tags: ['objects', 'unsupported'],
    matrixRows: ['docx.smartart'],
    expected:
      '`dgm:relIds` внутри `w:drawing` и часть `word/diagrams/data1.xml`; наличие даёт `docx-smartart-excluded`, визуальная поддержка не заявляется.',
    modes: ['print-layout'],
    notes: 'Пункт 17 приоритетного состава. Строка матрицы — `excluded`.',
  },
  {
    path: 'docx/embedded/ole_object.docx',
    title: 'OLE-объект с VML-превью',
    tags: ['objects', 'unsupported'],
    matrixRows: ['docx.ole'],
    expected:
      '`w:object` с `v:shape` и `r:id` на `embeddings/oleObject1.bin`; наличие даёт `docx-ole-excluded`, превью не считается поддержкой OLE.',
    modes: ['print-layout'],
    notes: 'Пункт 17 приоритетного состава. Строка матрицы — `excluded`.',
  },
  {
    path: 'docx/fonts/system_font_fallback.docx',
    title: 'Системные шрифты вне пакета и fallback',
    tags: ['fonts', 'text'],
    matrixRows: ['docx.bidi-cjk'],
    expected:
      'Times New Roman и Wingdings берутся из системы, отсутствующая гарнитура заменяется по умолчанию; текст не исчезает ни в одном случае.',
    modes: ['print-layout'],
    notes: 'Пункт 20 приоритетного состава (fallback-политика).',
  },
  {
    path: 'docx/headers_footers/even_odd.docx',
    title: 'Чётный, нечётный и первый колонтитулы',
    tags: ['headers', 'pagination'],
    matrixRows: ['docx.headers-footers'],
    expected:
      '`w:evenAndOddHeaders` в settings.xml: страница 1 (при `w:titlePg`) — «First page header», страница 2 — «Even page header», страница 3 — «Default header»; связи колонтитулов не теряются.',
    modes: ['print-layout', 'pdf'],
    notes:
      'Дополнительная фикстура строки docx.headers-footers; тип even проверяется раскладкой (`test_layout_even_and_odd_headers_differ`). Закрывает gap `even-page-type`.',
  },
  {
    path: 'docx/fields/toc_ref.docx',
    title: 'Поля TOC и REF с сохранённым кэшем',
    tags: ['text', 'formulas'],
    matrixRows: ['docx.fields-toc-refs'],
    expected:
      'TOC (` TOC \\o "1-3" \\h \\z \\u `) показывает кэш «Введение» и номер страницы после табуляции; REF (` REF _Ref123 \\h \\* MERGEFORMAT `) — «Раздел с закладкой» из `w:bookmarkStart w:name="_Ref123"`. Поля не пересчитываются.',
    modes: ['print-layout'],
    notes:
      'Пункт 6 приоритетного состава (TOC/REF через fldChar). Закрывает gap `TOC-REF`; автоматическая генерация оглавления не заявляется.',
  },
  {
    path: 'docx/images/inline_jpeg_emf.docx',
    title: 'Inline JPEG и EMF рядом',
    tags: ['images', 'objects'],
    matrixRows: ['docx.images-inline'],
    expected:
      'Два inline-рисунка в одном абзаце: JPEG 914400×685800 EMU и EMF 457200×457200 EMU. JPEG декодируется, EMF не декодируется и не роняет экспорт; размеры и порядок берутся из DrawingML, а не из формата.',
    modes: ['print-layout'],
    notes:
      'Пункт 9 приоритетного состава (артефакты drawing). Дополнительная фикстура строки docx.images-inline: проверяет, что неподдерживаемый формат не рвёт цепочку рисунков.',
  },
  {
    path: 'docx/pagination/keep_next_tall.docx',
    title: 'keepNext у абзаца длиннее страницы',
    tags: ['pagination'],
    matrixRows: ['docx.tables-pagination'],
    expected:
      'Абзац с `w:keepNext`, который сам не помещается на страницу, переносится по обычным правилам; следующий абзац не приклеивается к нему насильно и не появляется пустая страница.',
    modes: ['print-layout'],
    notes:
      'Пункт 2 приоритетного состава (keepNext). Дополнительная фикстура: правило не может удержать то, что длиннее полосы набора.',
  },
];

export default fixtures;
