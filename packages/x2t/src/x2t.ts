/**
 * Типизированная обвязка вокруг `x2t.js` — WASM-сборки конвертера форматов ONLYOFFICE
 * (`cryptpad/onlyoffice-x2t-wasm`, релиз v9.3.2+3). Артефакты релиза лежат в `vendor/x2t/`,
 * в пакет они не входят: оба URL — glue-скрипт и wasm — задаёт вызывающий.
 *
 * Порядок вызова повторяет браузерный клиент CryptPad (`www/common/outer/x2t.js` апстрима):
 * поднять рантайм, создать `/working*`, положить вход в MEMFS, собрать `params.xml`, вызвать
 * `main1(path)` через `ccall`, забрать выход из MEMFS. Оттуда же взята и схема `params.xml`
 * (см. `buildParamsXml`).
 */
import type { X2tModuleConfig, X2tModule } from './glue.js';

export type { X2tAnalyzePathResult, X2tFileStat, X2tFS, X2tModule } from './glue.js';

export interface X2tLoadOptions {
  /** URL glue-скрипта, обычно `/x2t/x2t.js`. Грузится в глобальную область, см. `loadX2t`. */
  readonly scriptUrl: string;
  /**
   * URL `x2t.wasm`. Если не задан, сборка ищет модуль сама — рядом с *скриптом воркера*
   * (так работает её вшитый `locateFile`), и этот путь легко промахивается. Для харнесса
   * всегда задавайте `wasmUrl`.
   */
  readonly wasmUrl?: string;
  /** Куда дублировать строки `print`/`printErr` модуля — для лога о старте и конвертации. */
  readonly onPrint?: (line: string) => void;
}

export interface ConvertOptions {
  /** Формат входа: расширение без точки (`docx`, `odt`, `rtf`, …). */
  readonly from: string;
  /** Формат выхода: расширение без точки (`odt`, `txt`, `xlsx`, …). */
  readonly to: string;
  /** Каталог шрифтов внутри MEMFS; по умолчанию `/working/fonts/`. */
  readonly fontDir?: string;
  /** Имя файла во входе — им же назван выход. По умолчанию `input`. */
  readonly fileName?: string;
}

export interface ConvertResult {
  /** Байты выходного файла; пусто, если конвертер его не создал. */
  readonly bytes: Uint8Array<ArrayBuffer>;
  /** Код возврата `main` (0 — успех). Ненулевой код не бросает исключение — решает вызывающий. */
  readonly code: number;
  /** Всё, что модуль успел напечатать в stderr за этот вызов. */
  readonly stderr: string;
}

/** Ошибка загрузки модуля или обвязки; текст — по-английски, как и сообщения Rust-крейтов. */
export class X2tError extends Error {
  readonly code: number;
  readonly stderr: string;

  constructor(message: string, code = -1, stderr = '') {
    super(message);
    this.name = 'X2tError';
    this.code = code;
    this.stderr = stderr;
  }
}

/** Каталоги, которые ждёт конвертер под `/working`. */
const WORKING_DIRS = ['/working', '/working/media', '/working/fonts', '/working/themes'] as const;

const PARAMS_PATH = '/working/params.xml';

/**
 * Символы, недопустимые в имени файла MEMFS (включая управляющие). Остальное, включая
 * кириллицу и пробелы, проходит: и MEMFS, и XML их переживают, а вот `&`/`<`/`>` в XML —
 * нет, поэтому их снимает `escapeXml`, а не санитайзер.
 */
const ILLEGAL_IN_NAME = /[/\\?<>:*|"\u0000-\u001f\u0080-\u009f]/g;

/** stderr конкретного вызова. На самом модуле его держать нельзя: `Module` — чужой объект. */
const STDERR_BY_MODULE = new WeakMap<X2tModule, string[]>();

/** Глобальная область: `x2t.js` объявляет в ней `var Module`. Больше обвязка глобал не трогает. */
interface X2tGlobalScope {
  Module?: X2tModuleConfig;
  importScripts?: (url: string) => void;
}

/** Объект `Module` в процессе загрузки: конфиг плюс то, что рантайм выкладывает позже. */
type X2tGlue = X2tModuleConfig & Partial<X2tModule>;

function globalScope(): X2tGlobalScope {
  return globalThis as unknown as X2tGlobalScope;
}

/**
 * Загружает glue-скрипт x2t и поднимает рантайм до состояния, когда MEMFS готов к работе.
 *
 * Скрипт исполняется в глобальной области (в этом смысл и `importScripts`, и косвенного
 * `eval`), поэтому на одну область — один экземпляр: повторный вызов поверх уже загруженного
 * модуля бросает `X2tError` вместо тихой подмены конфига.
 *
 * @param options — URL скрипта и wasm, приёмник строк модуля.
 * @returns Готовый модуль с методами `FS` и `ccall`.
 * @throws {X2tError} — если модуль уже загружен, скрипт или wasm не скачались, рантайм упал.
 */
export async function loadX2t(options: X2tLoadOptions): Promise<X2tModule> {
  const scope = globalScope();
  if (scope.Module !== undefined) {
    throw new X2tError('x2t glue is already loaded in this scope: one instance per worker is supported');
  }

  const stderrLines: string[] = [];
  const glue: X2tGlue = {
    // Вшитый в сборку pre-js выставляет оба флага и сам; дублируем явно, чтобы намерение
    // читалось здесь: `main1` зовём только мы, и рантайм обязан пережить много конвертаций.
    noInitialRun: true,
    noExitRuntime: true,
    print: (line) => options.onPrint?.(line),
    printErr: (line) => {
      stderrLines.push(line);
      options.onPrint?.(line);
    },
  };

  const ready = new Promise<void>((resolve, reject) => {
    glue.onRuntimeInitialized = () => resolve();
    // Единственный сигнал о падении загрузки: `createWasm()` в сборке не ждут, и без этого
    // хука отказ инстанцирования остался бы висеть необработанным промисом.
    glue.onAbort = (what) => reject(new X2tError(`x2t runtime aborted: ${String(what)}`));
  });

  if (options.wasmUrl !== undefined) {
    glue.wasmBinary = await fetchWasm(options.wasmUrl);
  }

  scope.Module = glue;
  await loadGlueScript(options.scriptUrl, scope);
  await ready;
  createWorkingDirs(glue);

  if (glue.FS === undefined || glue.ccall === undefined) {
    throw new X2tError('x2t runtime initialized without FS/ccall on Module');
  }
  const module = glue as X2tModule;
  STDERR_BY_MODULE.set(module, stderrLines);
  return module;
}

/**
 * Конвертирует документ в MEMFS модуля и возвращает результат.
 *
 * Вызов синхронный по своей природе (сборка без asyncify: `main1` крутится в потоке до конца
 * конвертации) и возвращает промис только ради единообразия с `loadX2t`. В Web Worker это
 * ожидаемо — там и место конвертации; на главном потоке он заморозит страницу.
 *
 * @param module — модуль, поднятый `loadX2t`.
 * @param input — байты входного файла.
 * @param options — форматы, имя в MEMFS и каталог шрифтов.
 * @returns Байты выхода, код возврата `main` и stderr этого вызова.
 * @throws {X2tError} — если конвертер упал на уровне рантайма (abort), а не вернул код.
 */
export async function convert(
  module: X2tModule,
  input: Uint8Array,
  options: ConvertOptions,
): Promise<ConvertResult> {
  const fs = module.FS;
  const from = normalizeExtension(options.from);
  const to = normalizeExtension(options.to);
  const stem = sanitizeStem(options.fileName ?? 'input', from);
  const inputPath = `/working/${stem}.${from}`;
  const outputPath = `/working/${stem}.${to}`;

  const stderrLines = STDERR_BY_MODULE.get(module) ?? [];
  STDERR_BY_MODULE.set(module, stderrLines);
  stderrLines.length = 0; // stderr прошлого вызова не должен читаться как stderr этого

  fs.writeFile(inputPath, input);
  fs.writeFile(
    PARAMS_PATH,
    buildParamsXml({
      from: inputPath,
      to: outputPath,
      fontDir: options.fontDir ?? '/working/fonts/',
    }),
  );

  let code: number;
  try {
    code = module.ccall('main1', 'number', ['string'], [PARAMS_PATH]);
  } catch (error) {
    const stderr = stderrLines.join('\n');
    throw new X2tError(
      `x2t crashed during conversion: ${error instanceof Error ? error.message : String(error)}`,
      -1,
      stderr,
    );
  }

  const bytes = fs.analyzePath(outputPath).exists
    ? fs.readFile(outputPath, { encoding: 'binary' })
    : new Uint8Array(0);

  // Убираем за собой: MEMFS один на весь модуль, и без этого вход каждой конвертации остаётся
  // в памяти до конца жизни воркера.
  removeIfExists(fs, inputPath);
  removeIfExists(fs, PARAMS_PATH);
  removeIfExists(fs, outputPath);

  return { bytes, code, stderr: stderrLines.join('\n') };
}

/**
 * Собирает `params.xml` (`TaskQueueDataConvert`) — единственный вход конвертера: `main1`
 * получает путь к этому файлу и читает из него всё остальное.
 *
 * Схема — как у браузерного клиента CryptPad (`www/common/outer/x2t.js`, функция
 * `x2tConvertDataInternal`) и его Node-харнесса (`test.js`, `convert()`):
 *
 * ```xml
 * <?xml version="1.0" encoding="utf-8"?>
 * <TaskQueueDataConvert xmlns:xsi="…" xmlns:xsd="…">
 *   <m_sFileFrom>/working/one_paragraph.docx</m_sFileFrom>
 *   <m_sFileTo>/working/one_paragraph.odt</m_sFileTo>
 *   <m_sFontDir>/working/fonts/</m_sFontDir>
 *   <m_sThemeDir>/working/themes</m_sThemeDir>
 *   <m_bIsNoBase64>false</m_bIsNoBase64>
 *   <m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>
 *   <m_nCsvDelimiter>4</m_nCsvDelimiter>
 * </TaskQueueDataConvert>
 * ```
 *
 * Числа у CSV-полей — из `cextracttools.h` апстрима: 46 — кодовая страница 65001 (UTF-8),
 * 4 — `TCSVD_COMMA`. Без первого `docx → txt` на кириллице приезжает в системной кодировке.
 * В CryptPad этих двух полей нет — они взяты из `test.js`, где выставлены оба.
 *
 * `m_nFormatFrom`/`m_nFormatTo` не задаём: x2t выводит форматы из расширений файлов, в обоих
 * референсах соответствующие строки закомментированы. `m_bIsNoBase64=false` — как в обоих
 * референсах; на наших направлениях (мимо служебного формата `.bin`) признак не участвует.
 */
function buildParamsXml(paths: {
  readonly from: string;
  readonly to: string;
  readonly fontDir: string;
}): string {
  return [
    '<?xml version="1.0" encoding="utf-8"?>',
    '<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"' +
      ' xmlns:xsd="http://www.w3.org/2001/XMLSchema">',
    `<m_sFileFrom>${escapeXml(paths.from)}</m_sFileFrom>`,
    `<m_sFileTo>${escapeXml(paths.to)}</m_sFileTo>`,
    `<m_sFontDir>${escapeXml(ensureDirSuffix(paths.fontDir))}</m_sFontDir>`,
    '<m_sThemeDir>/working/themes</m_sThemeDir>',
    '<m_bIsNoBase64>false</m_bIsNoBase64>',
    '<m_nCsvTxtEncoding>46</m_nCsvTxtEncoding>',
    '<m_nCsvDelimiter>4</m_nCsvDelimiter>',
    '</TaskQueueDataConvert>',
  ].join('');
}

/** Загружает байты wasm: их отдаём модулю через `wasmBinary`, и он не ходит за ними сам. */
async function fetchWasm(wasmUrl: string): Promise<ArrayBuffer> {
  const response = await fetch(wasmUrl);
  if (!response.ok) {
    throw new X2tError(`failed to fetch x2t.wasm: HTTP ${response.status} ${response.statusText}`);
  }
  return response.arrayBuffer();
}

/**
 * Исполняет glue-скрипт в глобальной области.
 *
 * Классическому воркеру хватает `importScripts`. Модульному он недоступен, поэтому там
 * скрипт приезжает через `fetch` и косвенный `eval` — и только косвенный: `new Function`
 * сделал бы `var Module` из скрипта локальной переменной, и выложенные на неё `FS`/`ccall`
 * остались бы внутри функции.
 *
 * Проверять `importScripts` на существование недостаточно: в модульном воркере свойство
 * есть, но вызов бросает `TypeError: Module scripts don't support importScripts()`.
 * Поэтому ветка выбирается попыткой — вызов синхронный и до загрузки скрипта не доходит.
 */
async function loadGlueScript(scriptUrl: string, scope: X2tGlobalScope): Promise<void> {
  if (typeof scope.importScripts === 'function') {
    try {
      scope.importScripts(scriptUrl);
      return;
    } catch {
      // Модульный воркер — грузим ниже, через fetch.
    }
  }

  const response = await fetch(scriptUrl);
  if (!response.ok) {
    throw new X2tError(`failed to fetch x2t glue script: HTTP ${response.status} ${response.statusText}`);
  }
  const code = await response.text();
  (0, eval)(code);
}

/**
 * Создаёт рабочие каталоги один раз на модуль. Проверка на существование обязательна:
 * `mkdir` поверх существующего каталога бросает, а `loadX2t` может вызываться повторно
 * (например, после перезагрузки воркера в том же тесте).
 */
function createWorkingDirs(glue: X2tGlue): void {
  const fs = glue.FS;
  if (fs === undefined) {
    throw new X2tError('x2t runtime initialized without FS on Module');
  }
  for (const dir of WORKING_DIRS) {
    if (!fs.analyzePath(dir).exists) {
      fs.mkdir(dir);
    }
  }
}

function removeIfExists(fs: X2tModule['FS'], path: string): void {
  if (fs.analyzePath(path).exists) {
    fs.unlink(path);
  }
}

/** Приводит расширение к виду без точки и в нижнем регистре. */
function normalizeExtension(extension: string): string {
  const normalized = extension.trim().replace(/^\./, '').toLowerCase();
  if (!/^[a-z0-9]+$/.test(normalized)) {
    throw new X2tError(`unsupported format extension: "${extension}"`);
  }
  return normalized;
}

/**
 * Имя файла без пути: расширение входа отбрасываем и приписываем своё, поэтому выход не
 * зависит от того, как вызывающий назвал файл. Битые символы снимаем — путь уходит в MEMFS
 * и в XML-параметры, где разделители путей и разметка недопустимы.
 */
function sanitizeStem(name: string, extension: string): string {
  const suffix = `.${extension}`;
  const withoutExtension = name.toLowerCase().endsWith(suffix) ? name.slice(0, -suffix.length) : name;
  const sanitized = withoutExtension.replace(ILLEGAL_IN_NAME, '').trim();
  return sanitized.slice(0, 200) || 'input';
}

function ensureDirSuffix(path: string): string {
  return path.endsWith('/') ? path : `${path}/`;
}

function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}
