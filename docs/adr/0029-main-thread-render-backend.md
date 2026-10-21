# ADR-0029: Рендер на main-потоке без OffscreenCanvas

**Статус:** Принято
**Дата:** 21.10.2026
**Контекст:** Спринт 14, слайс «main-thread Canvas fallback»
**Связанные ADR:** [ADR-0003](0003-render-boundary.md) (граница `render`),
[ADR-0021](0021-frame-transports.md) (транспорты кадра; worker-путь не
меняется), [ADR-0028](0028-typed-wire-protocol.md) (wire-типы протокола)

---

## Контекст

- Canvas попадает в Web Worker только через
  `canvas.transferControlToOffscreen()`. Где этой возможности нет — старый
  webview, урезанный embed, браузер без `OffscreenCanvas`, — вьюер падал на
  инициализации, хотя Canvas2D на main и `Worker` в окружении есть.
  `supportsOffscreen()` честно отвечал «нет», и просмотрщик не запускался
  вовсе.
- Main-thread fallback был единственным нереализованным пунктом §3.3
  `ROADMAP.md` («не реализован в v1: без OffscreenCanvas инициализация
  бросает ошибку»). Остальной движок в таком окружении работоспособен:
  парсинг и раскладка OOXML идут в wasm-воркере, экспорт PDF и текста — там
  же.
- Дорогая часть — не рисование, а парсинг и раскладка (`crates/xlsx`,
  `crates/docx`). Перенос их на main заморозил бы интерфейс на время
  открытия книги или документа.
- Painter в Rust трогает только общие методы Canvas2D (`fillRect`,
  `fillText`, `drawImage`, `setTransform`, `measureText`, пути, клипы) и
  `canvas.width/height` — они есть у `CanvasRenderingContext2d` и
  `OffscreenCanvasRenderingContext2d` одинаково.

## Решение

1. **Гибрид, а не полный перенос.** Парсинг, раскладка и сборка DisplayList
   остаются в Web Worker'е; на main переезжает только рисование и всё, что
   привязано к настоящему `<canvas>`.
2. **Опция `renderMode: 'auto' | 'worker' | 'main'`** в `XlsxViewerOptions`,
   `DocxViewerOptions` и `ViewerOptions`. `auto` (по умолчанию) выбирает
   worker при наличии `canvas.transferControlToOffscreen`, иначе main и один
   `console.warn` — смена режима видна в консоли. Явный `'worker'` без
   возможности — внятная ошибка инициализации, как `transport: 'sab'` без
   COOP/COEP ([ADR-0021](0021-frame-transports.md)): конфигурация не
   подменяется молча. `'main'` вместе с `transport: 'sab'` — ошибка
   конфигурации: SAB-кольцо живёт в воркере, main-режим всегда возит кадры
   буферами.
3. **Разрешённый режим виден в `viewer.renderMode`** (`'worker' | 'main'`), а
   `viewer.transport` в main-режиме всегда `'bytes'`.
4. **Протокол `frame` / `frame-ack`.** В main-режиме воркер, как и раньше,
   собирает DisplayList, но не рисует: он шлёт на main сообщение `frame` с
   буфером из пула на два `ArrayBuffer` (двойная буферизация, глубина два —
   как у транспорта `bytes`). Main рисует кадр тем же Rust-painter'ом,
   коалесцирует запросы по `requestAnimationFrame` (показывается самый свежий
   кадр) и возвращает буферы воркеру сообщением `frame-ack` — transfer, а не
   новая аллокация. Буфер узнаётся по номеру слота (`slot` в `frame` и
   `frame-ack`): идентичность `ArrayBuffer` через `postMessage` не переживает,
   на другой стороне это новый объект с той же памятью.
   `resize`, `tick` и регистрация `ImageBitmap` в этом режиме тоже на main:
   статистика кадра считается там, где он нарисован.
5. **Один painter на main — singleton.** Пиксельный контекст один на
   страницу, поэтому повторная инициализация main-вьюера даёт ошибку
   «main-режим: один вьюер на страницу». Воркер-путь этим не ограничен:
   у каждого вьюера свой воркер.
6. **`install_js` с duck-типизацией.** `init_painter` принимает типизированный
   `OffscreenCanvasRenderingContext2d`; для main добавлен
   `init_painter_main(ctx: web_sys::CanvasRenderingContext2d)`, который
   передаёт `JsValue` в `OffscreenPainter::install_js`. Тот проверяет
   `Reflect`-ом свойство `canvas` и ровно те методы, которые вызывает
   painter, и лишь после этого делает `unchecked_into` в
   `OffscreenCanvasRenderingContext2d`. Так один painter-код обслуживает оба
   контекста, а расхождение («painter зовёт метод, которого нет в списке»)
   ловится на инициализации, а не в середине кадра.
7. **`render/offscreen.ts` объявлен deprecated** (`initOffscreen`,
   `createViewer`, `ViewerHandle`, `ViewerOptions`), но типы продолжают
   экспортироваться: публичный API не ломается, а `createViewer` остаётся
   обёрткой-совместимостью. Актуальный вход — `createXlsxViewer` /
   `createDocxViewer`.
8. **Worker-путь не меняется:** `transport` по-прежнему `'sab' | 'bytes'`,
   выбор по `crossOriginIsolated`; main-режим — отдельная ветка только для
   отсутствия `transferControlToOffscreen`.

## Альтернативы

- **Полный main-поток: парсинг, раскладка и сборка на main** — отвергнуто:
  самая дорогая часть движка заморозила бы интерфейс, а выигрыша нет — canvas
  и так на main. Воркер нужен был бы всё равно ради экспорта и worker-пути.
- **Второй painter или дублирование renderer'а** (TS-реализация Canvas2D или
  отдельная сборка Rust под `HtmlCanvasElement`) — отвергнуто: два painter'а
  разъезжаются, DisplayList получает второго потребителя, а `crates/render`
  дублируется ради одного типа контекста.
- **Enum/generic над двумя контекстами (`Painter2D<Ctx>`)** — отвергнуто:
  диспетчеризация 20+ методов ради одного места вызова, плюс все помощники
  painter'а становятся generic. Duck-типизация на границе проще и проверяется
  один раз.
- **Оставить только worker-путь и ошибку** (статус-кво) — отвергнуто: целый
  класс окружений (embed, webview) не мог запустить просмотрщик, хотя всё
  остальное в нём работает.
- **Молчаливый fallback без `renderMode`** — отвергнуто: диагностика
  «почему кадр на main» и принудительный worker-путь в тестах были бы
  невозможны; `console.warn` при `auto` оставляет след.

## Последствия

- Публичный API получает `renderMode` и `viewer.renderMode`; `auto` —
  значение по умолчанию, существующий код не меняется.
- Ограничение: **один main-вьюер на страницу**. Второй `createDocxViewer`
  без OffscreenCanvas отклоняется с внятной ошибкой; это документировано в
  README и cookbook.
- `index.js` не тянет wasm статически: main-путь подгружает модуль
  динамическим `import`, поэтому приложения, которым хватает worker-пути, не
  платят за вторую загрузку.
- В main-режиме `exportPng` считается локально через
  `HTMLCanvasElement.toBlob` (canvas уже на main), а `exportPdf` и текстовый
  экспорт по-прежнему идут в воркер — данные документа живут там.
- Worker-путь не затронут: транспорт, SAB-кольцо и кадровый цикл прежние.
