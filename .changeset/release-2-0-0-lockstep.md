---
---

Версии десяти npm-пакетов и крейтов подняты до 2.0.0 релизным коммитом
(lockstep-гейт): мажор из-за удаления мёртвых публичных API — legacy
offscreen-блок и `VERSION` из `@meerrec/core`, Vec-экспорты
`*_build_display_list` из wasm-модулей, новые сигнатуры `Archive` в core.
Дедупликация общих примитивов (core::{ns,xml,units}, `doc-converter-wasm-common`,
общий `WorkerCore`/`viewer_core.ts`) и оптимизации горячих путей (кэши XLSX,
SAB 3→2 копии кадра, painter без аллокаций на команду, PDF-владение контентом).
Публикация идёт по тегу `v2.0.0`; пустой changeset нужен только push-гейту
`changeset status` и остаётся маркером релиза.
