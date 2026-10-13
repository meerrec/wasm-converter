# @doc-converter/x2t

Типизированная обвязка вокруг `x2t.js` — WASM-сборки конвертера форматов ONLYOFFICE
(`cryptpad/onlyoffice-x2t-wasm`, релиз v9.3.2+3). Оборачивает то, что иначе пришлось бы
повторять на каждом вызове: поднять рантайм, создать каталоги MEMFS, положить вход,
собрать `params.xml`, дёрнуть `main1`, забрать выход и убрать за собой.

Сам бинарник в пакет не входит. Артефакты релиза лежат в `vendor/x2t/` (`x2t.js`,
`x2t.wasm`), и оба URL — glue-скрипта и wasm — задаёт вызывающий: пакет не знает ни
раскладки каталогов потребителя, ни откуда ему приезжают 39 МиБ wasm.

## Вызов

```ts
import { convert, loadX2t } from '@doc-converter/x2t';

const module = await loadX2t({
  scriptUrl: '/x2t/x2t.js',
  wasmUrl: '/x2t/x2t.wasm',
  onPrint: (line) => console.log(line),
});

const { bytes, code, stderr } = await convert(module, inputBytes, {
  from: 'docx',
  to: 'odt',
  fileName: 'document', // имя в MEMFS; им же назван выход
});
```

`convert` синхронна по своей природе — сборка без asyncify, `main1` крутится в потоке до
конца конвертации. Промис здесь только ради единообразия, поэтому конвертировать нужно в
воркере: на главном потоке вызов заморозит страницу.

Ненулевой код возврата не бросает исключение — его читает вызывающий (`code`). Исключение
`X2tError` достаётся отказу загрузки и падению рантайма; в нём есть `code` и `stderr`.

Один экземпляр модуля на область видимости: glue-скрипт исполняется глобально и кладёт
`var Module`, поэтому повторный `loadX2t` в той же области бросает `X2tError`, а не тихо
подменяет конфиг.

## API

```ts
function loadX2t(options: X2tLoadOptions): Promise<X2tModule>;
function convert(module: X2tModule, input: Uint8Array, options: ConvertOptions): Promise<ConvertResult>;

interface X2tLoadOptions {
  readonly scriptUrl: string;
  readonly wasmUrl?: string;
  readonly onPrint?: (line: string) => void;
}

interface ConvertOptions {
  readonly from: string;    // расширение без точки: docx, odt, rtf, …
  readonly to: string;      // расширение без точки: odt, txt, xlsx, …
  readonly fontDir?: string; // по умолчанию /working/fonts/
  readonly fileName?: string; // по умолчанию input
}

interface ConvertResult {
  readonly bytes: Uint8Array<ArrayBuffer>; // пусто, если выхода нет
  readonly code: number;                    // код возврата main1, 0 — успех
  readonly stderr: string;                  // stderr этого вызова
}
```

Типы поверхности glue-скрипта (`FS`, `ccall`, хуки запуска) — в `src/glue.ts`.

## Разработка

```bash
pnpm -F @doc-converter/x2t test        # vitest: чистые части, без wasm
pnpm -F @doc-converter/x2t typecheck
pnpm -F @doc-converter/x2t build       # tsup: dist/ с декларациями
```

Тесты не поднимают wasm: модуль подменён заглушкой `FS`/`ccall`, а glue-скрипт — строкой из
подменённого `fetch`, так что пакет зеленеет и без бинарника.

Живьём обвязку гоняет харнесс `experiments/x2t-spike/`: он раздаёт `vendor/x2t/` как есть
(`publicDir`) и импортирует пакет по относительному пути.
