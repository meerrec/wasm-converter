# @meerrec/wasm-pdf

Ленивый модуль экспорта PDF: воркер грузит его по URL при первом экспорте,
поэтому в основной бандл он не входит. Точка входа —
`pkg/doc_converter_pdf_wasm.js` рядом с `pkg/doc_converter_pdf_wasm_bg.wasm`;
каталог `pkg/` генерирует `wasm-pack` (скрипт `scripts/build-wasm.sh`), руками
его править не нужно.

## Установка

Пакет опубликован в **GitHub Packages** этого репозитория
(`@meerrec/wasm-pdf@1.0.0`) — действующем зеркале; целевой основной реестр —
`registry.npmjs.org` (публикация готовится,
[ADR-0027](https://github.com/meerrec/wasm-converter/blob/main/docs/adr/0027-npmjs-primary-registry.md)).
Реестр и токен с правом `read:packages` — в `~/.npmrc`:

```ini
# ~/.npmrc — <токен> это personal access token (classic) с read:packages
@meerrec:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=<токен>
```

```bash
npm install @meerrec/wasm-pdf --registry=https://npm.pkg.github.com
```

Без реестра — тарбол (`meerrec-wasm-pdf-1.0.0.tgz`) в ассетах релиза
[`v1.0.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.0.0).

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
- Релиз [`v1.0.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.0.0) — тарболы всех пяти пакетов.
