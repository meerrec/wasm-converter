// Генератор фикстур DOCX.
//
// ZIP собирается своим writer'ом (scripts/docx-zip.ts) с фиксированными
// таймстампами и порядком записей, поэтому повторный запуск даёт побайтово
// те же файлы: `git status --porcelain test-fixtures/docx` после второго
// прогона пуст.
//
// Запуск:
//   npx tsx scripts/generate_docx_fixtures.ts           # коммитируемые фикстуры
//   npx tsx scripts/generate_docx_fixtures.ts --large   # только «большие» (>= 10 МиБ, в target/)
//   npx tsx scripts/generate_docx_fixtures.ts --large --only profile_50mib   # одну из них
//   npx tsx scripts/generate_docx_fixtures.ts --strict --out <dir> --only a,b   # Strict-подмножество

import { mkdir, readdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { buildZip } from './docx-zip.js';
import {
    type FixtureSpec,
    withDocTitle,
    A_NS,
    PIC_NS,
    R_NS,
    W_NS,
    WP_NS,
} from './docx-fixtures/kit.js';
import { legacyFixtures } from './docx-fixtures/legacy.js';
import { basicFixtures } from './docx-fixtures/basic.js';
import { stylesFixtures } from './docx-fixtures/styles.js';
import { numberingFixtures } from './docx-fixtures/numbering.js';
import { tablesFixtures } from './docx-fixtures/tables.js';
import { tablesLayoutFixtures } from './docx-fixtures/tables-layout.js';
import { imagesFixtures } from './docx-fixtures/images.js';
import { alternateContentFixtures } from './docx-fixtures/alternate-content.js';
import { trackChangesFixtures } from './docx-fixtures/track-changes.js';
import { fieldsFixtures } from './docx-fixtures/fields.js';
import { rtlFixtures } from './docx-fixtures/rtl.js';
import { cjkFixtures } from './docx-fixtures/cjk.js';
import { headersFootersFixtures } from './docx-fixtures/headers-footers.js';
import { notesFixtures } from './docx-fixtures/notes.js';
import { a11yFixtures } from './docx-fixtures/a11y.js';
import { brokenFixtures } from './docx-fixtures/broken.js';
import { generateLargeFixtures, largeFixtureNames } from './docx-fixtures/large.js';

const ROOT = path.resolve(import.meta.dirname, '..');
const OUT_DIR = path.join(ROOT, 'test-fixtures', 'docx');
const LARGE_DIR = path.join(ROOT, 'target', 'fixtures', 'docx-large');

const MIB = 1024 * 1024;

/**
 * Transitional → Strict: пары namespace'ов, которые переписываются в
 * `--strict`-прогоне (ADR-0034). Namespace пакета (`package/2006/…`) и
 * content types у обоих вариантов одни и те же и не трогаются.
 */
const STRICT_PAIRS: ReadonlyArray<readonly [string, string]> = [
    [W_NS, 'http://purl.oclc.org/ooxml/wordprocessingml/main'],
    [A_NS, 'http://purl.oclc.org/ooxml/drawingml/main'],
    [WP_NS, 'http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing'],
    [PIC_NS, 'http://purl.oclc.org/ooxml/drawingml/picture'],
    [R_NS, 'http://purl.oclc.org/ooxml/officeDocument/relationships'],
];

/** Переписать namespace'ы XML-части на Strict. */
const toStrict = (content: string): string => {
    let out = content;
    for (const [from, to] of STRICT_PAIRS) {
        out = out.replaceAll(from, to);
    }
    return out;
};

/** Минимум коммитируемых фикстур, который обязан собрать генератор. */
const MIN_COMMITTABLE = 95;

/** Все коммитируемые фикстуры; пути уникальны и не пересекаются между файлами. */
export const committableFixtures = (): FixtureSpec[] => [
    ...legacyFixtures,
    ...basicFixtures,
    ...stylesFixtures,
    ...numberingFixtures,
    ...tablesFixtures,
    ...tablesLayoutFixtures,
    ...imagesFixtures,
    ...alternateContentFixtures,
    ...trackChangesFixtures,
    ...fieldsFixtures,
    ...rtlFixtures,
    ...cjkFixtures,
    ...headersFootersFixtures,
    ...notesFixtures,
    ...a11yFixtures,
    ...brokenFixtures,
];

const checkUnique = (specs: FixtureSpec[]): void => {
    const seen = new Set<string>();
    for (const spec of specs) {
        if (seen.has(spec.name)) throw new Error(`duplicate fixture name: ${spec.name}`);
        if (!spec.name.includes('/')) throw new Error(`fixture without category prefix: ${spec.name}`);
        seen.add(spec.name);
    }
};

/**
 * Пишет `.docx` и сайдкар `.json`.
 *
 * Абзацы тела считаются вручную в описании фикстуры (`expectedParagraphs`) —
 * генератор намеренно не разбирает XML: он должен остаться тупым писателем.
 */
const writeFixture = async (
    spec: FixtureSpec,
    outDir: string,
    sidecar: boolean,
    strict: boolean,
): Promise<void> => {
    const docTitle = spec.docTitle ?? spec.description;
    const docxPath = path.join(outDir, `${spec.name}.docx`);
    await mkdir(path.dirname(docxPath), { recursive: true });
    if (strict && spec.bytes !== undefined) {
        throw new Error(
            `--strict: ${spec.name} собирается готовыми байтами и не переписывается на Strict`,
        );
    }
    // `bytes` — готовый файл (обрезанный ZIP); обычный путь — сборка из частей.
    // В `--strict`-прогоне XML-части переписываются на purl-namespace'ы, бинарные
    // (картинки) остаются как есть.
    const parts = strict
        ? spec.parts.map(
              ([name, content]): [string, string | Uint8Array | Buffer] => [
                  name,
                  typeof content === 'string' ? toStrict(content) : content,
              ],
          )
        : spec.parts;
    const docx = spec.bytes ?? buildZip(withDocTitle(parts, docTitle));
    // Приведение — расхождение дженериков Buffer/Uint8Array в @types/node.
    await writeFile(docxPath, docx as Uint8Array);

    if (!sidecar) return;
    const metadata: Record<string, unknown> = {
        name: spec.name,
        category: spec.name.split('/')[0],
        description: spec.description,
        docTitle,
        expectedParagraphs: spec.expectedParagraphs,
        expectedTables: spec.expectedTables,
        expectedImages: spec.expectedImages ?? 0,
        expectedWarnings: spec.expectedWarnings ?? [],
        ...spec.meta,
    };
    const payload = { metadata, content: spec.content ?? {} };
    await writeFile(path.join(outDir, `${spec.name}.json`), `${JSON.stringify(payload, null, 2)}\n`);
};

/**
 * Файлы каталога фикстур, которые генератор не создавал и потому не удаляет.
 *
 * `mammoth-oracle.json` кладёт differential-тест против mammoth
 * (`scripts/diff-mammoth.ts`): каталог выбран им по смыслу, но генератор
 * владеет только своими фикстурами и сайдкарами, а чужой артефакт в каталоге —
 * не его дело. Остальное, что генератор создаёт и чего больше нет в списке,
 * чистка удаляет как раньше.
 */
const FOREIGN_FILES = new Set<string>(['mammoth-oracle.json']);

/** Каталоги внутри каталога фикстур, которыми генератор не владеет. */
const FOREIGN_DIRS = new Set<string>(['strict']);

/** Убирает из каталога сгенерированные ранее файлы, которых больше нет в списке. */
const pruneStale = async (outDir: string, expected: Set<string>): Promise<number> => {
    let removed = 0;
    const walk = async (dir: string): Promise<void> => {
        const entries = await readdir(dir, { withFileTypes: true });
        for (const entry of entries) {
            const full = path.join(dir, entry.name);
            if (entry.isDirectory()) {
                if (FOREIGN_DIRS.has(entry.name)) {
                    continue;
                }
                await walk(full);
                continue;
            }
            if (!/\.(docx|json)$/.test(entry.name)) continue;
            const relative = path.relative(outDir, full);
            if (FOREIGN_FILES.has(relative)) continue;
            if (!expected.has(relative)) {
                await rm(full);
                removed++;
            }
        }
    };
    await walk(outDir);
    return removed;
};

const generate = async (
    outDir: string,
    only: string[] | undefined,
    strict: boolean,
): Promise<void> => {
    let specs = committableFixtures();
    if (only !== undefined) {
        const wanted = new Set(only);
        const unknown = only.filter((name) => !specs.some((spec) => spec.name === name));
        if (unknown.length > 0) {
            throw new Error(`unknown fixture names: ${unknown.join(', ')}`);
        }
        specs = specs.filter((spec) => wanted.has(spec.name));
    }
    checkUnique(specs);

    await mkdir(outDir, { recursive: true });
    const byCategory = new Map<string, number>();
    for (const spec of specs) {
        const category = spec.name.split('/')[0]!;
        byCategory.set(category, (byCategory.get(category) ?? 0) + 1);
        await writeFixture(spec, outDir, true, strict);
    }

    // Сайдкары пишутся и в strict-прогоне: модель от варианта не зависит,
    // ожидания те же, а тесты фикстур требуют пару `.docx`/`.json`.
    const expected = new Set(specs.flatMap((s) => [`${s.name}.docx`, `${s.name}.json`]));
    const removed = await pruneStale(outDir, expected);

    console.log(
        `DOCX fixtures: ${specs.length} (strict: ${strict}, stale removed: ${removed}) → ${path.relative(ROOT, outDir)}`,
    );
    for (const [category, count] of [...byCategory.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
        console.log(`  ${category.padEnd(18)} ${count}`);
    }
    if (specs.length < MIN_COMMITTABLE && only === undefined) {
        throw new Error(`expected at least ${MIN_COMMITTABLE} committable fixtures, got ${specs.length}`);
    }
};

/** Печатает ошибку использования и ставит код возврата 1 — без стектрейса. */
const die = (message: string): void => {
    console.error(message);
    process.exitCode = 1;
};

const main = async (): Promise<void> => {
    const large = process.argv.includes('--large');
    const strict = process.argv.includes('--strict');
    const outAt = process.argv.indexOf('--out');
    const rawOut = outAt === -1 ? undefined : process.argv[outAt + 1];
    const onlyAt = process.argv.indexOf('--only');
    // Значение не может начинаться с «-»: так `--only --large` (потерянное имя)
    // не превращается в запрос фикстуры с именем «--large», а падает.
    const rawOnly = onlyAt === -1 ? undefined : process.argv[onlyAt + 1];
    const only = rawOnly === undefined || rawOnly.startsWith('-') ? undefined : rawOnly;

    const outDir =
        rawOut === undefined || rawOut.startsWith('-') ? undefined : path.resolve(ROOT, rawOut);

    if (large) {
        if (strict || outAt !== -1) {
            die('--strict and --out work only with the committable fixtures');
            return;
        }
        if (onlyAt !== -1 && only === undefined) {
            die('--only requires a fixture name');
            return;
        }
        if (only !== undefined) {
            const names = largeFixtureNames();
            if (!names.includes(only)) {
                die(`unknown large fixture: ${only}\navailable: ${names.join(', ')}`);
                return;
            }
        }

        const written = await generateLargeFixtures(LARGE_DIR, only);
        console.log(`Large DOCX fixtures: ${written.length} → ${LARGE_DIR}`);
        for (const { name, bytes } of written) {
            console.log(`  ${name.padEnd(28)} ${(bytes / MIB).toFixed(1)} MiB`);
        }
        const small = written.find(({ bytes, minBytes }) => bytes < minBytes);
        if (small !== undefined) {
            throw new Error(
                `${small.name}: ${(small.bytes / MIB).toFixed(1)} MiB, expected at least ${small.minBytes / MIB} MiB`,
            );
        }
        return;
    }

    if (outAt !== -1 && outDir === undefined) {
        die('--out requires a directory');
        return;
    }
    if (strict && outDir === undefined) {
        die('--strict requires --out: committed fixtures must stay Transitional');
        return;
    }
    if (onlyAt !== -1 && only === undefined) {
        die('--only requires a fixture name');
        return;
    }
    if (onlyAt !== -1 && !strict) {
        die('--only works only together with --large or --strict: committable fixtures are always written in full');
        return;
    }

    await generate(outDir ?? OUT_DIR, only?.split(','), strict);
};

main().catch((error: unknown) => {
    console.error(error);
    process.exitCode = 1;
});
