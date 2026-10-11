// Мини-разбор PDF для сквозных проверок: страницы, их распакованное
// содержимое, операции и клипы.
//
// Полноценный парсер здесь не нужен: файл собирает наш же экспортёр, поэтому
// структура известна — страницы лежат отдельными объектами `/Type /Page`,
// ссылаются на поток `/Contents N 0 R`, поток сжат FlateDecode. Своей копии
// lopdf в Node нет, а `zlib` есть.
//
// Ту же структурную проверку на нативной стороне делает lopdf
// (`crates/pdf/tests/chart.rs`, `image.rs`, `annot.rs`); здесь важно другое —
// что до PDF доехал именно wasm-путь примера.

import { inflateSync } from 'node:zlib';

/** Объект PDF: словарь и, если он есть, байты потока. */
interface PdfObject {
  header: string;
  data: Buffer | null;
}

/** Операция потока содержимого: имя и операнды, накопленные перед ней. */
export interface PdfOp {
  op: string;
  operands: number[];
}

/** Прямоугольник в точках страницы. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Клип `q … W n … Q`: диапазон операций внутри и границы его пути. */
export interface Clip {
  /** Первая операция содержимого после `W n`. */
  start: number;
  /** Операция `Q`, закрывающая блок. */
  end: number;
  /** Границы пути-клипа; `null` — путь без координат. */
  rect: Rect | null;
}

/** Разобранный PDF: страницы с операциями и сырой текст файла. */
export interface PdfProbe {
  /** Число словарей `/Type /Page`. */
  pageCount: number;
  /** Операции каждой страницы в порядке следования. */
  pages: PdfOp[][];
  /**
   * Байты файла как latin1-строка: словари и аннотации не сжаты, поэтому
   * `/Subtype /Link` и `/URI (...)` ищутся прямо в них.
   */
  text: string;
}

/**
 * Собрать объекты файла: `N G obj … [stream … endstream] endobj`.
 *
 * После потока сканер перескакивает его байты: в сжатых данных случайно
 * встречается `obj`, и следующий объект был бы найден внутри картинки.
 */
function objects(bytes: Buffer): Map<number, PdfObject> {
  const text = bytes.toString('latin1');
  const found = new Map<number, PdfObject>();
  const start = /(\d+)\s+\d+\s+obj\b/g;
  let match: RegExpExecArray | null;
  while ((match = start.exec(text)) !== null) {
    const number = Number(match[1]);
    const bodyStart = match.index + match[0].length;
    const endObj = text.indexOf('endobj', bodyStart);
    const bodyEnd = endObj < 0 ? text.length : endObj;
    const streamAt = text.indexOf('stream', bodyStart);
    if (streamAt >= 0 && streamAt < bodyEnd) {
      const header = text.slice(bodyStart, streamAt);
      let dataStart = streamAt + 'stream'.length;
      // После ключевого слова обязателен перевод строки — CRLF или LF.
      if (text.startsWith('\r\n', dataStart)) dataStart += 2;
      else if (text[dataStart] === '\n' || text[dataStart] === '\r') dataStart += 1;
      const endStream = text.indexOf('endstream', dataStart);
      found.set(number, { header, data: bytes.subarray(dataStart, endStream) });
      start.lastIndex = endStream + 'endstream'.length;
    } else {
      found.set(number, { header: text.slice(bodyStart, bodyEnd), data: null });
      start.lastIndex = bodyEnd + 'endobj'.length;
    }
  }
  return found;
}

/** Содержимое страницы: поток распаковывается, несжатый берётся как есть. */
function content(object: PdfObject | undefined): string {
  if (!object?.data) throw new Error('поток содержимого страницы не найден');
  try {
    return inflateSync(object.data).toString('latin1');
  } catch {
    return object.data.toString('latin1');
  }
}

/** Разобрать PDF: страницы и их операции. */
export function probePdf(bytes: Buffer): PdfProbe {
  const text = bytes.toString('latin1');
  const found = objects(bytes);
  const pages: PdfOp[][] = [];
  for (const object of found.values()) {
    if (!/\/Type\s*\/Page(?!s)/.test(object.header)) continue;
    const ref = /\/Contents\s+(\d+)\s+\d+\s+R/.exec(object.header);
    if (!ref) throw new Error('у страницы нет /Contents N 0 R — разбор надо расширить');
    pages.push(operators(content(found.get(Number(ref[1])))));
  }
  return { pageCount: pages.length, pages, text };
}

/**
 * Операции потока содержимого с операндами.
 *
 * Строки (литеральные и hex) и имена вырезаются до токенизации: буквы внутри
 * текста иначе считались бы операторами, а цифры в именах — операндами.
 */
export function operators(content: string): PdfOp[] {
  const bare = content
    .replace(/\((?:\\.|[^\\()])*\)/g, '()')
    .replace(/<[0-9A-Fa-f\s]*>/g, '<>')
    // Имя: `/Имя` без спецсимволов; хвостовые цифры ресурса (`/F1`) тоже
    // убираются, иначе `1` попал бы в операнды следующего оператора.
    .replace(/\/[A-Za-z0-9#._+-]*/g, '/');

  const ops: PdfOp[] = [];
  let operands: number[] = [];
  for (const token of bare.matchAll(/-?\d*\.?\d+|[A-Za-z*'"]+/g)) {
    if (/^\d|^-|^\./.test(token[0])) {
      operands.push(Number(token[0]));
    } else {
      ops.push({ op: token[0], operands });
      operands = [];
    }
  }
  return ops;
}

/** Сколько раз оператор встретился. */
export function countOps(ops: PdfOp[], name: string): number {
  return ops.filter((entry) => entry.op === name).length;
}

/** Границы пути-клипа: точки `m`/`l`/`c` и прямоугольник `re`. */
function pathBounds(path: PdfOp[]): Rect | null {
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  const add = (x: number, y: number): void => {
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  };
  for (const { op, operands } of path) {
    if (op === 're' && operands.length >= 4) {
      add(operands[0]!, operands[1]!);
      add(operands[0]! + operands[2]!, operands[1]! + operands[3]!);
    } else if ((op === 'm' || op === 'l') && operands.length >= 2) {
      add(operands[0]!, operands[1]!);
    } else if (op === 'c') {
      for (let i = 0; i + 1 < operands.length; i += 2) add(operands[i]!, operands[i + 1]!);
    }
  }
  if (minX > maxX) return null;
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}

/**
 * Клипы страницы `q … W n … Q`: диапазон операций внутри и границы пути.
 *
 * Клип сериализуется как `q  m … l … h  W n` (см. `polygon_to_stream_ops`
 * в вендоренном printpdf), за `n` идут примитивы, закрывает блок `Q`. Клипы
 * вкладываются друг в друга штабелем, поэтому сканер считает вложенность.
 *
 * Копия обхода из `crates/pdf/tests/chart.rs::clip_blocks`: расхождение этих
 * двух сканеров означало бы, что e2e проверяет не то, что нативный тест.
 */
export function clips(ops: PdfOp[]): Clip[] {
  const stack: number[] = [];
  const open: Array<{ openedAt: number; start: number; path: PdfOp[] }> = [];
  const found: Clip[] = [];
  ops.forEach((entry, index) => {
    if (entry.op === 'q') {
      stack.push(index);
    } else if (entry.op === 'Q') {
      const openedAt = stack.pop();
      const position = open.findIndex((block) => block.openedAt === openedAt);
      if (position >= 0) {
        const block = open.splice(position, 1)[0]!;
        found.push({ start: block.start, end: index, rect: pathBounds(block.path) });
      }
    } else if ((entry.op === 'W' || entry.op === 'W*') && ops[index + 1]?.op === 'n') {
      const openedAt = stack[stack.length - 1];
      if (openedAt !== undefined) {
        open.push({ openedAt, start: index + 2, path: ops.slice(openedAt + 1, index) });
      }
    }
  });
  return found;
}

/** Сколько XObject-картинок объявлено в файле. */
export function imageXObjects(text: string): number {
  return [...text.matchAll(/\/Subtype\s*\/Image(?![A-Za-z])/g)].length;
}

/**
 * Аннотации-ссылки: URI внешней ссылки или `null` у внутренней (`/GoTo`).
 *
 * Аннотации вложены в словарь страницы (`/Annots [...]`), а не лежат
 * отдельными объектами, поэтому сканируется всё действие `/S` целиком: у
 * `/S/URI` за ним сразу идёт `/URI (...)`, у `/S/GoTo` — массив `/D`.
 */
export function linkAnnotations(text: string): Array<{ uri: string | null }> {
  return [
    ...text.matchAll(/\/S\s*\/(URI|GoTo)(?:\s*\/URI\s*\(((?:\\.|[^\\()])*)\))?/g),
  ].map((match) => ({ uri: match[2] ?? null }));
}

/** Строит ли оператор путь. */
export function buildsPath(op: PdfOp): boolean {
  return op.op === 'm' || op.op === 'l' || op.op === 'c' || op.op === 're';
}

/** Закрашивает ли оператор построенный путь. */
export function paintsPath(op: PdfOp): boolean {
  return ['f', 'f*', 'B', 'B*', 'S', 's'].includes(op.op);
}
