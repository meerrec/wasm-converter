# @doc-converter/browser-convert

Браузерный конвертер OOXML поверх обвязки `@doc-converter/x2t` (x2t.wasm). Пакет
закрывает то, что `x2t` не умеет сама: до и после конвертации документ нужно
разбирать и пересобирать — OOXML-контейнеры это zip, а рантайм ONLYOFFICE работает
с ними как с каталогом файлов.

Сейчас в пакете фундамент — zip-механика без зависимостей (`src/zip.ts`): разбор
central directory, чтение и пересборка архивов на Web Streams
(`DecompressionStream`/`CompressionStream`) и собственной табличной CRC32. Остальные
модули (конвертация, предобработка) появятся в следующих итерациях.

## API

```ts
function listZipEntries(bytes: Uint8Array): ZipEntry[];                 // бросит на не-zip/порче
function readZipEntry(bytes: Uint8Array, entry: ZipEntry | string): Promise<Uint8Array | null>;
function zipHasEntry(bytes: Uint8Array, name: string): boolean;
function rewriteZip(bytes: Uint8Array, options: ZipRewriteOptions): Promise<Uint8Array>;
```

Размеры и CRC берутся только из central directory: потоковые писатели оставляют в
local header нули (general-purpose bit 3) и кладут значения в data descriptor.
`readZipEntry` возвращает `null` для неподдерживаемого метода сжатия и повреждённой
записи, бросает — только на структурной порче архива. `rewriteZip` не трогает
необработанные записи (verbatim-копия) и пережимает только те, что прошли `filter`.

## Разработка

```bash
pnpm -F @doc-converter/browser-convert test        # vitest: архивы строятся руками в тестах
pnpm -F @doc-converter/browser-convert typecheck
pnpm -F @doc-converter/browser-convert build       # tsup: dist/ с декларациями
```
