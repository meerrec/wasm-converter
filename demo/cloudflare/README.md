# Native COOP/COEP для демо на Cloudflare Pages

Конфиг cross-origin isolation для демо: заголовки отдаёт сам хостинг, а не
service worker. GitHub Pages таких заголовков не умеет — там изоляцию ставит
корневой `sw.js` (см. `scripts/pages/`); Cloudflare Pages читает `_headers` из
корня загруженного каталога и отдаёт заголовки всем ответам.

## Что даёт

`_headers` из этого каталога:

```
/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
```

С ними страница демо получает `crossOriginIsolated === true`, вьюер выбирает
транспорт `sab` (`SharedArrayBuffer`), и SW-патч для изоляции не нужен: страницы
регистрируют `sw.js`, только пока изоляции нет. В отличие от SW-патча
(Chromium), native-заголовки работают во всех браузерах с поддержкой COOP/COEP.
Сайт движка Euro-Office уехал в отдельный репозиторий
(github.com/meerrec/eurooffice-demo, ADR-0038) и публикуется оттуда; здесь
заголовки накрывают только демо просмотрщика.

Без заголовков вьюер не ломается — уходит на `bytes` (ADR-0021).

## Сборка артефакта

Workflow `Pages` (`.github/workflows/pages.yml`) собирает сайт в `.pages/`:
демо — в `/viewer/`, корневой `sw.js` — в корне. Для Cloudflare Pages нужен
тот же артефакт, но с другой базой демо: CI запекает
`VITE_BASE=/wasm-converter/viewer/` (путь GitHub Pages), а на домене Cloudflare
демо живёт в `/viewer/`. Сборка с базой от корня (шаги — как в `pages.yml`;
нужны Rust/wasm-pack и pnpm):

```bash
bash scripts/build-wasm.sh
pnpm -F @meerrec/core build
VITE_BASE=/viewer/ pnpm -F @meerrec/example-viewer build
mkdir -p .pages/viewer
cp -R examples/viewer/dist/. .pages/viewer/
cp scripts/pages/sw.js .pages/sw.js
cp demo/cloudflare/_headers .pages/_headers
```

`_headers` кладётся в артефакт последним шагом, уже после компоновки.

## Подключение проекта

Проект — Direct Upload (Git-интеграция не нужна: сборка уже в GitHub Actions,
второй сборки на стороне Cloudflare не заводим). `_headers` обязан лежать в
корне загружаемого каталога — файл из этого каталога копируется в `.pages/`
(см. выше).

Варианты выгрузки:

- **Дашборд:** Workers & Pages → Create → Pages → Upload assets → перетащить
  **содержимое** `.pages/`.
- **Wrangler, локально:** `npx wrangler pages deploy .pages --project-name <имя>`;
  токен (`CLOUDFLARE_API_TOKEN`) или `wrangler login` — только на машине
  разработчика.

Новых секретов в репозитории нет и CI-деплой в Cloudflare не добавляется
намеренно: GitHub Actions продолжает публиковать GitHub Pages, Cloudflare —
второй хостинг с настоящими заголовками. Артефакт из run workflow `Pages`
скачивается вручную, но помните про `VITE_BASE`: он собран для пути
`/wasm-converter/`.

## Проверка

1. Заголовки:

```bash
curl -sI https://<домен>/viewer/ | grep -iE 'cross-origin-opener|cross-origin-embedder'
```

Ожидаются обе строки (`cross-origin-opener-policy: same-origin`,
`cross-origin-embedder-policy: require-corp`).

2. Консоль страницы `https://<домен>/viewer/`: `crossOriginIsolated` → `true`.
   В коде приложения — `viewer.transport === 'sab'`.

3. Playwright-smoke. `examples/viewer/playwright.config.ts` жёстко задаёт
   `baseURL: http://localhost:5178` и поднимает `webServer`; переопределения
   через env в конфиге нет, поэтому существующие спеки против CF-домена не
   запускаются. Проверка — отдельным `page.goto` из установленного
   `@playwright/test`:

```bash
curl -sI https://<домен>/viewer/ | grep -iE 'cross-origin-opener|cross-origin-embedder'
```

Ожидается `crossOriginIsolated = true`. Если браузеры Playwright не
установлены: `pnpm -F @meerrec/example-viewer exec playwright install chromium`.

## Ограничения

- `_headers` читается только из корня деплоя; после правки конфига нужен
  повторный деплой.
- CDN Cloudflare кэширует ответы: заголовки проверяйте после деплоя, при
  сомнении — с `curl -H 'Cache-Control: no-cache'`.
- `Cross-Origin-Embedder-Policy: require-corp` требует CORP/CORS у
  кросс-доменных ресурсов; демо тянет ассеты со своего домена, но сторонние
  встраивания и шрифты надо проверять отдельно.
