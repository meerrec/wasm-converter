---
---

Версии npm-пятёрки уже подняты до 1.1.0 релизным коммитом
(`pnpm changeset version` + правка `Cargo.toml`): гейт `release.yml`
требует lockstep — все пять npm-пакетов и крейт `doc-converter-core` на
одной версии, поэтому `wasm-docx`/`wasm-pdf`/`cli` подняты вместе с
`core`/`wasm`. Публикация идёт по тегу `v1.1.0` (шаг `changeset version`
на теге не запускается), поэтому релиз этот файл не планирует: пустой
changeset нужен только PR-гейту `changeset status --since=origin/main`
и удаляется в релизном коммите перед тегом (тег-гейт требует ноль
pending changesets).
