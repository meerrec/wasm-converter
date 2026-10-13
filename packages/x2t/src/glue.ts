/**
 * Типы поверхности glue-скрипта `x2t.js` — то, что сборка выкладывает на объект `Module`
 * (`ccall`, `FS`) и что читает с него (`print`, `printErr`, `wasmBinary`, хуки запуска).
 *
 * Описаны по факту вендоренного файла (релиз v9.3.2+3), а не по документации Emscripten:
 * сборка местами от неё отличается, и это отмечено в комментариях ниже. Грепать её заново
 * стоит по именам отсюда.
 *
 * `HEAPU8`/`wasmMemory` в этом типе нет намеренно: сборка держит их внутренними `var` и на
 * `Module` не выкладывает, то есть работать с памятью в обход `FS` через публичную
 * поверхность нельзя. Линейную память отдают только методы `FS`.
 *
 * Заводится обычным модулем, а не файлом `.d.ts`: сборщик деклараций пакета (`tsup`,
 * `dts: true`) собирает их из входного графа и рукописный `x2t.d.ts` рядом с `x2t.ts` принял
 * бы за собственный вывод — сборка падала бы на перезаписи входа. На рантайм это не влияет:
 * здесь только типы, и в бандл модуль не попадает.
 */

/** Запись каталога MEMFS — поля `FS.stat`, которыми реально пользуются. */
export interface X2tFileStat {
  readonly mode: number;
  readonly size: number;
  readonly mtime: number;
  readonly ctime: number;
}

/** Результат `FS.analyzePath`: есть ли путь, и если есть — его stat в `object`. */
export interface X2tAnalyzePathResult {
  readonly exists: boolean;
  readonly path: string;
  readonly object: X2tFileStat | null;
}

/** Опции `FS.writeFile`. */
export interface X2tWriteOptions {
  readonly flags?: number;
  readonly mode?: number;
}

/**
 * Файловая система Emscripten (MEMFS в корне `/`). Перечислены методы, которыми пользуются
 * обвязка и референсные клиенты; остальная поверхность (`FS.open`/`read`/`write`/`close`)
 * есть, но здесь не нужна.
 *
 * Побочный эффект, который стоит помнить: MEMFS — один на экземпляр модуля, файлы живут
 * между вызовами `main1`, а `mkdir` по существующему каталогу бросает исключение.
 */
export interface X2tFS {
  /** Создаёт каталог; бросает, если путь уже существует. */
  mkdir(path: string, mode?: number): void;
  /** Пишет файл (строка кодируется в UTF-8), перезаписывая существующий. */
  writeFile(path: string, data: Uint8Array | string, options?: X2tWriteOptions): void;
  /**
   * Читает файл: по умолчанию — байты (свежая копия в собственном `ArrayBuffer`), с
   * `encoding: 'utf8'` — строку.
   */
  readFile(path: string, options?: { readonly encoding?: 'binary' }): Uint8Array<ArrayBuffer>;
  readFile(path: string, options: { readonly encoding: 'utf8' }): string;
  /** Имена записей каталога, включая `.` и `..`. */
  readdir(path: string): string[];
  unlink(path: string): void;
  rmdir(path: string): void;
  stat(path: string, dontFollow?: boolean): X2tFileStat;
  /** Существует ли путь (без исключения на отсутствующем). */
  analyzePath(path: string, dontFollow?: boolean): X2tAnalyzePathResult;
  /** Проверка по `mode` из `stat`, а не по пути: `FS.isDir(stat.mode)`. */
  isDir(mode: number): boolean;
  isFile(mode: number): boolean;
}

/**
 * Конфигурация, которую glue читает при загрузке. Заполняется до выполнения скрипта: часть
 * полей (`wasmBinary`, `print`, `printErr`, хуки) сборка разбирает в момент своего запуска,
 * поэтому выставить их после `importScripts` уже поздно.
 */
export interface X2tModuleConfig {
  /** Не запускать `main` самому — в этой сборке `main1` дёргается только нашим `ccall`. */
  noInitialRun?: boolean;
  /** Не завершать рантайм после выхода из `main`: модуль должен пережить много конвертаций. */
  noExitRuntime?: boolean;
  /**
   * Подмена URL для файлов рантайма. В вендоренной сборке поле бесполезно: `pre-js.js`
   * вшит в `x2t.js` и перезаписывает `Module.locateFile` безусловно при загрузке скрипта.
   * Поле есть в типе, потому что сборка его читает, — но рассчитывать на него нельзя.
   */
  locateFile?: (path: string, prefix: string) => string;
  /**
   * Готовые байты `x2t.wasm`. Единственный надёжный способ указать сборке, откуда брать
   * модуль: с заполненным полем она не идёт на сеть вообще.
   */
  wasmBinary?: ArrayBuffer;
  /** stdout модуля. */
  print?: (line: string) => void;
  /** stderr модуля — сюда же уходят строки ошибок конвертера. */
  printErr?: (line: string) => void;
  /** Вызывается из `abort()` — единственный сигнал, что рантайм не поднялся. */
  onAbort?: (what: unknown) => void;
  /** Рантайм готов: вызван после загрузки wasm и инициализации `FS`. */
  onRuntimeInitialized?: () => void;
  /** Отладочный счётчик незавершённых зависимостей запуска. */
  monitorRunDependencies?: (left: number) => void;
}

/** Объект `Module` после инициализации рантайма. */
export interface X2tModule extends X2tModuleConfig {
  readonly FS: X2tFS;
  /**
   * Вызов экспортированной функции wasm. Единственная нужная нам точка входа — `main1`
   * (см. `wrap-main.cpp` апстрима): принимает путь к `params.xml`, возвращает код возврата
   * `main`. Сигнатура сужена под неё; общий вид `ccall(ident, returnType, argTypes, args)`
   * у сборки шире.
   */
  ccall(
    ident: 'main1',
    returnType: 'number',
    argTypes: readonly ['string'],
    args: readonly [string],
  ): number;
  /** Признак, что `run()` дошёл до конца (сборка выставляет его в `doRun`). */
  calledRun?: boolean;
}
