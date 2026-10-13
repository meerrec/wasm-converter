---
---

Правка пакетов обвязки x2t (Спринт 13): фикс загрузки glue на главном потоке
в `@doc-converter/x2t` и новый пакет `@doc-converter/browser-convert`.
В релиз v1.0.0 они не входят — publish-джоба публикует пять пакетов явным
списком (wasm, wasm-docx, wasm-pdf, core, cli), поэтому релиз эта правка
не планирует: пустой changeset нужен только PR-гейту `changeset status`.
