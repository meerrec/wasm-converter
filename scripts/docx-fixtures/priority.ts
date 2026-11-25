// Фикстуры приоритетного состава 1.0.0, закрывающие документированные пробелы
// матрицы сертификации (docs/strict-viewer/certification-matrix.md):
//
// - `fields/toc_ref` — TOC и REF: сохранённый кэш поля (`docx.fields-toc-refs`);
// - `images/inline_jpeg_emf` — JPEG рядом с EMF, который не декодируется
//   (`docx.images-inline`: порядок и размер рисунков не зависят от формата);
// - `pagination/keep_next_tall` — `keepNext` у абзаца, не помещающегося на
//   странице целиком (`docx.tables-pagination`: правило не может двигать
//   бесконечность).
//
// Детерминизм: байты JPEG собираются здесь (без импорта из
// `scripts/gen-fixtures.ts`, который при импорте пишет всю XLSX-корзину), EMF —
// минимальный корректный заголовок без записей и пикселей.

import {
    type Bytes,
    R_NS,
    W_NS,
    XML_DECL,
    type FixtureSpec,
    defaultSectPr,
    escapeXml,
    imageRelId,
    inlineImage,
    packageParts,
    para,
    run,
    sectionProps,
} from './kit.js';

// ==================== JPEG ====================

/** Коды Хаффмана по длинам из `bits` и символам из `values` (ITU T.81, Annex C). */
const huffmanCodes = (bits: number[], values: number[]): Map<number, [number, number]> => {
    const codes = new Map<number, [number, number]>();
    let code = 0;
    let i = 0;
    for (let length = 1; length <= 16; length += 1) {
        for (let n = 0; n < bits[length - 1]!; n += 1) {
            codes.set(values[i]!, [code, length]);
            code += 1;
            i += 1;
        }
        code <<= 1;
    }
    return codes;
};

/** Стандартная таблица Хаффмана для DC-коэффициентов яркости (T.81, K.3.1). */
const DC_BITS = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_VALUES = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/**
 * Минимальный baseline JPEG в оттенках серого: блоки 8×8 залиты ровным цветом,
 * поэтому кроме DC-коэффициента в блоке нет ничего и AC сводится к одному
 * символу EOB.
 *
 * `brightness(x, y)` задаёт яркость 0..255; блок кодируется средним по своим
 * пикселям. Размеры кратны восьми.
 */
const jpegBytes = (width: number, height: number, brightness: (x: number, y: number) => number): Bytes => {
    const dc = huffmanCodes(DC_BITS, DC_VALUES);
    const eob: [number, number] = [0, 1];

    const out: number[] = [];
    let accumulator = 0;
    let bits = 0;
    /** Записать `length` младших бит `value`; байт `0xFF` экранируется нулём. */
    const put = (value: number, length: number): void => {
        accumulator = (accumulator << length) | value;
        bits += length;
        while (bits >= 8) {
            const byte = (accumulator >>> (bits - 8)) & 0xff;
            out.push(byte);
            if (byte === 0xff) out.push(0);
            bits -= 8;
        }
        accumulator &= (1 << bits) - 1;
    };
    const marker = (id: number, payload: number[]): void => {
        out.push(0xff, id);
        const size = payload.length + 2;
        out.push(size >>> 8, size & 0xff, ...payload);
    };

    // Сегменты, кроме сканирования: таблица квантования из одних 16 — блоки
    // залиты ровным цветом, тонкость не нужна.
    out.push(0xff, 0xd8);
    marker(0xe0, [0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
    marker(0xdb, [0x00, ...new Array<number>(64).fill(16)]);
    marker(0xc0, [0x08, height >>> 8, height & 0xff, width >>> 8, width & 0xff, 0x01, 0x01, 0x11, 0x00]);
    marker(0xc4, [0x00, ...DC_BITS, ...DC_VALUES]);
    marker(0xda, [0x01, 0x01, 0x00, 0x00, 0x3f, 0x00]);

    for (let by = 0; by < height; by += 8) {
        for (let bx = 0; bx < width; bx += 8) {
            let sum = 0;
            for (let y = 0; y < 8; y += 1) {
                for (let x = 0; x < 8; x += 1) sum += brightness(bx + x, by + y);
            }
            // DC уровня 1 — среднее блока минус 128; AC — пусто.
            const coefficient = Math.round(sum / 64) - 128;
            const magnitude = Math.abs(coefficient);
            let size = 0;
            while (magnitude >> size > 0) size += 1;
            const [code, length] = dc.get(size) ?? eob;
            put(code, length);
            if (size > 0) put(coefficient < 0 ? coefficient + (1 << size) - 1 : coefficient, size);
            put(eob[0], eob[1]);
        }
    }
    // Последний неполный байт добивается единицами (T.81, F.1.2.3).
    if (bits > 0) put((1 << (8 - bits)) - 1, 8 - bits);
    out.push(0xff, 0xd9);
    return Uint8Array.from(out);
};

// ==================== EMF ====================

/**
 * Минимальный корректный EMF: заголовок 108 байт и одна запись `EMR_EOF`.
 * Размеры — в 0.01 мм, как требует формат.
 */
const emfBytes = (widthMm: number, heightMm: number): Bytes => {
    const header = new ArrayBuffer(108);
    const view = new DataView(header);

    view.setUint32(0, 1, true);
    view.setUint32(4, 108, true);
    view.setInt32(40, 0, true); // rclBounds.left
    view.setInt32(44, 0, true);
    view.setInt32(48, Math.round(widthMm * 100) - 1, true); // rclBounds.right
    view.setInt32(52, Math.round(heightMm * 100) - 1, true); // rclBounds.bottom
    view.setInt32(56, Math.round(widthMm * 100), true); // rclFrame.right
    view.setInt32(60, Math.round(heightMm * 100), true); // rclFrame.bottom
    view.setUint32(64, 0x464d4520, true); // подпись « EMF»
    view.setUint32(68, 0x0001_0000, true); // версия 1.0
    view.setUint32(72, 108 + 20, true); // nBytes: заголовок и EMR_EOF
    view.setUint32(76, 1, true); // nRecords
    view.setUint16(80, 1, true); // nHandles
    view.setUint32(88, 88, true); // offDescription
    view.setUint32(92, 0, true); // nDescription
    view.setUint32(96, 0x20, true); // nPalEntries
    view.setInt32(100, Math.round(widthMm * 100), true); // szlDevice.cx
    view.setInt32(104, Math.round(heightMm * 100), true); // szlDevice.cy

    const eof = new DataView(new ArrayBuffer(20));
    eof.setUint32(0, 14, true); // iType = EMR_EOF
    eof.setUint32(4, 20, true); // nSize
    eof.setUint32(8, 0, true); // nPalEntries
    eof.setInt32(12, 0, true); // offPalEntries
    eof.setUint32(16, 20, true); // nSizeLast

    const bytes = new Uint8Array(128);
    bytes.set(new Uint8Array(header), 0);
    bytes.set(new Uint8Array(eof.buffer), 108);
    return bytes;
};

// ==================== Поля TOC/REF ====================

/** Тело из блоков + финальный `w:sectPr`. */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Прогон с готовым содержимым: `w:fldChar`, `w:instrText`, `w:t`. */
const rawRun = (inner: string): string => `<w:r>${inner}</w:r>`;

/** `w:fldChar` — граница поля (begin/separate/end). */
const fldChar = (type: 'begin' | 'separate' | 'end'): string => rawRun(`<w:fldChar w:fldCharType="${type}"/>`);

/** `w:instrText` — инструкция поля; окружающие пробелы значимы. */
const instrText = (instruction: string): string =>
    rawRun(`<w:instrText xml:space="preserve">${escapeXml(instruction)}</w:instrText>`);

/**
 * `word/document.xml` с TOC и REF.
 *
 * Кэш TOC — строка «Введение» с табуляцией перед номером страницы: видимого
 * результата больше, чем у REF, и оба поля проверяются одним документом.
 * Закладка `_Ref123` обёрнута вокруг заголовка — цель REF.
 */
const tocRefXml = (): string =>
    XML_DECL +
    `<w:document xmlns:w="${W_NS}" xmlns:r="${R_NS}">` +
    `<w:body>${body(
        para('', run('Оглавление (поле TOC)')),
        para(
            '',
            fldChar('begin') +
                instrText(' TOC \\o "1-3" \\h \\z \\u ') +
                fldChar('separate') +
                run('Введение') +
                rawRun('<w:tab/>') +
                run('1') +
                fldChar('end'),
        ),
        '<w:p><w:bookmarkStart w:id="0" w:name="_Ref123"/>' +
            run('Раздел с закладкой') +
            '<w:bookmarkEnd w:id="0"/></w:p>',
        para(
            '',
            run('См. раздел ') +
                fldChar('begin') +
                instrText(' REF _Ref123 \\h \\* MERGEFORMAT ') +
                fldChar('separate') +
                run('Раздел с закладкой') +
                fldChar('end') +
                run('.'),
        ),
    )}</w:body></w:document>`;

export const priorityFixtures: FixtureSpec[] = [
    {
        name: 'fields/toc_ref',
        description: 'Поля TOC и REF: сохранённый кэш и закладка _Ref123',
        expectedParagraphs: 4,
        expectedTables: 0,
        expectedImages: 0,
        expectedWarnings: [],
        parts: packageParts({ document: tocRefXml() }),
        content: {
            paragraphs: [
                { text: 'Оглавление (поле TOC)' },
                { text: 'Введение\t1' },
                { text: 'Раздел с закладкой' },
                { text: 'См. раздел Раздел с закладкой.' },
            ],
            fields: [
                {
                    instruction: ' TOC \\o "1-3" \\h \\z \\u ',
                    type: 'unknown',
                    cached: 'Введение\t1',
                    form: 'fldChar',
                },
                {
                    instruction: ' REF _Ref123 \\h \\* MERGEFORMAT ',
                    type: 'unknown',
                    cached: 'Раздел с закладкой',
                    form: 'fldChar',
                },
            ],
            tables: [],
            images: [],
        },
        meta: { bookmark: '_Ref123' },
    },
    {
        name: 'images/inline_jpeg_emf',
        description: 'Два inline-рисунка рядом: JPEG и EMF, который не декодируется',
        expectedParagraphs: 2,
        expectedTables: 0,
        expectedImages: 2,
        expectedWarnings: [],
        parts: packageParts({
            body: body(
                para(
                    '',
                    `<w:r>${inlineImage(imageRelId(0), { id: 1, name: 'inline.jpg', cx: 914400, cy: 685800 })}</w:r>` +
                        `<w:r>${inlineImage(imageRelId(1), { id: 2, name: 'inline.emf', cx: 457200, cy: 457200 })}</w:r>`,
                ),
                para('', run('Подпись под рисунками.')),
            ),
            images: {
                'inline.jpg': jpegBytes(16, 16, (x, y) => ((x + y) % 2 === 0 ? 230 : 40)),
                'inline.emf': emfBytes(40, 30),
            },
            extraContentTypes:
                '<Default Extension="jpg" ContentType="image/jpeg"/>' +
                '<Default Extension="emf" ContentType="image/x-emf"/>',
        }),
        content: {
            paragraphs: [{ text: '' }, { text: 'Подпись под рисунками.' }],
            images: [
                { relId: imageRelId(0), file: 'inline.jpg', kind: 'inline', cx: 914400, cy: 685800 },
                { relId: imageRelId(1), file: 'inline.emf', kind: 'inline', cx: 457200, cy: 457200 },
            ],
        },
        meta: {
            mediaTypes: { 'inline.jpg': 'image/jpeg', 'inline.emf': 'application/octet-stream' },
        },
    },
    {
        name: 'pagination/keep_next_tall',
        description: 'keepNext у абзаца, который сам длиннее страницы',
        expectedParagraphs: 5,
        expectedTables: 0,
        expectedImages: 0,
        expectedWarnings: [],
        parts: packageParts({
            body: body(
                ...Array.from({ length: 3 }, (_, index) => para('', run(`Абзац ${index + 1}.`))),
                // Заведомо длиннее полосы набора: keepNext не может удержать
                // его и следующий абзац на одной странице.
                '<w:p><w:pPr><w:keepNext/></w:pPr>' +
                    run('Строка очень длинного абзаца, который не помещается на страницу целиком. '.repeat(60)) +
                    '</w:p>',
                para('', run('Абзац после длинного.')),
            ),
        }),
        meta: { keepNext: [4], tallParagraph: true },
    },
];
