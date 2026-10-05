# ADR 0004: SAB — ускорение, а не обязательное условие

**Статус:** принято 05.10.2026; реализация — Спринт 11
**Контекст:** [`crates/render/src/sab/ring.rs`](../../crates/render/src/sab/ring.rs),
[`packages/core/src/sab/protocol.ts`](../../packages/core/src/sab/protocol.ts),
[`packages/core/src/worker/worker.ts`](../../packages/core/src/worker/worker.ts),
[`crates/wasm/src/painter_api.rs`](../../crates/wasm/src/painter_api.rs),
[`crates/wasm/src/xlsx_api.rs`](../../crates/wasm/src/xlsx_api.rs)

## Задача

`SharedArrayBuffer` включается только при COOP/COEP-заголовках
(`Cross-Origin-Opener-Policy: same-origin`,
`Cross-Origin-Embedder-Policy: require-corp`). Заголовки ставит тот, кто отдаёт
страницу. В embed-сценарии — iframe на чужом сайте, расширение, Electron,
webview — потребитель их не контролирует, и SAB отваливается. Это не edge
case, а половина способов встроить просмотрщик.

Отсюда требование: все пути (RPC, DisplayList, PDF/PNG) обязаны работать без
SAB с приемлемой производительностью. SAB даёт ×N, а не «включает». Путь без
SAB — first-class, а не fallback «на всякий случай»: fallback, который не в CI
и не в бюджетах, обнаруживается сломанным ровно тогда, когда понадобился.

## Контекст: как устроено сейчас

**Ring.** [`ring.rs`](../../crates/render/src/sab/ring.rs) задаёт раскладку:
заголовок 16 байт (`HEADER_BYTES`) — `seq_writer`, `seq_reader`, `slot_len[0]`,
`slot_len[1]`, дальше два слота payload (`SLOT_COUNT = 2`). `RingState` —
чистая, тестируемая нативно машина состояний; `SabRing` (только wasm32)
ложится на `SharedArrayBuffer` поверх `Int32Array` и работает через `Atomics`.
Свободного слота нет — `try_write` возвращает `false`, кадр дропается: это
намеренный backpressure, за ним не гонятся устаревшими кадрами. TS-зеркало —
[`protocol.ts`](../../packages/core/src/sab/protocol.ts) (`HEADER_INTS = 4`,
`SLOT_COUNT = 2`) и [`reader.ts`](../../packages/core/src/sab/reader.ts)
(`SabReader`, «симметричен Rust `SabRing` — для дебага и тестов»; сейчас не
импортируется нигде).

**Входы painter'а** ([`painter_api.rs`](../../crates/wasm/src/painter_api.rs)):
`paint_display_list_sab(sab, slot_capacity)` читает текущий слот ring'а через
`copy_current_into`, освобождает его и рисует; `paint_display_list_bytes(bytes)`
принимает байты аргументом. Оба заканчиваются одним `Painter2D::paint_bytes` —
второго painter'а для пути без SAB не нужно. Сборка кадра
([`xlsx_api.rs`](../../crates/wasm/src/xlsx_api.rs)):
`xlsx_build_display_list_sab` сериализует кадр в слот ring'а и отдаёт
`{ written, cmds, build_ms }`, а `xlsx_build_display_list` возвращает `Vec<u8>`
— его можно передать в `postMessage` переводом владения.

**Где путь без SAB обрывается.** [`worker.ts`](../../packages/core/src/worker/worker.ts)
импортирует только SAB-варианты (`alloc_sab`, `xlsx_build_display_list_sab`,
`paint_display_list_sab`) и на `init` безусловно зовёт `alloc_sab`; сообщение
`ready` несёт `sab`, и в [`protocol.ts`](../../packages/core/src/protocol.ts)
поле `sab: SharedArrayBuffer` у `ReadyMsg` обязательное. Ветки, которая взяла
бы `xlsx_build_display_list` и `paint_display_list_bytes`, нет; на стороне
main [`initOffscreen`](../../packages/core/src/render/offscreen.ts) ждёт
`ready` именно с `sab` и иначе отклоняет промис. `assertFallbackAvailable()`
объявлена и экспортируется, но не вызывается ниоткуда. Значит, **путь без SAB
сегодня не реализован и не заглушка даже: это несобранная половина** — Rust-API
для неё готов, браузерной проводки нет. [`docs/workers.md`](../../docs/workers.md)
при этом уже обещает, что «без них движок работает на `ArrayBuffer` +
transferables»; это утверждение коду не соответствует.

**Тесты.** Сквозные тесты
([`examples/viewer-xlsx/e2e/viewer.spec.ts`](../../examples/viewer-xlsx/e2e/viewer.spec.ts),
5 штук) идут только в Chromium
([`playwright.config.ts`](../../examples/viewer-xlsx/playwright.config.ts)), а
Vite отдаёт COOP/COEP и в dev, и в preview
([`vite.config.ts`](../../examples/viewer-xlsx/vite.config.ts)) — то есть все
тесты живут в мире с SAB, и прогона без заголовков нет вовсе.

**Честно про «zero-copy».** Слово из комментариев пока означает цель, а не
факт: сборщик кадра делает `frame.to_bytes()` (аллокация) и копирует результат
в слот `try_write`, а painter копирует слот через `copy_current_into` в `Vec`
перед разбором. Замеры 1–3 мс на сборку и 0–5 мс на отрисовку получены вместе с
этими копиями. Это не повод менять решение, но повод не писать в документации,
что копирований нет.

## Рассмотренные варианты

| Вариант | Почему нет |
|---|---|
| SAB обязателен; требование COOP/COEP задокументировать | embed-сценарий заголовки не контролирует — просмотрщик просто не отрисует кадр |
| Полный отказ от SAB | теряем дешёвое ускорение там, где заголовки есть; SAB нужен не всем, но полезен |
| Путь без SAB как fallback | fallback без тестов и бюджета гниёт и ломается незаметно; расхождение поведения всплывёт у потребителя |
| **Оба пути first-class, различие только в транспорте** | выбран |

## Решение

- Один API, два транспорта. С SAB DisplayList уезжает в ring; без SAB — тем же
  кадром через `postMessage` с переводом владения (`ArrayBuffer`/`Uint8Array`
  — transferables). Для хоста разницы в поведении быть не должно, только в
  скорости.
- Оба пути тестируются в Playwright: одни и те же сценарии прогоняются с
  COOP/COEP-заголовками и без них. Прогон без заголовков — такой же обязательный
  в CI, как обычный.
- Бюджеты производительности фиксируются отдельно для каждого пути (ROADMAP §9).
- COOP/COEP документируются в README как ответственность потребителя при
  embed: что даёт SAB, какие заголовки для него нужны и что без них работа не
  прекращается. `docs/workers.md` перестаёт утверждать несуществующее
  поведение — оно там появится вместе с реализацией.

## Следствия

- Срок — Спринт 11 (ROADMAP §6 и §11, Риск C). Бюджеты из §9: RPC round-trip
  без SAB < 3 мс против < 1 мс с SAB; `buildDisplayList` — без деградации за
  счёт transferables.
- Что это значит для текущего кода:
  - `ReadyMsg` в [`protocol.ts`](../../packages/core/src/protocol.ts) перестаёт
    требовать `sab` — готовность воркера и наличие ring'а становятся
    независимыми фактами;
  - `worker.ts` получает развилку по наличию SAB и второй набор вызовов
    (`xlsx_build_display_list` + `paint_display_list_bytes`) — Rust-часть для
    него уже экспортирована, добавлять в неё почти нечего;
  - `initOffscreen`/`createXlsxViewer` перестают считать отсутствие `sab`
    ошибкой инициализации;
  - `assertFallbackAvailable()` либо обретает вызов, либо удаляется —
    экспортированная функция, которую никто не зовёт, вводит в заблуждение;
  - e2e-конфигурация получает второй прогон без заголовков.
- `dropped` в `PaintStats` начнёт означать разное: с SAB — переполнение ring'а
  (backpressure), без SAB — кадр не доехал или рисовать было нечего. Контракт
  статистики нужно уточнить, иначе интерфейс сложит эти случаи в один.
- `SabReader` в TS и `NativeRingReader` в Rust остаются инструментами тестов и
  отладки; рабочий путь main-потока через них не идёт — в
  `createViewer`/`createXlsxViewer` SAB живёт в воркере.
- Требование «SAB — ускорение» проверяется не документацией, а DoD Спринта 11:
  путь без SAB работает без деградации функциональности, и это видно в CI.
