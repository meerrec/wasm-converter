# Спайк: x2t (ONLYOFFICE) как рисующий бэкенд в браузере

**Дата:** 13.10.2026
**Контекст:** спайк 13.10.2026, следующим шагом после LOWA-спайка спринта 13
([отчёт спринта 13](../sprint-13/report.md)). Вопрос разведки — можно ли взять x2t, конвертер
ONLYOFFICE, «точным рисующим бэкендом» (экспорт PDF/PNG) в браузере. Проект рендерит
OOXML своим кодом и пишет PDF своим бэкендом ([ADR-0022](../adr/0022-own-pdf-backend.md)),
LOWA закрыта решением no-go ([ADR-0023](../adr/0023-libreoffice-wasm-exact-backend.md)).
Харнесс спайка — [`experiments/x2t-spike/`](../../experiments/x2t-spike/README.md),
обвязка — пакет [`@meerrec/x2t`](../../packages/x2t/README.md), артефакты —
[`packages/x2t/vendor/`](../../packages/x2t/vendor/) (`x2t.wasm` 38,7 МБ, Git LFS,
сборка `cryptpad/onlyoffice-x2t-wasm` v9.3.2+3).

## Что измерено

Прогоны харнесса (Chromium через Playwright, 13.10.2026) и разбор вендоренных
`x2t.js` / `x2t.wasm`.

| Конвертация | Код возврата `main1` |
|---|---|
| docx → pdf, html, png | 80 |
| xlsx → png, jpg, docx, odt, html | 80 |
| xlsx → ods, csv | 0 |
| docx → odt, rtf | 0 |

- **Код 80 = `AVS_FILEUTILS_ERROR_CONVERT`** (апстрим `core/Common/OfficeFileErrorDescription.h`,
  `getReturnCode` в `X2tConverter/src/cextracttools.cpp:72`). Так выглядит отрезанный рендер:
  80 — у рендер-целей (pdf, html, png, jpg) и у кросс-семейных конвертаций (xlsx → docx/odt),
  0 — у конвертации внутри семейства (docx → odt/rtf, xlsx → ods/csv).
- **114 заглушек `abort("missing function: …")` в `x2t.js`**, и все объявлены как
  `wasmImports` — то есть вызываются реально: OpenSSL — 71, libheif — 18,
  `NSJSBase::CJSContext*` / `CreateDefaults` — 12, `StarMath::CStarMathConverter` — 10,
  `CNativeControlEmbed` — 2, `COFDFile` — 1.
- **Вторая половина конвейера (метафайл → PDF/растр) в `x2t.wasm` есть:** RTTI `CPdfFile`,
  `PdfWriter::CEncrypt/CEncryptDict/CSignatureDict/CStructureTreeRoot`,
  `NSGraphics::IGraphicsRenderer`, `CGraphicsRenderer`, `Aggplus`,
  `NSFonts/CApplicationFonts/CFontManager`, литералы `%PDF-1.7`; есть даже PDF-ридер
  (`PdfReader::RendererOutputDev`). Литералов V8/ICU нет — движка раскладки нет вовсе.
- **На реальных документах конвертации падают или портят пакет:** повреждённый
  `word/footer8.xml`, падение `memory access out of bounds` (воспроизведено в харнессе).
- **Трансляция готового потока в PDF при этом работает:** проба в Node (13.10.2026) —
  `m_nFormatFrom=8196` (CANVAS_PDF) → `m_nFormatTo=513` (PDF) на 17 байтах нулей даёт код 0
  и валидный `%PDF-1.7` на 1576 Б; тот же вызов с docx вместо потока — код 80, а контроль
  `docx 65 → odt 67` — код 0 и те же 3424 Б, что в браузерном прогоне. То есть правая
  половина конвейера в сборке не просто присутствует, а работает; нет только производителя
  потока — sdkjs под движком.

## Кто рисует в апстриме

По исходникам апстрима (`ONLYOFFICE/core`, `sdkjs`, `DocumentServer`, `build_tools`)
конвейер DOCX → PDF трёхступенчатый:

1. **C++:** `BinDocxRW::CDocxSerializer` разбирает DOCX во внутренний bin (docx2doct).
2. **sdkjs под V8:** движок раскладывает и «печатает» страницы; результат — `pdf.bin`,
   **метафайл команд рисования, а не PDF**.
3. **C++:** `CPdfFile::OnlineWordToPdfFromBinary` (`PdfFile/OnlineOfficeBinToPdf.cpp`)
   переводит метафайл в настоящий PDF; альтернативный путь — `bin2image` → png/jpg.

Детали, важные для оценки:

- точки входа JS — `word/api.js` (`asc_nativeGetPDF` / `asc_nativePrint`), рендерер —
  `common/Drawings/Metafile.js` (`CDocumentRenderer`), для XLSX — `cell/graphics/pdfprinter.js`;
- граница C++ ↔ JS — `DesktopEditor/doctrenderer/doctrenderer.cpp`;
- движок — V8 (`doctrenderer/js_internal/v8/v8_base.cpp`); он же у docbuilder и x2t,
  разница только в обёртке;
- шрифты обязательны: `sdkjs/common/libfont/engine/fonts_native.js` (FreeType/HarfBuzz),
  `AllFonts.js`, `font_selection.bin`, генератор `allfontsgen` (`x2t -create-allfonts`);
  темы — `allthemesgen`;
- рисует (JS): pdf, png/jpg, html; чистая конвертация (C++): odt/rtf/txt/csv/ods/xlsb,
  docx↔bin, bin→pdf, pdf→docx;
- рядом с x2t в поставке `document-server` — 17 `lib*.so` (libdoctrenderer, graphics,
  PdfFile, …), `DoctRenderer.config`, `cmap.bin`, `docbuilder`, ICU.

## Почему в wasm не рисует

- **Рендер вырезан при сборке.** В `Dockerfile` сборки (тег v9.3.2+3), строки 466–468,
  `doctrenderer_empty.cpp`, `docbuilder_empty.cpp`, `docbuilder_p_empty.cpp` копируются
  поверх настоящих. В `doctrenderer_empty.cpp` `CDoctrenderer::ExecuteScript()` — пустая
  `{ return true; }`; вырезаны `NSDoctRenderer::CreateEditorContext` / `RunEditor`,
  `JSSmart<CJSContext>`, `asc_nativeGetPDF`.
- **Заглушки — следствие линковки**, а не замысел файла-замены: сборка идёт с
  `-sERROR_ON_UNDEFINED_SYMBOLS=0` (`embuild.sh:9`), поэтому остаточные вызовы движка
  не роняют линковку, а становятся `abort`-стабами (те самые 114).
- **V8 и JSC в дереве есть, но не компилируются** (`js_base.pri` закомментирован);
  из ~28 слинкованных архивов исключены также docbuilder/app_builder, AllFontsGen,
  allthemesgen.
- **Апстрим порт не планирует.** `ONLYOFFICE/DocumentServer#731` (2019): «code relying
  on doctrenderer which relies on v8» в wasm не работает; в 2023 — «not planned» и
  «module will be large». Сборка — emscripten/qmake, emsdk 4.0.11 закреплён, ~34 мин
  в CI на ubuntu-latest.
- **Рисующих сборок нет ни у кого.** Ветки, PR и issues репозитория — не про рендер;
  форки (`liangyingnan/very-of-x2t-wasm`, `xenode-in/…`) и npm `@x2t/wasm` — производные
  от той же сборки; README `@x2t/wasm` прямо пишет, что PDF/SVG/image требуют OnlyOffice
  Canvas renderer, которого в WASM-сборке нет, и советует LibreOffice headless или
  полный Document Server.

## Ответ на «а если движок в wasm»

- **V8 в wasm нет**: публичных сборок нет, jitless-режим не отдаёт wasm
  («Do not expose wasm in jitless mode»), а JIT в wasm невозможен.
- **В wasm реально живут**: QuickJS (`quickjs-emscripten`, ~1 МБ, ES2020) и SpiderMonkey
  (`bytecodealliance/StarlingMonkey`, ~8 МБ, интерпретатор + инлайн-кэши, без JIT;
  прод — Fastly/Spin). JavaScriptCore-в-wasm не найден.
- **Цена**: jitless-движки медленнее JIT-V8 в 3–12× (бенчи Rivet); сверх этого — мост к
  `NSJSBase::CJSContext` / JSSmart / `CNativeControlEmbed`, DOM/canvas-шим для sdkjs
  (`common/Native/native.js`, `CGraphicsEmbed`) и шрифтовой движок с нуля. Публичных
  прецедентов нет: это исследование на месяцы, а не спайк.

## Prior art: ONLYOFFICE в браузере

`ranuts/document` (AGPL-3.0, ~2k звёзд) — живой пример ONLYOFFICE в браузере, и он
подтверждает механику:

- **Рисует не x2t.** Экспорт PDF работает, потому что sdkjs сериализует уже отрисованный
  документ в canvas-поток, а x2t лишь транслирует поток в PDF: `m_nFormatFrom=8196`
  (CANVAS_PDF) → `m_nFormatTo=513` (PDF), `m_bIsNoBase64=true`, вызов через
  `AscCommon.x2t.convertFromBin`.
- В их же документации: эта серия сборок x2t.wasm поддерживает **только
  canvas-поток → PDF**; прямой docx/xlsx/pptx → PDF не поддерживается (код 80).
- У них вкоммичены sdkjs (185 МБ) и web-apps (141 МБ) как статика; серверная часть
  контейнера — тоже только статика; патчи репозитория — про LLM, не про x2t.
- Грабли шрифтов: x2t не встраивает глифы CFF/OTF (только TrueType), а имена шрифтов
  задаются манифестом (`Arial.ttf`, `Calibri.ttf`, …) — иначе текст в PDF невидим.

## Пути и цена

| Путь | Цена | Риск | Вердикт |
|---|---|---|---|
| x2t.wasm как рисующий бэкенд (docx/xlsx → pdf/png в браузере) | артефакт уже собран и обёрнут (38,7 МБ) | рисовать нечем: движка раскладки в сборке нет, все цели с рендером дают код 80 | **no-go** |
| Снять заглушку `ExecuteScript` и сшить `CJSContext` с jitless-движком в wasm | месяцы исследования | мост, DOM/canvas-шим и шрифтовой движок с нуля; движок в 3–12× медленнее; прецедентов нет; LOWA при этом уже даёт 4,62 с на 161 страницу | не окупается |
| x2t как конвертер внутри семейства форматов (docx↔odt/rtf/txt, xlsx↔ods/csv) | обвязка готова ([`@meerrec/x2t`](../../packages/x2t/README.md)) | на реальных документах падает и портит пакет | единственная рабочая область, и та с оговоркой |
| Серверный путь: Document Server или один нативный x2t | ~1,5 ГБ сжатый образ (7.2.1) + слой шрифтов ~190 МБ, 2+ ГБ RAM; нативный x2t легче — `FileConverter/bin` + ~12 либ + `DoctRenderer.config` + шрифты + `AllFonts.js` | AGPL-3.0, сетевое copyleft (Developer Edition — коммерческий); файлы пользователя покидают устройство; Document Builder CLI с 8.3.0 ставит водяной знак (8.2.0 — последняя без него) | вне клиентской модели продукта |

## Вывод

**No-go: x2t не может быть рисующим бэкендом в браузере.** Рисовать нечем: движка
раскладки (V8 + sdkjs) в сборке нет и не будет — апстрим порт в wasm не планирует, а
все известные сборки собраны с вырезанным doctrenderer. Нативная половина конвейера
(метафайл → PDF/растр) в wasm не просто присутствует, а работает — `CPdfFile`, `PdfWriter`,
`Aggplus`, `CFontManager`, `%PDF-1.7`, и проба это подтверждает, — но `pdf.bin` взять
неоткуда: его пишет sdkjs под V8.
Единственный теоретический путь — снять заглушку `ExecuteScript` и сшить
`NSJSBase::CJSContext` с jitless-движком (QuickJS/SpiderMonkey) в wasm — исследование
на месяцы без публичных прецедентов, и при живом LOWA-сравнении (50 МиБ, 4,62 с на
161 страницу) смысла не имеет.

x2t осмыслен только как конвертер внутри семейства форматов — и там на реальных
документах падает (`word/footer8.xml`, `memory access out of bounds`) — либо как
серверный сервис, что вне клиентской модели проекта.
