# @meerrec/wasm-pptx

Ленивый модуль просмотра PPTX: воркер грузит его по URL при открытии `.pptx`,
поэтому в основной бандл он не входит. Точка входа —
`pkg/doc_converter_pptx_wasm.js` рядом с `pkg/doc_converter_pptx_wasm_bg.wasm`;
каталог `pkg/` генерирует `wasm-pack` (скрипт `scripts/build-wasm.sh`), руками
его править не нужно.

## Установка

Пакет публикуется в `registry.npmjs.org` (`@meerrec/wasm-pptx`), зеркало —
GitHub Packages этого репозитория. Реестр и токен с правом `read:packages` —
в `~/.npmrc` (см. корневой [README](https://github.com/meerrec/wasm-converter#readme)).

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
- [ADR-0033](https://github.com/meerrec/wasm-converter/blob/main/docs/adr/0033-pptx-plugin.md) — скоуп и границы плагина PPTX.
