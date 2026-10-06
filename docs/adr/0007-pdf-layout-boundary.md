# ADR 0007: Граница раскладки PDF — без отдельного крейта `layout-core`

**Статус:** принято 06.10.2026; реализуется в Спринте 6 (базовый PDF-экспорт XLSX).
**Контекст:** [`crates/pdf/src/lib.rs`](../../crates/pdf/src/lib.rs),
[`crates/pdf/src/options.rs`](../../crates/pdf/src/options.rs),
[`crates/pdf/Cargo.toml`](../../crates/pdf/Cargo.toml),
[`crates/xlsx/src/layout.rs`](../../crates/xlsx/src/layout.rs),
[`crates/xlsx/src/paint.rs`](../../crates/xlsx/src/paint.rs),
[`crates/render/src/font.rs`](../../crates/render/src/font.rs),
[`crates/render/src/text_measure.rs`](../../crates/render/src/text_measure.rs),
[аудит раскладки](../analysis/layout-audit.md), [ADR-0003](0003-render-boundary.md)

## Контекст

Спринт 6 делает базовый экспорт XLSX-листа в PDF. Обсуждался отдельный крейт
`crates/layout-core` с `LayoutEngine` (вход — IR документа, выход — `Vec<PageLayout>`),
которому подчинялись бы и canvas, и PDF. Что показывает код:

- Раскладка XLSX уже живёт вне canvas:
  [`layout.rs`](../../crates/xlsx/src/layout.rs) — `SheetLayout` с координатами в
  пикселях раскладки (`layout.rs:94`), [`paint.rs`](../../crates/xlsx/src/paint.rs)
  собирает `DisplayList`. В `crates/xlsx/src` — 0 упоминаний `web_sys`,
  `wasm_bindgen`, `js_sys`, `OffscreenCanvas` ([аудит](../analysis/layout-audit.md)).
- Метрики и перенос уже общие и canvas-свободны: `render::font::FontRegistry`
  (`font.rs:72`, `skrifa`) и `render::text_measure::break_lines`
  (`text_measure.rs:31`). Canvas-путь зовёт их уже сейчас: `xlsx::paint::wrap_layout`
  (`paint.rs:1487`) → `break_lines` (`paint.rs:1498`).
- Пагинации нет нигде: grep `paginate|PageBreak|page_break` по `crates/render/src`
  и `crates/xlsx/src` — 0 хитов. Переносить «логику разбиения из canvas» нечего:
  её не существует.
- ROADMAP §6 предписывает PDF-модули внутри `crates/pdf` (`styles.rs`, `layout.rs`,
  `fonts.rs`, `text.rs`, `border.rs`, `background.rs`, `painter.rs`), а ADR-0003
  фиксирует границу: «`render` не знает слов `Cell`, `Row`, `Sheet`, `Paragraph`,
  `Run`». `PageSize`, `PageOrientation` и `Margins` из пункта ROADMAP про
  `layout.rs` уже лежат в `pdf/src/options.rs` (`options.rs:4,13,19`) — место в
  `layout.rs` свободно под пагинацию.
- Публичная сигнатура `PdfExporter::export_xlsx_sheet(&mut self, wb: &Workbook,
  sheet: usize) -> Result<Vec<u8>>` (`pdf/src/lib.rs:31`) сохраняется: pub-API
  существующих крейтов не меняется.

## Рассмотренные варианты

1. **Крейт `crates/layout-core` с `LayoutEngine` и IR.** Плюсы: один источник
   истины для canvas и PDF, общий IR пригодился бы второму формату. Минусы: IR,
   которого нет, — его пришлось бы проектировать и подгонять под него canvas-путь;
   противоречит ADR-0003 и ROADMAP §6; для DoD Спринта 6 выгоды нет. Отложено до
   раскладки DOCX (Спринт 9): там появляется второй реальный потребитель, и общий
   слой может понадобиться по-настоящему.
2. **PDF как интерпретатор DisplayList** (переиспользовать готовые команды кадра).
   Плюсы: совпадение с экраном по построению, дублировать нечего. Минусы:
   DisplayList — контракт Rust↔TS для экранного кадра: координаты viewport, клипы,
   z-порядок; PDF нужны страничные координаты, потоки контента и шрифты. В
   Спринте 7 через DisplayList пойдут только диаграммы (`render::chart::ChartData`,
   `chart.rs:47`), отрисовка останется у painter'а.
3. **Дублировать раскладку в `crates/pdf`.** Плюсы: PDF-крейт не зависит от чужих
   типов и волен оптимизировать под себя. Минусы: две реализации переноса
   разойдутся, а DoD Спринта 6 требует совпадения точек разрыва. Отвергнуто.

## Решение

Крейт `layout-core` не создаём. PDF-раскладка живёт в `crates/pdf/src/layout.rs` и
опирается на зависимости, уже объявленные в `crates/pdf/Cargo.toml`:
`doc-converter-xlsx` (`Workbook`, `SheetLayout` — координаты в пикселях раскладки)
и `doc-converter-render` (`FontRegistry`, `text_measure::break_lines`).

Совпадение точек переноса достигается по построению: `xlsx::paint::wrap_layout` и
PDF-путь вызывают одну и ту же `break_lines` с общим `FontRegistry`; тест на
совпадение (пункт DoD «Единая логика переноса») это фиксирует.

`crates/pdf` остаётся canvas-свободным: `web-sys`, `js-sys`, `wasm-bindgen` не
появляются в нём даже под wasm-таргетом.

## Следствия

- `crates/pdf` получает зависимости `doc-converter-xlsx` и `doc-converter-render`
  (уже объявлены в его `Cargo.toml`).
- Пагинация листа появляется сразу в `crates/pdf/src/layout.rs`: минимальная
  вертикальная — нужна для DoD «10 страниц < 300 мс». Полная пагинация (повтор
  заголовков, разбивка по столбцам) — Спринт 7, `pdf/src/pagination.rs`.
- PDF-крейт обязан оставаться canvas-свободным: не тянуть `web-sys` даже под
  wasm-таргет.
- Перенос совпадает с canvas потому, что переиспользуется `break_lines`, а не
  потому, что написан «такой же» алгоритм; отдельный тест страхует от
  расхождения.
- Наследуется ограничение canvas-пути: `paint.rs` измеряет все ячейки
  `DEFAULT_FONT_ID` (шрифт ячейки в метриках не участвует — `paint.rs:1497-1498`;
  правило `#####` — `paint.rs:1402`). PDF повторяет это поведение, чтобы ширины и
  точки разрыва не разошлись с экраном.
- Вынос раскладки в отдельный крейт — кандидат на Спринт 9, когда появится
  DOCX-раскладка и у общего слоя будет второй потребитель.
