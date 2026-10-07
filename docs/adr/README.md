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

Сводка принятых решений по спринтам — `ROADMAP.md` §11.
