---
'@meerrec/core': minor
'@meerrec/cli': minor
'@meerrec/wasm-xlsb': patch
'@meerrec/wasm-ods': patch
'@meerrec/wasm-odt': patch
'@meerrec/wasm-odp': patch
---

Четыре ленивых wasm-модуля (XLSB, ODS, ODT, ODP) и фабрики
`createXlsbViewer`/`createOdsViewer`/`createOdtViewer`/`createOdpViewer`:
воркер на формат, модуль грузится по URL при открытии файла.
CLI понимает `.xlsb`, `.ods`, `.odt`, `.odp` на входе.
