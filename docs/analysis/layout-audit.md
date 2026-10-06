# Аудит: зависимость раскладки XLSX от canvas/web-sys

**Дата:** 06.10.2026
**Контекст:** спринт 6 (базовый PDF-экспорт XLSX): что в раскладке привязано к
canvas, а что переиспользуется вне wasm32. Проверено по коду на коммите `65987e4`.

## TL;DR

- Canvas-зависимость изолирована: `render::canvas`,
  `render::painter::{painter_2d, chart, bitmap_cache}`, `render::sab::{writer,
  SabRing}` плюс `text_align_str` — под `#[cfg(target_arch = "wasm32")]` или на
  типах `web_sys`/`js_sys`. `crates/render/Cargo.toml:16-25` держит
  wasm-bindgen/js-sys/web-sys только под
  `[target.'cfg(target_arch = "wasm32")'.dependencies]`.
- Нативно компилируются и canvas не требуют: DisplayList, `FontRegistry`,
  `text_measure`, геометрия/viewport/hit_test, `ChartData` и весь `crates/xlsx`
  (модель, `SheetLayout`, `paint::build`); в `crates/xlsx/src` — 0 упоминаний
  `web_sys`, `wasm_bindgen`, `js_sys`, `OffscreenCanvas`.
- PDF-крейт может быть полностью canvas-свободным: координаты листа берутся из
  `SheetLayout` (пиксели раскладки), перенос строк — из `render::text_measure`,
  поэтому точки разрыва совпадут с canvas без общего painter'а.

## Модуль → зависимость от canvas → доказательство

| Модуль | Зависит | Доказательство |
|---|---|---|
| `render::display_list`, `font`, `text_measure`, `geometry`, `viewport`, `hit_test`, `chart` | нет | `lib.rs:8-16`; упоминаний web-типов в файлах нет |
| `render::sab::mod`, `sab::reader`, `RingState` | нет | `sab/mod.rs:1-2`; гейтятся только `writer` (`:4-5`) и `SabRing` (`:9-10`) |
| `render::painter::state`, `painter::text::ellipsize` | нет | `painter/mod.rs:1-2`; в `state.rs:1` — только `display_list` |
| `render::painter::text::text_align_str` | да, точечно | гейт `text.rs:24-25`; импорт `TextAlign` — `text.rs:1` |
| `render::canvas` | да | гейт `canvas.rs:11`; `wasm_bindgen`/`web_sys` — `:17-18` |
| `render::painter::painter_2d` | да | гейт `painter_2d.rs:1`; `web_sys::OffscreenCanvasRenderingContext2d` — `:10`, поля — `:20,:26,:34` |
| `render::painter::chart` | да | гейт в `painter/mod.rs:6-7`; `web_sys` — `chart.rs:10` |
| `render::painter::bitmap_cache` | да | гейт `bitmap_cache.rs:1`; `web_sys::ImageBitmap` — `:4` |
| `render::sab::writer` | да | гейт `writer.rs:3`; `wasm_bindgen::JsValue` — `:7` |
| `render::sab::ring::SabRing` | да | `ring.rs:64-68` (`mod wasm_impl`), реэкспорт — `:226-227` |
| `xlsx` (model, styles, layout, paint, …) | нет | `crates/xlsx/Cargo.toml` без wasm-зависимостей; grep по `crates/xlsx/src` — 0 хитов; `cargo check -p doc-converter-xlsx -p doc-converter-pdf` проходит нативно |

## Публичный API render, пригодный для PDF

- `DisplayList` (`display_list.rs:190`), `DrawCommand` (`:87`),
  `DisplayListReader` (`:467`): `intern` (`:225`), `to_bytes` (`:413`),
  `from_bytes` (`:419`); формат — `DLST`, `DL_VERSION = 4` (`:12-13`), заголовок
  20 байт (`:187`).
- `FontRegistry` (`font.rs:72`): `new(cache_size)` (`:85`), `register(id, bytes) ->
  Result<(), FontError>` (`:106`), `has(id)` (`:118`), `glyph_metrics(id, size_px,
  ch) -> GlyphMetrics` (`:129`), `measure(id, size_px, text) -> TextMetrics`
  (`:145`). Метрики — из TTF через `skrifa`; LRU-ключ — `(font, char, size_bits)`
  (`font.rs:34-41`). Шрифт по умолчанию — `DEFAULT_FONT_ID = 0` (`:23`), Carlito
  вкомпилирован (`:30`).
- `text_measure::measure_text(fonts, font_id, size_px, text) -> f32` (`:14`);
  `break_lines(fonts, font_id, size_px, text, max_width) -> Vec<Range<usize>>`
  (`:31`) — жадный word wrap: `\n` форсирует разрыв, слово шире строки режется по
  символам (`break_word`, `:142`).
- `geometry::Rect` (`:7`), `Point` (`:47`); `viewport::Viewport` (`:12`):
  `visible_range` (`:43`), `scroll_to` (`:57`), `layout_to_screen` (`:84`);
  `hit_test::hit_test` (`:55`).
- `chart::ChartData` (`:47`) — только данные диаграммы; `to_blob` (`:58`),
  `from_blob` (`:82`).

## Конвейер раскладки XLSX

1. `xlsx::open(bytes) -> Result<Workbook>` — `lib.rs:198`.
2. `SheetLayout::new` / `with_margin` — `layout.rs:112` / `:118`; охват — used range
   плюс запас строк и столбцов.
3. Координаты и размеры (пиксели раскладки): `column_width` (`:191`), `row_height`
   (`:200`), `column_x` (`:208`), `row_y` (`:224`), `total_width` (`:237`),
   `total_height` (`:243`); обратные — `column_at` (`:249`), `row_at` (`:281`).
4. `paint::build(book, sheet, viewport, options, out)` — `paint.rs:298`; внутри
   `draw_region` (`:422`) и `draw_text` (`:1331`) кладут команды в `DisplayList`.
5. Перенос внутри ячейки — `wrap_layout` (`:1487`): вызывает
   `render::text_measure::break_lines` (`:1498`), высоту строки берёт из
   `FontRegistry::measure` (`:1497`).
6. Число, не помещающееся в ширину (`measure_text(...) > w - padding * 2`),
   подменяется на `#####` — `paint.rs:1399-1406`, константа `HASHES` — `:1517`.

## Метрики текста

Ширины считает `render::font::FontRegistry` по вкомпилированному TTF через
`skrifa` (нативные зависимости — `render/Cargo.toml:13-14`): сумма advance'ов
глифов плюс ascender/descender/line_gap. `CanvasRenderingContext2D.measureText` в
этом пути не участвует. `paint.rs` держит один реестр на поток (`thread_local!
FONTS` — `paint.rs:44-52`, LRU на 4096 записей) и переиспользует тёплый кэш.

Оговорка: `paint.rs` измеряет все ячейки `DEFAULT_FONT_ID`, независимо от шрифта
ячейки — и в `wrap_layout` (`:1497-1498`), и в правиле `#####` (`:1402`). Имя
гарнитуры ячейки уезжает в кадр строкой (`DrawCommand::Text { font, .. }` —
`display_list.rs:113`), но на метрики не влияет: байтов шрифтов книги в пакете
нет, а политика зафиксирована в ADR-0005 — метрики детерминированы. Следствие:
один и тот же `FontRegistry` даёт PDF те же ширины и точки разрыва, что canvas.

## Чего нет

- Пагинации: grep `paginate|PageBreak|page_break|pagination` по `crates/render/src`
  и `crates/xlsx/src` — 0 хитов. `SheetLayout` даёт непрерывные координаты листа;
  разбиения на страницы нет ни в каком виде.
- Потоковой записи: `PdfExporter` объявлен (`pdf/src/lib.rs:11`), но `export_docx`
  и `export_xlsx_sheet` — `todo!` (`:26`, `:36`), причём сигнатура возвращает
  `Result<Vec<u8>>` — весь файл в памяти.
- Логики страниц: `pdf/src/options.rs` — только типы: `PageSize` (`:4`),
  `PageOrientation` (`:13`), `Margins` (`:19`), `PageConfig` (`:38`),
  `PdfOptions` (`:69`). Поля `fit_to_width`, `repeat_header_rows`,
  `center_horizontally` и прочие нигде не читаются: вне `options.rs` на них нет
  ссылок, а `opts` в `pdf/src/lib.rs:13-14` помечен `#[allow(dead_code)]`.
- DOCX-пагинации тоже нет: `Document::page_count` возвращает 1-заглушку
  (`docx/src/lib.rs:26-29`, TODO Фаза 6).

## Что это значит для PDF-бэкенда (факты)

- `crates/pdf` не тянет wasm и проверяется нативно (`cargo check -p
  doc-converter-xlsx -p doc-converter-pdf` — успешно); canvas-свободность
  PDF-крейта ничем не ограничена.
- Координаты листа — пиксели раскладки: `SheetLayout` (`layout.rs:94`) и его
  `column_x`/`row_y`/`total_width`/`total_height`/`column_at`/`row_at`. В
  `DisplayList` координаты — физические пиксели canvas (`paint.rs:3`), то есть
  пиксели раскладки × `Viewport::scale` (зум × DPR, `viewport.rs:5,21`). Перевод
  в пункты PDF — арифметика вызывающего; в раскладке пунктов нет.
- Перенос строк переиспользуется как есть (`render::text_measure`): другого
  алгоритма в репозитории нет, поэтому PDF и canvas совпадут по точкам разрыва
  при общем `FontRegistry`.
- Кадр самодостаточен: `DrawCommand::Text` несёт имя гарнитуры строкой, а не
  индексом реестра (`display_list.rs:110-113`), поэтому canvas-painter и PDF могут
  рисовать один и тот же `DisplayList`, каждый своим контекстом.
- Диаграммы: в кадре — только данные (`DrawCommand::Chart`, блоб `ChartData`),
  отрисовка — в canvas-painter'е (`painter/chart.rs`); сами данные canvas-свободны.
