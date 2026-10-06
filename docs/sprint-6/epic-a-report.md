# Эпик A — границы раскладки

- Спринт 6 «Базовый PDF» · ветка `sprint/6-basic-pdf`
- Коммиты: bb3809c (аудит), e9ef445 (ADR-0007), b934981 (каркас бэкенда)

## Что сделано

**A1. Аудит canvas-зависимости** (`docs/analysis/layout-audit.md`, bb3809c).
Проверено по коду на коммите 65987e4, что переиспользуется вне `wasm32`:

- canvas-зависимость изолирована: `render::canvas`,
  `render::painter::{painter_2d, chart, bitmap_cache}`,
  `render::sab::{writer, SabRing}` и `text_align_str` — под
  `#[cfg(target_arch = "wasm32")]` или на типах `web_sys`/`js_sys`;
- `crates/render/Cargo.toml` держит wasm-bindgen/js-sys/web-sys только в
  `[target.'cfg(target_arch = "wasm32")'.dependencies]`;
- нативно компилируются и canvas не требуют: `DisplayList`, `FontRegistry`,
  `text_measure`, геометрия/viewport/hit_test, `ChartData` и весь
  `crates/xlsx` (модель, `SheetLayout`, `paint::build`); в `crates/xlsx/src` —
  0 упоминаний web_sys, wasm_bindgen, js_sys, OffscreenCanvas;
- пагинации нет нигде: `crates/pdf` был заглушкой, DOCX-путь тоже
  (`Document::page_count` возвращает 1-заглушку). Лист задан непрерывной
  координатной плоскостью `SheetLayout` — вертикальную пагинацию PDF-путь
  строит сам.

**A2. ADR-0007 «Граница раскладки PDF»**
(`docs/adr/0007-pdf-layout-boundary.md`, e9ef445). Зафиксировано: крейт
`crates/layout-core` с `LayoutEngine` не создаётся; PDF-раскладка живёт в
`crates/pdf/src/layout.rs` и опирается на модель `xlsx` и
`render::{FontRegistry, text_measure}`. `xlsx::paint::wrap_layout` и PDF-путь
зовут одну и ту же `break_lines` с общим `FontRegistry`, поэтому совпадение
точек переноса получается по построению, а не синхронизацией двух реализаций;
на это кладётся тест F1.

**A3. Каркас PDF-бэкенда** (b934981). Модули
`crates/pdf/src/{styles, layout, fonts, text, border, background, painter, lib}.rs`;
`export_xlsx_sheet` собирает валидный PDF с текстом листа.

## Метрики

- wasm-зависимостей в `crates/xlsx` и `crates/pdf` — 0: `cargo check -p
  doc-converter-xlsx -p doc-converter-pdf` проходит нативно.
- Крейт `crates/layout-core` не появился (решение ADR-0007), `crates/pdf`
  зависит от `xlsx` и `render` напрямую.
- ADR-0007 принят 06.10.2026; в списке решений спринта — пункт 1, в
  отклонениях — отказ от `LayoutEngine`.
- 3 коммита эпика; 2 документа (`docs/analysis/layout-audit.md`,
  `docs/adr/0007-pdf-layout-boundary.md`).

## Отклонения от плана

- Вместо `crates/layout-core` с `LayoutEngine` (вход — IR документа, выход —
  `Vec<PageLayout>`), как в исходной постановке, — ADR о границе. Вынос
  раскладки в общий крейт отложен до DOCX-раскладки (Спринт 9): сейчас
  делить нечего, `xlsx`-модель canvas-свободна и так.
- Нумерация ADR 0007–0009 вместо 001–003: номера 0001–0006 заняты.
- Общих ADR/DOCX-наработок по пагинации не было — вертикальная пагинация
  листа сделана сразу в PDF-пути, вне этого эпика.
