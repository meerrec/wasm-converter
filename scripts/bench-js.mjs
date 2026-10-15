#!/usr/bin/env node
/**
 * Бенчмарк: наши wasm-конвертеры против JS-библиотек.
 *
 * Пары подобраны так, чтобы обе стороны делали одну и ту же работу:
 *   DOCX → HTML  — наш CLI (`convert x.docx -o y.html`) против `mammoth`;
 *   XLSX → PDF   — наш CLI против чтения книги `exceljs` и `xlsx` (SheetJS).
 * Вторая пара не эквивалентна: у нас нет отдельной операции «прочитать книгу»,
 * поэтому PDF-экспорт (разбор + раскладка + сериализация PDF) мерится против
 * чистого разбора в память. Отклонение зафиксировано в docs/bench-js.md.
 *
 * Обе стороны гоняются в одном процессе (кроме `ours-cli`, который меряет
 * настоящий запуск бинарника) — так из числа уходит старт Node и остаётся
 * старт wasm-модуля у нас против JIT-прогрева у библиотек. Время первого
 * вызова каждого движка пишется отдельно (`cold`), чтобы это было видно.
 *
 * Запуск: `pnpm bench:js` (см. корневой package.json).
 */

import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';

const execFileP = promisify(execFile);

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const CLI_ENTRY = join(ROOT, 'packages/cli/dist/index.js');
const CLI_BIN = join(ROOT, 'packages/cli/dist/cli.js');
const DOCX_DIR = join(ROOT, 'test-fixtures/docx');
const XLSX_DIR = join(ROOT, 'test-fixtures/xlsx');
// Большие (>10 МиБ) фикстуры генерируются в gitignored `target/`:
// `pnpm gen:docx-fixtures:large`. На мелких коммитнутых файлах (медиана 3 КиБ)
// число меряет накладные расходы, а не разбор, поэтому берём и их, если есть.
const LARGE_DOCX_DIR = join(ROOT, 'target/fixtures/docx-large');
const OUT_DIR = join(ROOT, 'target/bench-js');

/** Файлы, которые намеренно не открываются (см. test-fixtures/docx/broken). */
const SKIP_RE = /(^|[/\\])broken([/\\]|$)/;

// ─── CLI ────────────────────────────────────────────────────────────────────

function parseArgs(argv) {
  const opts = { reps: 5, sample: 24, large: true, only: null, timeoutMs: 600_000 };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const value = argv[i + 1];
    if (arg === '--reps') opts.reps = Number(value), (i += 1);
    else if (arg === '--sample') opts.sample = Number(value), (i += 1);
    else if (arg === '--only') opts.only = value, (i += 1);
    else if (arg === '--large') opts.large = true;
    else if (arg === '--no-large') opts.large = false;
    else if (arg === '--timeout') opts.timeoutMs = Number(value) * 1000, (i += 1);
    else if (arg === '--help' || arg === '-h') {
      process.stdout.write(
        'usage: pnpm bench:js [--reps N] [--sample N] [--only docx|xlsx] [--large|--no-large]\n',
      );
      process.exit(0);
    } else throw new Error(`unknown option '${arg}'`);
  }
  if (!(opts.reps >= 1)) throw new Error('--reps must be >= 1');
  return opts;
}

// ─── Выборка ────────────────────────────────────────────────────────────────

/** Все файлы с расширением `ext` под `dir`, кроме `broken/`, по убыванию размера. */
async function listFixtures(dir, ext) {
  if (!existsSync(dir)) return [];
  const entries = await readdir(dir, { recursive: true, withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (!entry.isFile() || !entry.name.endsWith(ext)) continue;
    const path = join(entry.parentPath ?? entry.path, entry.name);
    if (SKIP_RE.test(path)) continue;
    files.push({ path, size: (await stat(path)).size });
  }
  return files.sort((a, b) => b.size - a.size);
}

// ─── Движки ─────────────────────────────────────────────────────────────────

let cliModule;
async function cli() {
  cliModule ??= import(pathToFileURL(CLI_ENTRY).href);
  return cliModule;
}

const fmt = (bytes) => `${(bytes / 1024).toFixed(1)} КиБ`;

/** DOCX → HTML: наша сторона и mammoth дают один и тот же формат. */
const DOCX_ENGINES = [
  {
    id: 'wasm-cli',
    title: 'wasm (наш CLI, в процессе)',
    note: 'convertFile из @meerrec/cli',
    outExt: '.html',
    async run(input, output) {
      const { convertFile } = await cli();
      // Пустой объект — все опции PDF по умолчанию; для .html они не нужны.
      await convertFile({ input, output, format: 'html', pdf: {} });
    },
  },
  {
    id: 'wasm-bin',
    title: 'wasm (наш CLI, подпроцесс)',
    note: 'node packages/cli/dist/cli.js convert',
    outExt: '.html',
    async run(input, output) {
      await execFileP(process.execPath, [CLI_BIN, 'convert', input, '-o', output, '--force']);
    },
  },
  {
    id: 'mammoth',
    title: 'mammoth',
    note: 'convertToHtml + запись html',
    async run(input, output) {
      const mammoth = await import('mammoth');
      const { value } = await mammoth.convertToHtml({ path: input });
      await writeFile(output, value);
    },
  },
];

const XLSX_ENGINES = [
  {
    id: 'wasm-cli',
    title: 'wasm (наш CLI, в процессе)',
    note: 'convertFile, XLSX → PDF',
    outExt: '.pdf',
    async run(input, output) {
      const { convertFile } = await cli();
      await convertFile({ input, output, format: 'pdf', pdf: {} });
    },
  },
  {
    id: 'wasm-bin',
    title: 'wasm (наш CLI, подпроцесс)',
    note: 'node packages/cli/dist/cli.js convert, XLSX → PDF',
    outExt: '.pdf',
    async run(input, output) {
      await execFileP(process.execPath, [CLI_BIN, 'convert', input, '-o', output, '--force']);
    },
  },
  {
    id: 'exceljs',
    title: 'exceljs',
    note: 'Workbook.xlsx.readFile (разбор книги в память)',
    async run(input) {
      const ExcelJS = (await import('exceljs')).default;
      const workbook = new ExcelJS.Workbook();
      await workbook.xlsx.readFile(input);
      return workbook.worksheets.reduce((n, sheet) => n + sheet.rowCount, 0);
    },
  },
  {
    id: 'xlsx',
    title: 'xlsx (SheetJS)',
    note: 'XLSX.readFile (разбор книги в память)',
    async run(input) {
      // Пакет CJS: у ESM-обёртки часть экспортов не распознана и лежит в default.
      const mod = await import('xlsx');
      const XLSX = mod.default ?? mod;
      const workbook = XLSX.readFile(input);
      return workbook.SheetNames.length;
    },
  },
];

// ─── Измерение ──────────────────────────────────────────────────────────────

const now = () => Number(process.hrtime.bigint()) / 1e6;

/** Медиана и p95 (метод ближайшего ранга: при reps=5 p95 — это максимум). */
function stats(samples) {
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (q) => sorted[Math.min(sorted.length - 1, Math.ceil(q * sorted.length) - 1)];
  return {
    n: sorted.length,
    min: sorted[0],
    median: at(0.5),
    p95: at(0.95),
    max: sorted[sorted.length - 1],
  };
}

async function withTimeout(promise, ms, label) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`timeout after ${ms} ms`)), ms);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Прогон одного движка по выборке.
 *
 * Первый вызов движка вообще — «холодный»: он ловит загрузку wasm-модуля у нас
 * и первый проход JIT у библиотеки. Дальше на каждый файл идёт прогревочный
 * прогон (в статистику не входит) и `reps` измеренных.
 */
async function bench(engine, files, opts) {
  const result = { id: engine.id, title: engine.title, note: engine.note, cold: null, files: [], errors: [] };
  const scratch = join(OUT_DIR, 'out', engine.id);
  await mkdir(scratch, { recursive: true });

  for (const [index, file] of files.entries()) {
    const output = join(scratch, `${basename(file.path)}${engine.outExt ?? '.out'}`);
    const samples = [];
    try {
      if (index === 0) {
        const started = now();
        await withTimeout(engine.run(file.path, output), opts.timeoutMs, engine.id);
        result.cold = now() - started;
      } else {
        await withTimeout(engine.run(file.path, output), opts.timeoutMs, engine.id);
      }
      for (let rep = 0; rep < opts.reps; rep += 1) {
        const started = now();
        await withTimeout(engine.run(file.path, output), opts.timeoutMs, engine.id);
        samples.push(now() - started);
      }
      // Размер выхода — проверка, что стороны делают одну и ту же работу:
      // mammoth вклеивает картинки в html как base64, наш html их не несёт, и
      // на файле с картинками секунды против миллисекунд означают разное.
      let outBytes = null;
      try {
        outBytes = (await stat(output)).size;
      } catch {
        outBytes = null;
      }
      result.files.push({ ...file, ...stats(samples), outBytes });
    } catch (error) {
      // У CLI-подпроцесса в сообщении весь argv — в отчёт идёт первая строка.
      result.errors.push({ file: file.path, message: String(error.message).split('\n')[0] });
    }
    await rm(output, { force: true });
    process.stderr.write(
      `\r  ${engine.id}: ${index + 1}/${files.length} (ошибок: ${result.errors.length})`.padEnd(60),
    );
  }
  process.stderr.write('\r'.padEnd(61) + '\r');
  return result;
}

// ─── Отчёт ──────────────────────────────────────────────────────────────────

const num = (v, digits = 1) => (v === null || v === undefined ? '—' : v.toFixed(digits));

/** Коэффициент «их медиана / наша медиана» по файлам, где успешны оба движка. */
function compare(ours, theirs) {
  const byPath = new Map(ours.files.map((f) => [f.path, f]));
  const rows = [];
  for (const their of theirs.files) {
    const our = byPath.get(their.path);
    if (our) rows.push({ ...their, ours: our, speedup: their.median / our.median });
  }
  const speedups = rows.map((r) => r.speedup).sort((a, b) => a - b);
  const median = speedups.length ? speedups[Math.floor(speedups.length / 2)] : null;
  return { rows, speedups, median };
}

function table(rows, header, line) {
  return [`| ${header.join(' | ')} |`, `|${header.map(() => '---').join('|')}|`, ...rows.map(line)].join('\n');
}

/** Во сколько раз объёмы выходов расходятся, прежде чем считать пару разной работой. */
const OUTPUT_PARITY = 2;

function render(ours, theirs, opts) {
  const { rows, speedups, median } = compare(ours, theirs);
  const head = [
    'файл',
    'размер',
    `${ours.id}, медиана мс`,
    `${theirs.id}, медиана мс`,
    `${ours.id}, p95 мс`,
    `${theirs.id}, p95 мс`,
    `${ours.id}, выход`,
    `${theirs.id}, выход`,
    '×',
  ];
  const body = rows.map((r) => {
    const a = r.ours.outBytes;
    const b = r.outBytes;
    const odd = a && b && (b / a > OUTPUT_PARITY || a / b > OUTPUT_PARITY);
    return [
      `\`${basename(r.path)}\`${odd ? ' ⚠' : ''}`,
      fmt(r.size),
      num(r.ours.median),
      num(r.median),
      num(r.ours.p95),
      num(r.p95),
      a ? fmt(a) : '—',
      b ? fmt(b) : '—',
      r.speedup.toFixed(1),
    ].join(' | ');
  });
  const odd = rows.filter(
    (r) => r.ours.outBytes && r.outBytes && (r.outBytes / r.ours.outBytes > OUTPUT_PARITY || r.ours.outBytes / r.outBytes > OUTPUT_PARITY),
  );
  const lines = [
    `### ${ours.title} против ${theirs.title}`,
    '',
    `Файлов в сравнении: ${rows.length} (${opts.sampleNote}); повторов на файл: ${opts.reps}; ` +
      `в колонках — медиана и p95 по повторам.`,
    '',
    table(body, head, (r) => `| ${r} |`),
    '',
    `Коэффициент (медиана их / медиана наша), больше единицы — быстрее мы: ` +
      `медиана **×${num(median)}**, минимум ×${num(speedups[0])}, максимум ×${num(speedups[speedups.length - 1])}.`,
    '',
  ];
  let comparableMedian = median;
  let comparable = rows.length;
  if (odd.length) {
    const kept = rows.filter((r) => !odd.includes(r)).map((r) => r.speedup).sort((a, b) => a - b);
    comparable = kept.length;
    comparableMedian = kept.length ? kept[Math.floor(kept.length / 2)] : null;
    lines.push(
      `⚠ — выход одной стороны больше выхода другой более чем в ${OUTPUT_PARITY} раза ` +
        `(${odd.map((r) => `\`${basename(r.path)}\``).join(', ')}): стороны делают разный объём работы. ` +
        `Без них коэффициент: медиана **×${num(comparableMedian)}** (по ${comparable} из ${rows.length} файлов).`,
      '',
    );
  }
  return { markdown: lines.join('\n'), odd: odd.length, comparable, median, comparableMedian, speedups };
}

function renderCold(engines, kind) {
  const head = ['движок', 'первый вызов, мс', 'медиана по файлам, мс'];
  const body = engines.map((e) => {
    const medians = e.files.map((f) => f.median).sort((a, b) => a - b);
    const median = medians.length ? medians[Math.floor(medians.length / 2)] : null;
    return `${e.title} | ${num(e.cold)} | ${num(median)}`;
  });
  return [`### Холодный старт (${kind})`, '', table(body, head, (r) => `| ${r} |`), ''].join('\n');
}

function renderErrors(engines, kind) {
  const failed = engines.filter((e) => e.errors.length);
  if (!failed.length) return '';
  return [
    `### Не отработали (${kind})`,
    '',
    ...failed.map((e) => {
      const shown = e.errors.slice(0, 5).map((x) => `\`${basename(x.file)}\`: ${x.message}`);
      const rest = e.errors.length > shown.length ? `, и ещё ${e.errors.length - shown.length}` : '';
      return `- **${e.title}**: не отработал на ${e.errors.length} файлах — ${shown.join('; ')}${rest}`;
    }),
    '',
  ].join('\n');
}

// ─── main ───────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!existsSync(CLI_ENTRY) || !existsSync(CLI_BIN)) {
    throw new Error('packages/cli/dist пуст — сначала `pnpm -F @meerrec/cli build`');
  }
  await mkdir(OUT_DIR, { recursive: true });

  const sections = [];
  const report = { generatedAt: new Date().toISOString(), node: process.version, opts, kinds: {} };

  const kinds = [
    { kind: 'docx', dir: DOCX_DIR, ext: '.docx', engines: DOCX_ENGINES },
    { kind: 'xlsx', dir: XLSX_DIR, ext: '.xlsx', engines: XLSX_ENGINES },
  ];

  for (const { kind, dir, ext, engines } of kinds) {
    if (opts.only && opts.only !== kind) continue;
    const committed = await listFixtures(dir, ext);
    const sample = committed.slice(0, opts.sample);
    // Большие фикстуры идут сверх выборки: они нужны, чтобы коэффициент
    // отражал разбор, а не фиксированные накладные расходы на вызов.
    const large = opts.large && kind === 'docx' ? await listFixtures(LARGE_DOCX_DIR, ext) : [];
    const files = [...large, ...sample];
    const kindOpts = {
      ...opts,
      sampleNote:
        `топ-${sample.length} из ${committed.length} \`test-fixtures/${kind}\` по размеру` +
        (large.length ? `, плюс ${large.length} больших из \`target/fixtures/docx-large\`` : ''),
    };
    process.stderr.write(`\n${kind}: ${files.length} файлов\n`);
    const results = [];
    for (const engine of engines) {
      results.push(await bench(engine, files, kindOpts));
    }

    const pairs = [];
    for (const engine of results.slice(1).filter((e) => e.files.length)) {
      const rendered = render(results[0], engine, kindOpts);
      sections.push(rendered.markdown);
      pairs.push({
        theirs: engine.id,
        files: rendered.speedups.length,
        odd: rendered.odd,
        median: rendered.median,
        comparableMedian: rendered.comparableMedian,
      });
    }
    report.kinds[kind] = {
      files: files.map((f) => ({ path: f.path, size: f.size })),
      engines: results,
      pairs,
    };
    sections.push(renderCold(results, kind));
    const errors = renderErrors(results, kind);
    if (errors) sections.push(errors);
  }

  const markdown = [
    `Отчёт сгенерирован ${report.generatedAt}, Node ${report.node}.`,
    '',
    ...sections,
  ].join('\n');

  await writeFile(join(OUT_DIR, 'bench.json'), `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(join(OUT_DIR, 'report.md'), `${markdown}\n`);
  process.stdout.write(markdown);
  process.stderr.write(`\nJSON: ${join(OUT_DIR, 'bench.json')}\n`);
}

main().catch((error) => {
  process.stderr.write(`\nbench-js: ${error.stack}\n`);
  process.exitCode = 1;
});
