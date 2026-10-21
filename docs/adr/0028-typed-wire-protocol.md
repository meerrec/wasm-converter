# ADR-0028: Типизированный wire-протокол — Rust как источник истины

**Статус:** Принято 21.10.2026.
**Связанные ADR:** [ADR-0021](0021-frame-transports.md) (транспорты кадра;
этим ADR не пересматривается).

## Контекст

- Payload'ы протокола Main ↔ Worker (листы, страницы, картинки, статистика,
  гиперссылки, опции PDF, окно) были описаны дважды: структурами в
  `crates/wasm`, `crates/docx-wasm` и `crates/pdf-wasm` и вручную написанными
  интерфейсами в `packages/core/src/protocol.ts`. Копии уже разъехались
  деталями: `BuildStats` существовал в двух экземплярах, `ImageInfo` — тоже.
- Rust-сериализация (`serde_wasm_bindgen`) и TS-типы не проверялись друг
  против друга: переименование поля в Rust или потеря `Option` ломали
  воркер в рантайме, а не в CI.
- Граница wasm возвращает `JsValue`, то есть в сгенерированных
  wasm-bindgen-декларациях — `any`. Касты `as SheetInfo[]`, `as PaintStatsJs`,
  `as [number, number]` были разбросаны по `worker.ts`, и ничто не мешало
  касту разойтись с фактической формой ответа.
- Конверт (`InMsg`/`OutMsg`) содержит `OffscreenCanvas`, `ImageBitmap` и
  `ArrayBuffer` — типы, которых в Rust нет и не должно быть: конверт живёт
  только на стороне TS.

## Рассмотренные варианты

| Вариант | Почему нет |
|---|---|
| Оставить ручные DTO и касты, добавить больше тестов | Тесты проверяют известные случаи; дрейф появляется при изменении Rust, и его по-прежнему ловит только рантайм |
| Ручной кодогенератор (скрипт поверх `syn`) | Свой парсер Rust-структур и serde-атрибутов — цена и источник ошибок больше, чем у зрелого `ts-rs` |
| `#[ts(export)]` на каждом типе | Экспорт идёт при `cargo test` по одному файлу на тип, порядок и заголовок не контролируются; нужен один файл с фиксированным порядком |
| **Выбран:** `ts-rs` + пример-генератор в `core::wire` | Один источник истины, детерминированный файл, drift-гейт в CI; конверт остаётся ручным |

## Решение

1. **Источник истины payload'ов — Rust DTO в `doc_converter_core::wire`**
   (`crates/core/src/wire.rs`). Отдельный крейт не заводится: `core` уже
   публикуемый и транзитивно доступен всем трём wasm-крейтам. Имена — с
   суффиксом `Wire` (`ViewportWire`, `SheetInfoWire`, `PdfOptionsWire`, …).
2. **`ts-rs` версии 10.1**, зависимость опциональная, фича `ts`. 11.x/12.x
   требуют Rust 1.88, а MSRV проекта — 1.82; 10.1 объявляет MSRV 1.63.
   Подключение: `default-features = false`, `features = ["serde-compat"]` —
   переименования (`rename_all = "camelCase"`, варианты enum) берутся из
   тех же serde-атрибутов, что действуют в рантайме.
3. **Генератор — пример `crates/core/examples/export_wire.rs`**
   (`required-features = ["ts"]`), команда `pnpm gen:wire`. Типы выводятся
   через `TS::decl()` в фиксированном порядке и склеиваются в один файл
   `packages/core/src/generated/wire.ts` с единственным заголовком
   «сгенерировано, не править». Если объявление любого типа тянет `import`,
   пример падает: такое поле не должно пересекать границу воркера. Генерация
   детерминирована — повторный запуск даёт побайтово тот же файл.
4. **Конверт остаётся ручным в `protocol.ts`**: `InMsg`/`OutMsg` описывают
   сообщения с `OffscreenCanvas`, `ImageBitmap` и `ArrayBuffer`, которым в
   Rust нет места. Генерируются только payload'ы.
5. **Публичные TS-типы — алиасы/расширения сгенерированных:**
   `SheetInfo = SheetInfoWire`, `PageInfo = PageInfoWire`,
   `HyperlinkInfo = HyperlinkInfoWire`, `ImageInfo = ImageInfoWire`,
   `PaintStats extends PaintStatsWire { frameId; buildMs? }`,
   `PdfOptions = PdfOptionsWire & { allSheets? }`. `Viewport`, `RenderConfig`
   и `RenderRequest` остаются ручными: форма `Viewport` на проводе другая
   (`scrollX`/`scrollY`/`width`/`height`/`scale`), и единственный маппинг
   public → wire — `packages/core/src/worker/frame_args.ts`.
6. **`null` и отсутствие поля различаются осознанно.** `serde_wasm_bindgen`
   отдаёт `None` как `undefined`, поэтому выходные `Option`-поля
   (`HyperlinkInfoWire.display/tooltip`) сериализуются с
   `serialize_missing_as_null(true)` и в TS имеют тип `string | null`;
   отсутствующее поле — это `null`, а не пропущенное свойство. Входные
   `Option`-поля помечены `#[ts(optional)]` — в TS они `?:`, потому что
   интерфейс вправе их не присылать.
7. **Граница wasm — один файл.** `packages/core/src/worker/wasm_typed.ts` —
   единственное место с `as unknown as <WireType>`: wasm-bindgen отдаёт
   `any`, и сужение до сгенерированного типа происходит там. `worker.ts`
   больше не кастует ответы (`SheetInfo[]`, `PageInfo[]`, `PaintStatsJs`,
   `BuildStatsJs`, `[number, number]`, `HyperlinkInfo | null`).
8. **Compile-only проверка формы** — `packages/core/src/wire_assertions.ts`:
   взаимная присваиваемость публичных типов и wire-типов, форма `PaintStats`
   и `PdfOptions`, ключи `Viewport` против `ViewportWire`, типы возврата
   `frame_args`. Файл ниоткуда не импортируется и в бандл не попадает;
   проверяется `tsc --noEmit`.
9. **CI ловит дрейф.** В rust-джобе после тестов генератор запускается
   повторно и `git diff --exit-code -- packages/core/src/generated/wire.ts`
   падает на любом расхождении. Отдельный
   `cargo clippy -p doc-converter-core --features ts --all-targets -- -D warnings`
   проверяет пример и `ts-rs`-атрибуты, которых не видит общий clippy без
   `--all-features`.

## Следствия

- **Менять payload — только в Rust.** Правка `packages/core/src/generated/wire.ts`
  руками будет затёрта генератором и упадёт в CI. Изменение Rust-структуры
  требует `pnpm gen:wire` и коммита результата в том же изменении.
- **`core::wire` не знает про `render`/`xlsx`/`docx`.** DTO — плоские данные
  без методов; конвертация в доменные типы (`Viewport`, `PaintOptions`,
  `PdfOptions`, `ImageInfoWire::of`-подобные функции) остаётся в wasm-крейтах.
- **Дубли устранены:** `ViewportJs` (XLSX/DOCX), `OptionsJs`, `SheetInfo`,
  `PageInfo`, `ImageInfo` (XLSX/DOCX), `HyperlinkInfo`, `PaintStatsJs`,
  `BuildStats` (XLSX/DOCX), `PdfOptionsJs`/`MarginsJs`/`PageSizeJs`/
  `OrientationJs` живут в `core::wire` в одном экземпляре.
- **Сгенерированный файл — часть публикации** `@meerrec/core`: типы из него
  попадают в `.d.ts` пакета. Он коммитится вместе с исходниками.
- **Добавление нового payload'а** — структура в `wire.rs`, строка в
  фиксированном списке генератора, `pnpm gen:wire`, при необходимости —
  алиас в `protocol.ts` и проверка в `wire_assertions.ts`.
