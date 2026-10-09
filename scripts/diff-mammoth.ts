// Дифференциальный тест парсера DOCX против mammoth — независимого оракула.
//
// Наш разбор берётся готовым примером `crates/docx/examples/dump_model.rs`
// (он же — контракт нормализации), mammoth читает те же байты своим,
// никак не связанным с нами кодом. Совпадение списков абзацев на десятках
// фикстур — то, что нельзя получить, сверяя парсер с самим собой.
//
//     npx tsx scripts/diff-mammoth.ts                  # сухой прогон, файлы не пишутся
//     npx tsx scripts/diff-mammoth.ts --update-oracle  # перезаписать оракул и отчёт
//     npx tsx scripts/diff-mammoth.ts --check          # сверить пересчёт с репозиторием
//
// `--check` пересчитывает всё заново и сверяет байты обоих файлов с тем, что
// лежит в репозитории: прогон обязан быть идемпотентным.
//
// Коды возврата: 0 — гейт пройден (и `--check` сошёлся), 1 — провал гейта или
// расхождение с репозиторием, 2 — инфраструктурная ошибка (cargo, mammoth).
//
// Нормализация — контракт, дословно тот же, что в
// `crates/docx/examples/dump_model.rs`, и продублирован здесь:
//
// - обход в порядке документа с рекурсией в таблицы (строки, ячейки, их
//   блоки — в тот же плоский список);
// - текст абзаца = конкатенация: текст прогонов как есть, `w:tab` → `"\t"`,
//   разрывы/символы/рисунки/неизвестное → ничего; внутри гиперссылки и
//   результата поля — рекурсивно;
// - `trim()`, пустые строки выброшены;
// - сноски, концевые сноски, комментарии и колонтитулы в список не входят.
//
// На стороне mammoth это `extractRawText` (`\n\n` между абзацами), он же —
// вход в оракул: наш список абзацев в оракул НЕ кладётся, иначе проверка
// (Rust-тест) стала бы тавтологией. Счётчики `model.counts` — диагностика
// отчёта и в гейт не идут.

import { execFile } from 'node:child_process';
import { readdirSync, statSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

const ROOT = path.resolve(import.meta.dirname, '..');
const FIXTURE_ROOT = path.join(ROOT, 'test-fixtures', 'docx');
const ORACLE_PATH = path.join(FIXTURE_ROOT, 'mammoth-oracle.json');
const REPORT_PATH = path.join(ROOT, 'docs', 'sprint-8', 'diff-report.md');

/** Потолок на `cargo run`: сборка примера на холодную — это десятки секунд. */
const DUMP_TIMEOUT_MS = 5 * 60 * 1000;

/** Доля совпадений, ниже которой гейт не проходит (ROADMAP §9). */
const THRESHOLD = 0.95;
/** Нижняя граница сопоставимых: на десяти фикстурах доля ничего не значит. */
const MIN_COMPARABLE = 50;
/** Потолок на `non-goal`: исключать из знаменателя можно только осознанно. */
const MAX_NON_GOAL = 8;
/** Кандидатов ровно столько; разошлось — это ошибка набора, а не «ладно». */
const EXPECTED_CANDIDATES = 76;

/**
 * Категории вне набора кандидатов и почему — причина называет сторону:
 *
 * - `broken/` — битые пакеты: наш разбор (и его политика ошибок, ADR-0016) —
 *   предмет отдельных тестов, сверка текста на них ничего не значит;
 * - `alternate_content/` — mammoth читает `mc:Fallback`, мы `mc:Choice`:
 *   сравнивать нечего, это разные ветки одного и того же;
 * - `track_changes/` — правки у нас non-goal (ревизии не применяются);
 * - `notes/` — сноски и комментарии mammoth выносит отдельно от тела.
 */
const EXCLUDED: Record<string, string> = {
  broken: 'битые пакеты: политика ошибок (ADR-0016), а не сверка текста',
  alternate_content: 'mammoth читает mc:Fallback, мы — mc:Choice: это разные ветки',
  track_changes: 'правки (w:ins/w:del) у нас non-goal, ревизии не применяются',
  notes: 'сноски и комментарии mammoth выносит отдельно от тела документа',
};

type Kind = 'expected' | 'non-goal' | 'bug';

interface Classification {
  kind: Kind;
  /** Непустая причина: называет сторону (наша модель / mammoth) и решение. */
  reason: string;
}

/** Почему `w:fldSimple` — расхождение на стороне mammoth, а не наш дефект. */
const FLD_SIMPLE =
  'mammoth не читает w:fldSimple (элемента нет в карте элементов body-reader.js, ' +
  'неизвестный элемент отбрасывается вместе с детьми) — расхождение на его стороне';

/**
 * Таблица расхождений — единственный источник классов, кроме вычисляемого
 * `match`. Всё, чего здесь нет, но что разошлось, — `unclassified`, и гейт
 * такое не пропускает: молчаливое «ну почти» здесь не работает.
 */
const CLASSIFICATION: Record<string, Classification> = {
  // mammoth читает только сложные поля (`w:fldChar` + `w:instrText`) и не знает
  // `w:fldSimple` вовсе: в его карте элементов (`lib/docx/body-reader.js`) такого
  // имени нет, а неизвестный элемент отбрасывается вместе с детьми — то есть
  // вместе с кэшем поля, который в документе и виден. Мы кэш читаем
  // (`Inline::Field::result`), и на этих фикстурах расходимся ровно на нём.
  'fields/fld_simple': { kind: 'non-goal', reason: FLD_SIMPLE },
  'fields/date_field': { kind: 'non-goal', reason: FLD_SIMPLE },
};

/** Срез API mammoth, который здесь используется (1.13 деклараций не несёт). */
interface Mammoth {
  extractRawText(input: { buffer: Buffer }): Promise<{ value: string }>;
  convertToHtml(input: { buffer: Buffer }): Promise<{ value: string }>;
}

interface MammothCounts {
  p: number;
  tables: number;
  list_items: number;
  hyperlinks: number;
}

interface ModelCounts {
  tables: number;
  list_items: number;
  hyperlinks: number;
  images: number;
}

interface ModelFixture {
  paragraphs: string[];
  tables: number;
  list_items: number;
  hyperlinks: number;
  images: number;
}

interface Entry {
  /** Ключ фикстуры: путь относительно `test-fixtures/docx` без расширения. */
  key: string;
  /** `false` — фикстура вне набора кандидатов (категория из `EXCLUDED`). */
  inSet: boolean;
  /** `true` — фикстура в наборе и mammoth её прочитал; только такие считаются. */
  comparable: boolean;
  /** Причина несопоставимости; `null` у сопоставимых. */
  skipReason: string | null;
  /** `null` только у несопоставимых. */
  classification: 'match' | Kind | 'unclassified' | null;
  reason: string;
  mammoth: { paragraphs: string[]; counts: MammothCounts } | null;
  model: { counts: ModelCounts } | null;
  /** Наш список абзацев — только для отчёта, в оракул не попадает. */
  modelParagraphs: string[];
  /** Модель не разобрала фикстуру (`null` в дампе). */
  modelFailed: boolean;
}

/**
 * Собрать все `.docx` под `dir`; ключ — путь относительно `dir` без расширения.
 * Порядок задаёт вызывающий сортировкой, а не файловая система.
 */
function discover(dir: string, prefix: string, out: string[]): void {
  for (const name of readdirSync(dir)) {
    const full = path.join(dir, name);
    const nested = prefix === '' ? name : `${prefix}/${name}`;
    if (statSync(full).isDirectory()) {
      discover(full, nested, out);
    } else if (name.endsWith('.docx')) {
      out.push(nested.slice(0, -'.docx'.length));
    }
  }
}

/**
 * Запустить пример-дамп нашего разбора и разобрать его stdout как JSON.
 *
 * Ненулевой код возврата или не-JSON — инфраструктурная ошибка (2): молча
 * продолжать с пустой моделью значило бы объявить совпадением отсутствие
 * данных.
 */
async function runModelDump(): Promise<Record<string, ModelFixture | null>> {
  let stdout: string;
  try {
    ({ stdout } = await execFileAsync(
      'cargo',
      ['run', '-q', '-p', 'doc-converter-docx', '--example', 'dump_model'],
      { cwd: ROOT, timeout: DUMP_TIMEOUT_MS, maxBuffer: 64 * 1024 * 1024 },
    ));
  } catch (error) {
    const failed = error as { stderr?: string; message: string };
    process.stderr.write(`cargo run --example dump_model упал: ${failed.message}\n`);
    if (failed.stderr) process.stderr.write(failed.stderr);
    process.exit(2);
  }
  try {
    return JSON.parse(stdout) as Record<string, ModelFixture | null>;
  } catch {
    process.stderr.write('stdout dump_model — не JSON:\n');
    process.stderr.write(`${stdout.slice(0, 2000)}\n`);
    process.exit(2);
  }
}

/** Абзацы mammoth: тот же `trim` и тот же выброс пустых, что у нас. */
function paragraphsOf(raw: string): string[] {
  return raw
    .split('\n\n')
    .map((paragraph) => paragraph.trim())
    .filter((paragraph) => paragraph !== '');
}

/** Счётчики HTML — диагностика отчёта, в гейт не идут. */
function countsOfHtml(html: string): MammothCounts {
  const count = (needle: string) => html.split(needle).length - 1;
  return {
    p: count('<p'),
    tables: count('<table'),
    list_items: count('<li'),
    hyperlinks: count('<a '),
  };
}

function readMammothVersion(): string {
  const require_ = createRequire(import.meta.url);
  const packageJson = require_('mammoth/package.json') as { version: string };
  return packageJson.version;
}

/** Прогнать одну фикстуру через mammoth; отказ — причина, а не падение. */
async function readMammoth(
  mammoth: Mammoth,
  file: string,
): Promise<{ paragraphs: string[]; counts: MammothCounts } | { error: string }> {
  const buffer = await readFile(file);
  try {
    const [raw, html] = await Promise.all([
      mammoth.extractRawText({ buffer }),
      mammoth.convertToHtml({ buffer }),
    ]);
    return { paragraphs: paragraphsOf(raw.value), counts: countsOfHtml(html.value) };
  } catch (error) {
    return { error: (error as Error).message };
  }
}

function classify(entry: Entry): void {
  if (!entry.comparable) {
    entry.classification = null;
    entry.reason = entry.skipReason ?? '';
    return;
  }
  const ours = entry.modelFailed ? null : entry.modelParagraphs;
  const theirs = entry.mammoth?.paragraphs ?? null;
  if (ours !== null && theirs !== null && sameParagraphs(ours, theirs)) {
    entry.classification = 'match';
    entry.reason = '';
    return;
  }
  const known = CLASSIFICATION[entry.key];
  if (known === undefined) {
    entry.classification = 'unclassified';
    entry.reason = '';
    return;
  }
  entry.classification = known.kind;
  entry.reason = known.reason;
}

function sameParagraphs(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((text, index) => text === right[index]);
}

interface Summary {
  candidates: number;
  comparable: number;
  matches: number;
  expected: number;
  nonGoal: number;
  bug: number;
  unclassified: number;
  ratio: number;
  failures: string[];
}

function summarize(entries: Entry[]): Summary {
  const candidates = entries.filter((entry) => entry.inSet);
  const comparable = candidates.filter((entry) => entry.comparable);
  const matches = comparable.filter((entry) => entry.classification === 'match').length;
  const countOf = (kind: Kind | 'unclassified') =>
    comparable.filter((entry) => entry.classification === kind).length;
  const expected = countOf('expected');
  const nonGoal = countOf('non-goal');
  const bug = countOf('bug');
  const unclassified = countOf('unclassified');
  const denominator = candidates.length - nonGoal;
  const ratio = denominator === 0 ? 0 : matches / denominator;

  const failures: string[] = [];
  if (candidates.length !== EXPECTED_CANDIDATES) {
    failures.push(`кандидатов ${candidates.length}, ожидалось ${EXPECTED_CANDIDATES}`);
  }
  if (comparable.length < MIN_COMPARABLE) {
    failures.push(`сопоставимых ${comparable.length}, порог ${MIN_COMPARABLE}`);
  }
  if (unclassified !== 0) failures.push(`unclassified = ${unclassified}`);
  if (bug !== 0) failures.push(`bug = ${bug}`);
  if (nonGoal > MAX_NON_GOAL) failures.push(`non-goal = ${nonGoal}, потолок ${MAX_NON_GOAL}`);
  if (ratio < THRESHOLD) {
    failures.push(
      `доля ${matches}/${denominator} = ${ratio.toFixed(4)}, порог ${THRESHOLD}`,
    );
  }
  return {
    candidates: candidates.length,
    comparable: comparable.length,
    matches,
    expected,
    nonGoal,
    bug,
    unclassified,
    ratio,
    failures,
  };
}

/** Оракул: наш абзацный текст в него не кладётся — только сторона mammoth. */
function buildOracle(entries: Entry[], mammothVersion: string, candidates: number): string {
  const fixtures: Record<string, unknown> = {};
  for (const entry of entries) {
    fixtures[entry.key] = {
      comparable: entry.comparable,
      skip_reason: entry.skipReason,
      classification: entry.classification,
      reason: entry.reason,
      mammoth:
        entry.mammoth === null
          ? null
          : { paragraphs: entry.mammoth.paragraphs, counts: entry.mammoth.counts },
      model: entry.model,
    };
  }
  const oracle = {
    generator: 'scripts/diff-mammoth.ts',
    mammoth: mammothVersion,
    threshold: THRESHOLD,
    min_comparable: MIN_COMPARABLE,
    candidates,
    fixtures,
  };
  return `${JSON.stringify(oracle, null, 2)}\n`;
}

function buildReport(entries: Entry[], summary: Summary, mammothVersion: string): string {
  const lines: string[] = [];
  const yes = summary.failures.length === 0;
  lines.push('# Спринт 8: differential-тест DOCX против mammoth');
  lines.push('');
  lines.push(
    'Отчёт пишется `npx tsx scripts/diff-mammoth.ts --update-oracle` (сухой прогон —',
    'без флагов, сверка с репозиторием — `--check`). Оракул —',
    '[`test-fixtures/docx/mammoth-oracle.json`](../../test-fixtures/docx/mammoth-oracle.json):',
    'абзацы и счётчики mammoth; список абзацев нашей стороны в него не кладётся,',
    'Rust-тест гейта считает его сам — иначе проверка сравнивала бы разбор сам с собой.',
  );
  lines.push('');
  lines.push(`Версия mammoth: ${mammothVersion}.`);
  lines.push('');
  lines.push('## Что сравнивается');
  lines.push('');
  lines.push(
    'Нормализация — контракт из `crates/docx/examples/dump_model.rs`, дословно тот же с',
    'обеих сторон: обход в порядке документа с рекурсией в таблицы (строки, ячейки, их',
    'блоки — в один плоский список); текст абзаца = текст прогонов как есть, `w:tab` →',
    '`"\\t"`, разрывы/символы/рисунки/неизвестное → ничего, внутри гиперссылки и',
    'результата поля — рекурсивно; `trim()`, пустые строки выброшены; сноски, концевые',
    'сноски, комментарии и колонтитулы в список не входят.',
  );
  lines.push('');
  lines.push(
    `Кандидаты — все 97 фикстур минус \`broken/\`, \`alternate_content/\` (mammoth читает`,
    '`mc:Fallback`, мы `mc:Choice`), `track_changes/` (наши non-goal) и `notes/`',
    `(mammoth выносит их отдельно): ${summary.candidates}.`,
  );
  lines.push('');
  lines.push('## Оракул');
  lines.push('');
  lines.push(
    '[`test-fixtures/docx/mammoth-oracle.json`](../../test-fixtures/docx/mammoth-oracle.json)',
    'пишет тот же скрипт: заголовок (`generator`, `mammoth`, `threshold`,',
    '`min_comparable`, `candidates`) и `fixtures` — по записи на каждую из 97 фикстур,',
    'ключи по возрастанию:',
  );
  lines.push('');
  lines.push('- `comparable` — фикстура входит в набор и mammoth её прочитал;');
  lines.push('- `skip_reason` — почему нет (вне набора или отказ mammoth), иначе `null`;');
  lines.push('- `classification` — класс, `null` у несопоставимых;');
  lines.push('- `reason` — почему класс такой, пустая строка у `match`;');
  lines.push(
    '- `mammoth.paragraphs` / `mammoth.counts` — сторона оракула: абзацы и счётчики',
    '  (`p`, `table`, `li`, `a`) из HTML;',
  );
  lines.push('- `model.counts` — наши счётчики (таблицы, пункты списка, ссылки, рисунки);');
  lines.push('  абзацев нашей стороны в оракуле нет — их считает тест-гейт.');
  lines.push('');
  lines.push('Классы расхождений:');
  lines.push('');
  lines.push('- `match` — списки абзацев совпали полностью;');
  lines.push(
    '- `expected` — расхождение принято и объяснено (политика mammoth или наше решение);',
    '  **остаётся в знаменателе метрики**;',
  );
  lines.push(
    '- `non-goal` — фикстура вне области сравнения (элемента нет в модели v1 или его не',
    '  читает mammoth); **выводится из знаменателя**, потолок — 8 из 76;',
  );
  lines.push('- `bug` — дефект нашего парсера;');
  lines.push('- `unclassified` — решение не принято — значение по умолчанию.');
  lines.push('');
  lines.push(
    `Гейт: \`unclassified = 0\`, \`bug = 0\`, \`non-goal ≤ ${MAX_NON_GOAL}\`,`,
    `\`match / (кандидаты − non-goal) ≥ ${THRESHOLD}\` и сопоставимых не меньше ${MIN_COMPARABLE}.`,
    'Классификация — таблица `CLASSIFICATION` в `scripts/diff-mammoth.ts`; всё, что',
    'разошлось и в таблицу не попало, падает в `unclassified`, а не угадывается.',
  );
  lines.push('');
  lines.push('## Сводка');
  lines.push('');
  lines.push('| Метрика | Значение |');
  lines.push('|---|---|');
  lines.push(`| Кандидатов | ${summary.candidates} |`);
  lines.push(`| Сопоставимо (mammoth прочитал) | ${summary.comparable} |`);
  lines.push(`| \`match\` | ${summary.matches} |`);
  lines.push(`| \`expected\` | ${summary.expected} |`);
  lines.push(`| \`non-goal\` | ${summary.nonGoal} |`);
  lines.push(`| \`bug\` | ${summary.bug} |`);
  lines.push(`| \`unclassified\` | ${summary.unclassified} |`);
  lines.push(
    `| Доля match / (кандидаты − non-goal) | ${summary.matches}/${summary.candidates - summary.nonGoal} = ${summary.ratio.toFixed(4)} (порог ${THRESHOLD}) |`,
  );
  lines.push(`| Гейт | ${yes ? 'пройден' : 'НЕ пройден'} |`);
  lines.push('');
  if (!yes) {
    lines.push('Не выполнено:');
    lines.push('');
    for (const failure of summary.failures) lines.push(`- ${failure};`);
    lines.push('');
  }
  lines.push('## Кандидаты');
  lines.push('');
  lines.push('| Фикстура | Класс | Причина | Абзацев (мы / mammoth) |');
  lines.push('|---|---|---|---|');
  for (const entry of entries.filter((entry) => entry.inSet)) {
    const kind = entry.comparable ? (entry.classification ?? 'unclassified') : '—';
    const reason = entry.reason === '' ? '—' : entry.reason.replaceAll('|', '\\|');
    const counts = entry.comparable
      ? `${entry.modelParagraphs.length} / ${entry.mammoth?.paragraphs.length ?? 0}`
      : '—';
    lines.push(`| \`${entry.key}\` | ${kind} | ${reason} | ${counts} |`);
  }
  lines.push('');
  const emptyMatches = entries.filter(
    (entry) =>
      entry.inSet && entry.classification === 'match' && (entry.mammoth?.paragraphs.length ?? 0) === 0,
  );
  if (emptyMatches.length > 0) {
    lines.push(
      `Совпадение на пустых списках (0 / 0) — ${emptyMatches.length}: ` +
        `${emptyMatches.map((entry) => `\`${entry.key}\``).join(', ')}. Текста в абзацах нет ни с`,
      'одной стороны — это совпадение, но проверяет оно только то, что ни один из парсеров',
      'не выдумывает текст из разметки; счётчики `model.counts.images` по ним — в оракуле.',
    );
    lines.push('');
  }
  lines.push('## Вне набора');
  lines.push('');
  lines.push('| Фикстура | Причина |');
  lines.push('|---|---|');
  for (const entry of entries.filter((entry) => !entry.inSet)) {
    lines.push(`| \`${entry.key}\` | ${entry.skipReason ?? ''} |`);
  }
  lines.push('');
  lines.push('## Расхождения');
  lines.push('');
  const differing = entries.filter(
    (entry) => entry.comparable && entry.classification !== 'match',
  );
  if (differing.length === 0) {
    lines.push('Расхождений нет.');
    lines.push('');
  }
  for (const entry of differing) {
    lines.push(`### \`${entry.key}\` — ${entry.classification}`);
    lines.push('');
    lines.push(`${entry.reason === '' ? 'Причина не заведена.' : entry.reason}`);
    lines.push('');
    lines.push('Наш список:');
    lines.push('');
    for (const paragraph of entry.modelParagraphs) {
      lines.push(`- \`${escapeInline(paragraph)}\``);
    }
    if (entry.modelParagraphs.length === 0) lines.push('- *(пусто)*');
    lines.push('');
    lines.push('mammoth:');
    lines.push('');
    for (const paragraph of entry.mammoth?.paragraphs ?? []) {
      lines.push(`- \`${escapeInline(paragraph)}\``);
    }
    if ((entry.mammoth?.paragraphs.length ?? 0) === 0) lines.push('- *(пусто)*');
    lines.push('');
  }
  return `${lines.join('\n')}`;
}

/** Инлайновый код в таблице/списке: обратные кавычки и переводы строк не ломают. */
function escapeInline(text: string): string {
  return text.replaceAll('`', '\\`').replaceAll('\n', '\\n').replaceAll('\r', '\\r');
}

async function compare(mammoth: Mammoth): Promise<{ entries: Entry[]; summary: Summary }> {
  const dump = await runModelDump();
  const keys: string[] = [];
  discover(FIXTURE_ROOT, '', keys);
  keys.sort();

  const entries: Entry[] = [];
  for (const key of keys) {
    const entry = makeEntry(key, dump);
    if (entry.inSet) {
      const read = await readMammoth(mammoth, path.join(FIXTURE_ROOT, `${key}.docx`));
      if ('error' in read) {
        entry.skipReason = `mammoth не прочитал пакет: ${read.error}`;
      } else {
        entry.comparable = true;
        entry.mammoth = read;
      }
    }
    classify(entry);
    entries.push(entry);
  }
  return { entries, summary: summarize(entries) };
}

/** Заготовка записи: всё, что известно до обращения к mammoth. */
function makeEntry(key: string, dump: Record<string, ModelFixture | null>): Entry {
  const excluded = EXCLUDED[key.split('/')[0] ?? ''] ?? null;
  const model = dump[key] ?? null;
  return {
    key,
    inSet: excluded === null,
    comparable: false,
    skipReason: excluded,
    classification: null,
    reason: excluded ?? '',
    mammoth: null,
    // Счётчики, а не весь отчёт: абзацы нашей стороны в оракул не попадают.
    model:
      model === null
        ? null
        : {
            counts: {
              tables: model.tables,
              list_items: model.list_items,
              hyperlinks: model.hyperlinks,
              images: model.images,
            },
          },
    modelParagraphs: model?.paragraphs ?? [],
    modelFailed: model === null,
  };
}

function printSummary(summary: Summary): void {
  console.log(`кандидатов: ${summary.candidates}`);
  console.log(
    `match: ${summary.matches}; expected: ${summary.expected}; non-goal: ${summary.nonGoal}; ` +
      `bug: ${summary.bug}; unclassified: ${summary.unclassified}`,
  );
  console.log(
    `доля: ${summary.matches}/${summary.candidates - summary.nonGoal} = ${summary.ratio.toFixed(4)} ` +
      `(порог ${THRESHOLD})`,
  );
  console.log(summary.failures.length === 0 ? 'гейт пройден' : 'гейт НЕ пройден');
  for (const failure of summary.failures) console.error(`  - ${failure}`);
}

/** Полные списки абзацев расхождений — то, ради чего отчёт и нужен. */
function printDifferences(entries: Entry[]): void {
  for (const entry of entries) {
    if (!entry.comparable || entry.classification === 'match') continue;
    console.log('');
    console.log(`${entry.key} [${entry.classification}] ${entry.reason}`);
    console.log(`  мы:      ${JSON.stringify(entry.modelParagraphs)}`);
    console.log(`  mammoth: ${JSON.stringify(entry.mammoth?.paragraphs ?? null)}`);
  }
}

/** Неверный вызов — это ошибка использования, а не провал гейта. */
function usage(message: string): never {
  process.stderr.write(`${message}\n`);
  process.stderr.write('usage: diff-mammoth.ts [--update-oracle | --check]\n');
  process.exit(2);
}

async function checkFresh(fresh: Map<string, string>): Promise<boolean> {
  const drift: string[] = [];
  for (const [file, expected] of fresh) {
    let actual: string | null = null;
    try {
      actual = await readFile(file, 'utf8');
    } catch {
      actual = null;
    }
    if (actual !== expected) drift.push(path.relative(ROOT, file));
  }
  if (drift.length > 0) {
    for (const file of drift) process.stderr.write(`расходится с репозиторием: ${file}\n`);
    return false;
  }
  console.log('оба файла совпадают с репозиторием');
  return true;
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const update = args.includes('--update-oracle');
  const check = args.includes('--check');
  for (const arg of args) {
    if (arg !== '--update-oracle' && arg !== '--check') usage(`unknown flag: ${arg}`);
  }
  // Вместе флаги дали бы запись файлов и «сверку» с тем, что только что записано, —
  // то есть зелёную проверку, которая ничего не проверила.
  if (update && check) usage('--update-oracle и --check несовместимы');

  let mammoth: Mammoth;
  try {
    mammoth = createRequire(import.meta.url)('mammoth') as Mammoth;
  } catch (error) {
    process.stderr.write(`mammoth не подключается: ${(error as Error).message}\n`);
    process.exit(2);
  }
  const mammothVersion = readMammothVersion();
  const { entries, summary } = await compare(mammoth);
  const oracle = buildOracle(entries, mammothVersion, summary.candidates);
  const report = buildReport(entries, summary, mammothVersion);

  if (update) {
    await writeFile(ORACLE_PATH, oracle);
    await writeFile(REPORT_PATH, report);
    console.log(`записано: ${path.relative(ROOT, ORACLE_PATH)}, ${path.relative(ROOT, REPORT_PATH)}`);
  }

  printSummary(summary);
  printDifferences(entries);

  const fresh = new Map([
    [ORACLE_PATH, oracle],
    [REPORT_PATH, report],
  ]);
  let ok = summary.failures.length === 0;
  if (check && !(await checkFresh(fresh))) ok = false;
  process.exitCode = ok ? 0 : 1;
}

await main();
