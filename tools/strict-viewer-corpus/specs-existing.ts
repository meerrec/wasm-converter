// Записи реестра для фикстур, которые уже лежат в `test-fixtures/`, но не были
// в `corpus.json`. Все пути проверены на диске; `id` вычисляет `add.ts`.
//
// Связь со строками матрицы — по `docs/strict-viewer/certification-matrix.md`:
// каждая запись закрывает ровно тот пункт приоритетного состава, ради которого
// фикстура создавалась.

import type { FixtureInput } from './add.js';

const fixtures: FixtureInput[] = [
  {
    path: 'docx/basic/breaks_and_tabs.docx',
    title: 'Разрывы строки и страницы, табуляция',
    tags: ['text', 'pagination'],
    matrixRows: ['docx.paragraph-runs'],
    expected:
      'Четыре абзаца с разрывом строки, разрывом страницы и табуляцией: разрыв страницы начинает новую страницу, табуляция не превращается в пробел.',
    modes: ['print-layout'],
    notes: 'Пункт 2 приоритетного состава (разрывы). `keepNext` покрывает отдельная фикстура.',
  },
  {
    path: 'docx/tables/alignment_widths.docx',
    title: 'Явные ширины колонок и выравнивание ячеек',
    tags: ['tables', 'geometry'],
    matrixRows: ['docx.tables-fixed-autofit'],
    expected:
      'Ширины w:tblGrid и w:tcW применяются как заданы; выравнивание w:jc и w:vAlign меняет положение текста внутри ячейки.',
    modes: ['print-layout'],
    notes: 'Пункт 7 приоритетного состава (AutoFit + выравнивание).',
  },
  {
    path: 'docx/tables/nested.docx',
    title: 'Вложенная таблица внутри ячейки',
    tags: ['tables', 'geometry'],
    matrixRows: ['docx.tables-merged-nested'],
    expected:
      'Внутренняя таблица рисуется внутри ячейки внешней: границы обеих видны, содержимое не дублируется и не пропадает.',
    modes: ['print-layout'],
    notes: 'Пункт 8 приоритетного состава (nested).',
  },
  {
    path: 'docx/tables/cant_split_rows.docx',
    title: 'Строки, которые нельзя разорвать между страницами',
    tags: ['tables', 'pagination'],
    matrixRows: ['docx.tables-pagination'],
    expected:
      'Двенадцать строк с w:cantSplit: строка целиком переносится на следующую страницу, а не разрывается; седьмая строка выше остальных.',
    modes: ['print-layout'],
    notes: 'Пункт 9 приоритетного состава (`cantSplit`).',
  },
  {
    path: 'docx/tables/row_taller_than_page.docx',
    title: 'Строка таблицы выше страницы',
    tags: ['tables', 'pagination'],
    matrixRows: ['docx.tables-pagination'],
    expected:
      'Ячейка с 60 абзацами выше полосы набора A4: таблица разрывается по страницам без потери текста и без пустых страниц-артефактов.',
    modes: ['print-layout'],
    notes: 'Пункт 9 приоритетного состава (строка выше страницы).',
  },
  {
    path: 'docx/alternate_content/group_shape.docx',
    title: 'Группа DrawingML-фигур с VML-фолбэком',
    tags: ['objects', 'unsupported'],
    matrixRows: ['docx.shapes-textboxes', 'docx.vml-alternate'],
    expected:
      'wpg:wgp из двух фигур в mc:Choice рисуется как группа; VML-группа в mc:Fallback не используется (ADR-0014).',
    modes: ['print-layout'],
    notes: 'Пункты 12–13 приоритетного состава (группы shapes, AlternateContent).',
  },
  {
    path: 'docx/cjk/mixed_latin.docx',
    title: 'Смешанный абзац: латиница и CJK',
    tags: ['text', 'fonts', 'bidi'],
    matrixRows: ['docx.bidi-cjk'],
    expected:
      'Латиница и иероглифы в разных прогонах одного абзаца: порядок символов и базовая линия не меняются, шрифт подбирается по покрытию.',
    modes: ['print-layout'],
    notes: 'Пункт 18 приоритетного состава (CJK).',
  },
  {
    path: 'docx/rtl/mixed_direction.docx',
    title: 'Смешанное направление: RTL и LTR в одном абзаце',
    tags: ['text', 'fonts', 'bidi'],
    matrixRows: ['docx.bidi-cjk'],
    expected:
      'Арабский и английский в одном абзаце: bidi разбивает текст на уровни, латинский фрагмент сохраняет LTR-порядок.',
    modes: ['print-layout'],
    notes: 'Пункт 18 приоритетного состава (RTL).',
  },
  {
    path: 'xlsx/layout-row-heights.xlsx',
    title: 'Высоты строк: ручные и автоматические',
    tags: ['geometry'],
    matrixRows: ['xlsx.dimensions'],
    expected:
      'Пять строк с customHeight 10/15/20/40/80 px: высота каждой совпадает с заданной, automatic height не перебивает ручную.',
    modes: ['normal'],
    notes: 'Пункт 3 приоритетного состава (automatic row heights).',
  },
  {
    path: 'xlsx/layout-hidden-rows.xlsx',
    title: 'Скрытые строки',
    tags: ['geometry'],
    matrixRows: ['xlsx.hidden-outlines'],
    expected:
      'Скрытые строки не занимают места в раскладке и не создают пустых разрывов между видимыми.',
    modes: ['normal'],
    notes: 'Пункт 5 приоритетного состава (скрытые строки).',
  },
  {
    path: 'xlsx/formats-currency-ruble.xlsx',
    title: 'Числовой формат: валюта, рубли',
    tags: ['formats'],
    matrixRows: ['xlsx.number-formats'],
    expected:
      'Рублёвый формат показывает символ валюты в позиции из numFmt, отрицательные значения по своей секции; `#####` появляется только при нехватке ширины.',
    modes: ['normal'],
    notes: 'Пункт 6 приоритетного состава (валюты).',
  },
  {
    path: 'xlsx/cf-data-bar.xlsx',
    title: 'Условное форматирование: data bars',
    tags: ['conditional', 'styles'],
    matrixRows: ['xlsx.conditional'],
    expected:
      'Шесть правил dataBar в двух диапазонах: полоса заполнения пропорциональна значению, min/max из правила, цвет из правила.',
    modes: ['normal'],
    notes: 'Пункт 9 приоритетного состава (data bars).',
  },
  {
    path: 'xlsx/cf-icon-set.xlsx',
    title: 'Условное форматирование: icon sets',
    tags: ['conditional', 'styles'],
    matrixRows: ['xlsx.conditional'],
    expected:
      'Двенадцать iconSet-правил: значок выбирается порогом, набор иконок и его поворот берутся из правила.',
    modes: ['normal'],
    notes: 'Пункт 9 приоритетного состава (icon sets).',
  },
];

export default fixtures;
