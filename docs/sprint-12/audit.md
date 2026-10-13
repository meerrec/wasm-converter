# Спринт 12, слайс S4.1: аудит безопасности и лицензий

**Файл:** `docs/sprint-12/audit.md`
**Дата прогона:** 13.10.2026, ветка `main`
**Слайс:** S4.1 плана `docs/sprint-12/plan.md`, запущен раньше фазы 4 намеренно —
находки обрабатываются с запасом времени.
**Инструменты:** cargo-deny 0.20.2, cargo-audit 0.22.2 (база 1308 advisories),
pnpm 9.12.0 на Node 25.6.1.

Только проверки и фиксация находок; зависимости и конфиги не менялись.

## Резюме

| Проверка | Exit | Вердикт |
|---|---|---|
| `cargo deny check` | 0 | Жёсткие проверки чисто; 4 warning'а — не блокеры (§1) |
| `cargo audit` | 0 | Чисто только за счёт задокументированного ignore; без него — 1 находка: lopdf, CVSS 7.5 (§2) |
| `pnpm audit` | 1 | 22 находки (4 critical, 6 high, 11 moderate, 1 low), все — dev-тулинг; политика CI «информационно» сохранена (§3) |
| Лицензии вручную | — | Шрифты ✓, README ✓, **файл LICENSE в корне отсутствует** ✗ (§4) |

## 1. `cargo deny check` — чисто (жёсткие проверки), 4 warning'а

Команда: `cargo deny check` из корня. Exit 0. Итоговая строка вывода:
`advisories ok, bans ok, licenses ok, sources ok`.

Конфиг `deny.toml` прочитан целиком: лицензионный allowlist из семи записей
(confidence 0.9), `multiple-versions = "warn"`, `wildcards = "deny"`,
бан `openssl`, `unmaintained = "workspace"`,
`ignore = ["RUSTSEC-2026-0187"]`.

Разбор каждого пункта вывода:

| Вывод | Разбор | Классификация |
|---|---|---|
| `warning[license-not-encountered]` — `BSD-2-Clause` | Запись allowlist не встречается ни у одного крейта дерева. Мёртвая запись, на проверку не влияет | Не блокер. Гигиена: удалить строку из allowlist |
| `warning[duplicate]` `miniz_oxide` 0.8.9 / 0.9.1 | 0.8.9 тянут `backtrace` (через miette) и `png` (через image, только pdf); 0.9.1 — `flate2` 1.1.10 (zip/lopdf/png). Двойной кодек в wasm/pdf-сборках — немного лишних байт | Не блокер: конфиг явно ставит `warn` |
| `warning[duplicate]` `syn` 2.0.119 / 3.0.6 | 2.x тянут miette-derive, tracing-attributes, zerocopy-derive; 3.x — serde_derive, thiserror, wasm-bindgen, displaydoc и прочие. Proc-macro-дубль, в рантайм не попадает | Не блокер: `warn` |
| `warning[duplicate]` `unicode-width` 0.1.14 / 0.2.2 | Обе версии — под miette (0.1.14 напрямую, 0.2.2 через textwrap) | Не блокер: `warn` |
| `advisories ok` | Достигнуто за счёт ignore `RUSTSEC-2026-0187` — разбор в §2 | См. §2 |
| `bans ok` | `openssl` в дереве нет; wildcard-версий нет | Чисто |
| `licenses ok` | Все лицензии дерева в allowlist | Чисто |
| `sources ok` | Источники — только crates.io (конфиг по умолчанию) | Чисто |

## 2. `cargo audit` — чисто с ignore; под ним 1 находка (lopdf)

Команда: `cargo audit`. Exit 0, 211 зависимостей, 0 уязвимостей.

Важная деталь: репозиторий дублирует исключение в `.cargo/audit.toml`
(`cargo audit` не читает `deny.toml`). Комментарий в файле называет это
«осознанной записью». Чтобы проверить, что под исключением реально есть
находка, audit прогнан из чистой директории без конфига:

```
$ cargo audit --file Cargo.lock   # из каталога без .cargo/audit.toml
Crate:     lopdf
Version:   0.35.0
Title:     Stack overflow in lopdf via deeply nested PDF objects
Date:      2026-06-21
ID:        RUSTSEC-2026-0187
URL:       https://rustsec.org/advisories/RUSTSEC-2026-0187
Severity:  7.5 (high)
Solution:  Upgrade to >=0.42.0
error: 1 vulnerability found!
```

**Суть advisory:** `lopdf::Document::load_mem` (и прочие `load*`) парсят
вложенные PDF-массивы/словари неограниченной рекурсией; PDF с ~10 000 уровнями
вложенности (~21 КБ) валит процесс по stack overflow (`SIGABRT`, не ловится
`catch_unwind`) — DoS для любого сервиса, парсящего недоверенный PDF.
Зафиксировано в 0.42.0.

**Оценка эксплуатируемости в нашем продукте:** `grep` по `crates/pdf/src`
показывает, что `Document::load_mem` вызывается только внутри
`#[cfg(test)]`-модулей (`pdf/session.rs`, модуль с строки 502; `pdf/fonts.rs`,
с строки 360) — обратная проверка собственного вывода. Публичное API крейта —
только запись: `export_docx`, `export_xlsx_sheet`, `export_xlsx_book`.
Продукт PDF **не парсит** (lopdf используется как модель объектов на запись,
ADR-0022); недоверенный вход через `load_mem` в проде не проходит.

**Классификация:** не блокер при сохранении ignore. Две рекомендации,
требующие решения до релиза:

1. **Предпочтительно:** обновить `lopdf` до ≥ 0.42.0 и снять ignore в обоих
   местах (`deny.toml`, `.cargo/audit.toml`) — audit станет честно зелёным.
2. **Альтернатива:** оставить ignore, но зафиксировать обоснование в ADR
   (сейчас оно живёт только в комментариях двух конфигов), с пометкой: как
   только в продукте появится чтение PDF — advisory становится блокером.

Примечание: ignore продублирован в двух файлах — риск рассинхрона при будущих
правках; при любом варианте стоит оставить одно место истины.

## 3. `pnpm audit` — 22 находки, все в dev-тулинге

Exit 1; сводка pnpm: `22 vulnerabilities — 1 low | 11 moderate | 6 high |
4 critical`. В CI команда информационная (`|| true`) — политика сохранена.

Главный факт: рантайм-поверхность публикуемых пакетов чиста.
`@doc-converter/core` зависит только от `@doc-converter/wasm`, `@doc-converter/cli` —
от workspace `wasm-docx`/`wasm-pdf`; сторонних рантайм-зависимостей нет.
Все находки — в `devDependencies` корня и пакетов, либо в приватном
`examples/viewer`.

16 уникальных advisory (pnpm считает 22 по путям):

| Пакет (версия) | Sev | Advisory | Фикс | Где | Оценка |
|---|---|---|---|---|---|
| vitest 2.1.9 | critical | GHSA-5xrq-8626-4rwp (UI-сервер: чтение/выполнение файлов) | ≥ 3.2.6 | devDeps cli, core | UI-сервер в CI не запускается (`vitest run`). Не блокер |
| tinypool 1.1.1 | critical | GHSA-5gmw-xhrv-c9v3 (prototype pollution → RCE в worker options) | ≥ 2.1.1 | через vitest | Требует контроля опций воркера атакующим; CI-тулинг. Не блокер |
| tinypool 1.1.1 | critical | GHSA-85c8-ppgw-ccpr (то же в `run()`) | ≥ 2.1.2 | через vitest | То же. Не блокер |
| xlsx 0.18.5 | high | GHSA-4r6h-8v6p-xvw6 (prototype pollution) | нет на npm | devDeps корня (gen-fixtures, bench) | npm-пакет не обновлялся с 2022; SheetJS публикует 0.20.x на своём CDN. Dev-only, доверенный вход. Не блокер |
| xlsx 0.18.5 | high | GHSA-5pgg-2g8v-p4x9 (ReDoS) | нет на npm | то же | То же |
| vite 5.4.21 | high | GHSA-fx2h-pf6j-xcff (обход `server.fs.deny` на Windows) | ≥ 6.4.3 | dev viewer + внутри vitest | Dev-сервер, пример приватный. Не блокер |
| braces 3.0.3 | high | GHSA-vfj7-8cjw-p6xm (ReDoS на вложенных паттернах) | нет | через @changesets/cli | Тулинг релизов на доверенном входе. Не блокер |
| source-map-js 1.2.1 | high | GHSA-68fv-2mgg-jv7q (DoS через source-map) | ≥ 1.2.2 | через vite/postcss | Dev-сборка. Не блокер |
| esbuild ≤ 0.24.2 | moderate | GHSA-67mh-4wv8-2f99 (dev-сервер отдаёт ответы любому сайту) | ≥ 0.25.0 | через vite | Dev. Не блокер |
| vite 5.4.21 | moderate | GHSA-4w7w-66w2-5vf9 (path traversal в `.map` оптимизированных deps) | ≥ 6.4.2 | dev viewer + vitest | Dev. Не блокер |
| uuid 8.3.2 | moderate | GHSA-w5hq-g745-h8pq (нет проверки границ буфера) | ≥ 11.1.1 | через exceljs (devDep корня) | Оракул/фикстуры. Не блокер |
| vite 5.4.21 | moderate | GHSA-v6wh-96g9-6wx3 (утечка NTLMv2-хеша на Windows) | ≥ 6.4.3 | dev viewer + vitest | Dev, Windows-only. Не блокер |
| vitest 2.1.9 | moderate | GHSA-82fw-gwwq-j7x9 (path traversal через redirect-mock) | ≥ 4.1.11 | devDeps cli, core | Тесты локально/CI. Не блокер |
| @vitest/mocker 2.1.9 | moderate | GHSA-82fw-gwwq-j7x9 | ≥ 4.1.11 | через vitest | То же |
| sprintf-js 1.0.3 | moderate | GHSA-hp3w-g68c-fv3c (DoS через неограниченную точность) | нет | через @changesets/cli | Тулинг релизов. Не блокер |
| esbuild 0.27.7 | low | GHSA-g7r4-m6w7-qqqr (чтение файлов на Windows) | ≥ 0.28.1 | через tsup (devDep core) | Dev. Не блокер |

**Рекомендация:** после релиза — волна обновления dev-тулинга: vitest 2 → 4
(закрывает все 4 critical и 2 moderate), vite 5 → 7, esbuild ≥ 0.28.1,
source-map-js ≥ 1.2.2, свежий changesets. `xlsx`/`braces`/`sprintf-js` patched
на npm не имеют — остаются dev-only с записью о доверенном входе.

## 4. Лицензии вручную

**Шрифты — чисто.** `crates/render/src/fonts/OFL.txt` (94 строки, полный текст
SIL OFL 1.1, «Copyright 2013 The Carlito Project Authors… with Reserved Font
Name "Carlito"») лежит рядом с четырьмя подрезанными `.ttf` — требование
лицензии выполнено; соседний `README.md` фиксирует источник и обязанность.
Других вендоренных шрифтов/ресурсов с лицензионными обязательствами нет
(`find` по `*.ttf|*.otf|*.woff*` — только четыре Carlito).

**Корень — находка.** Файла `LICENSE` нет нигде в репозитории (`find` по
`LICENSE*|COPYING*|NOTICE*` пуст). При этом лицензия задекларирована везде:
README («## Лицензия — Apache-2.0»), workspace `Cargo.toml`
(`license = "Apache-2.0"`), все восемь крейтов (`license.workspace` или явно),
все пять публикуемых npm-пакетов (`"license": "Apache-2.0"`).
`examples/viewer` — `private: true`, поля нет — допустимо.

Apache-2.0 §4(a) требует прикладывать копию лицензии к дистрибутивам.
npm кладёт `LICENSE` в тарболл автоматически, cargo — тоже, но файла нет,
значит текст лицензии не попадёт ни в один публикуемый артефакт.
**Рекомендация:** до публикации добавить `LICENSE` с текстом Apache-2.0 в
корень и копии в публикуемые пакеты/крейты (или `license-file` в
`Cargo.toml`); стыкуется с чек-листом S2.4 (пункт «лицензии»).
**Классификация:** не блокер инструментов (ни один CI-гейт не падает), но
обязательная гигиена перед v1.0.0 — публикация без текста лицензии для
Apache-2.0 недопустима.

**`packages/` — замечаний нет.** SPDX-заголовков в TS-файлах нет, но
конвенции репозитория их не требуют (лицензия объявлена в `package.json`),
отсутствие консистентно.

## 5. Что блокирует релиз

**Жёстких блокеров нет:** все гейты CI остаются зелёными при текущих конфигах
(`cargo deny check` и `cargo audit` проходят с задокументированными ignore).

**Обязательно до публикации (иначе релиз v1.0.0 некорректный):**

1. **LICENSE-файл.** Добавить текст Apache-2.0 в корень и в публикуемые
   пакеты/крейты (S2.4/S4.2).
2. **Решение по lopdf.** Либо обновление до ≥ 0.42.0 со снятием ignore в
   `deny.toml` и `.cargo/audit.toml`, либо ADR с обоснованием осознанного
   ignore («продукт PDF не парсит; `load_mem` — только тесты»). Оставить как
   есть без ADR — значит принять скрытое решение; при появлении чтения PDF
   advisory станет блокером автоматически.

**Гигиена после релиза (не блокеры):** обновление pnpm dev-тулинга (§3),
дубли крейтов `miniz_oxide`/`syn`/`unicode-width` и мёртвая запись
`BSD-2-Clause` в allowlist `deny.toml` (§1).

## Отклонения

Нет. Все команды из задания прогнаны, весь вывод разобран; правок (кроме
этого отчёта) не вносилось.
