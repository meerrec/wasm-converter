# @meerrec/wasm-odp

Ленивый модуль просмотра ODP: воркер грузит его по URL при открытии
соответствующего файла, поэтому в основной бандл он не входит. Точка входа —
`pkg/doc_converter_odp_wasm.js` рядом с `pkg/doc_converter_odp_wasm_bg.wasm`; каталог `pkg/`
генерирует `wasm-pack` (скрипт `scripts/build-wasm.sh`), руками его править
не нужно.

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
- [ADR-0047-odf-plugin](https://github.com/meerrec/wasm-converter/blob/main/docs/adr/0047-odf-plugin.md) — скоуп и границы плагина.
