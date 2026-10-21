# @meerrec/core

Ядро просмотрщиков: протокол воркера, обвязка `OffscreenCanvas` и транспорт
DisplayList. Фабрики `createXlsxViewer` и `createDocxViewer` берут wasm-модули
из `@meerrec/wasm` — в монорепозитории это зависимость `workspace:*`.

## Установка

Пакет опубликован в **GitHub Packages** этого репозитория
(`@meerrec/core@1.0.0`) — действующем зеркале; целевой основной реестр —
`registry.npmjs.org` (публикация готовится,
[ADR-0027](https://github.com/meerrec/wasm-converter/blob/main/docs/adr/0027-npmjs-primary-registry.md)).
Реестр и токен с правом `read:packages` — в `~/.npmrc`:

```ini
# ~/.npmrc — <токен> это personal access token (classic) с read:packages
@meerrec:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=<токен>
```

```bash
npm install @meerrec/core --registry=https://npm.pkg.github.com
```

Без реестра — тарбол (`meerrec-core-1.0.0.tgz`) в ассетах релиза
[`v1.0.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.0.0).

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
- Релиз [`v1.0.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.0.0) — тарболы всех пяти пакетов.
