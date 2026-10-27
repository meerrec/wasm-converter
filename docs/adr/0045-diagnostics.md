# ADR-0045: Единая диагностика браузера и CLI

**Статус:** Принято
**Дата:** 27.10.2026
**Контекст:** To-Be §3.1 (диагностируемость), M2
**Связанные ADR:** [ADR-0028](0028-typed-wire-protocol.md) (типизированный
wire-протокол), [ADR-0037](0037-opentelemetry.md) (телеметрия — opt-in и только
API)

---

## Контекст

- До этого решения ошибка доходила до потребителя двумя разными путями.
  Браузер получал [`WorkerErrorMsg`](../../packages/core/src/protocol.ts), в
  котором `WorkerErrorCode` — пять строковых значений (`init`, `open`,
  `export`, `cancelled`, `internal`): они различают стадию сбоя и сбой воркера
  целиком, но не причину. CLI печатал `error: message` и ничего больше.
- Rust-ошибки пересекают wasm-границу строками (`String(e)`,
  `EngineError::to_string()`). Отмена помечена префиксом `[cancelled]`
  ([ADR-0032](0032-cancellation.md)) — единственная разметка, которую кто-то
  разбирает (см. `openErrorCode` в
  [`worker_core.ts`](../../packages/core/src/worker/worker_core.ts)). Всё
  остальное — непрозрачный текст: битый ZIP, несшаяся раскладка и неудачный
  экспорт выглядят одинаково.
- Практические вопросы, на которые нет машинного ответа: это
  `malformed package` или `layout`/`export`? Стоит ли повторять операцию? На
  каком слое чинить? CLI не может ни сгруппировать отказы в отчёте, ни отдать
  их вызывающему скрипту иначе как разбором текста.
- `message` — публичный текст: его читает человек, он живёт в логах, тестах и
  документации. Менять его нельзя, поэтому структурная информация обязана
  ехать **рядом** с текстом, а не вместо него.
- [ADR-0028](0028-typed-wire-protocol.md) уже сделал Rust источником истины
  wire-типов: DTO в `core::wire`, генерация `wire.ts` через `pnpm gen:wire`,
  дрейф ловится в CI. Диагностика — такой же payload и должна появиться там
  же, а не ручным интерфейсом в `protocol.ts`.
- [ADR-0037](0037-opentelemetry.md) задал рамку для метрик: только
  `@opentelemetry/api`, без SDK и экспортёров в зависимостях, атрибуты —
  размеры, форматы и числа, но никогда не содержимое документа и не имена
  файлов. Наблюдаемость не должна становиться вторым каналом утечки.
- To-Be §3.1 (`docs/architecture.md`) требует диагностируемости как свойства
  целевой архитектуры; веха M2 — первый потребитель этого требования.

## Решение

1. **`DiagnosticWire` — структурная диагностика на проводе.** В
   [`crates/core/src/wire.rs`](../../crates/core/src/wire.rs) описаны
   `DiagnosticCodeWire`, `DiagnosticLayerWire` и `DiagnosticWire`
   (`code`/`layer`/`message`/`format?`/`retryable`). Код и слой — закрытые
   enum'ы в `kebab-case` (`malformed-package`, `layout-failed`,
   `export-failed`, …; слои `parse`/`layout`/`render`/`transport`/`export`/
   `orchestration`/`cli`/`internal`): набор значений — часть контракта и
   меняется только новым ADR. `format` необязателен — у отказа транспорта
   формата нет; `retryable` говорит, имеет ли смысл повтор. TS-типы
   генерируются в [`wire.ts`](../../packages/core/src/generated/wire.ts)
   командой `pnpm gen:wire`, руками файл не правится
   ([ADR-0028](0028-typed-wire-protocol.md)).

2. **`EngineError::diagnostic()` — единый маппинг ошибок движка.** В
   [`crates/render/src/plugin.rs`](../../crates/render/src/plugin.rs) у
   `EngineError` появился метод, сводящий варианты к паре «код/слой»:
   `Parse` → `malformed-package`/`parse`, `Layout` → `layout-failed`/`layout`,
   `Cancelled` → `cancelled`/`orchestration` (`retryable: true`), `Export` →
   `export-failed`/`export`, `Unsupported` → `unsupported-format`/
   `orchestration`, `Limit` → `limit-exceeded`/`parse`, `Internal` →
   `internal`/`internal`. `message` — прежний `self.to_string()`, `format`
   остаётся `None`: какой формат обрабатывался, знает вызывающий, а не
   ошибка trait-слоя.

3. **`WorkerErrorMsg.diagnostic?` — аддитивное поле.** `message` и `code`
   остаются на месте для совместимости: старый потребитель, не знающий про
   диагностику, продолжает работать. Просмотрщик собирает
   [`ViewerError`](../../packages/core/src/diagnostics.ts) с полем
   `diagnostic` и реджектит им запрос — наследник `Error` с
   `name === 'ViewerError'`, поэтому `catch` и `rejects.toThrow` ведут себя
   как раньше, а `code`/`layer` доступны без разбора текста. Нормализация —
   `toDiagnostic`: структурный объект берётся как есть, строка разбирается по
   префиксам `[cancelled]`/`[parse]`/`[malformed]`/`[layout]`/`[render]`/
   `[transport]`/`[export]`/`[limit]`/`[unsupported]`/`[io]`, а
   `workerDiagnostic` достраивает диагностику из `WorkerErrorCode`.
   Неразмеченный текст не теряется: он остаётся `message` с fallback-кодом.

4. **CLI несёт ту же диагностику.** [`CliError`](../../packages/cli/src/errors.ts)
   получает поле `diagnostic: Diagnostic`. У `convert` появляется `--json`:
   один объект — `ok`, `input`/`output`, `bytes`/`ms` и `stats`, а при отказе
   `ok: false` и `diagnostic`; у `batch` — JSON Lines, по объекту на файл плюс
   итоговая строка с `total`/`failed`/`ms`/`workers`. Человекочитаемый вывод —
   `formatDiagnostic`, то есть `layer/code: [format] message`. Коды возврата
   **не меняются**: 0 — успех, 1 — неудача конвертации, 2 — ошибка аргументов и
   путей.

5. **`ConversionStats` — общая форма метрик.** В
   [`diagnostics.ts`](../../packages/core/src/diagnostics.ts) описаны
   `format`, `bytesIn`, `bytesOut`, `ms` и необязательные `units`/`workers`
   (листы/страницы/слайды и число воркеров batch-пула). Браузер отдаёт те же
   числа через `tick` и спаны, CLI — полем `stats` в JSON. Новых счётчиков
   не вводится: форма одна, источники разные.

6. **OpenTelemetry остаётся opt-in.** SDK, процессоры и экспортёры в
   зависимостях не появляются; без зарегистрированного провайдера
   инструментация — no-op ([ADR-0037](0037-opentelemetry.md)). В атрибуты
   попадают только размеры, формат и числа — никогда байты документа, их
   фрагменты, имена файлов или пути. Диагностика и телеметрия не
   пересекаются: `DiagnosticWire` описывает отказ, спаны — его длительность
   и исход.

7. **RPC не вводится.** Диагностика едет в существующем сообщении `error`:
   канал остаётся один и прямой, корреляция — по `id`, ошибка без `id` —
   сбой воркера целиком. Второго протокола над теми же сообщениями не
   появляется.

## Альтернативы

- **Оставить строки и разбирать их регулярками** — отвергнуто: текст
  публичный и меняется ради читаемости, а разбор текста в CLI — тот же
  хрупкий контракт, только неявный. Машинно отличить `malformed package` от
  `layout`/`export` по-прежнему нельзя.
- **Ввести отдельный канал ошибок** (второй протокол или поток событий) —
  отвергнуто: канал уже есть, ошибка уже адресуется `id`; второй протокол
  над теми же сообщениями добавил бы командную иерархию и рассинхрон, не
  меняя ни одной причины отказа ([ADR-0044](0044-unified-execution-path.md)).
- **Писать JSON в stderr всегда** — отвергнуто: ломает UX интерактивного
  `convert` и привычный `error: …` в терминале; JSON нужен по запросу
  (`--json`), а не по умолчанию.
- **Включить SDK OpenTelemetry и экспортёр** — отвергнуто: нарушает политику
  «файлы пользователя не покидают устройство», тянет транспорт и не влезает
  в бюджет `@meerrec/core` ([ADR-0037](0037-opentelemetry.md)).

## Последствия

- Поле `diagnostic` в протоколе аддитивно: старые сборки воркера и
  просмотрщика совместимы, `message`/`code` не меняются, мажорного
  изменения пакета решение не требует.
- `packages/core/src/generated/wire.ts` перегенерируется `pnpm gen:wire`;
  правка руками будет затёрта, а расхождение поймает CI-гейт
  ([ADR-0028](0028-typed-wire-protocol.md)).
- Тесты: нормализация и форматирование —
  [`diagnostics.spec.ts`](../../packages/core/test/diagnostics.spec.ts);
  маппинг `EngineError` — inline-тесты в
  [`plugin.rs`](../../crates/render/src/plugin.rs). Фикстуры и golden-тексты
  ошибок остаются в силе: `message` у `ViewerError` совпадает с
  `diagnostic.message`, прежние тексты не переписаны.
- У CLI появляется стабильная JSON-схема (`convert` — объект, `batch` —
  JSON Lines + итог); её изменение — ломающее и требует нового ADR, как и
  изменение набора кодов/слоёв.
- `Diagnostic`, `DiagnosticCode`, `DiagnosticLayer`, `ViewerError`,
  `toDiagnostic`, `formatDiagnostic`, `workerDiagnostic` и `ConversionStats`
  — публичный экспорт `@meerrec/core`
  ([`index.ts`](../../packages/core/src/index.ts)); CLI импортирует их
  оттуда и не заводит свою копию.
- Коды возврата CLI (0/1/2) и человекочитаемый формат ошибок сохранены;
  `--json` — единственное новое поведение, и только по явному запросу.
- В телеметрию не попадают ни содержимое документа, ни имена файлов и пути —
  ни в атрибутах спанов, ни в будущих полях ([ADR-0037](0037-opentelemetry.md)).
  `DiagnosticWire.message` — публичный текст ошибки нижнего слоя и остаётся как
  есть, но в атрибуты спанов он не переносится.
