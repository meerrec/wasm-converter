# @meerrec/wasm-xlsb

Ленивый модуль просмотра XLSB: воркер грузит его по URL при открытии
соответствующего файла, поэтому в основной бандл он не входит. Точка входа —
`pkg/doc_converter_xlsb_wasm.js` рядом с `pkg/doc_converter_xlsb_wasm_bg.wasm`; каталог `pkg/`
генерирует `wasm-pack` (скрипт `scripts/build-wasm.sh`), руками его править
не нужно.

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
- [ADR-0048-xlsb-plugin](https://github.com/meerrec/wasm-converter/blob/main/docs/adr/0048-xlsb-plugin.md) — скоуп и границы плагина.
