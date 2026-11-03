// Прозрачный DOM-слой поверх canvas: выделение текста и ARIA-семантика.
//
// Геометрия приходит из Rust (`TextLayerWire`): здесь ничего не измеряется и не
// пересчитывается. Слой живёт в layout-координатах и масштабируется transform'ом
// вместе с зумом, поэтому scroll не требует пересборки spans, а виртуализация
// ограничена страницами окна ±1.
//
// Визуальные spans строятся по строке, а не по фрагменту: canvas рисует строку
// целиком, и браузер шейпит её целиком; пофрагментные spans разорвали бы
// арабское соединение и разъехались бы с картинкой. Фрагменты остаются в
// семантическом дереве и в данных для будущего выделения (Sprint 4).

import type { PageInfo, TextLayer, TextRun } from '../protocol.js';

/** Сколько страниц держим в кэше: окно ±1 плюс запас на быстрый скролл. */
const CACHE_PAGES = 8;

interface LineData {
  readonly key: string;
  readonly page: number;
  readonly cells: TextRun[];
  readonly text: string;
  readonly first: TextRun;
}

interface RenderedLine {
  readonly data: LineData;
  readonly span: HTMLSpanElement;
}

/** Зависимости слоя: откуда брать страницы и что подписать у документа. */
export interface TextLayerDeps {
  /** TextLayer одной страницы из воркера. */
  fetchPage: (page: number) => Promise<TextLayer>;
  /** Полный TextLayer — для семантического дерева и Ctrl+A. */
  fetchAll: () => Promise<TextLayer>;
  /** Подпись `role=document`; не задана — документ без имени. */
  label?: string;
  /**
   * Клик по фрагменту визуального слоя — источник правки текста: вьюер
   * передаёт его странице, та адресует run по `paragraphIndex`/`runIndex`.
   * Не задан — spans остаются только выделяемым текстом.
   */
  onRunClick?: (run: TextRun) => void;
}

/** Управление слоем; создаётся фабрикой, как и вьюеры. */
export interface TextLayerController {
  /** Корневой элемент слоя — его кладёт в scroller создатель вьюера. */
  readonly element: HTMLDivElement;
  /** Размеры страниц и стека после открытия документа. */
  configure(pages: PageInfo[], pageTops: number[]): void;
  /** Зум и поле слева: только transform, spans не пересобираются. */
  setTransform(padLeft: number, zoom: number, width: number, height: number): void;
  /** Обновить окно видимых страниц; зовётся из общего rAF вьюера. */
  updateWindow(scrollTop: number, viewportHeight: number, zoom: number): void;
  /** Убрать слой и слушатели. */
  destroy(): void;
}

/**
 * Создать слой. `a11y` решает вьюер: без флага слой не создаётся вовсе.
 */
export function createTextLayer(deps: TextLayerDeps): TextLayerController {
  const element = document.createElement('div');
  element.className = 'dc-text-layer';
  element.setAttribute('role', 'document');
  element.tabIndex = 0;
  if (deps.label !== undefined) element.setAttribute('aria-label', deps.label);
  element.style.cssText =
    'position:absolute;left:0;top:0;transform-origin:0 0;pointer-events:none;' +
    'user-select:text;z-index:1;color:transparent;';

  const semantic = document.createElement('div');
  semantic.className = 'dc-text-semantic';
  semantic.style.cssText =
    'position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;' +
    'clip:rect(0 0 0 0);clip-path:inset(50%);white-space:nowrap;border:0;' +
    'user-select:none;pointer-events:none;';
  const visual = document.createElement('div');
  visual.className = 'dc-text-visual';
  visual.setAttribute('aria-hidden', 'true');
  visual.style.cssText = 'position:absolute;left:0;top:0;pointer-events:none;';
  element.append(semantic, visual);

  let pages: PageInfo[] = [];
  let pageTops: number[] = [];
  let padLeft = 0;
  let zoom = 1;
  let stackWidth = 0;
  let stackHeight = 0;
  let destroyed = false;
  let fullText = '';
  let allSelected = false;

  const cache = new Map<number, TextLayer>();
  const pending = new Map<number, Promise<TextLayer>>();
  const pageNodes = new Map<number, HTMLDivElement>();
  const rendered = new Map<string, RenderedLine>();
  /** Страницы с начатым монтированием: окно может позвать их ещё раз до ответа. */
  const mounting = new Set<number>();
  let firstVisible = -1;
  let lastVisible = -1;
  /** Поколение документа: ответы старого открытия не должны рисоваться. */
  let generation = 0;

  const fetchPage = (page: number): Promise<TextLayer> => {
    const cached = cache.get(page);
    if (cached) {
      // LRU: освежаем порядок вставки.
      cache.delete(page);
      cache.set(page, cached);
      return Promise.resolve(cached);
    }
    const inFlight = pending.get(page);
    if (inFlight) return inFlight;
    const request = deps
      .fetchPage(page)
      .then((layer) => {
        pending.delete(page);
        cache.set(page, layer);
        while (cache.size > CACHE_PAGES) {
          const oldest = cache.keys().next().value;
          if (oldest === undefined) break;
          cache.delete(oldest);
        }
        return layer;
      })
      .catch((error: unknown) => {
        pending.delete(page);
        throw error;
      });
    pending.set(page, request);
    return request;
  };

  const setTransform = (nextPadLeft: number, nextZoom: number): void => {
    padLeft = nextPadLeft;
    zoom = nextZoom;
    element.style.width = `${stackWidth}px`;
    element.style.height = `${stackHeight}px`;
    element.style.transform = `translate(${padLeft}px, 0) scale(${zoom})`;
  };

  const pageAt = (layoutY: number): number => {
    let index = 0;
    for (let i = 1; i < pageTops.length; i += 1) {
      const top = pageTops[i];
      if (top === undefined || top > layoutY) break;
      index = i;
    }
    return index;
  };

  const updateWindow = (scrollTop: number, viewportHeight: number, nextZoom: number): void => {
    if (destroyed || pageTops.length === 0) return;
    zoom = nextZoom;
    const top = scrollTop / zoom;
    const bottom = (scrollTop + viewportHeight) / zoom;
    const first = Math.max(0, pageAt(top) - 1);
    const last = Math.min(pages.length - 1, pageAt(bottom) + 1);
    if (first === firstVisible && last === lastVisible) return;
    firstVisible = first;
    lastVisible = last;
    for (let page = first; page <= last; page += 1) {
      void mountPage(page);
    }
    for (const page of pageNodes.keys()) {
      if (page < first || page > last) unmountPage(page);
    }
  };

  const mountPage = async (page: number): Promise<void> => {
    if (destroyed) return;
    // Окно зовёт монтирование каждой страницы из [first..last] на каждом
    // изменении видимой области, включая зумы и появление полос прокрутки.
    // Повторное монтирование уже нарисованной (или ещё грузящейся) страницы
    // пересобрало бы её spans: нативное выделение пользователя привязано к
    // узлам текста и молча схлопнулось бы прямо во время drag через страницы.
    if (pageNodes.has(page) || mounting.has(page)) return;
    mounting.add(page);
    try {
      const current = generation;
      // Ошибка страницы не роняет слой: следующая попытка будет при новом окне.
      const layer = await fetchPage(page).catch(() => null);
      if (layer === null) return;
      // Пока грузилась страница, открыли другой документ — её ответ устарел.
      if (destroyed || current !== generation) return;
      if (page < firstVisible || page > lastVisible) return;
      renderPage(page, layer);
    } finally {
      mounting.delete(page);
    }
  };

  const unmountPage = (page: number): void => {
    const node = pageNodes.get(page);
    if (!node) return;
    for (const key of [...rendered.keys()]) {
      if (rendered.get(key)?.data.page === page) rendered.delete(key);
    }
    node.remove();
    pageNodes.delete(page);
  };

  const renderPage = (page: number, layer: TextLayer): void => {
    const info = pages[page];
    if (!info) return;
    let node = pageNodes.get(page);
    if (!node) {
      node = document.createElement('div');
      node.className = 'dc-text-page';
      node.dataset.page = String(page);
      node.style.cssText =
        `position:absolute;left:0;top:${pageTops[page] ?? 0}px;` +
        `width:${info.width}px;height:${info.height}px;pointer-events:none;`;
      visual.append(node);
      pageNodes.set(page, node);
    }
    // Перерисовка страницы: старые spans убираем, чтобы не плодить дубли.
    for (const key of [...rendered.keys()]) {
      if (rendered.get(key)?.data.page === page) {
        rendered.get(key)?.span.remove();
        rendered.delete(key);
      }
    }
    node.replaceChildren();
    for (const line of linesOf(layer)) {
      const span = lineSpan(line);
      node.append(span);
      rendered.set(line.key, { data: line, span });
    }
  };

  const lineSpan = (line: LineData): HTMLSpanElement => {
    const span = document.createElement('span');
    span.className = 'dc-text-line';
    span.dataset.line = line.key;
    span.textContent = line.text;
    const rect = line.first.lineRect;
    const style = line.first.style;
    span.dir = line.first.direction;
    span.style.cssText =
      `position:absolute;left:${rect.x}px;top:${rect.y}px;` +
      `width:${rect.width}px;height:${rect.height}px;` +
      `font-family:${cssFamily(style.fontFamily)};font-size:${style.sizePx}px;` +
      `font-weight:${style.bold ? 700 : 400};` +
      `font-style:${style.italic ? 'italic' : 'normal'};` +
      `text-decoration:${style.underline ? 'underline' : 'none'};` +
      `text-align:${line.first.textAlign};line-height:${rect.height}px;` +
      'white-space:pre;color:transparent;cursor:text;pointer-events:auto;user-select:text;';
    if (deps.onRunClick !== undefined) {
      // Клик идёт во фрагмент по координате в координатах раскладки: `bbox`
      // фрагмента сравнивается с левым краем строки, и зум снимается делением.
      span.addEventListener('click', (event) => {
        const bounds = span.getBoundingClientRect();
        const x = (event.clientX - bounds.left) / zoom;
        const origin = line.first.lineRect;
        const hit =
          line.cells.find(
            (run) =>
              run.bbox.x - origin.x <= x && x <= run.bbox.x - origin.x + run.bbox.width,
          ) ?? line.first;
        if (hit.paragraphIndex !== null && hit.runIndex !== null) {
          deps.onRunClick?.(hit);
        }
      });
    }
    return span;
  };

  const configure = (nextPages: PageInfo[], nextTops: number[]): void => {
    // Новый документ в том же вьюере: старые слои и ответы не должны
    // пережить открытие.
    generation += 1;
    cache.clear();
    pending.clear();
    for (const node of pageNodes.values()) node.remove();
    pageNodes.clear();
    rendered.clear();
    mounting.clear();
    firstVisible = -1;
    lastVisible = -1;
    fullText = '';
    semantic.replaceChildren();

    pages = nextPages;
    pageTops = nextTops;
    stackWidth = Math.max(0, ...nextPages.map((page) => page.width));
    stackHeight = nextTops.length === 0
      ? 0
      : (nextTops[nextTops.length - 1] ?? 0) + (nextPages[nextPages.length - 1]?.height ?? 0);
    setTransform(padLeft, zoom);
    // Семантическое дерево строится один раз после открытия и не зависит от окна.
    void buildSemantic(generation);
  };

  const buildSemantic = async (current: number): Promise<void> => {
    let layer: TextLayer;
    try {
      layer = await deps.fetchAll();
    } catch {
      return;
    }
    if (destroyed || current !== generation) return;
    const built = semanticTree(layer);
    semantic.replaceChildren(built.element);
    fullText = built.text;
  };

  const copy = (event: Event): void => {
    // FireFox не заполняет `clipboardData` у синтетического ClipboardEvent,
    // поэтому берём его структурно: у настоящего copy-события поле есть.
    const clipboard =
      (event as ClipboardEvent).clipboardData ??
      (event as Event & { dataTransfer?: DataTransfer }).dataTransfer;
    if (!clipboard) return;
    const text = allSelected ? fullText : selectedText();
    if (text === '') return;
    clipboard.setData('text/plain', text);
    event.preventDefault();
    allSelected = false;
  };

  const selectedText = (): string => {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return '';
    const range = selection.getRangeAt(0);
    const selected: RenderedLine[] = [];
    for (const line of rendered.values()) {
      if (range.intersectsNode(line.span)) selected.push(line);
    }
    selected.sort((left, right) => left.data.first.readingOrder - right.data.first.readingOrder);
    return serialize(selected.map((line) => line.data));
  };

  const onKeyDown = (event: KeyboardEvent): void => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a') {
      event.preventDefault();
      allSelected = true;
      // Нативное выделение не трогаем: визуальный слой виртуализирован, и
      // Ctrl+A должен отдавать весь документ, а не окно страниц.
    }
  };

  const onCopy = (event: Event): void => {
    copy(event);
  };

  const onMouseDown = (): void => {
    allSelected = false;
    element.focus({ preventScroll: true });
  };

  element.addEventListener('keydown', onKeyDown);
  element.addEventListener('copy', onCopy);
  element.addEventListener('mousedown', onMouseDown);

  const destroy = (): void => {
    destroyed = true;
    element.removeEventListener('keydown', onKeyDown);
    element.removeEventListener('copy', onCopy);
    element.removeEventListener('mousedown', onMouseDown);
    element.remove();
    cache.clear();
    pending.clear();
    pageNodes.clear();
    rendered.clear();
    mounting.clear();
  };

  return {
    element,
    configure,
    setTransform: (nextPadLeft, nextZoom, width, height) => {
      stackWidth = width;
      stackHeight = height;
      setTransform(nextPadLeft, nextZoom);
    },
    updateWindow,
    destroy,
  };
}

/** Семейство шрифта для span'а; без имени — то же, что у canvas. */
function cssFamily(family: string | null): string {
  const value = family ?? 'Carlito';
  return `'${value.replace(/'/g, "\\'")}', sans-serif`;
}

/** Runs страницы, сгруппированные по строкам, в порядке чтения. */
function linesOf(layer: TextLayer): LineData[] {
  const lines = new Map<string, TextRun[]>();
  for (const run of layer.runs) {
    const key = `${run.page}:${run.lineIndex}`;
    const runs = lines.get(key);
    if (runs) runs.push(run);
    else lines.set(key, [run]);
  }
  const result: LineData[] = [];
  for (const [key, runs] of lines) {
    const first = runs[0];
    if (!first) continue;
    result.push({
      key,
      page: first.page,
      cells: runs,
      text: runs.map((run) => run.text).join(''),
      first,
    });
  }
  result.sort((left, right) => left.first.readingOrder - right.first.readingOrder);
  return result;
}

/** Сериализовать выбранные строки таблицы: TSV по логическим row/col. */
function serializeTable(lines: LineData[]): string {
  // tableId → row → col → части текста ячейки.
  const tables = new Map<number, Map<number, Map<number, string[]>>>();
  for (const line of lines) {
    const table = line.first.table;
    if (!table) continue;
    let rows = tables.get(table.tableId);
    if (!rows) {
      rows = new Map();
      tables.set(table.tableId, rows);
    }
    let cells = rows.get(table.row);
    if (!cells) {
      cells = new Map();
      rows.set(table.row, cells);
    }
    const parts = cells.get(table.col);
    if (parts) parts.push(line.text);
    else cells.set(table.col, [line.text]);
  }
  const result: string[] = [];
  for (const rows of tables.values()) {
    const orderedRows = [...rows.entries()].sort(([left], [right]) => left - right);
    for (const [, cells] of orderedRows) {
      const ordered = [...cells.entries()].sort(([left], [right]) => left - right);
      result.push(ordered.map(([, parts]) => parts.join('').trim()).join('\t'));
    }
  }
  return result.filter((row) => row !== '').join('\n');
}

/**
 * Сериализовать выбранные строки: таблица — TSV, абзацы — строки через
 * перевод строки, строки одного абзаца — через пробел.
 */
function serialize(lines: LineData[]): string {
  if (lines.length > 0 && lines.every((line) => line.first.table != null)) {
    return serializeTable(lines);
  }
  const chunks: string[] = [];
  let blockKey: string | null = null;
  let block: string[] = [];
  const flush = (): void => {
    if (block.length > 0) chunks.push(block.join(' ').trim());
    block = [];
  };
  for (const line of lines) {
    const key = `block:${line.first.nodeId}`;
    if (key !== blockKey) {
      flush();
      blockKey = key;
    }
    block.push(line.text);
  }
  flush();
  return chunks.filter((chunk) => chunk !== '').join('\n');
}

/** Семантическое дерево и полный текст для Ctrl+A. */
function semanticTree(layer: TextLayer): { element: DocumentFragment; text: string } {
  const fragment = document.createDocumentFragment();
  const ordering = [...layer.runs].sort((a, b) => a.readingOrder - b.readingOrder);
  const tables = new Map<number, HTMLDivElement>();
  const rows = new Map<string, HTMLDivElement>();
  const cells = new Map<string, HTMLDivElement>();
  const paragraphs = new Map<number, { element: HTMLElement; lines: Map<string, string[]> }>();
  const ordered: (
    | { kind: 'paragraph'; nodeId: number }
    | { kind: 'table'; tableId: number }
  )[] = [];
  const seenHeaderRows = new Set<string>();

  for (const run of ordering) {
    if (run.text.trim() === '') continue;
    if (run.table) {
      const table = run.table;
      if (table.isRepeatedHeader) {
        const headerKey = `${table.tableId}:${table.row}`;
        if (seenHeaderRows.has(headerKey)) continue;
        seenHeaderRows.add(headerKey);
      }
      let tableElement = tables.get(table.tableId);
      if (!tableElement) {
        tableElement = document.createElement('div');
        tableElement.setAttribute('role', 'table');
        fragment.append(tableElement);
        tables.set(table.tableId, tableElement);
        ordered.push({ kind: 'table', tableId: table.tableId });
      }
      const rowKey = `${table.tableId}:${table.row}`;
      let rowElement = rows.get(rowKey);
      if (!rowElement) {
        rowElement = document.createElement('div');
        rowElement.setAttribute('role', 'row');
        rowElement.setAttribute('aria-rowindex', String(table.row + 1));
        tableElement.append(rowElement);
        rows.set(rowKey, rowElement);
      }
      const cellKey = `${rowKey}:${table.col}`;
      let cellElement = cells.get(cellKey);
      if (!cellElement) {
        cellElement = document.createElement('div');
        cellElement.setAttribute('role', table.isHeader ? 'columnheader' : 'cell');
        cellElement.setAttribute('aria-colindex', String(table.col + 1));
        if (table.colSpan > 1) cellElement.setAttribute('aria-colspan', String(table.colSpan));
        if (table.rowSpan > 1) cellElement.setAttribute('aria-rowspan', String(table.rowSpan));
        rowElement.append(cellElement);
        cells.set(cellKey, cellElement);
      }
      cellElement.append(document.createTextNode(run.text));
      continue;
    }
    let paragraph = paragraphs.get(run.nodeId);
    if (!paragraph) {
      const element = document.createElement('div');
      if (run.blockKind === 'heading') {
        element.setAttribute('role', 'heading');
        element.setAttribute('aria-level', String(run.headingLevel ?? 2));
      } else {
        element.setAttribute('role', 'paragraph');
      }
      fragment.append(element);
      paragraph = { element, lines: new Map() };
      paragraphs.set(run.nodeId, paragraph);
      ordered.push({ kind: 'paragraph', nodeId: run.nodeId });
    }
    const lineKey = `${run.page}:${run.lineIndex}`;
    const line = paragraph.lines.get(lineKey);
    if (line) line.push(run.text);
    else paragraph.lines.set(lineKey, [run.text]);
  }

  // Текст абзацев: строки одного абзаца склеиваем пробелом, абзацы — переводом.
  for (const paragraph of paragraphs.values()) {
    paragraph.element.textContent = [...paragraph.lines.values()]
      .map((parts) => parts.join(''))
      .join(' ')
      .trim();
  }
  // Полный текст — в порядке первого появления блоков, а не «сначала абзацы,
  // потом таблицы»: Ctrl+A обязан повторять документ.
  const textParts: string[] = [];
  for (const block of ordered) {
    if (block.kind === 'paragraph') {
      const text = paragraphs.get(block.nodeId)?.element.textContent?.trim() ?? '';
      if (text !== '') textParts.push(text);
    } else {
      const tableElement = tables.get(block.tableId);
      if (!tableElement) continue;
      for (const row of tableElement.children) {
        const cellsText = [...row.children]
          .map((cell) => cell.textContent?.trim() ?? '')
          .join('\t');
        if (cellsText !== '') textParts.push(cellsText);
      }
    }
  }
  return { element: fragment, text: textParts.join('\n') };
}
