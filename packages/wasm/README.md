# @meerrec/wasm

Основной wasm-bindgen-модуль просмотрщика: painter, SAB-аллокатор и
XLSX-просмотрщик. Точка входа — `pkg/doc_converter_wasm.js` рядом с
`pkg/doc_converter_wasm_bg.wasm`; каталог `pkg/` генерирует `wasm-pack`
(скрипт `scripts/build-wasm.sh`), руками его править не нужно.

## Установка

Пакет живёт в **GitHub Packages** этого репозитория (`@meerrec/wasm@1.0.0`), а не в
`registry.npmjs.org`: реестр и токен с правом `read:packages` — в `~/.npmrc`:

```ini
# ~/.npmrc — <токен> это personal access token (classic) с read:packages
@meerrec:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=<токен>
```

```bash
npm install @meerrec/wasm --registry=https://npm.pkg.github.com
```

Без реестра — тарбол (`meerrec-wasm-1.0.0.tgz`) в ассетах релиза
[`v1.0.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.0.0).

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
- Релиз [`v1.0.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.0.0) — тарболы всех пяти пакетов.
