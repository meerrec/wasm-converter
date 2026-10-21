# ADR-0033: PPTX-плагин — просмотр, PDF, PNG и текст по общему trait-слою

**Статус:** Принято
**Дата:** 21.10.2026
**Контекст:** Эпик 3, задача T3.1; отмена non-goal'а PPTX —
[ADR-0039](0039-cancel-v1-non-goals.md)
**Связанные ADR:** [ADR-0030](0030-format-plugin-traits.md) (trait-слой форматов),
[ADR-0011](0011-chart-geometry.md) (геометрия диаграмм),
[ADR-0015](0015-ooxml-zip-limits.md) (лимиты ZIP),
[ADR-0029](0029-main-thread-render-backend.md) (main-режим),
[ADR-0031](0031-streaming-zip.md) (потоковый вход),
[ADR-0032](0032-cancellation.md) (кооперативная отмена)

---

## Контекст

- PPTX был non-goal'ом v1 (`ROADMAP.md` §2). ADR-0039 снимает этот статус:
  документ архитектуры держит PPTX в матрице To-Be и требует полного паритета
  с DOCX/XLSX — просмотр, экспорт PDF, PNG и текста.
- `crates/render` уже несёт общий контракт форматов
  (`FormatPlugin`/`DocumentModel`/`RenderContext`, ADR-0030) и общую геометрию
  диаграмм (`render::chart`, ADR-0011). DisplayList — единственный контракт
  кадра: painter читает его из SAB и не может спросить вызывающего, что
  значит команда.
- Ленивые wasm-модули — принятая граница (ADR-0030 §г): DOCX живёт в
  `crates/docx-wasm`, XLSX — в основном модуле. Единого wasm-роутера нет:
  формат выбирает TS-сторона по явному `DocFormat`.
- Пакет PPTX — OOXML ZIP: `ppt/presentation.xml` (размер слайда
  `p:sldSz`), `ppt/slides/slideN.xml`, мастера `p:sldMaster`, темы
  `a:theme`, картинки `ppt/media/*`, диаграммы `ppt/charts/chartN.xml`.

## Решение

1. **Скоуп — паритет с DOCX/XLSX.** Просмотр слайдов, экспорт PDF (слайд =
   страница, размер из `p:sldSz`), PNG (штатный путь кадра через canvas,
   `export-png` протокола), текстовый конспект в Markdown/HTML/plain text из
   текстовых рамок слайдов. Редактирование, заметки докладчика, анимации и
   переходы в скоуп не входят.
2. **Новый крейт `crates/pptx`** — модель, разбор, раскладка, рендер,
   текстовый экспорт и плагин. Крейт зависит только от `doc-converter-core` и
   `doc-converter-render`: PDF собирается в `crates/pptx-wasm` через общий
   DisplayList-экспорт `crates/pdf`, поэтому нативный разбор не тянет PDF и
   его шрифты.
3. **Модель `Presentation`.** Слайды (`p:sld`), размер (`p:sldSz`), шейпы
   `p:sp` (текст из `a:p`/`a:r`/`a:t`, свойства `a:rPr`/`a:defRPr`),
   `p:pic` (media-часть по `r:embed`), `p:graphicFrame` с диаграммой
   (`c:chart` → `render::chart::ChartData`), мастер-слайды `p:sldMaster` и
   тема `a:theme` (шрифты major/minor, цвета `a:clrScheme`). Наследование
   геометрии плейсхолдеров — слайд → layout → master; каскад текста —
   `a:rPr` → `a:defRPr` абзаца → стиль уровня мастера
   (`p:titleStyle`/`p:bodyStyle`/`p:otherStyle`) → умолчания темы.
4. **Раскладка — фиксированный размер слайда без пагинации.** Слайд — одна
   страница рендера; координаты EMU переводятся в пиксели раскладки при
   96 dpi (1 px = 9525 EMU). Текст переносится тем же
   `render::text_measure`, что у DOCX; отступы `a:bodyPr`, якорь
   `t`/`ctr`/`b`, маркеры `a:buChar`/`a:buAutoNum`. Автоподбор кегля
   (`a:normAutofit`) не применяется: текст, не влезший в рамку, остаётся
   видимым, а не уменьшается.
5. **Рендер — DisplayList, `DL_VERSION` не трогается.** Команды — те же
   `Rect`/`Line`/`Text`/`Image`/`Chart`. Картинки отдаются интерфейсу тем же
   путём, что у DOCX: `pptx_images`/`pptx_image_bytes` → `ImageBitmap` →
   `register_bitmap`; `bitmap_id` команды `Image` — id media-части.
   Диаграммы сериализуются в блоб `render::chart` и рисуются painter'ом и
   PDF-бэкендом по общей геометрии.
6. **Плагин по контракту ADR-0030.** `PptxPlugin { id: "pptx" }`,
   `PptxDocument: DocumentModel`: `page_count` — число слайдов,
   `build_display_list` — первый слайд от начала координат. Конкретные кадры
   собирает `pptx::render` из wasm-модуля, как DOCX.
7. **Ленивый wasm-модуль `crates/pptx-wasm`** (по образцу `crates/docx-wasm`):
   `pptx_open`/`pptx_close`/`pptx_is_open`/`pptx_slide_count`,
   `pptx_images`/`pptx_image_bytes`,
   `pptx_build_display_list(_sab/_into)`, `export_pptx_pdf`,
   `export_pptx_text`. Пакет `@meerrec/wasm-pptx` генерируется wasm-pack'ом,
   в основной бандл не входит; бюджет — 480 КБ gzip.
8. **Диспетчеризация — на TS-стороне.** `DocFormat` получает `'pptx'`,
   `InitMsg` — `PptxModuleUrls`; воркер грузит модуль лениво в ветке `open`
   по формату. Wasm-роутера нет.
9. **PDF — через общий экспорт DisplayList.** `crates/pdf` получает
   `PdfExporter::export_display_list(pages, bitmaps)`: каждая страница —
   размер в пикселях раскладки и байты DisplayList. Это единственная новая
   точка PDF-крейта; `doc-converter-pdf` не зависит от `doc-converter-pptx`,
   а слайд не пересчитывается второй раз.
10. **Отмена — кооперативная (ADR-0032).** Разбор проверяет флаг между
    слайдами; wasm-модуль сбрасывает его в начале операции, как DOCX.

## Последствия

- PPTX не появляется в основном модуле: бюджет `wasm (viewer)` не растёт,
  гейт «нет `docx_*` в viewer» расширяется до `pptx_*`.
- `crates/pdf` учится рисовать произвольный DisplayList — это переиспользуемо
  для будущих форматов (ODF) и не создаёт второй раскладки.
- Ограничения первой версии: автоподбор кегля, анимации, заметки и
  трёхмерные эффекты не рисуются; скругления `Rect` в PDF пока рисуются
  прямыми углами (canvas — радиусом).
- Оракул — LibreOffice Impress (`soffice --convert-to pdf`, SSIM ≥ 0.95);
  независимого дифференциального эталона для слайдов нет, поэтому SSIM-оракул
  остаётся единственным гейтом (решение задачи T3.1).
- Фаззинг — цель `pptx_presentation` (пакет с подставленной частью слайда),
  ночной прогон по общему расписанию `fuzz.yml`.

## Верификация

- `cargo test -p doc-converter-pptx` — разбор, раскладка, текст, PDF-общие
  тесты; фикстуры `test-fixtures/pptx`.
- `wasm-pack test --headless --chrome --release crates/pptx-wasm`.
- `pnpm size-limit` — строка `wasm (pptx)` ≤ 480 КБ gzip.
- e2e `examples/viewer/e2e/pptx.spec.ts` и оракул-джоба `pptx` в
  `oracle.yml`.
