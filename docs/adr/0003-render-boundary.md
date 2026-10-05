# ADR 0003: Граница `crates/render`

**Статус:** принято 05.10.2026; модули выносятся в Спринте 5.5
**Контекст:** [`crates/render/src/lib.rs`](../../crates/render/src/lib.rs),
[`crates/render/src/geometry.rs`](../../crates/render/src/geometry.rs),
[`crates/xlsx/src/layout.rs`](../../crates/xlsx/src/layout.rs),
[`crates/xlsx/src/paint.rs`](../../crates/xlsx/src/paint.rs),
[`crates/docx/src/lib.rs`](../../crates/docx/src/lib.rs)

## Задача

Провести границу между `render` — общим слоем кадра — и `xlsx`/`docx`, которые
знают форматы. Вопрос не академический: `render` обязан оставаться независимым
от того, что именно рисуется, иначе второй потребитель (DOCX, Спринт 9) либо
скопирует листовую раскладку, либо потащит `xlsx` ради геометрии.

Почему это решается сейчас, до появления DOCX: у границы пока один потребитель,
и все неоднозначности видны. С приходом второго любая ошибка границы
закрепляется в двух местах сразу.

## Контекст: что в коде сейчас

- `crates/render/src/lib.rs` объявляет ровно три модуля: `display_list`,
  `painter`, `sab`. Наружу выходят `DisplayList`, `DrawCommand`,
  `DisplayListReader`, `Color`, `StringRef`, `TextAlign`, `TextBaseline`,
  `DecodeError`, painter (`Painter2D`, `PaintStats`) и ring (`SabRing`,
  `RingState`, `HEADER_BYTES`).
- `render::viewport`, `render::canvas`, `render::hit_test` как модули **не
  появились** — это сильнее, чем «не вынесены». Файлы
  [`geometry.rs`](../../crates/render/src/geometry.rs) (`Rect`, `Viewport`) и
  [`font.rs`](../../crates/render/src/font.rs) (`FontRegistry`) лежат в дереве,
  но в `lib.rs` не объявлены: они не компилируются вовсе. `cargo check -p
  doc-converter-render --all-targets` проходит без них, а `lru`, на который
  ссылается `font.rs`, отсутствует в графе зависимостей (`cargo tree -i lru` не
  находит пакета, хотя в `[workspace.dependencies]` запись есть). Там же лежит
  осиротевший `color.rs` со своим `Color { r, g, b, a }` — он уже разошёлся с
  `display_list::Color` (упакованный `RRGGBBAA`).
- Работу видимого диапазона, прокрутки и попадания по точке выполняют
  `xlsx::layout` и `xlsx::paint`:
  - [`layout.rs`](../../crates/xlsx/src/layout.rs) — `SheetLayout` с
    `column_x`/`row_y`, `column_at`/`row_at`, `columns_in`/`rows_in`,
    `column_width`/`row_height`, `total_width`/`total_height` и переводом ширин
    Excel в пиксели (`width_to_px`, `MAX_DIGIT_WIDTH`);
  - [`paint.rs`](../../crates/xlsx/src/paint.rs) — свой `Viewport { scroll_x,
    scroll_y, width, height, scale }` (стр. 31), своя `Geometry` (стр. 101) и
    `hit_test(sheet, viewport, x, y) -> CellRef` (стр. 172); `build` (стр. 209)
    режет окно на квадранты закреплённых областей и оборачивает каждый в
    `PushClip`/`PopClip`.
- Canvas-часть тоже раздвоена: `Painter2D` живёт в
  [`painter_2d.rs`](../../crates/render/src/painter/painter_2d.rs) (только
  wasm32), но жизненный цикл контекста — `init_painter`, `resize_canvas` с
  пересчётом под DPR — в [`crates/wasm/src/painter_api.rs`](../../crates/wasm/src/painter_api.rs),
  а коалесирование кадров — и вовсе в TS,
  [`packages/core/src/worker/frame_loop.ts`](../../packages/core/src/worker/frame_loop.ts).
- Что уже принадлежит `render` и работает: формат DisplayList (магия `DLST`,
  `DL_VERSION = 2`, заголовок 20 байт, однобайтовые теги), команды `Clear`,
  `Rect`, `Line`, `Text`, `Image`, `PushClip`/`PopClip`,
  `PushTransform`/`PopTransform`; текст несёт гарнитуру строкой в общем пуле и
  флаги `bold`/`italic` — формат самодостаточен, потому что painter читает его
  из SAB и не может спросить вызывающего. Геометрии как типов в `render` при
  этом нет: координаты в командах — голые `f32`, `PushClip` повторяет `x, y, w, h`.
- `docx` — заглушка: [`lib.rs`](../../crates/docx/src/lib.rs) открывает архив,
  зовёт `validate_ooxml` и возвращает `Document { _private: () }`;
  `page_count()` отдаёт 1 с TODO. Ни модели параграфов, ни раскладки — второго
  потребителя границы пока нет.

## Рассмотренные варианты

| Вариант | Почему нет |
|---|---|
| Оставить как есть: viewport/layout/hit-test в `xlsx` | с DOCX `Geometry`, квадранты и `hit_test` придётся копировать в `docx` или импортировать `xlsx` ради геометрии |
| Вынести в `render` весь `SheetLayout` | раскладка листа говорит на языке формата: ширины в символах Calibri, `<cols>`/`<row>` из SpreadsheetML; в `render` это чужой словарь |
| Вынести в `render` чистую геометрию: viewport, видимый диапазон, hit-test по DisplayList | выбран |
| Завести отдельный крейт `layout` над `render` | лишний крейт ради двух потребителей; граница «геометрия против модели формата» та же, только этажом выше |

## Решение

Граница из ROADMAP §11 (Риск B), развёрнутая по реальным типам:

| В `render` | В `xlsx` / `docx` |
|---|---|
| `DisplayList`, `DrawCommand`, `StringRef`, `Color`, кодеки `to_bytes`/`from_bytes`, версия и теги формата | Разбор OOXML и модель: `Workbook`, `Sheet`, `Cell`, `Merges`, `Pane`; `Document`, параграфы, runs |
| `Painter2D` (OffscreenCanvas 2D), `PaintState`, `BitmapCache` | — |
| `FontRegistry` и `render::text_measure` (ADR-0002, ADR-0005) | Решение, что рисовать: `numfmt`, `display_text`, `#####` для чисел |
| Геометрия: `Rect`, `Point`, clip/transform как команды DisplayList | Раскладка листа: `SheetLayout` — ширины в единицах формата, закрепления |
| `render::viewport`: скролл, видимый диапазон, зум и DPR | Семантика скролла: закреплённые области, границы листа |
| `render::hit_test` по геометрии DisplayList: точка → прямоугольник/команда | Маппинг попадания в ячейку или слово: `xlsx::paint::hit_test` → `CellRef` |
| `render::canvas`: жизненный цикл контекста, resize, коалесирование кадров | — |

Правило, по которому граница проверяется: **`render` не знает слов `Cell`,
`Row`, `Sheet`, `Paragraph`, `Run`**, а `xlsx`/`docx` не вызывают canvas — их
выход один, `DisplayList`. Всё, что нужно painter'у, уже лежит в командах
(`Text` с гарнитурой и начертанием, clip, transform); всё, что зависит от
формата (какой у ячейки формат числа, где у неё граница, что делать с числом,
не влезшим в столбец), остаётся у адаптера.

Отдельно фиксируется разделение масштаба: `xlsx::layout` считает координаты в
пикселях раскладки, а зум и DPR применяет `xlsx::paint::build`, собирая кадр
сразу в физических пикселях canvas. После выноса `render::viewport` владеет
переводом «раскладка → экран» (скролл, зум, DPR, видимый диапазон), а
`xlsx::layout` продолжает отвечать только за единицы формата (ширина столбца в
символах, высота строки в пунктах). Тогда `xlsx::paint::Viewport` и
`xlsx::paint::Geometry` схлопываются в `render::viewport` либо явно
обосновывают остаток — семантику закреплений и полос заголовков.

`xlsx::paint::cellAtPoint` из ROADMAP в коде не появился: функция называется
`hit_test` и станет адаптером над `render::hit_test`, а не самостоятельной
геометрией.

## Следствия

- Спринт 5.5 выносит `render::{viewport, canvas, hit_test, font, text_measure}`
  как модули; `xlsx::layout` и `xlsx::paint` остаются адаптерами. DoD Спринта
  5.5 прямо требует, чтобы эти модули существовали.
- Осиротевшие `geometry.rs`, `font.rs`, `color.rs` должны либо стать основой
  модулей (`geometry` — да, `font` — да, ADR-0005), либо исчезнуть. Файл,
  который лежит в дереве и не компилируется, опаснее отсутствующего: он
  выглядит как существующая граница, и на него ссылаются планы. `render::color`
  уже показывает цену: мёртвый дубль разошёлся с рабочим `display_list::Color`.
- При появлении DOCX его раскладка и пагинация идут в `docx`, а не в `render`:
  общими остаются DisplayList, painter, метрики и геометрия. `docx` добавит
  свои команды только если формат кадра перестанет их вмещать — тогда это
  изменение `DL_VERSION` и TS-стороны, а не расширение API `render`.
- Формат DisplayList остаётся контрактом между Rust и TS. Меняя раскладку
  команд, синхронизируйте `DL_VERSION`, теги и TS-читателей; версия в заголовке
  существует именно для того, чтобы старая сторона отказывалась, а не
  разбирала мусор.
- Риск B закрыт, когда `render` компилирует геометрию и метрики, а ни один
  формат не содержит прокрутки и попадания по точке в собственной реализации.
