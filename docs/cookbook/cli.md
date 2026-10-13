# Рецепты: CLI

`doc-converter` — Node-CLI на тех же wasm-модулях, что и просмотрщик.
Форматы: `.docx` → `.pdf`, `.md`, `.html`, `.txt`; `.xlsx` → `.pdf`.
Выходные коды: 0 — всё сошлось, 1 — файл(ы) не сконвертировались,
2 — ошибка вызова.

---

### <a id="cli-01"></a>Рецепт 1. convert: DOCX → PDF

**Задача.** Один документ в PDF, формат по расширению выхода.

```bash
doc-converter convert report.docx -o report.pdf
```

**Под капотом.** Формат берётся из расширения `--out`; без известного
расширения (или при `-o каталог/`) имя строится от входа, а формат надо
указать `-f` — иначе ошибка вызова с подсказкой.

---

### <a id="cli-02"></a>Рецепт 2. convert: DOCX → Markdown/HTML/текст

**Задача.** Выгрузить документ в текстовые форматы.

```bash
doc-converter convert report.docx -o report.md
doc-converter convert report.docx -o report.html
doc-converter convert report.docx -o report.txt

# Выход — каталог: имя берётся от входа, формат — явно
doc-converter convert report.docx -o out/ -f md
```

**Под капотом.** `convert` принимает один файл; пара «вход → выход»
проверяется до чтения (`assertSupported`), так что `xlsx → md` упадёт сразу
внятной ошибкой, а не после разбора.

---

### <a id="cli-03"></a>Рецепт 3. convert: XLSX → PDF и выбор листа

**Задача.** Книга в PDF целиком или один лист.

```bash
doc-converter convert book.xlsx -o book.pdf          # вся книга, листы подряд
doc-converter convert book.xlsx -o sheet2.pdf --sheet 1   # один лист, 0-based
```

**Под капотом.** По умолчанию XLSX идёт целиком (`export_pdf_book`), а
`--sheet n` переключает на одиночный `export_pdf(bytes, sheet, options)` —
индексация ноль-базированная, как в API просмотрщика.

---

### <a id="cli-04"></a>Рецепт 4. Опции PDF в CLI

**Задача.** Размер страницы, ориентация, масштаб и поля (все — XLSX only).

```bash
doc-converter convert book.xlsx -o book.pdf \
  --page-size A3 --orientation landscape --scale 0.8 \
  --margins 15,15,15,15
```

**Под капотом.** Все четыре опции помечены в справке как «XLSX only»:
DOCX сохраняет размер страницы и поля из своего `w:sectPr`, поэтому эти
флаги для него игнорируются, а не пересчитывают раскладку.

---

### <a id="cli-05"></a>Рецепт 5. batch: glob и пул воркеров

**Задача.** Конвертировать пачку файлов параллельно.

```bash
doc-converter batch './in/**/*.xlsx' --out ./pdf --workers 4
```

**Под капотом.** Каждая задача — конвертация одного файла в пуле
`node:worker_threads`; очередь общая, поэтому медленный файл не держит
остальных, а упавший воркер не роняет прогон — его задача помечается
неудачной, и место занимает новый (замер: 116 файлов, 4 воркера — 0,20 с).

---

### <a id="cli-06"></a>Рецепт 6. batch: каталог рекурсивно

**Задача.** Сконвертировать всё внутри каталога, не перечисляя файлы.

```bash
doc-converter batch ./in --out ./out --format pdf --workers 8
```

**Под капотом.** Входами `batch` могут быть файлы, каталоги (обходятся
рекурсивно, берутся `.docx`/`.xlsx`) и glob-паттерны; выход пишется плоско
в `--out` как `<name>.<format>`, поэтому вложенность входа не переносится.

---

### <a id="cli-07"></a>Рецепт 7. Коллизии имён в batch

**Задача.** Что будет, если два файла дают одно имя выхода — и как решать.

```bash
# a/report.docx и b/report.docx оба лягут в out/report.pdf — это ошибка:
doc-converter batch ./in --out ./out --format pdf   # duplicate names are an error

# Перезапись существующих выходов — тоже только с явным флагом:
doc-converter batch ./in --out ./out --format pdf --force
```

**Под капотом.** Плоский вывод `<name>.<format>` — осознанный контракт
USAGE: коллизия имён ошибка, а не молчаливая перезапись; `--force`
распространяется и на `convert`.

---

### <a id="cli-08"></a>Рецепт 8. Выходные коды и диагностика CLI

**Задача.** Встроить CLI в скрипты и различать «не конвертнулось» и «звали
не так».

```bash
doc-converter convert missing.docx -o out.pdf; echo $?   # 1 — файл не сконвертировался
doc-converter convert book.docx -o out.xyz; echo $?      # 2 — ошибка вызова + подсказка 'doc-converter help'
doc-converter help                                        # полная справка (USAGE)
doc-converter --version
```

**Под капотом.** `run` ловит `CliError` и пишет в stderr; код 2 печатает
подсказку, а прогресс `batch` глушится `-q/--quiet` — воркеров по умолчанию
берётся по числу ядер, но не больше 8 (`defaultWorkers`).

---

Ссылки: справка — `packages/cli/src/args.ts` (USAGE), пул —
`packages/cli/src/pool.ts`, примеры — [README](../../README.md) «CLI».
