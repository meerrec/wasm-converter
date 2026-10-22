# ADR: индекс решений

Архитектурные решения проекта doc-converter. Один ADR — одно решение;
пересмотр решения — это новый ADR, а не правка принятого. Источник истины —
сами файлы в этом каталоге.

| ADR | Тема | Файл |
|---|---|---|
| 0001 | Раскладка ячеек листа — CSR | [`0001-worksheet-csr.md`](0001-worksheet-csr.md) |
| 0002 | Парсинг шрифтов — `skrifa` | [`0002-font-parsing.md`](0002-font-parsing.md) |
| 0003 | Граница `crates/render` | [`0003-render-boundary.md`](0003-render-boundary.md) |
| 0004 | SAB — ускорение, а не обязательное условие | [`0004-sab-acceleration.md`](0004-sab-acceleration.md) |
| 0005 | Источник ширин текста — `skrifa` + таблицы шрифта | [`0005-text-metrics.md`](0005-text-metrics.md) |
| 0006 | Алгоритмы раскладки и переноса строк для DOCX | [`0006-docx-layout-and-text-wrapping.md`](0006-docx-layout-and-text-wrapping.md) |
| 0007 | Граница раскладки PDF — без отдельного крейта `layout-core` | [`0007-pdf-layout-boundary.md`](0007-pdf-layout-boundary.md) |
| 0008 | PDF-библиотека и кодирование текста — `printpdf` | [`0008-pdf-library-and-encoding.md`](0008-pdf-library-and-encoding.md) |
| 0009 | Подрезка шрифтов — четыре подрезанных Carlito | [`0009-font-subsetting.md`](0009-font-subsetting.md) |
| 0010 | Форк printpdf 0.8.2 — сжатие и стриминг | [`0010-printpdf-fork.md`](0010-printpdf-fork.md) |
| 0011 | Геометрия диаграмм, общая для canvas и PDF | [`0011-chart-geometry.md`](0011-chart-geometry.md) |
| 0012 | Tagged PDF — структура без сертификации | [`0012-tagged-pdf.md`](0012-tagged-pdf.md) |
| 0013 | Каскад стилей DOCX | [`0013-docx-style-cascade.md`](0013-docx-style-cascade.md) |
| 0014 | `mc:AlternateContent` | [`0014-alternate-content.md`](0014-alternate-content.md) |
| 0015 | Лимиты ZIP для OOXML | [`0015-ooxml-zip-limits.md`](0015-ooxml-zip-limits.md) |
| 0016 | Политика ошибок парсинга DOCX | [`0016-docx-error-policy.md`](0016-docx-error-policy.md) |
| 0017 | Strict vs Transitional OOXML | [`0017-strict-vs-transitional.md`](0017-strict-vs-transitional.md) |
| 0018 | Границы редактирования в v1 | [`0018-editing-boundaries.md`](0018-editing-boundaries.md) |
| 0019 | NodeId | [`0019-node-id.md`](0019-node-id.md) |
| 0020 | Границы парсера и writer | [`0020-parser-writer-boundary.md`](0020-parser-writer-boundary.md) |
| 0021 | Два транспорта кадра: SAB и ArrayBuffer | [`0021-frame-transports.md`](0021-frame-transports.md) |
| 0022 | Свой PDF-бэкенд вместо форка printpdf | [`0022-own-pdf-backend.md`](0022-own-pdf-backend.md) |
| 0023 | LibreOffice-WASM как «точный» бэкенд — no-go | [`0023-libreoffice-wasm-exact-backend.md`](0023-libreoffice-wasm-exact-backend.md) |
| 0024 | x2t (ONLYOFFICE) как рисующий бэкенд — no-go | [`0024-x2t-rendering-backend.md`](0024-x2t-rendering-backend.md) |
| 0025 | Граница раскладки XLSX — без отдельного крейта `xlsx-layout` | [`0025-xlsx-layout-boundary.md`](0025-xlsx-layout-boundary.md) |
| 0026 | Дистрибутив Rust-крейтов: GitHub Packages + ассеты релиза, crates.io — non-goal | [`0026-rust-crates-distribution.md`](0026-rust-crates-distribution.md) |
| 0027 | Реестры npm: npmjs.org — основной, GitHub Packages — зеркало | [`0027-npmjs-primary-registry.md`](0027-npmjs-primary-registry.md) |
| 0028 | Типизированный wire-протокол — Rust как источник истины | [`0028-typed-wire-protocol.md`](0028-typed-wire-protocol.md) |
| 0029 | Рендер на main-потоке без OffscreenCanvas | [`0029-main-thread-render-backend.md`](0029-main-thread-render-backend.md) |
| 0030 | Trait-слой форматов | [`0030-format-plugin-traits.md`](0030-format-plugin-traits.md) |
| 0031 | Потоковый ZIP и `ReadableStream`-вход | [`0031-streaming-zip.md`](0031-streaming-zip.md) |
| 0032 | Отмена операций — кооперативный флаг и `AbortSignal` | [`0032-cancellation.md`](0032-cancellation.md) |
| 0033 | PPTX-плагин — просмотр, PDF, PNG и текст по общему trait-слою | [`0033-pptx-plugin.md`](0033-pptx-plugin.md) |
| 0034 | Strict OOXML — поддержка для DOCX и XLSX | [`0034-strict-ooxml.md`](0034-strict-ooxml.md) |
| 0037 | OpenTelemetry-инструментация — opt-in и только API | [`0037-opentelemetry.md`](0037-opentelemetry.md) |
| 0038 | Вынос AGPL-части в `meerrec/eurooffice-demo` | [`0038-agpl-separation.md`](0038-agpl-separation.md) |
| 0039 | Отмена non-goal'ов v1 | [`0039-cancel-v1-non-goals.md`](0039-cancel-v1-non-goals.md) |
| 0040 | Bidi — UAX #9 через `unicode-bidi`, без `rustybuzz` в Спринте 2 | [`0040-bidi-strategy.md`](0040-bidi-strategy.md) |
| 0041 | TextLayer и DOM-overlay — доступность без изменения `DisplayList` | [`0041-text-layer-dom-overlay.md`](0041-text-layer-dom-overlay.md) |
| 0042 | Покрытие RTL-шрифтов — системный fallback и ленивый сабсет | [`0042-rtl-font-coverage.md`](0042-rtl-font-coverage.md) |

Сводка принятых решений по спринтам — `ROADMAP.md` §11.
