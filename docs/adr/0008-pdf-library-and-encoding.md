# ADR 0008: PDF-библиотека и кодирование текста — `printpdf`

**Статус:** принято 06.10.2026; реализуется в Спринте 6 (базовый PDF-экспорт XLSX).
**Контекст:** [`Cargo.toml`](../../Cargo.toml), [`crates/pdf/Cargo.toml`](../../crates/pdf/Cargo.toml),
[`crates/pdf/src/painter.rs`](../../crates/pdf/src/painter.rs),
[`crates/pdf/src/fonts.rs`](../../crates/pdf/src/fonts.rs),
[`crates/pdf/src/lib.rs`](../../crates/pdf/src/lib.rs),
[`crates/core/src/error.rs`](../../crates/core/src/error.rs),
[`crates/pdf/tests/export.rs`](../../crates/pdf/tests/export.rs),
[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml), [`deny.toml`](../../deny.toml),
[ADR-0002](0002-font-parsing.md), [ADR-0007](0007-pdf-layout-boundary.md)

## Контекст

Спринту 6 нужен экспорт листа XLSX в PDF: кириллица, subsetting, 1000 ячеек < 200 КБ
(DoD ROADMAP §6). Выбор библиотеки задаёт не только запись потоков, но и то, кто
отвечает за кодирование текста: от этого зависит, извлекается ли кириллица из
готового PDF и сколько кода пишем мы сами.

`printpdf` 0.8.2 уже в дереве ([`Cargo.toml:47`](../../Cargo.toml)). Он даёт из
коробки:

- subsetting через `allsorts-subset-browser` (`SubsetProfile::Web`) — безусловный:
  `PreparedFont::new` подрезает шрифт по использованным глифам при сериализации;
- встраивание как Type0 / `Identity-H` + `CIDFontType2` + `ToUnicode` (отдельные
  потоки CIDToGIDMap/CMap) — текст в PDF не Win-1252, а CID;
- запись текста `Op::WriteText` / `Op::WriteCodepoints`.

Ограничения стека, принимаемые сознательно (решение архитектора):

- документ копится целиком в памяти: `PdfDocument` → `lopdf::Document` → `save_writer`,
  потоковой записи у printpdf нет;
- потоки не сжимаются: `optimize` не вызывает `doc.compress()` — вызов
  закомментирован в `serialize.rs`; поток шрифта пишется `with_compression(false)`,
  это требование самого printpdf.

Обе оговорки закрывает Спринт 7: там уже стоят «500 страниц < 3 с» и сжатие `flate2`.

## Рассмотренные варианты

1. **`printpdf` 0.8.2 без default-фич** (выбран). Плюсы: уже в дереве, subsetting и
   Type0/Identity-H/ToUnicode готовы, ровно тот минимум, что нужен Спринту 6. Минусы:
   документ в памяти, потоки без сжатия, `shape_text` и `from_html` при
   `default-features = false` недоступны.
2. **`pdf-writer`.** Низкоуровневая запись PDF: контент-потоки, словари шрифтов, CMap,
   ToUnicode и subsetting — вручную. Плюсы: полный контроль над кодированием и сжатием.
   Минусы: DoD Спринта 6 — кириллица и subsetting — это ровно та работа, которую
   printpdf уже делает; для базового экспорта выгоды нет.
3. **Своя сериализация PDF.** Отвергнуто: пришлось бы писать шрифтовую часть (CID,
   CIDToGIDMap, ToUnicode, subsetting) без выигрыша в функциональности.

## Решение

Берём `printpdf` 0.8.2 с `default-features = false`: дефолтная фича `html` тянет
azul-*/cssparser/selectors (MPL-2.0 — не проходит `cargo-deny`) и resvg/svg2pdf,
а XLSX→PDF этого не требует.

Кодирование текста — штатное для printpdf: Type0 / `Identity-H`, `CIDFontType2`,
`ToUnicode` CMap. Своих потоков CIDToGIDMap/CMap и своей карты Unicode не пишем.

Шейпинга нет: `shape_text` (фича `text_layout`) и `from_html` (фича `html`)
недоступны, текст пишется `WriteText`/`WriteCodepoints`. Это совпадает с canvas-путём,
где `FontRegistry` суммирует advance'ы без кернинга и лигатур, — новых расхождений
экрана и PDF не добавляется.

Непотоковость и отсутствие сжатия — принятые ограничения Спринта 6, а не свойства,
которые планируется сохранять: стриминг и `flate2` ставит Спринт 7.

## Следствия

- В [`crates/core/src/error.rs`](../../crates/core/src/error.rs) добавляется вариант
  `Export(String)`: ядро не знает про PDF. Сейчас ошибка идёт как
  `Malformed("pdf export: …")` ([`crates/pdf/src/lib.rs:65`](../../crates/pdf/src/lib.rs)),
  и диагностика печатает неверный префикс «malformed OOXML: …».
- `lopdf::extract_text` на PDF от printpdf **не работает**: CMap-парсер lopdf
  принимает только `CIDSystemInfo`/`CMapName`/`CMapType`, а printpdf пишет ещё
  `/CMapVersion` и `/WMode`. Это строгость lopdf, а не порча CMap: `pdftotext`
  лишние ключи игнорирует.
- Приёмка кириллицы в CI — через `pdftotext`: `poppler-utils` ставится в джобе `test`
  ([`.github/workflows/ci.yml:43`](../../.github/workflows/ci.yml)). Юнит-тесты
  [`crates/pdf/tests/export.rs`](../../crates/pdf/tests/export.rs) несут мини-декодер
  `ToUnicode` и проверяют текст без внешних утилит.
- Ограничения printpdf остаются ограничениями продукта до Спринта 7: экспорт большой
  книги пропорционален её размеру в памяти, PDF не сжимается.
