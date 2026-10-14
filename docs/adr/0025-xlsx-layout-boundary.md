# ADR-0025: Граница раскладки XLSX — без отдельного крейта

**Статус:** Принято 14.10.2026.
**Связанные ADR:** [ADR-0003](0003-render-boundary.md) (граница `render`),
[ADR-0007](0007-pdf-layout-boundary.md) (граница раскладки PDF),
[ADR-0023](0023-libreoffice-wasm-exact-backend.md) (no-go Спринта 13).

## Контекст

14.10.2026 рассмотрено предложение (черновик ADR и техпроект, вне репозитория)
вынести раскладку листа XLSX в новый крейт `doc-converter-xlsx-layout`: движок
`build()` / `frame()` / `paginate()` / `hit()`, IR `SheetFrame`, canvas-адаптер
внутри крейта, переезд `crates/pdf` на страницы-`SheetFrame`. Главной проблемой
в предложении названо дублирование семантики раскладки между canvas- и
PDF-путями. Проверка предложения против кода репозитория:

- **Дублирования нет.** Перенос строк — одна функция на оба пути:
  `render::text_measure::break_lines`
  ([`text_measure.rs:31`](../../crates/render/src/text_measure.rs)). Её зовут
  canvas-путь `xlsx::paint` ([`paint.rs:1507`](../../crates/xlsx/src/paint.rs))
  и PDF-путь ([`text.rs:75`](../../crates/pdf/src/text.rs)); совпадение точек
  разрыва фиксирует тест `pdf_wraps_text_exactly_like_canvas`
  ([`export.rs:496`](../../crates/pdf/tests/export.rs)). `display_text` и
  `anchor_rect` PDF импортирует из `xlsx::paint`
  ([`painter.rs:28`](../../crates/pdf/src/painter.rs)), `resolve_color` —
  оттуда же ([`border.rs:8`](../../crates/pdf/src/border.rs),
  [`styles.rs:9`](../../crates/pdf/src/styles.rs)).
- **Условие ADR-0007 не наступило.** Там вынос раскладки в отдельный крейт
  назван кандидатом на Спринт 9 — «когда появится DOCX-раскладка и у общего
  слоя будет второй потребитель»
  ([ADR-0007](0007-pdf-layout-boundary.md), «Следствия»).
  Предложение отвергает общий слой для XLSX и DOCX, то есть снимает условие,
  которым обосновано.
- **Случай уже разобран.** ADR-0003 отверг вариант «Завести отдельный крейт
  `layout` над `render`»: «лишний крейт ради двух потребителей; граница
  „геометрия против модели формата“ та же, только этажом выше»
  ([ADR-0003](0003-render-boundary.md), «Рассмотренные варианты»). Третий
  потребитель в предложении — гипотетические будущие батч-превью.
- **Порядок слоёв зафиксирован в ROADMAP.** «Раскладка и попадание по
  координате живут в `crates/xlsx`, а не в `crates/render`… Когда появится
  DOCX, общее переедет в `render`, а не наоборот»
  ([`ROADMAP.md`](../../ROADMAP.md), §6).
- **Цель «тестировать раскладку без wasm» уже достижима.** Тесты
  [`crates/xlsx/tests/`](../../crates/xlsx/tests) (`open`, `snapshots`,
  `budget`, `charts`, `images`, `print_settings`, `fixtures`) нативные;
  `insta` подключён ([`crates/xlsx/Cargo.toml:43`](../../crates/xlsx/Cargo.toml),
  [`Cargo.toml:78`](../../Cargo.toml)).
- **Зависимость PDF от модели формата не снимается.** `crates/pdf` использует
  `Workbook`, `Sheet`, `Cell`, `CellRef`, `Range`, `PrintSettings`,
  `HyperlinkTarget`, `Comment`, `WorkbookImage` (модули `pagination.rs`,
  `painter.rs`, `annot.rs`, `styles.rs`); общего IR для них предложение не
  даёт. Граница `pdf → xlsx` зафиксирована ADR-0007 осознанно.
- **Цена переезда.** Модули раскладки в `crates/xlsx`:
  [`paint.rs`](../../crates/xlsx/src/paint.rs) (3744 строки),
  [`conditional.rs`](../../crates/xlsx/src/conditional.rs) (2260),
  [`layout.rs`](../../crates/xlsx/src/layout.rs) (578); переключение
  [`pagination.rs`](../../crates/pdf/src/pagination.rs) (2194) и
  [`painter.rs`](../../crates/pdf/src/painter.rs) (1507); окно сосуществования
  двух путей раскладки. Плюс перечни крейтов: корневой
  [`Cargo.toml`](../../Cargo.toml),
  [`scripts/build-wasm.sh`](../../scripts/build-wasm.sh),
  [`ci.yml`](../../.github/workflows/ci.yml),
  [`release.yml`](../../.github/workflows/release.yml),
  [`oracle.yml`](../../.github/workflows/oracle.yml),
  [`AGENTS.md`](../../AGENTS.md).
- **Момент.** Спринт 12 закрыт и ожидает публикации v1.0.0, Спринт 13 закрыт
  решением no-go ([ADR-0023](0023-libreoffice-wasm-exact-backend.md)); переезд
  такого размера в релизном окне не оправдан.
- Часть утверждений предложения опиралась на устаревшие слои документации —
  доккомментарий [`crates/pdf/src/layout.rs`](../../crates/pdf/src/layout.rs)
  про «Спринт 7», шапку ROADMAP §6 «состояние после Спринта 5», перечни
  крейтов, отставшие от фактического состава `crates/`.

## Рассмотренные варианты

| Вариант | Почему нет |
|---|---|
| Новый крейт `doc-converter-xlsx-layout` с `build()` / `frame()` / `paginate()` / `hit()`, IR `SheetFrame` и переездом PDF на его страницы | Устранять нечего: перенос — одна функция `break_lines` на оба пути; зависимость PDF от модели формата остаётся; условие ADR-0007 не наступило, а «крейт над `render`» уже отвергнут ADR-0003; цена — переезд модулей раскладки (~6,6 тыс. строк) и переключение PDF-модулей ради гипотетического потребителя |
| Перенести раскладку XLSX в `crates/render` | Раскладка листа говорит на языке формата: ширины в единицах SpreadsheetML, `<cols>`/`<row>`, закрепления; в `render` это чужой словарь (ADR-0003). Движение обратное: общее переезжает в `render`, когда у него появляется второй потребитель, — раскладка листа в их число не входит |
| Оставить границу как есть: раскладка в `crates/xlsx`, PDF зависит от `xlsx` и `render` | выбран |

## Решение

Раскладка XLSX остаётся в `crates/xlsx`; крейт `doc-converter-xlsx-layout` не
создаётся. Границы ADR-0003 и ADR-0007 подтверждаются без изменений: `render`
не знает слов `Cell`, `Row`, `Sheet`, а `crates/pdf` опирается на модель
`crates/xlsx` и метрики `crates/render`.

Пересмотр — только новым ADR и только при появлении **реального** второго
потребителя раскладки (например, ленивая подгрузка или батч-превью v2): общий
слой проектируется под конкретного потребителя, а не под гипотетического.

## Следствия

- `crates/pdf` продолжает зависеть от `doc-converter-xlsx` и
  `doc-converter-render` — это осознанная граница (ADR-0007), а не долг.
- Чистка раскладки, если понадобится, идёт **внутри** `crates/xlsx`: `paint.rs`
  распиливается на модули (геометрия окна, текст ячейки, границы), без нового
  крейта и без второго пути раскладки.
- Дедупликация рёбер границ между соседними ячейками — отдельный микро-слайс в
  `crates/xlsx`; проверяется SSIM-оракулом.
- Нативные `insta`-снапшоты IR раскладки можно добавлять уже сейчас, в
  [`crates/xlsx/tests/`](../../crates/xlsx/tests): цель «тестирование без wasm»
  не требует нового крейта.
- Пока реального второго потребителя нет, граница не пересматривается;
  появление потребителя оформляется новым ADR, а не правкой этого.
