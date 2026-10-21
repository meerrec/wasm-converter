# Пример: демо-приложение (DOCX, XLSX)

`@meerrec/example-viewer` — витрина проекта на GitHub Pages: две страницы под
хеш-маршрутами, все — целиком в браузере, без серверного рендера.

| Маршрут | Страница | Что показывает |
|---|---|---|
| `#/docx` | «Слово» | просмотрщик DOCX: воркер, `wasm-docx`, экспорт |
| `#/xlsx` | «ДваИксель» | просмотрщик XLSX: воркер, `wasm`, экспорт |

Корень демо ведёт на `#/docx`. Публикуется демо подкаталогом `/viewer/` сайта
Pages (`VITE_BASE` в workflow `pages.yml`); корневой `sw.js` ставит заголовки
cross-origin isolation (COOP/COEP), которые GitHub Pages сам не отдаёт.

## Страницы конвертера и Euro-Office

Маршруты `#/convert` («Конвертер» на x2t) и `#/eurooffice` («Euro-Office»,
редактор iframe'ом через Embed API) жили здесь до Спринта 14 и вынесены вместе
с пакетами `@meerrec/{x2t,browser-convert,eurooffice}` и сборкой движка в
отдельный репозиторий [github.com/meerrec/eurooffice-demo](https://github.com/meerrec/eurooffice-demo)
([ADR-0038](../../docs/adr/0038-agpl-separation.md)): монорепозиторий — чистый
Apache-2.0, а всё, что грузит или собирает AGPL-движок, живёт там.

## Разработка

```bash
pnpm dev          # dev-сервер с фикстурами и COOP/COEP-заголовками
pnpm build        # vite build; база — VITE_BASE (на Pages /<repo>/viewer/)
pnpm test         # Playwright e2e
```

E2E сам поднимает dev-сервер на порту 5178; браузеры ставятся один раз:
`pnpm exec playwright install chromium firefox webkit`.
