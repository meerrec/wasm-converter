# Спринт 8: Парсинг DOCX + fuzz

Отчёт по [плану](plan.md) v2. Ветка `sprint/8-docx-parse-fuzz`, коммиты
`f6fa1b9`…`4759014`.

## Волны 1–2

Волны закрывают парсер, фикстуры и fuzz-каркас. Волна 3 (differential,
бюджеты, покрытие, длинный fuzz) — следующий заход, см. «Что вне захода».

### Что сделано

**`crates/core` — примитивы, на которые опирается парсер:**

- `NodeId`/`NodeIdAllocator` — стабильные идентификаторы узлов
  ([ADR-0019](../adr/0019-node-id.md));
- `ParseWarning`/`WarningKind`/`WarningLocation`/`Warnings` с порогами
  100/1000 ([ADR-0016](../adr/0016-docx-error-policy.md));
- `ZipLimits` + `Archive::open_with_limits`/`part`/`rels_for` и предупреждения
  о symlink ([ADR-0015](../adr/0015-ooxml-zip-limits.md)); percent-декодирование
  целей rels. 54 теста.

**`crates/docx` — модель §3 плана и парсеры:** 13 модулей в `src/model/`;
парсеры `xml`, `document`, `styles`, `numbering`, `settings`, `rels`,
`metadata`, `footnotes`, `comments`; публичные `parse_docx(bytes, ZipLimits)`
и `open`. Тесты: 196 unit + 3 на фикстурах + 7 round-trip + 9 политики ошибок.

**Фикстуры:** 97 коммитируемых (16 существующих + 81 новых) по всем категориям
§2.3 плана. Генератор детерминирован (`pnpm gen:docx-fixtures`, собственный
ZIP-writer вместо системного `zip`: повторный прогон не меняет дерево). Пять
«больших» фикстур (11–15 МиБ) генерируются по требованию в gitignored
`target/fixtures/docx-large/`.

**Fuzz:** `crates/fuzz/` — отдельный workspace, чтобы nightly и sanitizer не
задевали stable; 8 целей (`core_zip`, `core_xml_reader`, `core_rels`,
`xlsx_worksheet`, `docx_document`, `docx_styles`, `docx_numbering`,
`docx_rels`). DOCX-цели фаззят публичный `parse_docx`, подставляя данные в
конкретную часть пакета. `.github/workflows/fuzz.yml` — ночной cron, 15 мин на
цель, `-timeout=30`; на PR — 60 с. Короткие локальные прогоны (3000 итераций
на 5 целей) падений не дали.

**Проверки round-trip:** `Model → JSON → Model` lossless на 94 разобранных
фикстурах (3 фатальные пропущены); 1331 `NodeId` без повторов; разбор
детерминирован на 94 фикстурах; текст 117 абзацев сверен с сайдкарами.

### Метрики

| Метрика | Значение |
|---|---|
| Фикстуры | 97 коммитируемых (16 + 81) + 5 «больших» генерируются по требованию |
| Разбираются | 97/97: 94 без фатальных ошибок + 3 фатальные по замыслу |
| Round-trip | lossless на 94 фикстурах, 1331 `NodeId` без повторов |
| Тесты | `core` 54; `docx` 196 unit + 3 фикстуры + 7 round-trip + 9 политики |
| Fuzz | 8 целей; локально 3000 итераций × 5 целей — без падений |
| Предупреждения | 67 «лишних» `unknown_element` в 20 фикстурах — элементы вне модели v1 (track changes, ссылки на сноски/комментарии, RTL/CJK-разметка), сохраняются как `Unknown` по ADR-0016 |

Приёмка зелёная: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `cargo deny check`,
`cargo audit`, сборка `doc-converter-docx` под `wasm32-unknown-unknown`,
`cargo fuzz build --fuzz-dir .`.

### Отклонения от плана

1. **Имена из ADR разошлись с кодом.** ADR говорят `OoxmlArchive`/`ParseError`,
   в коде `Archive`/`Error`. Публичное API не переименовывалось; ZIP-лимиты
   добавлены в `core::Error`, а DOCX-специфичные фатальные варианты
   (`EncryptedDocument`, `MacroEnabledDocument`, `LegacyFormat`,
   `StrictNotSupported`, `TooManyWarnings`) — в новый `docx::Error`.
2. **Модель уточнена там, где план допускал неоднозначность.** Одно имя
   `Spacing` из §3.2 разведено на `ParagraphSpacing`/`CharacterSpacing`,
   `VerticalAlign` — на `CellVAlign`/`VertAlign` (в плане одно имя покрывало
   два разных XML-типа); `Document` расширен `footnotes`/`endnotes`/
   `comments`/`headers`/`footers`/`metadata`; таблицы стилей и нумерации —
   `BTreeMap` вместо `HashMap` ради детерминированного JSON; добавлен
   `RawTblPr` + `TableStyle.tbl_pr`/`ConditionalFormat.tbl_pr` (иначе теряются
   границы и заливка табличных стилей); `Cell.v_merge` — `Option<VMerge>`.
3. **Порядок слайсов изменён.** S6 (`xml.rs`) выполнен до параллельной волны:
   S8–S11 без общих хелперов не собрались бы. S7a вошёл в ту же волну, S11
   (сноски/комментарии) — после S7a, потому что вызывает
   `document::parse_blocks`.
4. **Пять субагентов упали с переполнением контекста** на больших файлах
   (`document.rs`, `styles.rs`, `numbering.rs`, `parse.rs`), не закоммитив
   работу; восстановление шло отдельными ремонтными слайсами. Регламент
   изменён: коммит после каждой фичи, чтение файлов только участками.
5. **Fuzz требует `--fuzz-dir`.** `cargo fuzz` 0.13.2 ищет проект как
   `<корень>/fuzz/Cargo.toml`, поэтому команда — `--fuzz-dir .` из
   `crates/fuzz` или `--fuzz-dir crates/fuzz` из корня. DOCX-цели фаззят
   публичное API: парсерные функции частей не публичны.
6. **`cargo audit` не читает `deny.toml`** — добавлен `.cargo/audit.toml` с тем
   же `ignore` (RUSTSEC-2026-0187, lopdf) и шаг `cargo audit` в CI-джобу
   `audit` (пункт §2.2 плана).
7. **`broken/missing_root_rels` оказалась не фатальной**: сайдкар ожидает
   `Ok` + warning `MissingRels`. Победил сайдкар как источник ожиданий, а не
   формулировка постановки слайса.
8. **`parse_docx(bytes, ZipLimits)` сделан публичным** после слайса S12a — нужен
   fuzz-целям с лимитами и ADR-0016.
9. **Генератор:** удалён устаревший `scripts/generate_docx_fixtures_manual.ts`;
   сайдкары правятся только вместе с генератором, иначе повторный прогон менял
   бы дерево.

### Что вне захода

- Differential vs mammoth: `scripts/diff-mammoth.ts`, `diff-report.md`.
- Бюджеты памяти: `memory-report.md` и бенч `openDocx` на 50 МБ.
- Покрытие: `cargo-llvm-cov` в CI и порог (Rust ≥ 85%, TS ≥ 80%),
  `coverage-report.md`.
- Длинный fuzz: 24 часа без падений, `fuzz-report.md`; решение по OSS-Fuzz.
- Волна 3 целиком и финальный отчёт спринта с ретро.
