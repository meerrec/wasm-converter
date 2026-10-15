# @meerrec/cli

Конвертер OOXML из командной строки: DOCX в PDF, Markdown, HTML или текст,
XLSX в PDF. Вся конвертация — те же Rust/WASM-модули, что и в просмотрщике
(`@meerrec/wasm-pdf`, `@meerrec/wasm-docx`), поэтому результат
совпадает с тем, что отдаёт интерфейс.

## Установка

Пакет живёт в **GitHub Packages** этого репозитория (`@meerrec/cli@1.0.0`), а не в
`registry.npmjs.org`: реестр и токен с правом `read:packages` — в `~/.npmrc`:

```ini
# ~/.npmrc — <токен> это personal access token (classic) с read:packages
@meerrec:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=<токен>
```

```bash
npm install -g @meerrec/cli --registry=https://npm.pkg.github.com
```

Без реестра — тарбол (`meerrec-cli-1.0.0.tgz`) в ассетах релиза
[`v1.0.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.0.0).

Node ≥ 20 (как в корневом `package.json`). Из монорепозитория:

```bash
pnpm install
pnpm build:wasm                 # обязательный шаг: pkg/ wasm-модулей
pnpm -F @meerrec/cli build
node packages/cli/dist/cli.js --version
# или из корня
pnpm cli convert report.docx -o report.pdf
```

После установки пакета bin называется `doc-converter`:

```bash
doc-converter convert report.docx -o report.pdf
```

## Команды

### `convert`

```bash
doc-converter convert report.docx -o report.pdf
doc-converter convert report.docx -o report.md
doc-converter convert book.xlsx -o book.pdf --sheet 0
```

`-o` может быть каталогом — тогда имя файла собирается из входа и формата:

```bash
doc-converter convert report.docx -o ./out --format pdf   # ./out/report.pdf
```

### `batch`

Входы — файлы, каталоги (обходятся рекурсивно) или glob-шаблоны. Файлы
конвертируются параллельно, пул воркеров раздаёт задачи по одному файлу на
воркер; wasm-модуль грузится в воркере один раз и переиспользуется.

```bash
doc-converter batch ./in/*.xlsx --out ./pdf --workers 4
doc-converter batch ./in --out ./out --format pdf --workers 8 --scale 0.9
doc-converter batch './in/**/*.docx' --out ./out --format md
```

Прогресс печатается в stderr: в терминале — перерисовываемая полоса, в пайпе
— построчный лог (`ok 5/12 ...`). `--quiet` выключает прогресс, итоговая
строка и ошибки остаются.

Имена выходных файлов — плоские (`<имя>.<формат>`); два входа с одинаковым
именем — ошибка до запуска воркеров. Существующий выход не перезаписывается
без `--force`.

## Опции

| Опция | Команды | Смысл |
|---|---|---|
| `-o, --out <path>` | обе | выходной файл (`convert`) или каталог (`batch`) |
| `-f, --format <fmt>` | обе | `pdf`, `md`, `html`, `txt`; у `batch` по умолчанию `pdf` |
| `-w, --workers <n>` | `batch` | число воркеров, по умолчанию — ядра, не больше 8 |
| `-q, --quiet` | `batch` | без прогресса |
| `--force` | обе | перезаписывать существующие файлы |
| `--page-size <A4\|A3\|Letter\|Legal>` | обе | только XLSX |
| `--orientation <portrait\|landscape>` | обе | только XLSX |
| `--scale <factor>` | обе | только XLSX, например `0.8` |
| `--margins <top,right,bottom,left>` | обе | только XLSX, миллиметры |
| `--sheet <n>` | обе | только XLSX, лист с нуля; без неё книга целиком |

Коды возврата: `0` — успех, `1` — файлы не сконвертировались, `2` — ошибка
вызова (неизвестный флаг, неподдерживаемая пара, занятый выход).

## Ограничения

- XLSX конвертируется только в PDF: Markdown/HTML/текст поддержаны лишь для
  DOCX.
- У DOCX в PDF размер страницы, поля и ориентация берутся из `w:sectPr`, а не
  из опций: страницы PDF — это страницы раскладки документа.
- `compress` PDF (в крейте `doc_converter_pdf` он включён по умолчанию) через
  wasm не проброшен — флага сжатия в CLI нет.
- Легаси-форматы `.doc`/`.xls` не поддерживаются: нужен OOXML-пакет.
- Каталоги обходятся без скрытых подкаталогов.

## Разработка

```bash
pnpm -F @meerrec/cli build       # tsc → dist/ (+ shebang +x)
pnpm -F @meerrec/cli test        # vitest: аргументы, glob, пул, wasm-фикстуры, bin
pnpm -F @meerrec/cli typecheck
```

Тесты `test/cli.e2e.spec.ts` запускают `dist/cli.js` и потому требуют сборки
(`turbo` собирает пакет перед `test`); при прямом `vitest` без `dist` файл
пропускается.
