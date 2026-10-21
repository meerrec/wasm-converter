# @meerrec/wasm-pptx-pdf

Ленивый модуль экспорта PPTX в PDF: воркер презентации грузит его по URL при
первом клике «Экспорт в PDF», поэтому в основные бандлы он не входит. Точка
входа — `pkg/doc_converter_pptx_pdf_wasm.js` рядом с
`pkg/doc_converter_pptx_pdf_wasm_bg.wasm`; каталог `pkg/` генерирует
`wasm-pack` (скрипт `scripts/build-wasm.sh`), руками его править не нужно.
Просмотрщик и экспортёр PPTX разнесены по модулям, как у DOCX (ADR-0033).

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
- [ADR-0033](https://github.com/meerrec/wasm-converter/blob/main/docs/adr/0033-pptx-plugin.md) — скоуп и границы плагина PPTX.
