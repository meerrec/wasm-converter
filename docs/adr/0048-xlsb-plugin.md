# ADR-0048: XLSB-плагин: BIFF12 в модель XLSX

**Статус:** Принято
**Дата:** 27.10.2026
**Контекст:** To-Be §3.1 (XLSB), M3
**Связанные ADR:** [ADR-0030](0030-format-plugin-traits.md) (trait-слой форматов),
[ADR-0039](0039-cancel-v1-non-goals.md) (отмена non-goal'ов v1),
[ADR-0046](0046-format-plugin-contract.md) (контракт FormatPlugin и семейные модели)

---

## Контекст

- XLSB был non-goal'ом v1; [ADR-0039](0039-cancel-v1-non-goals.md) снял статус
  («не реализовано», а не «non-goal»), но задач не появилось — реализаций до
  этого решения не было.
- XLSB — не отдельный движок рендера, а второе бинарное представление той же
  модели, что у XLSX: это OOXML-пакет (ZIP) с обычными `[Content_Types].xml` и
  связями, но части `xl/*.bin` записаны записями BIFF12 (MS-XLSB) вместо XML.
  Книга разбирается в ту же `xlsx::Workbook`, поэтому рендер и PDF — те же.
- [ADR-0046](0046-format-plugin-contract.md) уже назвал XLSB вторым фронтендом
  `xlsx::Workbook` наравне с ODS. To-Be §3.1 требует расширяемости через
  контракты; XLSB — второй потребитель требования в семействе `Sheets`.

## Решение

1. **Крейт `crates/xlsb` — читатель записей BIFF12.** `RecordReader` разбирает
   заголовок по MS-XLSB §2.1.4: тип записи — 1–2 байта по семь значащих бит,
   размер — 1–4 байта по семь бит (старший бит четвёртого байта игнорируется).
   Предел на данные одной записи — `MAX_RECORD_BYTES` (16 МиБ), проверяется до
   выделения памяти. Результат — `xlsx::Workbook` (ADR-0046: та же модель, тот
   же рендер/PDF/текст).
2. **Поддержанные записи.** Каталог книги: `BrtBeginBook`, `BrtWbProp`
   (система дат `f1904`), `BrtBundleSh` (имя, видимость, `iTabID`, `strRelID`).
   Лист: `BrtBeginSheet`, `BrtRowHdr`, ячейки `BrtCellBlank`/`BrtCellRk`/
   `BrtCellError`/`BrtCellBool`/`BrtCellReal`/`BrtCellSt`/`BrtCellIsst`/
   `BrtCellRString`, формулы `BrtFmlaString`/`BrtFmlaNum`/`BrtFmlaBool`/
   `BrtFmlaError` (значение сохраняется, формула — нет). Строки:
   `BrtBeginSst`/`BrtSSTItem`/`BrtEndSst`. Границы `BrtBeginBundleShs`/
   `BrtEndBundleShs`/`BrtBeginSheetData`/`BrtEndSheetData`/`BrtEndBook`/
   `BrtEndSheet` узнаются, данных для модели не несут. Всё неизвестное
   игнорируется. `BrtMergeCell` (объединения) в этот срез не входит —
   следующий срез.
3. **Плагин и ошибка.** `XlsbPlugin` (`id "xlsb"`, `Sheets`, `["xlsb"]`),
   `XlsbError { Core, Malformed, Unsupported, Limit }` (у `Malformed` — часть и
   смещение записи). Лист-модуль (нет `strRelID`) и листы не-рабочего типа
   (chart sheet и т.п.) → `Unsupported`: пропуск изменил бы состав книги.
4. **Строки общей таблицы встраиваются в ячейки.** `BrtCellIsst` разрешается в
   текст сразу, и значение кладётся в `CellValue::InlineString`: конструктора
   `SharedStrings::from_items` в `xlsx` нет, поэтому `Workbook::shared_strings`
   остаётся пустой, но текст не теряется. Стили/тема/печать/гиперссылки/
   примечания/объединения — следующий срез (TODO в `styles.rs`).
5. **Спецификация сверялась, не по памяти.** Идентификаторы записей — из
   Record Enumeration (MS-XLSB §2.3), кодирование заголовка — §2.1.4
   (openspecs: learn.microsoft.com → openspecs → ms-xlsb).
6. **wasm/TS/CLI — отдельным шагом.** Это решение фиксирует Rust-слой разбора;
   ленивый wasm-пакет, воркер, фабрика и бюджет — будущие ADR/задачи.

## Альтернативы

- **Отдельный рендер/PDF для XLSB** — отвергнуто (ADR-0046): XLSB — то же
  `xlsx::Workbook`, отдельный рендер дублировал бы `xlsx::paint` и PDF-бэкенд.
- **Внешний парсер** — отвергнуто: лицензия/размер и wasm-несовместимость;
  читатель записей — маленький, и весь остальной стек уже свой.
- **Sniffing магических байт** (`.bin`/BIFF12-сигнатура) — отвергнуто
  (ADR-0030): формат задаётся явно `DocFormat`, `can_parse` и реестра нет.

## Последствия

- Workspace-член `doc-converter-xlsb` (зависит только от `core`/`xlsx`/
  `render`), плюс `Cargo.lock`.
- Тесты 53+ (inline `#[cfg(test)]` в `reader`/`workbook`/`sheet`/`strings`/
  `styles`/`plugin`/`error`/`lib`): заголовки записей всех длин, декодирование
  `RkNumber`/ошибок/UTF-16, разрешение `BrtCellIsst`, обрывы и лимит 16 МиБ,
  лист-модуль и chart sheet — `Unsupported`, игнор неизвестных записей.
- Будущие wasm/CLI добавят свой бюджет в `.size-limit.mjs` и строку `DocFormat`
  `'xlsb'` аддитивно, как в ADR-0045/0046.
