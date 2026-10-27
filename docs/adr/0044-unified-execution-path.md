# ADR-0044: Единый путь исполнения кадра — TransportAdapter и RenderAdapter

**Статус:** Принято
**Дата:** 27.10.2026
**Контекст:** To-Be §3.3 A, M0
**Связанные ADR:** [ADR-0021](0021-frame-transports.md) (два транспорта кадра),
[ADR-0029](0029-main-thread-render-backend.md) (рендер на main-потоке),
[ADR-0030](0030-format-plugin-traits.md) (trait-слой форматов)

---

## Контекст

- До этого решения путь кадра выбирался в двух независимых местах: main решал,
  **где** рисовать (`resolveRenderMode` в
  [`render/offscreen.ts`](../../packages/core/src/render/offscreen.ts)), воркер —
  **каким транспортом** возить (`pickTransport` в `worker/transport.ts`).
  Входные данные у решений были одни, а места разные: main отправлял `backend`,
  воркер по `backend` и `transport` собирал пару заново.
- `WorkerCore` держал матрицу `backend` × `transport` четырьмя ad-hoc ветками:
  `init` (main против worker), `resize`, сборка кадра
  (`buildBytes`/`buildSab`/`buildFrameMain`) и отрисовка (`paintFrame`). Каждая
  ветка знала про `sab`/`pool`/`relay` и проверяла свою половину инварианта.
- Недопустимая пара `main + sab` проверялась в двух местах — на main и в
  воркере — и это уже расхождение: проверка появляется там, где о ней
  вспомнили, а не там, где пара образуется.
- Логика кадра разъезжалась с транспортом: выбор транспорта, рост буфера,
  отрисовка и возврат буфера с main жили в разных методах ядра и менялись
  вместе только по памяти автора. `FramePool` и `FrameRelay` при этом уже были
  общими — не хватало контракта, который их объединяет.

## Решение

1. **ExecutionPlan — единственная точка выбора.** Модуль
   [`execution/plan.ts`](../../packages/core/src/execution/plan.ts) описывает
   матрицу один раз: `RenderTarget = 'worker' | 'main'`,
   `ExecutionPlan { render, transport }`. `resolveRenderTarget` выбирает место,
   `resolveExecutionPlan` — транспорт на уже известном месте. Допустимые планы —
   `worker+sab`, `worker+bytes`, `main+bytes`; пара `main+sab` **не
   представима**: для `main` `resolveExecutionPlan` не может вернуть ничего,
   кроме `bytes`, а `resolveRenderTarget` отклоняет `sab` сразу. Модуль
   чистый — без DOM и wasm, — поэтому его вызывают и main, и воркер, а
   тестируется он в node.

2. **TransportAdapter — контракт перевозки кадра.**
   [`execution/transport.ts`](../../packages/core/src/execution/transport.ts)
   объявляет `kind`, `slotCapacity`, `buildLocal`/`paintLocal` (offscreen-путь),
   `buildForward`/`ack` (main-путь), `grow` и `close`; фабрика
   `createTransportAdapter` — единственное место, где по плану появляется
   реализация. Реализаций две: `SharedArrayBufferTransport`
   ([`transport_sab.ts`](../../packages/core/src/execution/transport_sab.ts)) и
   `ArrayBufferTransport`
   ([`transport_array_buffer.ts`](../../packages/core/src/execution/transport_array_buffer.ts)).
   Метод чужого пути бросает ошибку: неверный план должен падать громко, а не
   слать кадр не туда. Переиспользуются существующие `FramePool`
   ([`worker/frame_pool.ts`](../../packages/core/src/worker/frame_pool.ts)),
   `FrameRelay` ([`worker/frame_relay.ts`](../../packages/core/src/worker/frame_relay.ts)),
   циклы кадра ([`frame_loop.ts`](../../packages/core/src/worker/frame_loop.ts),
   [`relay_loop.ts`](../../packages/core/src/worker/relay_loop.ts)). Третий
   транспорт и RPC этой задачей не вводятся.

3. **RenderAdapter — контракт отрисовки.** Воркерная сторона
   ([`worker/execution/render.ts`](../../packages/core/src/worker/execution/render.ts))
   — `target`, `start`/`request`/`resize`/`ack`/`stop`; реализаций две:
   `OffscreenRenderAdapter`
   ([`render_offscreen.ts`](../../packages/core/src/worker/execution/render_offscreen.ts))
   рисует в воркере, `ForwardingRenderAdapter`
   ([`render_forward.ts`](../../packages/core/src/worker/execution/render_forward.ts))
   только собирает DisplayList и отправляет кадр на main. Main-сторона —
   `MainRenderAdapter` (в постановке M0 — `MainThreadRenderAdapter`;
   [`render/render_adapter.ts`](../../packages/core/src/render/render_adapter.ts),
   фабрика `createMainRenderAdapter`): в воркер-режиме тонкий (resize и снимок
   PNG), в main-режиме держит painter, цикл кадра и битмапы. `WorkerCore` знает
   только интерфейс; выбор реализации — одна ветка на `init`, а не ветки в
   каждом обработчике.

4. **Инварианты и отказ.** `main` всегда получает `bytes`: SAB-кольцо живёт в
   воркере, а кадр на main уезжает переводом владения. `auto` выбирает воркер
   при `canvas.transferControlToOffscreen` и main без него. Явные
   несовместимости — ошибка инициализации с внятным текстом, а не подмена:
   `worker` без OffscreenCanvas, `sab` без COOP/COEP (`crossOriginIsolated`),
   `main` вместе с `sab` ([ADR-0021](0021-frame-transports.md),
   [ADR-0029](0029-main-thread-render-backend.md)). Fallback `auto` остаётся
   видимым: одно `console.warn` в
   [`render/offscreen.ts`](../../packages/core/src/render/offscreen.ts), где
   теперь живёт только DOM-часть решения — есть ли `transferControlToOffscreen`.

5. **Контракты не меняются.** DisplayList и его версия (`DL_VERSION` в
   `crates/render/src/display_list.rs`), структура `sab`-кольца (`HEADER_INTS`,
   `SLOT_COUNT` в `packages/core/src/sab/protocol.ts` и
   `crates/render/src/sab/ring.rs`), wire-протокол
   ([`protocol.ts`](../../packages/core/src/protocol.ts): `InMsg`/`OutMsg`,
   `backend` в `InitMsg`, `FrameMsg`) и публичный API
   (`transport`/`renderMode` в опциях, `viewer.transport`/`viewer.renderMode` в
   handle) остаются прежними. Это рефакторинг исполнения, а не контракта:
   потребитель не должен заметить разницы.

6. **RPC-слой не возвращается.** Канал остаётся один и прямой: сообщения
   `InMsg`/`OutMsg`, корреляция по `id`, ошибки с `id` и `code`
   ([ADR-0028](0028-typed-wire-protocol.md)). Адаптеры — внутренние швы
   исполнения, а не новый протокол: второй командной иерархии над теми же
   сообщениями не появляется.

## Альтернативы

- **Оставить ветвления** (`resolveRenderMode` + `pickTransport` + матрица в
  `WorkerCore`) — отвергнуто: каждая новая комбинация умножает ветки, а
  инвариант `main + sab` проверяется в двух местах, одно из которых неизбежно
  отстаёт. Цена ошибки — кадр, ушедший не туда, или тихая подмена транспорта.
- **Ввести RPC** — отвергнуто: wire уже типизирован
  ([ADR-0028](0028-typed-wire-protocol.md)), и RPC добавил бы вторую командную
  иерархию над теми же сообщениями, не меняя ни выбора пути, ни доставки кадра.
  Дубль DisplayList в виде `RenderCommand` уже отклонён
  ([ADR-0030](0030-format-plugin-traits.md)).
- **Оставить только SAB** — отвергнуто ([ADR-0021](0021-frame-transports.md)):
  SAB требует COOP/COEP, которых потребитель не всегда может поставить; embed,
  Electron/webview и GitHub Pages без изоляции перестали бы работать.
- **Перенести раскладку и сборку на main** — отвергнуто
  ([ADR-0029](0029-main-thread-render-backend.md)): самая дорогая часть движка
  заморозила бы интерфейс на время открытия, а выигрыша нет — canvas и так на
  main. На main переезжает только рисование.

## Последствия

- Файлы: появились `execution/plan.ts`, `execution/transport.ts`,
  `execution/transport_sab.ts`, `execution/transport_array_buffer.ts`,
  `worker/execution/{render,render_offscreen,render_forward}.ts`,
  `render/render_adapter.ts`; `FramePool` вынесен в
  [`worker/frame_pool.ts`](../../packages/core/src/worker/frame_pool.ts), а
  `worker/transport.ts` (`pickTransport`) удалён. `WorkerCore` сократился:
  матрица ушла в адаптеры, в ядре остались сборка кадра по формату и состояние
  документа.
- Тесты: матрица плана —
  [`test/execution_plan.spec.ts`](../../packages/core/test/execution_plan.spec.ts)
  (допустимые планы и три отказа), контракт транспортов без wasm —
  [`test/transport_adapter.spec.ts`](../../packages/core/test/transport_adapter.spec.ts).
  [`test/frame_transport.spec.ts`](../../packages/core/test/frame_transport.spec.ts)
  остался тестом примитива `FramePool` — проверка `pickTransport` ушла в
  `execution_plan.spec.ts`. Сквозные тесты вьюеров и `tick` остаются в силе:
  наблюдаемое поведение не менялось.
- Публичный API не меняется: `transport`/`renderMode` в опциях и
  `viewer.transport`/`viewer.renderMode` в handle — прежние; мажорного
  изменения пакета решение не требует.
- Диагностика сохраняется: тексты ошибок (`main` + `sab`, `sab` без COOP/COEP,
  `worker` без OffscreenCanvas) остаются внятными, `console.warn` на
  auto-fallback никуда не делся.
- main-режим остаётся singleton painter'ом: один вьюер на страницу
  ([ADR-0029](0029-main-thread-render-backend.md)) — ограничение не снято и не
  спрятано.
- Третий транспорт или третье место рендера встают новой реализацией контракта
  и строкой в `ExecutionPlan`, а не новой веткой в `WorkerCore`; недопустимая
  пара отклоняется в одном месте. Изменение матрицы — новый ADR, а не правка
  этого.
