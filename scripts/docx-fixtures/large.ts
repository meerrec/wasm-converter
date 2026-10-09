// «Большие» фикстуры (>= 10 МиБ) для бюджетов памяти и скорости.
//
// В git они не попадают: каталог лежит под `target/`, который в .gitignore.
// Отдельная функция, а не часть общего списка, чтобы обычный прогон генератора
// оставался быстрым и не тянул сотни мегабайт в рабочее дерево.
//
// Размер набирается «шумом» из детерминированного PRNG: осмысленный текст
// deflate сжимает в сотни раз, а случайные слова — лишь до ~0.75, поэтому
// итоговый файл действительно большой.
//
// Последние две фикстуры — профили бюджетов ROADMAP §9 (время `openDocx` и
// пиковая память). Их состав задан роадмапом, поэтому за размер отвечает длина
// «шума», а не число абзацев или картинок.

import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { type Bytes, buildZip, makePrng, pngBytes } from '../docx-zip.js';
import {
    XML_DECL,
    W_NS,
    defaultSectPr,
    defaultSettingsXml,
    defaultStylesXml,
    headerRelId,
    inlineImage,
    imageRelId,
    p,
    packageParts,
    para,
    run,
    sectionProps,
    tc,
    tr,
} from './kit.js';

/** Части пакета: путь в ZIP → содержимое. */
type PackageParts = Array<[string, string | Bytes]>;

/** Итог генерации большой фикстуры. */
export interface LargeFixtureResult {
    name: string;
    bytes: number;
    /** Пол размера: ниже него фикстура вырождена, и бюджеты по ней не мерятся. */
    minBytes: number;
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

const MIB = 1024 * 1024;

/** Пол «больших» фикстур: они нужны бюджетами, а не полнотой покрытия. */
const MIN_LARGE_BYTES = 10 * MIB;

/**
 * Пол фикстур-профилей бюджетов: 50 МиБ из ROADMAP §9 плюс запас — на самом
 * пороге проверка мигала бы от любого шевеления профиля.
 */
const MIN_BUDGET_BYTES = 50 * MIB;

/**
 * Предел распакованного размера одной части из ADR-0015: `ZipLimits::default()
 * .per_part_uncompressed` в `crates/core/src/zip_limits.rs`. Генератор не может
 * спросить значение у Rust-крейта, поэтому держит копию — она проверяется
 * после сборки архива.
 */
const MAX_PART_BYTES = 64 * MIB;

/** Подпись конца центрального каталога ZIP ("PK\x05\x06"). */
const EOCD_SIG = Buffer.from([0x50, 0x4b, 0x05, 0x06]);
const CENTRAL_SIG = 0x02014b50;

/**
 * Самая тяжёлая распакованная часть архива.
 *
 * Лимит ADR-0015 считается по распакованным байтам, а не по размеру файла:
 * фикстура на 54 МиБ, у которой `word/document.xml` разворачивается в 78 МиБ,
 * парсером не открывается вовсе — гейт бюджета на ней просто не запустится.
 *
 * @param zip готовый архив
 * @returns имя части и её распакованный размер
 */
const largestPart = (zip: Buffer): { name: string; size: number } => {
    const eocd = zip.lastIndexOf(EOCD_SIG);
    if (eocd === -1) throw new Error('no ZIP end of central directory record');
    const count = zip.readUInt16LE(eocd + 10);
    let at = zip.readUInt32LE(eocd + 16);
    let biggest = { name: '', size: 0 };
    for (let i = 0; i < count; i++) {
        if (zip.readUInt32LE(at) !== CENTRAL_SIG) throw new Error('broken ZIP central directory');
        const size = zip.readUInt32LE(at + 24);
        const nameLength = zip.readUInt16LE(at + 28);
        if (size > biggest.size) {
            biggest = { name: zip.toString('utf8', at + 46, at + 46 + nameLength), size };
        }
        at += 46 + nameLength + zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32);
    }
    return biggest;
};

const numberingXml = (): string =>
    XML_DECL +
    `<w:numbering xmlns:w="${W_NS}">` +
    '<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="multilevel"/>' +
    '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/>' +
    '<w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>' +
    '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>';

const buildManyParagraphs = (): PackageParts => {
    const rnd = makePrng(0x51a1);
    const blocks: string[] = [];
    for (let i = 0; i < 100_000; i++) {
        blocks.push(p(noiseText(170, rnd)));
    }
    return packageParts({
        title: 'many_paragraphs',
        body: blocks.join('') + sectionProps(defaultSectPr),
    });
};

const buildWideTable = (): PackageParts => {
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
    return packageParts({
        title: 'wide_table',
        body: tableXml + sectionProps(defaultSectPr),
    });
};

const buildManyImages = (): PackageParts => {
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
    return packageParts({
        title: 'many_images',
        body: paragraphs.join('') + sectionProps(defaultSectPr),
        images,
    });
};

const buildNumberedList = (): PackageParts => {
    const rnd = makePrng(0x51a4);
    const blocks: string[] = [];
    for (let i = 0; i < 50_000; i++) {
        blocks.push(p(noiseText(320, rnd), { numId: 1, ilvl: 0 }));
    }
    return packageParts({
        title: 'numbered_list',
        body: blocks.join('') + sectionProps(defaultSectPr),
        numbering: numberingXml(),
        settings: defaultSettingsXml(),
        styles: defaultStylesXml(),
    });
};

const buildMixed = (): PackageParts => {
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
    return packageParts({
        title: 'mixed',
        body: blocks.join('') + sectPr,
        images,
        headers: {
            'header1.xml': XML_DECL + `<w:hdr xmlns:w="${W_NS}">${p('Large fixture')}</w:hdr>`,
        },
        settings: defaultSettingsXml(),
        styles: defaultStylesXml(),
    });
};

// ==================== Профили бюджетов ROADMAP §9 ====================
//
// Числа профилей менять нельзя: под них подписаны бюджеты времени и памяти.
// Размер набирается длиной «шума» — она подобрана эмпирически так, чтобы файл
// попадал в 52–60 МиБ, то есть с запасом над порогом 50 МиБ.

/** Профиль бюджета времени `openDocx`: 100 000 абзацев, 500 таблиц, 50 картинок. */
const PROFILE_PARAGRAPHS = 100_000;
const PROFILE_TABLES = 500;
const PROFILE_IMAGES = 50;
/**
 * ~350 знаков на абзац: шум из PRNG deflate жмёт лишь до ~0.75, поэтому текст
 * даёт ~42 МиБ распакованного `word/document.xml` — это ниже лимита на часть
 * (ADR-0015, 64 МиБ) с запасом на рост профиля. Вес сверх того несут картинки:
 * они лежат отдельными частями и лимит на `document.xml` не расходуют.
 */
const PROFILE_PARAGRAPH_CHARS = 350;
const PROFILE_TABLE_ROWS = 6;
const PROFILE_TABLE_CELLS = 4;
const PROFILE_CELL_CHARS = 240;
/** 448 px: шумовой PNG весит ~588 КиБ, 50 штук дают ~29 МиБ медиа. */
const PROFILE_IMAGE_PX = 448;
/** 448 px при 96 dpi: 448 / 96 × 914400 — размер картинки на странице. */
const PROFILE_IMAGE_EMU = 4_266_240;
/** 100 000 / 500 = 200 — таблица каждые 200 абзацев, ровно 500 штук. */
const PROFILE_TABLE_EVERY = PROFILE_PARAGRAPHS / PROFILE_TABLES;
/** 100 000 / 50 = 2000 — картинка каждые 2000 абзацев, ровно 50 штук. */
const PROFILE_IMAGE_EVERY = PROFILE_PARAGRAPHS / PROFILE_IMAGES;

/** Одна таблица профиля: без границ — считаются абзацы и ячейки, а не вид. */
const profileTable = (rnd: () => number): string => {
    const rows: string[] = [];
    for (let r = 0; r < PROFILE_TABLE_ROWS; r++) {
        const cells: string[] = [];
        for (let c = 0; c < PROFILE_TABLE_CELLS; c++) cells.push(tc(noiseText(PROFILE_CELL_CHARS, rnd)));
        rows.push(tr(cells.join('')));
    }
    return '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>' + rows.join('') + '</w:tbl>';
};

const buildProfile50MiB = (): PackageParts => {
    const rnd = makePrng(0x51a6);
    const images: Record<string, Bytes> = {};
    const blocks: string[] = [];
    let imageIndex = 0;
    for (let i = 0; i < PROFILE_PARAGRAPHS; i++) {
        let runs = run(noiseText(PROFILE_PARAGRAPH_CHARS, rnd));
        if (i % PROFILE_IMAGE_EVERY === 0) {
            const fileName = `image${imageIndex + 1}.png`;
            images[fileName] = pngBytes(PROFILE_IMAGE_PX, PROFILE_IMAGE_PX, 0x5100 + imageIndex);
            runs +=
                '<w:r>' +
                inlineImage(imageRelId(imageIndex), {
                    id: imageIndex + 1,
                    name: fileName,
                    cx: PROFILE_IMAGE_EMU,
                    cy: PROFILE_IMAGE_EMU,
                }) +
                '</w:r>';
            imageIndex++;
        }
        // Картинка лежит в том же абзаце, что и текст: абзацев ровно 100 000,
        // а не «100 000 плюс по абзацу на каждую картинку».
        blocks.push(para('', runs));
        if (i % PROFILE_TABLE_EVERY === 0) blocks.push(profileTable(rnd));
    }
    return packageParts({
        title: 'profile_50mib',
        body: blocks.join('') + sectionProps(defaultSectPr),
        images,
    });
};

/** Профиль бюджета памяти: ≥ 50 МиБ, вес набирают картинки, а не текст. */
const MEMORY_PARAGRAPHS = 6_000;
const MEMORY_IMAGES = 75;
const MEMORY_PARAGRAPH_CHARS = 200;
/** 512 px: шумовой PNG весит ~770 КиБ, 75 штук дают ~56 МиБ. */
const MEMORY_IMAGE_PX = 512;
/** 512 px при 96 dpi: 512 / 96 × 914400 — размер картинки на странице. */
const MEMORY_IMAGE_EMU = 4_876_800;
/** 6 000 / 75 = 80 — картинка каждые 80 абзацев, ровно 75 штук. */
const MEMORY_IMAGE_EVERY = MEMORY_PARAGRAPHS / MEMORY_IMAGES;

const buildMemory50MiB = (): PackageParts => {
    const rnd = makePrng(0x51a7);
    const images: Record<string, Bytes> = {};
    const blocks: string[] = [];
    let imageIndex = 0;
    for (let i = 0; i < MEMORY_PARAGRAPHS; i++) {
        let runs = run(noiseText(MEMORY_PARAGRAPH_CHARS, rnd));
        if (i % MEMORY_IMAGE_EVERY === 0) {
            const fileName = `image${imageIndex + 1}.png`;
            images[fileName] = pngBytes(MEMORY_IMAGE_PX, MEMORY_IMAGE_PX, 0x5300 + imageIndex);
            runs +=
                '<w:r>' +
                inlineImage(imageRelId(imageIndex), {
                    id: imageIndex + 1,
                    name: fileName,
                    cx: MEMORY_IMAGE_EMU,
                    cy: MEMORY_IMAGE_EMU,
                }) +
                '</w:r>';
            imageIndex++;
        }
        blocks.push(para('', runs));
    }
    return packageParts({
        title: 'memory_50mib',
        body: blocks.join('') + sectionProps(defaultSectPr),
        images,
    });
};

/** Большая фикстура: имя файла, пол размера и ленивый билдер частей пакета. */
interface LargeFixture {
    name: string;
    minBytes: number;
    build: () => PackageParts;
}

/** Большая фикстура общего пола размера. */
const large = (name: string, build: () => PackageParts): LargeFixture => ({
    name,
    minBytes: MIN_LARGE_BYTES,
    build,
});

/** Фикстура-профиль бюджета: на ней стоят замеры времени или памяти ROADMAP §9. */
const budget = (name: string, build: () => PackageParts): LargeFixture => ({
    name,
    minBytes: MIN_BUDGET_BYTES,
    build,
});

/**
 * Все большие фикстуры в порядке генерации.
 *
 * Таблица имя → билдер, а не список спеков: по имени фильтрует `--only`, и
 * фильтр обязан сработать до сборки — иначе запрос одной маленькой фикстуры
 * собирал бы десятки мегабайт профиля впустую.
 */
const LARGE_FIXTURES: readonly LargeFixture[] = [
    large('many_paragraphs', buildManyParagraphs),
    large('wide_table', buildWideTable),
    large('many_images', buildManyImages),
    large('numbered_list', buildNumberedList),
    large('mixed', buildMixed),
    budget('profile_50mib', buildProfile50MiB),
    budget('memory_50mib', buildMemory50MiB),
];

/** Имена больших фикстур в порядке генерации — для подсказки в CLI. */
export const largeFixtureNames = (): string[] => LARGE_FIXTURES.map((fixture) => fixture.name);

/**
 * Генерирует большие фикстуры в указанный каталог.
 *
 * @param outDir каталог назначения (обычно `target/fixtures/docx-large`)
 * @param only имя единственной фикстуры; без него каталог перезаписывается
 *   целиком, иначе старые большие файлы копятся. С ним остальные файлы
 *   каталога остаются на месте — ими пользуются бюджеты соседних профилей.
 * @returns имена файлов, размеры и полы размера
 */
export const generateLargeFixtures = async (outDir: string, only?: string): Promise<LargeFixtureResult[]> => {
    const selected = only === undefined ? LARGE_FIXTURES : LARGE_FIXTURES.filter((f) => f.name === only);
    if (selected.length === 0) {
        throw new Error(`unknown large fixture: ${only}\navailable: ${largeFixtureNames().join(', ')}`);
    }
    if (only === undefined) {
        await rm(outDir, { recursive: true, force: true });
    }
    await mkdir(outDir, { recursive: true });

    const results: LargeFixtureResult[] = [];
    for (const fixture of selected) {
        const bytes = buildZip(fixture.build());
        const largest = largestPart(bytes);
        if (largest.size > MAX_PART_BYTES) {
            throw new Error(
                `${fixture.name}: part ${largest.name} unpacks to ${(largest.size / MIB).toFixed(1)} MiB, ` +
                    `over the ${MAX_PART_BYTES / MIB} MiB per-part limit (ADR-0015)`,
            );
        }
        // Приведение — расхождение дженериков Buffer/Uint8Array в @types/node.
        await writeFile(path.join(outDir, `${fixture.name}.docx`), bytes as Uint8Array);
        results.push({ name: fixture.name, bytes: bytes.length, minBytes: fixture.minBytes });
    }
    return results;
};
