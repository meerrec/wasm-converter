// «Большие» фикстуры (>= 10 МиБ) для бюджетов памяти и скорости.
//
// В git они не попадают: каталог лежит под `target/`, который в .gitignore.
// Отдельная функция, а не часть общего списка, чтобы обычный прогон генератора
// оставался быстрым и не тянул сотни мегабайт в рабочее дерево.
//
// Размер набирается «шумом» из детерминированного PRNG: осмысленный текст
// deflate сжимает в сотни раз, а случайные слова — лишь до ~0.75, поэтому
// итоговый файл действительно большой.

import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { type Bytes, buildZip, makePrng, pngBytes } from '../docx-zip.js';
import {
    XML_DECL,
    W_NS,
    type FixtureSpec,
    defaultSectPr,
    defaultSettingsXml,
    defaultStylesXml,
    headerRelId,
    inlineImage,
    imageRelId,
    p,
    packageParts,
    sectionProps,
    tc,
    tr,
} from './kit.js';

/** Итог генерации большой фикстуры. */
export interface LargeFixtureResult {
    name: string;
    bytes: number;
}

const ALPHABET = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
const ALPHABET_CODES = Uint8Array.from(ALPHABET, (c) => c.charCodeAt(0));

/** Случайный «текст» заданной длины: слова по 3–12 символов через пробел. */
const noiseText = (length: number, rnd: () => number): string => {
    const buf = Buffer.allocUnsafe(length);
    let pos = 0;
    while (pos < length) {
        const wordLength = 3 + Math.floor(rnd() * 10);
        for (let i = 0; i < wordLength && pos < length; i++) {
            buf[pos++] = ALPHABET_CODES[Math.floor(rnd() * ALPHABET_CODES.length)]!;
        }
        if (pos < length) buf[pos++] = 0x20;
    }
    return buf.toString('latin1');
};

const numberingXml = (): string =>
    XML_DECL +
    `<w:numbering xmlns:w="${W_NS}">` +
    '<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="multilevel"/>' +
    '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/>' +
    '<w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>' +
    '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>';

const spec = (
    name: string,
    description: string,
    parts: FixtureSpec['parts'],
): FixtureSpec => ({
    name,
    description,
    expectedParagraphs: 0,
    expectedTables: 0,
    parts,
});

const buildManyParagraphs = (): FixtureSpec => {
    const rnd = makePrng(0x51a1);
    const blocks: string[] = [];
    for (let i = 0; i < 100_000; i++) {
        blocks.push(p(noiseText(170, rnd)));
    }
    return spec('many_paragraphs', '100 000 абзацев текста', packageParts({
        title: 'many_paragraphs',
        body: blocks.join('') + sectionProps(defaultSectPr),
    }));
};

const buildWideTable = (): FixtureSpec => {
    const rnd = makePrng(0x51a2);
    const rows: string[] = [];
    for (let r = 0; r < 2000; r++) {
        const cells: string[] = [];
        for (let c = 0; c < 20; c++) cells.push(tc(noiseText(450, rnd)));
        rows.push(tr(cells.join('')));
    }
    const tableXml =
        `<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>` +
        ['top', 'left', 'bottom', 'right', 'insideH', 'insideV']
            .map((side) => `<w:${side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>`)
            .join('') +
        '</w:tblBorders></w:tblPr>' +
        rows.join('') +
        '</w:tbl>';
    return spec('wide_table', 'Таблица 2000×20', packageParts({
        title: 'wide_table',
        body: tableXml + sectionProps(defaultSectPr),
    }));
};

const buildManyImages = (): FixtureSpec => {
    const count = 200;
    const images: Record<string, Bytes> = {};
    const paragraphs: string[] = [];
    for (let i = 0; i < count; i++) {
        const fileName = `image${i + 1}.png`;
        images[fileName] = pngBytes(160, 160, 0x5100 + i);
        paragraphs.push(
            '<w:p><w:r>' + inlineImage(imageRelId(i), { id: i + 1, name: fileName, cx: 812800, cy: 812800 }) + '</w:r></w:p>',
        );
    }
    return spec('many_images', '200 изображений', packageParts({
        title: 'many_images',
        body: paragraphs.join('') + sectionProps(defaultSectPr),
        images,
    }));
};

const buildNumberedList = (): FixtureSpec => {
    const rnd = makePrng(0x51a4);
    const blocks: string[] = [];
    for (let i = 0; i < 50_000; i++) {
        blocks.push(p(noiseText(320, rnd), { numId: 1, ilvl: 0 }));
    }
    return spec('numbered_list', '50 000 абзацев с нумерацией', packageParts({
        title: 'numbered_list',
        body: blocks.join('') + sectionProps(defaultSectPr),
        numbering: numberingXml(),
        settings: defaultSettingsXml(),
        styles: defaultStylesXml(),
    }));
};

const buildMixed = (): FixtureSpec => {
    const rnd = makePrng(0x51a5);
    const images: Record<string, Bytes> = {};
    const blocks: string[] = [];
    for (let i = 0; i < 45_000; i++) {
        blocks.push(p(noiseText(320, rnd)));
        if (i % 3000 === 0) {
            const rows: string[] = [];
            for (let r = 0; r < 40; r++) {
                rows.push(tr(Array.from({ length: 10 }, () => tc(noiseText(120, rnd))).join('')));
            }
            blocks.push(
                '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>' + rows.join('') + '</w:tbl>',
            );
            const index = Object.keys(images).length;
            const fileName = `image${index + 1}.png`;
            images[fileName] = pngBytes(160, 160, 0x5200 + index);
            blocks.push(
                '<w:p><w:r>' +
                    inlineImage(imageRelId(index), { id: index + 1, name: fileName, cx: 812800, cy: 812800 }) +
                    '</w:r></w:p>',
            );
        }
    }
    const sectPr =
        `<w:sectPr><w:headerReference w:type="default" r:id="${headerRelId(0)}"/>${defaultSectPr}</w:sectPr>`;
    return spec('mixed', 'Текст + таблицы + изображения', packageParts({
        title: 'mixed',
        body: blocks.join('') + sectPr,
        images,
        headers: {
            'header1.xml': XML_DECL + `<w:hdr xmlns:w="${W_NS}">${p('Large fixture')}</w:hdr>`,
        },
        settings: defaultSettingsXml(),
        styles: defaultStylesXml(),
    }));
};

/**
 * Генерирует 5 больших фикстур в указанный каталог.
 *
 * @param outDir каталог назначения (обычно `target/fixtures/docx-large`)
 * @returns имена файлов и их размеры
 */
export const generateLargeFixtures = async (outDir: string): Promise<LargeFixtureResult[]> => {
    // Каталог перезаписывается целиком: иначе старые большие файлы копятся.
    await rm(outDir, { recursive: true, force: true });
    await mkdir(outDir, { recursive: true });

    const specs = [
        buildManyParagraphs(),
        buildWideTable(),
        buildManyImages(),
        buildNumberedList(),
        buildMixed(),
    ];

    const results: LargeFixtureResult[] = [];
    for (const item of specs) {
        const bytes = buildZip(item.parts);
        // Приведение — расхождение дженериков Buffer/Uint8Array в @types/node.
        await writeFile(path.join(outDir, `${item.name}.docx`), bytes as Uint8Array);
        results.push({ name: item.name, bytes: bytes.length });
    }
    return results;
};
