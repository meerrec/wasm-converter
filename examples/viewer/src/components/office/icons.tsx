import type { ReactElement, ReactNode } from 'react';

/**
 * Имена глифов ленты.
 *
 * Список закрыт, а таблица `GLYPHS` ниже типизирована как `Record<IconName, ReactNode>`:
 * полноту набора проверяет компилятор — пропущенный глиф не превратится в пустое
 * место на кнопке, а сломает typecheck.
 */
export type IconName =
  | 'paste'
  | 'cut'
  | 'copy'
  | 'format-painter'
  | 'font'
  | 'font-size'
  | 'bold'
  | 'italic'
  | 'underline'
  | 'strikethrough'
  | 'subscript'
  | 'superscript'
  | 'text-color'
  | 'highlight'
  | 'bullets'
  | 'numbering'
  | 'multilevel-list'
  | 'indent-decrease'
  | 'indent-increase'
  | 'align-left'
  | 'align-center'
  | 'align-right'
  | 'align-justify'
  | 'line-spacing'
  | 'paragraph-shading'
  | 'borders'
  | 'styles'
  | 'find'
  | 'replace'
  | 'select'
  | 'undo'
  | 'redo'
  | 'save'
  | 'search'
  | 'print'
  | 'share'
  | 'grid'
  | 'headers'
  | 'zoom-in'
  | 'zoom-out'
  | 'sheet'
  | 'cell'
  | 'sum'
  | 'sort'
  | 'filter'
  | 'clear'
  | 'format'
  | 'table'
  | 'conditional-formatting'
  | 'chart'
  | 'picture'
  | 'link'
  | 'comment'
  | 'page-setup'
  | 'formulas'
  | 'data'
  | 'review'
  | 'view'
  | 'help'
  | 'file'
  | 'pdf'
  | 'markdown'
  | 'html'
  | 'text'
  | 'export-book'
  | 'check'
  | 'cancel'
  | 'function'
  | 'freeze'
  | 'wrap-text'
  | 'merge'
  | 'percent'
  | 'currency'
  | 'comma'
  | 'decimal-increase'
  | 'decimal-decrease'
  | 'chevron-down'
  | 'more'
  | 'paragraph-mark'
  | 'spell-check'
  | 'ruler'
  | 'account';

/**
 * Глифы: координаты 20×20, полезная область примерно 2..18.
 *
 * Все фигуры — контуры без заливки, толщина линии берётся с корневого `<svg>`;
 * `strokeWidth` в элементе стоит только там, где нужна «залитая» полоса
 * (подчёркивание, цвет текста, выделение) или мелкая деталь.
 */
const GLYPHS: Record<IconName, ReactNode> = {
  paste: (
    <>
      <path d="M8 3.5H6.6A1.6 1.6 0 0 0 5 5.1v9.8A1.6 1.6 0 0 0 6.6 16.5h6.8a1.6 1.6 0 0 0 1.6-1.6V5.1a1.6 1.6 0 0 0-1.6-1.6H12" />
      <rect x="7.4" y="2.5" width="5.2" height="3.4" rx="1" />
    </>
  ),
  cut: (
    <>
      <circle cx="6" cy="15.4" r="2.4" />
      <circle cx="14" cy="15.4" r="2.4" />
      <path d="M7.7 13.6 14.2 3.8M12.3 13.6 5.8 3.8" />
    </>
  ),
  copy: (
    <>
      <rect x="7" y="2.5" width="10" height="12" rx="1.5" />
      <path d="M13.5 14.5v1.5a1.5 1.5 0 0 1-1.5 1.5H4.5A1.5 1.5 0 0 1 3 16V6a1.5 1.5 0 0 1 1.5-1.5h2.5" />
    </>
  ),
  'format-painter': (
    <>
      <path d="M10.8 9.2 16.6 3.4" />
      <path d="M12 10.4 6.6 15.8a1.7 1.7 0 0 1-2.4-2.4L9.6 8z" />
    </>
  ),
  font: (
    <>
      <path d="M3.2 16 6.6 5.2 10 16M4.6 12.2h4" />
      <circle cx="14.4" cy="13.6" r="2.5" />
      <path d="M16.9 11.1v5" />
    </>
  ),
  'font-size': (
    <>
      <path d="M2.6 16 5.9 5.4 9.2 16M3.9 12.3h4" />
      <circle cx="12.2" cy="13.6" r="2.1" />
      <path d="M14.3 11.5v4.5" />
      <path d="M16.8 7.6v8.8M15.6 8.8 16.8 7.6l1.2 1.2M15.6 15.2 16.8 16.4l1.2-1.2" />
    </>
  ),
  bold: (
    <>
      <path d="M7 3.5v13" />
      <path d="M7 3.5h3.8a3.25 3.25 0 0 1 0 6.5H7" />
      <path d="M7 10h4.3a3.25 3.25 0 0 1 0 6.5H7" />
    </>
  ),
  italic: (
    <>
      <path d="M12.4 3.5 8.2 16.5" />
      <path d="M9.6 3.5h4.6M5.6 16.5h4.6" />
    </>
  ),
  underline: (
    <>
      <path d="M5.6 3.5v6.6a4.4 4.4 0 0 0 8.8 0V3.5" />
      <path d="M4 17.2h12" strokeWidth={2.5} />
    </>
  ),
  strikethrough: (
    <>
      <path d="M12.8 6.5a3.2 3.2 0 0 0-2.8-3h-.5a3 3 0 0 0-1.2 5.7l3 1.3a3 3 0 0 1-1.2 5.7h-.5a3.2 3.2 0 0 1-2.8-3" />
      <path d="M3.4 10.2h13.2" strokeWidth={2.5} />
    </>
  ),
  subscript: (
    <>
      <path d="M3.5 4 10.5 12M10.5 4 3.5 12" />
      <path d="M13.4 12.9a1.7 1.7 0 1 1 2.9 1.3c-.4.5-2.9 1.4-2.9 3.2h3.1" />
    </>
  ),
  superscript: (
    <>
      <path d="M3.5 8.5 10.5 16.5M10.5 8.5 3.5 16.5" />
      <path d="M13.4 2.9a1.7 1.7 0 1 1 2.9 1.3c-.4.5-2.9 1.4-2.9 3.2h3.1" />
    </>
  ),
  'text-color': (
    <>
      <path d="M3.5 13.6 7.5 3.4 11.5 13.6M5 10.1h5" />
      <path d="M3.5 16.6h13" strokeWidth={2.5} />
    </>
  ),
  highlight: (
    <>
      <path d="M11.9 5.1 14.9 8.1 9.4 13.6 6.4 10.6z" />
      <path d="M8.6 8.4 11.6 11.4" />
      <path d="M4.5 17.2h11" strokeWidth={2.5} />
    </>
  ),
  bullets: (
    <>
      <path d="M8 5.5h9M8 10h9M8 14.5h9" />
      {/* `h.01` со скруглённым концом рисует точку: заливка фигур не нужна. */}
      <path d="M4.4 5.5h.01M4.4 10h.01M4.4 14.5h.01" strokeWidth={2.5} />
    </>
  ),
  numbering: (
    <>
      <path d="M8.5 5.5h8M8.5 10.5h8M8.5 15h8" />
      <path d="M3.4 5.1 4.8 4.2v3.4" strokeWidth={1.2} />
      <path d="M3.1 9.3a1.5 1.5 0 1 1 2.5 1.2c-.6.6-2.5 1.3-2.5 2.6h2.6" strokeWidth={1.2} />
      <path d="M3.2 14.1a1.4 1.4 0 1 1 1.9 1.9 1.4 1.4 0 1 1-1.9 1.9" strokeWidth={1.2} />
    </>
  ),
  'multilevel-list': (
    <>
      <path d="M7.5 5h9M10.5 10h6M13.5 15h3" />
      <path d="M3.6 5h.01M6.6 10h.01M9.6 15h.01" strokeWidth={2.3} />
    </>
  ),
  'indent-decrease': (
    <>
      <path d="M11 4.5h6.6M11 9.5h6.6M11 14.5h6.6" />
      <path d="M8.6 6.8 5.8 9.5l2.8 2.7" />
    </>
  ),
  'indent-increase': (
    <>
      <path d="M11 4.5h6.6M11 9.5h6.6M11 14.5h6.6" />
      <path d="M5.8 6.8 8.6 9.5l-2.8 2.7" />
    </>
  ),
  'align-left': <path d="M3 4h14M3 8h9M3 12h14M3 16h9" />,
  'align-center': <path d="M3 4h14M5.5 8h9M3 12h14M5.5 16h9" />,
  'align-right': <path d="M3 4h14M8 8h9M3 12h14M8 16h9" />,
  'align-justify': <path d="M3 4h14M3 8h14M3 12h14M3 16h14" />,
  'line-spacing': (
    <>
      <path d="M3 5h9M3 10h9M3 15h9" />
      <path d="M15.5 4.5v11M13.7 6.3 15.5 4.5l1.8 1.8M13.7 13.7 15.5 15.5l1.8-1.8" />
    </>
  ),
  'paragraph-shading': (
    <>
      <path d="M4 5.5h12M4 14.5h7" />
      <path d="M4 10h12" strokeWidth={2.5} />
    </>
  ),
  borders: (
    <>
      <rect x="3.5" y="3.5" width="13" height="13" rx="1.5" strokeWidth={2.5} />
      <path d="M3.5 10h13M10 3.5v13" />
    </>
  ),
  styles: (
    <>
      <rect x="2.5" y="3.5" width="15" height="13" rx="1.5" />
      <path d="M4.6 13.5 7 5.6l2.4 7.9M5.4 11.2h3.3" />
      <circle cx="13.6" cy="11" r="2.1" />
      <path d="M15.7 8.9v4.6" />
    </>
  ),
  find: (
    <>
      <circle cx="8.8" cy="8.8" r="5.3" />
      <path d="M12.6 12.6 17 17" />
    </>
  ),
  replace: (
    <>
      <circle cx="7.6" cy="7.6" r="4.4" />
      <path d="M10.8 10.8 13.4 13.4" />
      <path d="M6.5 16.6h9M13.3 14.5 15.5 16.6l-2.2 2.1" />
    </>
  ),
  select: (
    <>
      <rect x="3" y="4" width="14" height="12" rx="1.5" strokeDasharray="2.6 2.2" />
      <path d="M8.6 8.3v7.2l1.9-1.8 1.2 2.5 1.5-.7-1.2-2.4 2.5-.3z" />
    </>
  ),
  undo: (
    <>
      <path d="M17 16.6v-3.3a5.2 5.2 0 0 0-5.2-5.2H7.7" />
      <path d="M7.6 5.7 4.6 8.1l3 2.4" />
    </>
  ),
  redo: (
    <>
      <path d="M3 16.6v-3.3a5.2 5.2 0 0 1 5.2-5.2h4.1" />
      <path d="M12.4 5.7 15.4 8.1l-3 2.4" />
    </>
  ),
  save: (
    <>
      <path d="M3.5 5.5a2 2 0 0 1 2-2h7.1a2 2 0 0 1 1.4.6l1.9 1.9a2 2 0 0 1 .6 1.4v7.1a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2z" />
      <path d="M7 3.5v3.8h5V3.5" />
      <rect x="6.8" y="11" width="6.4" height="5" rx="1" />
    </>
  ),
  search: (
    <>
      <circle cx="8.8" cy="8.8" r="5.3" />
      <path d="M12.6 12.6 17 17" />
    </>
  ),
  print: (
    <>
      <path d="M6.5 8V3.5h7V8" />
      <path d="M6.5 15.5h-2A1.5 1.5 0 0 1 3 14v-4a1.5 1.5 0 0 1 1.5-1.5h11A1.5 1.5 0 0 1 17 10v4a1.5 1.5 0 0 1-1.5 1.5h-2" />
      <path d="M6.5 12.5h7v5h-7z" />
    </>
  ),
  share: (
    <>
      <circle cx="15" cy="4.4" r="2.3" />
      <circle cx="5" cy="10" r="2.3" />
      <circle cx="15" cy="15.6" r="2.3" />
      <path d="M7 8.8 12.9 5.6M7 11.2l5.9 3.2" />
    </>
  ),
  grid: (
    <>
      <rect x="3" y="4" width="14" height="12" rx="1.2" />
      <path d="M3 10h14M10 4v12" />
    </>
  ),
  headers: (
    <>
      <rect x="3" y="4" width="14" height="12" rx="1.2" />
      <path d="M3 8.4h14M9.5 8.4v7.6" />
      <path d="M3.7 6.2h12.6" strokeWidth={2.5} />
    </>
  ),
  'zoom-in': (
    <>
      <circle cx="8.6" cy="8.6" r="5.4" />
      <path d="M12.5 12.5 17 17" />
      <path d="M8.6 5.9v5.4M5.9 8.6h5.4" />
    </>
  ),
  'zoom-out': (
    <>
      <circle cx="8.6" cy="8.6" r="5.4" />
      <path d="M12.5 12.5 17 17" />
      <path d="M5.9 8.6h5.4" />
    </>
  ),
  sheet: (
    <>
      <path d="M3.5 5.3A1.8 1.8 0 0 1 5.3 3.5h5.3l6 6v6.7a1.8 1.8 0 0 1-1.8 1.8H5.3a1.8 1.8 0 0 1-1.8-1.8z" />
      <path d="M10.6 3.5v3.8a1.8 1.8 0 0 0 1.8 1.8H16.6" />
      <path d="M3.5 11.4h8.2M3.5 14.4h5.6" />
    </>
  ),
  cell: (
    <>
      <rect x="3" y="3" width="14" height="14" rx="2" />
      <rect x="6.5" y="6.5" width="7" height="7" rx="1" strokeWidth={2.5} />
    </>
  ),
  sum: <path d="M14.6 4H6l4.5 6L6 16h8.6" />,
  sort: (
    <>
      <path d="M6.5 16.4V4.2M3.9 6.8 6.5 4.2l2.6 2.6" />
      <path d="M13.5 3.6v12.2M10.9 13.2 13.5 15.8l2.6-2.6" />
    </>
  ),
  filter: <path d="M3.5 4.5h13l-5 6v5.4l-3-1.5v-3.9z" />,
  clear: (
    <>
      <path d="M11.15 7.45 6.85 11.75a1.2 1.2 0 0 0 0 1.7L7.55 14.15a1.2 1.2 0 0 0 1.7 0L13.55 9.85a1.2 1.2 0 0 0 0-1.7L12.85 7.45a1.2 1.2 0 0 0-1.7 0z" />
      <path d="M13.3 7.9 7.3 13.9" />
    </>
  ),
  format: (
    <>
      <path d="M9.9 2.6 11.4 7.1 15.9 8.6 11.4 10.1 9.9 14.6 8.4 10.1 3.9 8.6 8.4 7.1z" />
      <path d="M15.6 13 16.3 15.1 18.4 15.8 16.3 16.5 15.6 18.6 14.9 16.5 12.8 15.8 14.9 15.1z" />
    </>
  ),
  table: (
    <>
      <rect x="3" y="4" width="14" height="12" rx="1.2" />
      <path d="M3 8.2h14M3 12.4h14M10 4v12" />
    </>
  ),
  'conditional-formatting': (
    <>
      <rect x="3.5" y="3.5" width="13" height="13" rx="1.2" />
      <path d="M3.5 10h13M10 3.5v13" />
      <path d="M4.6 9.2 9.2 4.6M7.4 9.2 9.2 7.4" />
    </>
  ),
  chart: (
    <>
      <path d="M3.4 3.2v13.6h13.4" />
      <path d="M7.2 16.8V10.4M10.4 16.8v-5.6M13.6 16.8V7.6" />
    </>
  ),
  picture: (
    <>
      <rect x="3" y="3.5" width="14" height="13" rx="1.5" />
      <circle cx="7.4" cy="7.6" r="1.3" />
      <path d="M3.4 14.6 8 10a1 1 0 0 1 1.4 0l3.1 3.1" />
      <path d="M10.6 14.9 13.2 12.3a1 1 0 0 1 1.4 0l1.9 1.9" />
    </>
  ),
  link: (
    <>
      <path d="M7.6 10h4.8" />
      <path d="M7.6 6.6H6.1a3.4 3.4 0 0 0 0 6.8h1.5" />
      <path d="M12.4 6.6h1.5a3.4 3.4 0 0 1 0 6.8h-1.5" />
    </>
  ),
  comment: (
    <>
      <path d="M3.5 5.8A2.3 2.3 0 0 1 5.8 3.5h8.4a2.3 2.3 0 0 1 2.3 2.3v5.4a2.3 2.3 0 0 1-2.3 2.3H8.7l-3.9 3.1a.6.6 0 0 1-1-.5z" />
    </>
  ),
  'page-setup': (
    <>
      <rect x="5.5" y="5.5" width="11.5" height="12" rx="1.5" />
      <path d="M2.6 8.2 3.7 7.1l1.1 1.1M2.6 14.8 3.7 15.9l1.1-1.1M3.7 7.4v8.2" />
    </>
  ),
  formulas: (
    <>
      <path d="M5.6 3.2h-.5a1.3 1.3 0 0 0-1.3 1.3v3M3.3 5h2.3" />
      <path d="M7 3.6 9.4 6.6M9.4 3.6 7 6.6" />
      <rect x="8" y="9.5" width="9" height="7.5" rx="1" />
      <path d="M8 12.8h9M11.7 9.5v7.5" />
    </>
  ),
  data: (
    <>
      <rect x="2.5" y="4" width="10" height="12" rx="1.2" />
      <path d="M2.5 8h10M7.5 8v8" />
      <path d="M15.5 7.5v8M13.4 13.4 15.5 15.5l2.1-2.1" />
    </>
  ),
  review: (
    <>
      <rect x="3.5" y="3.5" width="13" height="13" rx="2.5" />
      <path d="M6.8 10.2 9.2 12.6 13.8 7.4" />
    </>
  ),
  view: (
    <>
      <path d="M2.6 10c1.9-3.1 4.4-4.6 7.4-4.6s5.5 1.5 7.4 4.6c-1.9 3.1-4.4 4.6-7.4 4.6S4.5 13.1 2.6 10z" />
      <circle cx="10" cy="10" r="2.6" />
    </>
  ),
  help: (
    <>
      <circle cx="10" cy="10" r="7.3" />
      <path d="M7.8 7.7a2.2 2.2 0 0 1 4.2 1.2c0 1.5-2.1 1.9-2.1 3.2" />
      <path d="M10 15.1h.01" strokeWidth={2.2} />
    </>
  ),
  file: (
    <>
      <path d="M4 5.3A1.8 1.8 0 0 1 5.8 3.5h5.3l6 6v6.7a1.8 1.8 0 0 1-1.8 1.8H5.8A1.8 1.8 0 0 1 4 16.2z" />
      <path d="M11.1 3.5v3.8a1.8 1.8 0 0 0 1.8 1.8h3.9" />
    </>
  ),
  pdf: (
    <>
      <path d="M4 5.3A1.8 1.8 0 0 1 5.8 3.5h4.4l6.3 6.3v5.9a1.8 1.8 0 0 1-1.8 1.8H5.8A1.8 1.8 0 0 1 4 15.7z" />
      <path d="M10.2 3.5v3.4a1.8 1.8 0 0 0 1.8 1.8h4.4" />
      <path d="M7.7 15.8V9.4h1.8a1.8 1.8 0 0 1 0 3.6H7.7" />
    </>
  ),
  markdown: (
    <>
      <rect x="2.5" y="4.5" width="15" height="11" rx="1.8" />
      <path d="M5.4 12.6V7.2l2.6 3.2 2.6-3.2v5.4" />
      <path d="M14.2 7.6v4.6M12.6 10.4l1.6 1.7 1.6-1.7" />
    </>
  ),
  html: (
    <>
      <path d="M7 7 3.2 10 7 13" />
      <path d="M13 7l3.8 3-3.8 3" />
      <path d="M11.4 5.4 8.6 14.6" />
    </>
  ),
  text: (
    <>
      <rect x="4" y="3.5" width="12" height="13" rx="1.5" />
      <path d="M7 7.5h6M7 10.5h6M7 13.5h4" />
    </>
  ),
  'export-book': (
    <>
      <rect x="2.5" y="4" width="8.5" height="12" rx="1.2" />
      <path d="M6.5 4v12" />
      <path d="M14.5 7v7.2M12.4 12.1 14.5 14.2l2.1-2.1" />
    </>
  ),
  check: <path d="M4 10.5 8 14.5 16.4 5.4" />,
  cancel: (
    <>
      <circle cx="10" cy="10" r="7.3" />
      <path d="M7.3 7.3 12.7 12.7M12.7 7.3 7.3 12.7" />
    </>
  ),
  function: (
    <>
      <path d="M7.5 16.5c-1.2 0-1.8-.8-1.6-2l1.2-8.2c.2-1.4 1-2.2 2.3-2.2" />
      <path d="M5.4 9h4.2" />
      <path d="M11.5 9 16 15.5M15.6 9 11.1 15.5" />
    </>
  ),
  freeze: (
    <>
      <path d="M10 2.6v14.8M3.6 6.3l12.8 7.4M16.4 6.3 3.6 13.7" />
      <path d="M7.4 4.4 10 2.6l2.6 1.8M7.4 15.6 10 17.4l2.6-1.8" />
    </>
  ),
  'wrap-text': (
    <>
      <path d="M3.5 4.5h13" />
      <path d="M3.5 16h7.5a4.5 4.5 0 0 0 0-9H6.5" />
      <path d="M8.5 5.2 6.3 7l2.2 1.8" />
    </>
  ),
  merge: (
    <>
      <rect x="2.5" y="6.5" width="15" height="7.5" rx="1.2" />
      <path d="M4.6 10.2h3M6.4 8.6 8 10.2l-1.6 1.6M15.4 10.2h-3M13.6 8.6 12 10.2l1.6 1.6" />
    </>
  ),
  percent: (
    <>
      <path d="M16.4 3.6 3.6 16.4" />
      <circle cx="6.2" cy="6.2" r="2.4" />
      <circle cx="13.8" cy="13.8" r="2.4" />
    </>
  ),
  currency: (
    <>
      <path d="M7 17.5V3.5h3.6a3.6 3.6 0 0 1 0 7.2H7" />
      <path d="M4.6 13.6h5.6" />
    </>
  ),
  comma: (
    <>
      <path d="M10.2 8.2h.01" strokeWidth={2.5} />
      <path d="M10.9 9.1c.8 2.6 0 4.8-2.1 6.5a8.6 8.6 0 0 1-2.5 1.2" />
    </>
  ),
  'decimal-increase': (
    <>
      <ellipse cx="4.4" cy="7.6" rx="1.7" ry="2.3" />
      <ellipse cx="11.4" cy="7.6" rx="1.7" ry="2.3" />
      <path d="M6.6 9.6c.5.2.7.6.6 1s-.5.7-.9.7" />
      <path d="M8 16h8M13.8 13.9 15.9 16l-2.1 2.1" />
    </>
  ),
  'decimal-decrease': (
    <>
      <ellipse cx="4.4" cy="7.6" rx="1.7" ry="2.3" />
      <ellipse cx="11.4" cy="7.6" rx="1.7" ry="2.3" />
      <path d="M6.6 9.6c.5.2.7.6.6 1s-.5.7-.9.7" />
      <path d="M12 16H4M6.2 13.9 4.1 16l2.1 2.1" />
    </>
  ),
  'chevron-down': <path d="M5 7.8 10 12.8 15 7.8" />,
  more: <path d="M5 10h.01M10 10h.01M15 10h.01" strokeWidth={2.5} />,
  'paragraph-mark': (
    <>
      <path d="M12.4 4.8H8.6a3.4 3.4 0 0 0 0 6.8h3.8" />
      <path d="M12.4 4.8v12.4M8.2 11.6v5.6" />
    </>
  ),
  'spell-check': (
    <>
      <path d="M2.4 12.6 5.6 4.4l3.2 8.2M3.6 9.8h4" />
      <path d="M11.4 12.4 13.8 14.8 18.2 8.6" />
    </>
  ),
  ruler: (
    <>
      <rect x="2.5" y="7" width="15" height="6" rx="1.2" />
      <path d="M6 7v2.4M9 7v3.2M12 7v2.4M15 7v3.2" />
    </>
  ),
  account: (
    <>
      <circle cx="10" cy="10" r="7" />
      <circle cx="10" cy="8.2" r="2.4" />
      <path d="M5.5 16.1a4.9 4.9 0 0 1 9 0" />
    </>
  ),
};

/**
 * Иконка ленты: линейный глиф 20×20 в цвете текста.
 *
 * `size` — сторона квадрата в пикселях; глиф масштабируется вектором, поэтому
 * размер не влияет на резкость. Иконка декоративна: у неё `aria-hidden`,
 * подпись кнопке даёт текст рядом.
 */
export function Icon({ name, size }: { name: IconName; size?: number }): ReactElement {
  return (
    <svg
      viewBox="0 0 20 20"
      width={size ?? 20}
      height={size ?? 20}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {GLYPHS[name]}
    </svg>
  );
}
