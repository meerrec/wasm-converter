# Мини-спринт 2.5: `fitToPage` для экспорта XLSX → PDF

**Файл:** `docs/sprint-2.5-fit-to-page/plan.md`
**Версия:** 1.0 (23.10.2026)
**Длительность:** 1 день
**Команда:** R1 (Rust), TS, QA
**Статус:** план утверждён к старту
**Предшествует:** Спринт 2 (TextLayer, bidi, a11y), релиз v1.3.0
**Блокирует:** —

## 1. Цель

Широкий лист XLSX сейчас режется полосами столбцов: таблица разъезжается по
нескольким страницам. PDF-крейт уже умеет «уместить по ширине/высоте»
(`PageConfig::fit_to_width/height`, листовой `fitToPage`), но публичный API
экспорта эти поля не выставляет. Мини-спринт открывает их в `@meerrec/core`,
CLI и wire-протоколе.

## 2. Решение

- `PdfOptions` получает `fitToPage?: boolean`, `fitToWidth?: number`,
  `fitToHeight?: number`.
- Семантика:
  - `fitToPage: true` — сахар для `fitToWidth = fitToHeight = 1`;
  - явные `fitToWidth`/`fitToHeight` сильнее флага;
  - `0` — без ограничения (естественная высота/ширина);
  - `scale` и fit-режим взаимоисключающие: явные fit-поля перебивают масштаб.
- Wire: `PdfOptionsWire` + `pnpm gen:wire`; поля camelCase.
- CLI: `--fit-to-page`, `--fit-to-width <n>`, `--fit-to-height <n>`.
- DOCX/PPTX: поля игнорируются — страницы берутся из секции/слайда; это
  документируется в JSDoc и cookbook.

## 3. Слайсы

| Слайс | Что | Проверка |
|---|---|---|
| S1 | `PdfOptionsWire` + маппинг в `pdf-wasm` | wasm-тесты `fit_to_page.rs` |
| S2 | CLI-флаги и проброс в wasm | `vitest` args/convert |
| S3 | Cookbook-рецепт, мини-спринт-доки | ревью |
| S4 | Полные гейты, changeset, PR | CI |

## 4. DoD

1. `exportPdf({ fitToPage: true })` укладывает лист на одну страницу.
2. `fitToWidth`/`fitToHeight` перебивают флаг; `0` — без ограничения.
3. CLI-флаги работают в `convert` и `batch`.
4. Rust/TS/CLI-тесты зелёные; бюджеты не выросли.
5. Cookbook-рецепт и JSDoc обновлены.
