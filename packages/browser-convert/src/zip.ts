/**
 * ZIP-механика без зависимостей: разбор central directory, чтение и пересборка архивов.
 *
 * Разжатие/сжатие — на Web Streams (`DecompressionStream`/`CompressionStream` с форматом
 * `deflate-raw`, он глобально доступен и в браузерах, и в Node 18+), CRC32 — собственной
 * табличной реализацией (пакет браузерный, `node:zlib` недоступен). Никаких сторонних
 * библиотек: OOXML-контейнеры это тот же zip, и конвертеру нужны лишь чтение записей и
 * точечная замена содержимого без пересборки всего архива.
 *
 * Почему размеры и CRC берутся ТОЛЬКО из central directory: потоковые писатели (в том
 * числе многие генераторы DOCX/XLSX) не знают длину данных заранее, ставят в general
 * purpose bit field флаг 3 и оставляют в local header нули, а настоящие значения кладут
 * в data descriptor после данных. Верить local header — значит видеть «пустую» запись;
 * central directory при этом заполнена всегда.
 */

/** Сигнатура записи central directory. */
const CENTRAL_SIGNATURE = 0x02014b50;
/** Сигнатура local file header. */
const LOCAL_SIGNATURE = 0x04034b50;
/** Сигнатура EOCD (End of central directory). */
const EOCD_SIGNATURE = 0x06054b50;
/** Сигнатура data descriptor; в самом descriptor'е она опциональна. */
const DESCRIPTOR_SIGNATURE = 0x08074b50;
/** Сигнатура locator'а zip64 EOCD — лежит ровно перед EOCD и выдаёт zip64-архив. */
const ZIP64_LOCATOR_SIGNATURE = 0x07064b50;
/** Метод сжатия: без сжатия. */
const METHOD_STORED = 0;
/** Метод сжатия: deflate. */
const METHOD_DEFLATE = 8;
/** Длина фиксированной части EOCD без комментария. */
const EOCD_BASE_LENGTH = 22;
/** Максимальная длина комментария EOCD (поле занимает 2 байта). */
const MAX_EOCD_COMMENT_LENGTH = 0xffff;
/** Длина фиксированной части записи central directory. */
const CENTRAL_FIXED_LENGTH = 46;
/** Длина фиксированной части local file header. */
const LOCAL_FIXED_LENGTH = 30;

/** Декодировщик имён записей. Имена декодируются как UTF-8 независимо от флага кодировки. */
const UTF8_DECODER = new TextDecoder('utf-8');
const UTF8_ENCODER = new TextEncoder();

export interface ZipEntry {
  /** Имя записи; каталоги оканчиваются на '/'. */
  readonly name: string;
  readonly isDir: boolean;
  /** Метод сжатия: 0 — без сжатия, 8 — deflate. */
  readonly method: number;
  /** Размер в байтах БЕЗ сжатия. Из central directory, не из local header. */
  readonly size: number;
  readonly compressedSize: number;
  readonly crc32: number;
  /** Смещение local header записи в архиве. */
  readonly localOffset: number;
}

/** Запись central directory вместе со служебными смещениями, вычисленными при разборе. */
interface EntryRecord {
  readonly entry: ZipEntry;
  /** Сырые байты записи central directory — для verbatim-копирования при пересборке. */
  readonly cdRecord: Uint8Array;
  /** Смещение первого байта данных записи: сразу после local header (имя + extra). */
  readonly dataOffset: number;
  /** Длина data descriptor после данных (0, если флаг 3 не стоит); подпись в нём опциональна. */
  readonly descriptorLength: number;
}

interface ArchiveParse {
  readonly records: EntryRecord[];
  /** Комментарий EOCD исходного архива — сохраняется при пересборке. */
  readonly comment: Uint8Array;
}

export interface ZipRewriteOptions {
  /** Какие записи обрабатываются transform'ом; остальные копируются байт-в-байт. */
  readonly filter?: (entry: ZipEntry) => boolean;
  /** Принять разжатые байты записи; вернуть новые байты/строку (строка кодируется UTF-8) или null — запись остаётся без изменений. */
  readonly transform?: (data: Uint8Array, entry: ZipEntry) => Uint8Array | string | null;
  /** Записи, которые добавить в архив (deflate). */
  readonly inject?: ReadonlyArray<{ readonly name: string; readonly data: Uint8Array }>;
}

/** Таблица CRC32 (отражённый полином 0xEDB88320) — считается один раз при загрузке модуля. */
const CRC32_TABLE = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let value = i;
  for (let bit = 0; bit < 8; bit++) {
    value = (value & 1) === 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  }
  CRC32_TABLE[i] = value >>> 0;
}

/** CRC32 (IEEE 802.3, как в zip) для массива байтов. */
function crc32(data: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of data) {
    crc = CRC32_TABLE[(crc ^ byte) & 0xff]! ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** Сливает куски в один массив байтов. */
function concatBytes(parts: readonly Uint8Array[]): Uint8Array {
  let total = 0;
  for (const part of parts) {
    total += part.length;
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

/** Считывает весь поток в один массив байтов. */
async function collectStream(stream: ReadableStream<Uint8Array>): Promise<Uint8Array> {
  const chunks: Uint8Array[] = [];
  const reader = stream.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    chunks.push(value);
  }
  return concatBytes(chunks);
}

/** Сжимает данные raw-deflate (без zlib-обёртки — именно такой поток zip кладёт в записи). */
async function deflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream('deflate-raw');
  const writer = stream.writable.getWriter();
  const collected = collectStream(stream.readable);
  // Потоки в lib.dom типизированы под ArrayBuffer-вид, а наше API принимает
  // Uint8Array<ArrayBufferLike>. На SAB здесь данных не бывает, и рантайм принимает
  // любой view — сужение типа без копии.
  await writer.write(data as BufferSource);
  await writer.close();
  return collected;
}

/** Разжимает raw-deflate; null — данные повреждены и не разжимаются. */
async function inflateRaw(compressed: Uint8Array): Promise<Uint8Array | null> {
  const stream = new DecompressionStream('deflate-raw');
  const writer = stream.writable.getWriter();
  const collected = collectStream(stream.readable).catch(() => null);
  try {
    await writer.write(compressed as BufferSource);
    await writer.close();
  } catch {
    try {
      await writer.abort();
    } catch {
      // abort у упавшего потока может и сам отклониться — это не меняет исхода.
    }
    return null;
  }
  return collected;
}

/**
 * Ищет EOCD с конца архива: сигнатура 0x06054b50 может встретиться и в комментарии,
 * поэтому кандидат проверяется по длине комментария — она обязана совпадать с остатком
 * байтов после записи.
 */
function findEocdOffset(bytes: Uint8Array, view: DataView): number {
  const searchStart = Math.max(0, bytes.length - MAX_EOCD_COMMENT_LENGTH - EOCD_BASE_LENGTH);
  for (let offset = bytes.length - EOCD_BASE_LENGTH; offset >= searchStart; offset--) {
    if (view.getUint32(offset, true) !== EOCD_SIGNATURE) {
      continue;
    }
    const commentLength = view.getUint16(offset + 20, true);
    if (commentLength === bytes.length - offset - EOCD_BASE_LENGTH) {
      return offset;
    }
  }
  throw new Error('not a ZIP archive: end of central directory not found');
}

/** Результат разбора local header: начало данных и флаг data descriptor. */
interface LocalInfo {
  readonly dataOffset: number;
  /** Стоит ли флаг 3 — значения размеров/CRC лежат в data descriptor после данных. */
  readonly hasDescriptor: boolean;
}

/**
 * Парсит local header минимально: имя + extra могут занимать место, поэтому начало
 * данных вычисляется по их длинам из самого заголовка, а не как «30 байт от смещения».
 */
function parseLocalHeader(bytes: Uint8Array, view: DataView, localOffset: number, cdOffset: number): LocalInfo {
  if (localOffset + LOCAL_FIXED_LENGTH > cdOffset) {
    throw new Error('corrupt ZIP archive: local header out of bounds');
  }
  if (view.getUint32(localOffset, true) !== LOCAL_SIGNATURE) {
    throw new Error('corrupt ZIP archive: bad local header signature');
  }
  const flag = view.getUint16(localOffset + 6, true);
  const nameLength = view.getUint16(localOffset + 26, true);
  const extraLength = view.getUint16(localOffset + 28, true);
  const dataOffset = localOffset + LOCAL_FIXED_LENGTH + nameLength + extraLength;
  if (dataOffset > cdOffset) {
    throw new Error('corrupt ZIP archive: entry data out of bounds');
  }
  return { dataOffset, hasDescriptor: (flag & 0x0008) !== 0 };
}

/**
 * Разбирает central directory архива. Бросает на структурной порче и на zip64:
 * раскладка zip64 меняет смысл полей EOCD и записей, поэтому молча «поддержать» её,
 * прочитав только половину поля, нельзя.
 */
function parseArchive(bytes: Uint8Array): ArchiveParse {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const eocdOffset = findEocdOffset(bytes, view);
  const diskNumber = view.getUint16(eocdOffset + 4, true);
  const cdDisk = view.getUint16(eocdOffset + 6, true);
  const diskEntries = view.getUint16(eocdOffset + 8, true);
  const totalEntries = view.getUint16(eocdOffset + 10, true);
  const cdSize = view.getUint32(eocdOffset + 12, true);
  const cdOffset = view.getUint32(eocdOffset + 16, true);
  const isZip64Eocd =
    diskNumber === 0xffff ||
    cdDisk === 0xffff ||
    diskEntries === 0xffff ||
    totalEntries === 0xffff ||
    cdSize === 0xffffffff ||
    cdOffset === 0xffffffff;
  const hasZip64Locator =
    eocdOffset >= 20 && view.getUint32(eocdOffset - 20, true) === ZIP64_LOCATOR_SIGNATURE;
  if (isZip64Eocd || hasZip64Locator) {
    throw new Error('zip64 archives are not supported');
  }
  if (cdOffset + cdSize > eocdOffset) {
    throw new Error('corrupt ZIP archive: central directory out of bounds');
  }
  const commentLength = view.getUint16(eocdOffset + 20, true);
  const comment = bytes.slice(eocdOffset + EOCD_BASE_LENGTH, eocdOffset + EOCD_BASE_LENGTH + commentLength);

  const records: EntryRecord[] = [];
  const cdEnd = cdOffset + cdSize;
  let offset = cdOffset;
  while (offset < cdEnd) {
    if (offset + CENTRAL_FIXED_LENGTH > cdEnd) {
      throw new Error('corrupt ZIP archive: central directory entry out of bounds');
    }
    if (view.getUint32(offset, true) !== CENTRAL_SIGNATURE) {
      throw new Error('corrupt ZIP archive: bad central directory signature');
    }
    const method = view.getUint16(offset + 10, true);
    const entryCrc = view.getUint32(offset + 16, true);
    const compressedSize = view.getUint32(offset + 20, true);
    const size = view.getUint32(offset + 24, true);
    const nameLength = view.getUint16(offset + 28, true);
    const extraLength = view.getUint16(offset + 30, true);
    const commentLengthEntry = view.getUint16(offset + 32, true);
    const diskStart = view.getUint16(offset + 34, true);
    const localOffset = view.getUint32(offset + 42, true);
    if (size === 0xffffffff || compressedSize === 0xffffffff || localOffset === 0xffffffff) {
      throw new Error('zip64 archives are not supported');
    }
    if (diskStart !== 0) {
      throw new Error('multi-disk ZIP archives are not supported');
    }
    const recordLength = CENTRAL_FIXED_LENGTH + nameLength + extraLength + commentLengthEntry;
    if (offset + recordLength > cdEnd) {
      throw new Error('corrupt ZIP archive: central directory entry out of bounds');
    }
    const name = UTF8_DECODER.decode(bytes.subarray(offset + CENTRAL_FIXED_LENGTH, offset + CENTRAL_FIXED_LENGTH + nameLength));
    const localInfo = parseLocalHeader(bytes, view, localOffset, cdOffset);
    if (localInfo.dataOffset + compressedSize > cdOffset) {
      throw new Error('corrupt ZIP archive: entry data out of bounds');
    }
    let descriptorLength = 0;
    if (localInfo.hasDescriptor) {
      // Data descriptor лежит сразу за данными; подпись 0x08074b50 в нём опциональна,
      // поэтому длина либо 16 (с подписью), либо 12 байт. Без учёта descriptor'а
      // verbatim-копия потеряла бы его, и local header с флагом 3 остался бы висеть
      // с нулевыми размерами — для сторонних читателей архив стал бы битым.
      if (localInfo.dataOffset + compressedSize + 12 > cdOffset) {
        throw new Error('corrupt ZIP archive: data descriptor out of bounds');
      }
      const hasSignature = view.getUint32(localInfo.dataOffset + compressedSize, true) === DESCRIPTOR_SIGNATURE;
      descriptorLength = hasSignature ? 16 : 12;
      if (localInfo.dataOffset + compressedSize + descriptorLength > cdOffset) {
        throw new Error('corrupt ZIP archive: data descriptor out of bounds');
      }
    }
    const entry: ZipEntry = {
      name,
      isDir: name.endsWith('/'),
      method,
      size,
      compressedSize,
      crc32: entryCrc,
      localOffset,
    };
    records.push({
      entry,
      cdRecord: bytes.slice(offset, offset + recordLength),
      dataOffset: localInfo.dataOffset,
      descriptorLength,
    });
    offset += recordLength;
  }
  if (records.length !== totalEntries) {
    throw new Error('corrupt ZIP archive: entry count mismatch');
  }
  return { records, comment };
}

/** Достаёт данные записи по внутреннему record'у; null — запись повреждена или метод не поддерживается. */
async function readRecordData(bytes: Uint8Array, record: EntryRecord): Promise<Uint8Array | null> {
  const { entry, dataOffset } = record;
  if (entry.method === METHOD_STORED) {
    if (dataOffset + entry.size > bytes.length) {
      return null;
    }
    return bytes.slice(dataOffset, dataOffset + entry.size);
  }
  if (entry.method === METHOD_DEFLATE) {
    if (dataOffset + entry.compressedSize > bytes.length) {
      return null;
    }
    const inflated = await inflateRaw(bytes.subarray(dataOffset, dataOffset + entry.compressedSize));
    if (inflated === null || inflated.length !== entry.size) {
      return null;
    }
    return inflated;
  }
  return null;
}

/** Ищет запись архива: по имени или по объекту из listZipEntries этого же архива. */
function resolveRecord(bytes: Uint8Array, entry: ZipEntry | string): EntryRecord | undefined {
  const { records } = parseArchive(bytes);
  if (typeof entry === 'string') {
    return records.find((candidate) => candidate.entry.name === entry);
  }
  // Объект записи обязан принадлежать этому же архиву; чужая запись здесь не совпадёт
  // ни по имени, ни по смещению, ни по CRC — это не порча архива, а ошибка вызова.
  return records.find((candidate) => {
    const candidateEntry = candidate.entry;
    return (
      candidateEntry.name === entry.name &&
      candidateEntry.localOffset === entry.localOffset &&
      candidateEntry.crc32 === entry.crc32 &&
      candidateEntry.compressedSize === entry.compressedSize
    );
  });
}

/** Собирает local header для новой записи: deflate, флаг data descriptor не ставится — размеры известны заранее. */
function buildLocalHeader(name: string, entryCrc: number, compressedSize: number, size: number): Uint8Array {
  const nameBytes = UTF8_ENCODER.encode(name);
  const header = new Uint8Array(LOCAL_FIXED_LENGTH + nameBytes.length);
  const view = new DataView(header.buffer);
  view.setUint32(0, LOCAL_SIGNATURE, true);
  view.setUint16(4, 20, true); // version needed: 2.0 — достаточно для deflate
  view.setUint16(8, METHOD_DEFLATE, true);
  view.setUint32(14, entryCrc, true);
  view.setUint32(18, compressedSize, true);
  view.setUint32(22, size, true);
  view.setUint16(26, nameBytes.length, true);
  header.set(nameBytes, LOCAL_FIXED_LENGTH);
  return header;
}

/** Собирает запись central directory для новой записи. */
function buildCentralRecord(name: string, entryCrc: number, compressedSize: number, size: number, localOffset: number): Uint8Array {
  const nameBytes = UTF8_ENCODER.encode(name);
  const record = new Uint8Array(CENTRAL_FIXED_LENGTH + nameBytes.length);
  const view = new DataView(record.buffer);
  view.setUint32(0, CENTRAL_SIGNATURE, true);
  view.setUint16(4, 20, true); // version made by: 2.0
  view.setUint16(6, 20, true); // version needed: 2.0
  view.setUint16(10, METHOD_DEFLATE, true);
  view.setUint32(16, entryCrc, true);
  view.setUint32(20, compressedSize, true);
  view.setUint32(24, size, true);
  view.setUint16(28, nameBytes.length, true);
  view.setUint32(42, localOffset, true);
  record.set(nameBytes, CENTRAL_FIXED_LENGTH);
  return record;
}

/** Собирает EOCD с сохранённым комментарием исходного архива. */
function buildEocd(entryCount: number, cdSize: number, cdOffset: number, comment: Uint8Array): Uint8Array {
  const eocd = new Uint8Array(EOCD_BASE_LENGTH + comment.length);
  const view = new DataView(eocd.buffer);
  view.setUint32(0, EOCD_SIGNATURE, true);
  view.setUint16(8, entryCount, true);
  view.setUint16(10, entryCount, true);
  view.setUint32(12, cdSize, true);
  view.setUint32(16, cdOffset, true);
  view.setUint16(20, comment.length, true);
  eocd.set(comment, EOCD_BASE_LENGTH);
  return eocd;
}

/** Записи архива из central directory. Бросает на повреждённом/не-zip. */
export function listZipEntries(bytes: Uint8Array): ZipEntry[] {
  return parseArchive(bytes).records.map((record) => record.entry);
}

/** Существует ли запись с таким именем. */
export function zipHasEntry(bytes: Uint8Array, name: string): boolean {
  return resolveRecord(bytes, name) !== undefined;
}

/**
 * Прочитать и разжать запись (по объекту записи или имени).
 * null — метод сжатия не поддерживается или запись повреждена; бросать только на
 * структурной порче архива. CRC32 здесь не сверяется: несовпадение не портит
 * содержимое, а отбрасывать читаемую запись из-за ошибки чужого писателя — дороже;
 * при необходимости CRC можно проверить самому через `entry.crc32`.
 */
export async function readZipEntry(bytes: Uint8Array, entry: ZipEntry | string): Promise<Uint8Array | null> {
  const record = resolveRecord(bytes, entry);
  if (record === undefined) {
    return null;
  }
  return readRecordData(bytes, record);
}

/**
 * Пересобрать архив: transform по filter + inject.
 * Необработанные записи копируются verbatim (local header вместе с данными и, если
 * стоял флаг 3, data descriptor — без пережатия и без изменения полей). Обработанные
 * сжимаются deflate-raw заново, их local header собирается без data descriptor.
 */
export async function rewriteZip(bytes: Uint8Array, options: ZipRewriteOptions): Promise<Uint8Array> {
  const { records, comment } = parseArchive(bytes);
  const filter = options.filter;
  const transform = options.transform;
  const inject = options.inject;
  const injectCount = inject === undefined ? 0 : inject.length;
  if (records.length + injectCount > 0xffff) {
    throw new Error('zip64 archives are not supported: too many entries');
  }

  const localParts: Uint8Array[] = [];
  const centralRecords: Uint8Array[] = [];
  let localBytes = 0;

  for (const record of records) {
    const { entry, cdRecord } = record;
    const shouldTransform = filter === undefined || filter(entry);
    let replacement: Uint8Array | string | null = null;
    if (shouldTransform && transform !== undefined) {
      const data = await readRecordData(bytes, record);
      if (data !== null) {
        replacement = transform(data, entry);
      }
    }

    if (replacement === null || transform === undefined || !shouldTransform) {
      // Verbatim-копия: local header + сжатые данные (и data descriptor, если он был)
      // байт-в-байт. В central directory меняется только смещение local header.
      const localPart = bytes.slice(
        entry.localOffset,
        record.dataOffset + entry.compressedSize + record.descriptorLength,
      );
      localParts.push(localPart);
      const patchedRecord = cdRecord.slice();
      new DataView(patchedRecord.buffer, patchedRecord.byteOffset, patchedRecord.byteLength).setUint32(
        42,
        localBytes,
        true,
      );
      centralRecords.push(patchedRecord);
      localBytes += localPart.length;
    } else {
      const data = typeof replacement === 'string' ? UTF8_ENCODER.encode(replacement) : replacement;
      const compressed = await deflateRaw(data);
      const entryCrc = crc32(data);
      centralRecords.push(buildCentralRecord(entry.name, entryCrc, compressed.length, data.length, localBytes));
      const localHeader = buildLocalHeader(entry.name, entryCrc, compressed.length, data.length);
      localParts.push(localHeader);
      localParts.push(compressed);
      localBytes += localHeader.length + compressed.length;
    }
  }

  if (inject !== undefined) {
    for (const item of inject) {
      const compressed = await deflateRaw(item.data);
      const entryCrc = crc32(item.data);
      centralRecords.push(buildCentralRecord(item.name, entryCrc, compressed.length, item.data.length, localBytes));
      const localHeader = buildLocalHeader(item.name, entryCrc, compressed.length, item.data.length);
      localParts.push(localHeader);
      localParts.push(compressed);
      localBytes += localHeader.length + compressed.length;
    }
  }

  const centralParts = concatBytes(centralRecords);
  if (centralParts.length > 0xffffffff || localBytes > 0xffffffff) {
    throw new Error('zip64 archives are not supported: archive too large');
  }
  return concatBytes([concatBytes(localParts), centralParts, buildEocd(records.length + injectCount, centralParts.length, localBytes, comment)]);
}
