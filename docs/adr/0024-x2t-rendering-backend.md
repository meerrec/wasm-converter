# ADR 0024: x2t (ONLYOFFICE) как рисующий бэкенд в браузере — решение no-go

**Статус:** Принято 13.10.2026 — **no-go**. Направление «x2t как рисующий бэкенд
в браузере» закрыто; условия пересмотра — в конце документа.
**Контекст:** [`docs/analysis/x2t-rendering-spike.md`](../analysis/x2t-rendering-spike.md),
[ADR-0022](0022-own-pdf-backend.md), [ADR-0023](0023-libreoffice-wasm-exact-backend.md),
[`experiments/x2t-spike/`](../../experiments/x2t-spike/README.md),
[`packages/x2t/vendor/`](../../packages/x2t/vendor/README.md).

## Контекст

Спайк 13.10.2026 — следующий шаг после LOWA, закрытой решением no-go
([ADR-0023](0023-libreoffice-wasm-exact-backend.md)). Проект рендерит OOXML своим кодом
и пишет PDF своим бэкендом ([ADR-0022](0022-own-pdf-backend.md)); «точный» рисующий
бэкенд закрывал бы перечислимые пробелы собственного рендера по построению. Кандидат —
**x2t**, конвертер форматов из состава ONLYOFFICE: та же нативная кодовая база, что
у Document Server, уже собранная в WASM. Артефакт и обвязка готовы: релизная сборка
`cryptpad/onlyoffice-x2t-wasm` v9.3.2+3 (`x2t.wasm` 38,7 МБ, Git LFS —
[`packages/x2t/vendor/`](../../packages/x2t/vendor/README.md)), пакет
[`@doc-converter/x2t`](../../packages/x2t/README.md) и харнесс
[`experiments/x2t-spike/`](../../experiments/x2t-spike/README.md): настоящий Chromium
через Playwright гоняет конвертации и показывает коды возврата `main1`.

Вопрос спайка был узкий: **может ли этот артефакт рисовать** — отдавать pdf/png/jpg/html, —
а не «хорош ли x2t как конвертер». Ответ по рисованию отрицательный; конвертерная
годность проверялась попутно и тоже ограничена (см. «Последствия»).

## Решение и обоснование

**No-go. x2t не может быть рисующим бэкендом в браузере — рисовать нечем.**

- **Рисует в апстриме не x2t, а sdkjs под V8.** Конвейер DOCX → PDF трёхступенчатый:
  C++ `BinDocxRW::CDocxSerializer` разбирает DOCX во внутренний bin; **sdkjs под V8**
  раскладывает и «печатает» страницы, результат — `pdf.bin`, **метафайл команд рисования,
  а не PDF**; затем C++ `CPdfFile::OnlineWordToPdfFromBinary` переводит метафайл в настоящий
  PDF (альтернативный путь — `bin2image` → png/jpg). x2t и docbuilder — обёртки над одним
  и тем же движком: точки входа `word/api.js` (`asc_nativeGetPDF`), рендерер
  `common/Drawings/Metafile.js`, для XLSX — `cell/graphics/pdfprinter.js`.
- **В wasm-сборке движок вырезан при сборке.** В `Dockerfile` (тег v9.3.2+3)
  `doctrenderer_empty.cpp` копируется поверх настоящего: `CDoctrenderer::ExecuteScript()` —
  пустая `{ return true; }`, вырезаны `NSDoctRenderer::CreateEditorContext`/`RunEditor`,
  `JSSmart<CJSContext>`, `asc_nativeGetPDF`. В `x2t.wasm` нет ни литералов V8, ни ICU —
  движка раскладки нет вовсе.
- **114 живых заглушек — следствие линковки, а не замысел.** Сборка идёт с
  `-sERROR_ON_UNDEFINED_SYMBOLS=0` (`embuild.sh:9`), поэтому остаточные вызовы движка
  не роняют линковку, а становятся `abort`-стабами; все они объявлены как `wasmImports`,
  то есть вызываются реально (OpenSSL — 71, libheif — 18, `NSJSBase::CJSContext` —
  12, `StarMath` — 10, `CNativeControlEmbed` — 2, `COFDFile` — 1).
- **Наблюдаемый результат — код 80 = `AVS_FILEUTILS_ERROR_CONVERT`** (апстрим,
  `getReturnCode` в `X2tConverter/src/cextracttools.cpp:72`). Так выглядит отрезанный рендер:
  прогоны харнесса дают 80 у всех рендер-целей (docx → pdf/html/png, xlsx → png/jpg) и у
  кросс-семейных конвертаций; 0 — только внутри семейства (docx → odt/rtf, xlsx → ods/csv).
- **Нативная половина конвейера в сборке есть, но `pdf.bin` взять неоткуда.** В `x2t.wasm`
  присутствуют RTTI `CPdfFile`, `PdfWriter`, `NSGraphics::IGraphicsRenderer`, `Aggplus`,
  `CFontManager`, литералы `%PDF-1.7` — это вторая половина (метафайл → PDF/растр), которую
  питает sdkjs под V8, а движка раскладки нет.
- **Апстрим порт не планирует** (`ONLYOFFICE/DocumentServer#731`: «not planned»),
  рисующих wasm-сборок нет ни у апстрима, ни у форков; README `@x2t/wasm` прямо пишет,
  что PDF/SVG/image требуют OnlyOffice Canvas renderer, которого в WASM-сборке нет.
- **Prior art подтверждает механику.** `ranuts/document` рисует не x2t: sdkjs сериализует
  уже отрисованный документ в canvas-поток (`m_nFormatFrom=8196`, CANVAS_PDF), а x2t лишь
  транслирует поток в PDF (`m_nFormatTo=513`); прямой docx/xlsx → PDF там тоже код 80.

| Путь | Цена | Риск | Вердикт |
|---|---|---|---|
| x2t.wasm как рисующий бэкенд (docx/xlsx → pdf/png в браузере) | артефакт уже собран и обёрнут (38,7 МБ) | рисовать нечем: движка раскладки в сборке нет, все рендер-цели дают код 80 | **no-go** |
| Снять заглушку `ExecuteScript` и сшить `CJSContext` с jitless-движком в wasm (QuickJS/SpiderMonkey) | месяцы исследования | мост, DOM/canvas-шим и шрифтовой движок с нуля; движок в 3–12× медленнее JIT-V8; публичных прецедентов нет | не окупается |
| x2t как конвертер внутри семейства форматов (docx↔odt/rtf/txt, xlsx↔ods/csv) | обвязка готова ([`@doc-converter/x2t`](../../packages/x2t/README.md)) | на реальных документах падает и портит пакет (`word/footer8.xml`, `memory access out of bounds`) | единственная рабочая область, и та с оговоркой |
| Серверный путь: Document Server или нативный x2t | ~1,5 ГБ сжатый образ (7.2.1) + слой шрифтов ~190 МБ, 2+ ГБ RAM | AGPL-3.0, файлы пользователя покидают устройство; Document Builder CLI с 8.3.0 ставит водяной знак | вне клиентской модели продукта |

## Последствия

- **Рисующий бэкенд в браузере закрыт.** Собственный рендер XLSX/DOCX остаётся основным
  путём; ADR-0022 (свой PDF-бэкенд) действует без изменений.
- **x2t остаётся разрешённым только как конвертер внутри семейства форматов** —
  docx↔odt/rtf/txt, xlsx↔ods/csv — через [`@doc-converter/x2t`](../../packages/x2t/README.md).
  И там с оговоркой: на реальных документах конвертации падают или портят пакет
  (повреждённый `word/footer8.xml`, `memory access out of bounds`, воспроизведено
  в харнессе). Это не «поддерживаемая возможность», а зафиксированная граница.
- **Что NOT входит в это решение:** серверный ONLYOFFICE (Document Server или один нативный
  x2t). Таблица выше фиксирует его цену, но выбор серверной модели — отдельное решение
  и здесь не принимается; продукт клиентский, файлы пользователя не покидают устройство.
- **Харнесс и обвязка остаются как есть:** [`experiments/x2t-spike/`](../../experiments/x2t-spike/README.md)
  и [`packages/x2t/`](../../packages/x2t/README.md) — рабочая площадка замера, с которой
  начинается пересмотр решения (если условия изменятся), а не с новой разведки.

## Когда решение пересматривается

1. **Появление публичной wasm-сборки x2t с рабочим движком раскладки** — с настоящим
   doctrenderer (V8 + sdkjs), а не `doctrenderer_empty.cpp`. Сегодня таких сборок нет
   ни у апстрима, ни у форков.
2. **Готовый мост `NSJSBase::CJSContext` к jitless-движку** (QuickJS/SpiderMonkey)
   в wasm: снимает заглушку `ExecuteScript` и позволяет переиспользовать нативную половину
   (метафайл → PDF). Сегодня его нет, а сборка такого моста — исследование на месяцы;
   сравнивать придётся с уже работающим LOWA-решением (4,62 с на 161 страницу).
3. **Смена клиентской модели продукта** — появление серверной части, при которой серверный
   ONLYOFFICE перестаёт быть «вне клиентской модели» (см. «Последствия»). Это продуктовое
   решение владельца, а не следствие этого ADR.

До любого из этих пунктов вердикт — no-go, и направление не открывается заново
«на всякий случай».

## Ссылки

- Разбор спайка: [`docs/analysis/x2t-rendering-spike.md`](../analysis/x2t-rendering-spike.md) —
  прогоны харнесса, разбор вендоренных `x2t.js`/`x2t.wasm`, разбор апстрима, prior art, таблица путей.
- Харнесс: [`experiments/x2t-spike/README.md`](../../experiments/x2t-spike/README.md).
- Обвязка: [`packages/x2t/README.md`](../../packages/x2t/README.md); артефакты сборки —
  [`packages/x2t/vendor/README.md`](../../packages/x2t/vendor/README.md).
- Смежные решения: [ADR-0022](0022-own-pdf-backend.md) (свой PDF-бэкенд),
  [ADR-0023](0023-libreoffice-wasm-exact-backend.md) (LibreOffice-WASM — no-go).
- Отчёт спринта 13: [`docs/sprint-13/report.md`](../sprint-13/report.md).
