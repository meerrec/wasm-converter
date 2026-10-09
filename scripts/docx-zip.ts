// Минимальный детерминированный ZIP-writer для фикстур DOCX.
//
// Системный `zip` не годится: он кладёт в архив текущее время модификации,
// поэтому два прогона генератора дают разные байты. Здесь всё фиксировано:
// DOS-дата 1980-01-01 00:00, порядок записей = порядок ключей объекта,
// никаких extra-полей и data descriptor'ов, CRC32 считается своей таблицей
// (не зависим от версии Node).
//
// Дополнительно — запись PNG: images в DOCX лежат готовыми файлами, а нам
// нужны картинки, которые не сжимаются deflate'ом (иначе «большие» фикстуры
// схлопнутся до килобайт).

import { deflateRawSync, deflateSync } from 'node:zlib';

/**
 * Байты части пакета. Buffer и есть Uint8Array, но в @types/node у них
 * расходятся дженерики, поэтому объединяем — иначе `Buffer` не пройдёт туда,
 * где ожидается `Uint8Array`.
 */
export type Bytes = Uint8Array | Buffer;

/** CRC32 (IEEE 802.3), таблица строится один раз. */
const CRC_TABLE = (() => {
    const table = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
        let c = n;
        for (let k = 0; k < 8; k++) {
            c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
        }
        table[n] = c >>> 0;
    }
    return table;
})();

/** @param data байты @returns CRC32 как беззнаковое 32-битное число */
export const crc32 = (data: Bytes): number => {
    let crc = 0xffffffff;
    for (let i = 0; i < data.length; i++) {
        crc = CRC_TABLE[(crc ^ data[i]!) & 0xff]! ^ (crc >>> 8);
    }
    return (crc ^ 0xffffffff) >>> 0;
};

// Дата 1980-01-01 (day=1, month=1, year=0). Именно 0x0021, а не 0: нулевой
// день/месяц формально невалиден и некоторые читатели на нём ругаются.
const DOS_DATE = 0x0021;
const DOS_TIME = 0x0000;

const VERSION_NEEDED = 20;
const VERSION_MADE_BY = 20;
const METHOD_DEFLATE = 8;
const METHOD_STORE = 0;

const SIG_LOCAL = 0x04034b50;
const SIG_CENTRAL = 0x02014b50;
const SIG_EOCD = 0x06054b50;

/** Запись архива: имя (путь внутри ZIP, прямой слэш) и содержимое. */
export type ZipEntry = readonly [name: string, data: string | Uint8Array | Buffer];

/**
 * Собирает ZIP в памяти. Порядок записей сохраняется как передан.
 *
 * @param entries записи архива
 * @returns байты архива
 */
export const buildZip = (entries: readonly ZipEntry[]): Buffer => {
    const localChunks: Buffer[] = [];
    const centralChunks: Buffer[] = [];
    let offset = 0;

    for (const [name, raw] of entries) {
        const nameBuf = Buffer.from(name, 'utf8');
        const data: Buffer = typeof raw === 'string' ? Buffer.from(raw, 'utf8') : Buffer.from(raw as Uint8Array);
        const crc = crc32(data);
        const deflated = deflateRawSync(data as Uint8Array, { level: 9 });
        // Как `zip -9`: несжимаемое (PNG) кладём как есть, иначе оно растёт.
        const useDeflate = deflated.length < data.length;
        const payload = useDeflate ? deflated : data;
        const method = useDeflate ? METHOD_DEFLATE : METHOD_STORE;

        const local = Buffer.alloc(30);
        local.writeUInt32LE(SIG_LOCAL, 0);
        local.writeUInt16LE(VERSION_NEEDED, 4);
        local.writeUInt16LE(0, 6); // flags: без UTF-8-флага (имена ASCII), без data descriptor
        local.writeUInt16LE(method, 8);
        local.writeUInt16LE(DOS_TIME, 10);
        local.writeUInt16LE(DOS_DATE, 12);
        local.writeUInt32LE(crc, 14);
        local.writeUInt32LE(payload.length, 18);
        local.writeUInt32LE(data.length, 22);
        local.writeUInt16LE(nameBuf.length, 26);
        local.writeUInt16LE(0, 28); // extraLen

        localChunks.push(local, nameBuf, payload);

        const central = Buffer.alloc(46);
        central.writeUInt32LE(SIG_CENTRAL, 0);
        central.writeUInt16LE(VERSION_MADE_BY, 4);
        central.writeUInt16LE(VERSION_NEEDED, 6);
        central.writeUInt16LE(0, 8); // flags
        central.writeUInt16LE(method, 10);
        central.writeUInt16LE(DOS_TIME, 12);
        central.writeUInt16LE(DOS_DATE, 14);
        central.writeUInt32LE(crc, 16);
        central.writeUInt32LE(payload.length, 20);
        central.writeUInt32LE(data.length, 24);
        central.writeUInt16LE(nameBuf.length, 28);
        central.writeUInt16LE(0, 30); // extraLen
        central.writeUInt16LE(0, 32); // commentLen
        central.writeUInt16LE(0, 34); // diskStart
        central.writeUInt16LE(0, 36); // internalAttrs
        central.writeUInt32LE(0, 38); // externalAttrs
        central.writeUInt32LE(offset, 42);
        centralChunks.push(central, nameBuf);

        offset += local.length + nameBuf.length + payload.length;
    }

    const centralSize = centralChunks.reduce((sum, buf) => sum + buf.length, 0);
    const eocd = Buffer.alloc(22);
    eocd.writeUInt32LE(SIG_EOCD, 0);
    eocd.writeUInt16LE(0, 4); // diskNum
    eocd.writeUInt16LE(0, 6); // cdStartDisk
    eocd.writeUInt16LE(entries.length, 8);
    eocd.writeUInt16LE(entries.length, 10);
    eocd.writeUInt32LE(centralSize, 12);
    eocd.writeUInt32LE(offset, 16);
    eocd.writeUInt16LE(0, 20); // commentLen

    return concatBytes([...localChunks, ...centralChunks, eocd]);
};

/** mulberry32 — детерминированный PRNG, чтобы «шумные» данные не сжимались. */
export const makePrng = (seed: number): (() => number) => {
    let state = seed >>> 0;
    return () => {
        state = (state + 0x6d2b79f5) >>> 0;
        let t = state;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
};

// Дженерики Buffer/Uint8Array в @types/node расходятся, поэтому собираем через хелпер.
const concatBytes = (chunks: Buffer[]): Buffer => Buffer.concat(chunks as Uint8Array[]);

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const pngChunk = (type: string, data: Buffer): Buffer => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length, 0);
    const typeBuf = Buffer.from(type, 'ascii');
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(concatBytes([typeBuf, data])), 0);
    return concatBytes([length, typeBuf, data, crc]);
};

/**
 * Детерминированный PNG (RGB, 8 бит) с шумом по seed.
 *
 * Шум — намеренно: deflate почти не жмёт случайные байты, поэтому размер
 * «больших» фикстур задаётся числом картинок, а не их содержимым.
 *
 * @param width ширина в пикселях
 * @param height высота в пикселях
 * @param seed зерно PRNG
 * @returns байты PNG
 */
export const pngBytes = (width: number, height: number, seed: number): Buffer => {
    const ihdr = Buffer.alloc(13);
    ihdr.writeUInt32BE(width, 0);
    ihdr.writeUInt32BE(height, 4);
    ihdr.writeUInt8(8, 8); // bit depth
    ihdr.writeUInt8(2, 9); // color type: truecolor RGB
    ihdr.writeUInt8(0, 10); // compression
    ihdr.writeUInt8(0, 11); // filter
    ihdr.writeUInt8(0, 12); // interlace

    const raw = Buffer.alloc(height * (1 + width * 3));
    const rnd = makePrng(seed);
    for (let y = 0; y < height; y++) {
        const rowStart = y * (1 + width * 3);
        raw[rowStart] = 0; // filter type: None
        for (let x = 0; x < width * 3; x++) {
            raw[rowStart + 1 + x] = Math.floor(rnd() * 256);
        }
    }

    return concatBytes([
        PNG_SIGNATURE,
        pngChunk('IHDR', ihdr),
        pngChunk('IDAT', deflateSync(raw as Uint8Array, { level: 9 })),
        pngChunk('IEND', Buffer.alloc(0)),
    ]);
};
