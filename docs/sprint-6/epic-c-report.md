# Эпик C — шрифты и кириллица

- Спринт 6 «Базовый PDF» · ветка `sprint/6-basic-pdf`
- Коммиты: 5bd5837 (набор subset + №), ac21a62 (начертание по стилю ячейки),
  ebfb668 (ADR-0009)

## Что сделано

**C2, C4. Подрезанные Carlito.** В `crates/render/src/fonts/` — четыре
начертания (`carlito-subset.ttf`, `carlito-bold-subset.ttf`,
`carlito-italic-subset.ttf`, `carlito-bolditalic-subset.ttf`), рядом лицензия
`OFL.txt` и `README.md`. Набор собирает идемпотентный `scripts/subset-fonts.sh`:
`uvx --from fonttools==4.66.1 pyftsubset` с `--no-hinting --name-IDs='*'`,
источник пришпилен коммитом googlefonts/carlito `3a810cab78eb…`, скачанный
файл сверяется по sha256 — повторный запуск не переписывает результат.

Набор — 465 кодпоинтов на файл: ASCII (95), кириллица U+0400–U+04FF (254;
U+0476 и U+0487 в Carlito отсутствуют), пунктуация, `U+2116` № (в прежнем
subset его не было — добавлен). Замена regular проверена: advance-ширины
совпали на всех 464 общих кодпоинтах, upem 2048.

**C1. Кодирование.** Подтверждено спайком B1 и закреплено в ADR-0009:
subsetting и Type0/Identity-H/ToUnicode даёт printpdf (`subsetter` 0.42
приходит транзитивно), свой код не пишем.

**C5. ADR-0009 «Подрезка шрифтов»** (`docs/adr/0009-font-subsetting.md`).
Политика начертаний: bold/italic выбираются по стилю ячейки, а метрики и
перенос считаются **всегда** по regular (`DEFAULT_FONT_ID`) — ровно как в
canvas-пути, иначе совпадение переносов теряется. Выбор начертания по стилю —
ac21a62.

**C3.** Тест `pdftotext | grep …` живёт в CI-пути F3
(`crates/pdf/tests/external.rs`, коммит 5411d00).

## Метрики

| Начертание | Файл | Размер |
| --- | --- | --- |
| Regular | `carlito-subset.ttf` | 88 624 Б |
| Bold | `carlito-bold-subset.ttf` | 92 816 Б |
| Italic | `carlito-italic-subset.ttf` | 87 740 Б |
| BoldItalic | `carlito-bolditalic-subset.ttf` | 108 436 Б |

- Три новых начертания — 288 992 Б (прогноз спайка ~289 КБ сошёлся).
- `--no-hinting` не косметика: без него regular весит 140 864 Б против
  88 624 Б (экономия 37%).
- 465 кодпоинтов на файл; кириллический блок U+0400–U+04FF закрыт целиком.

## Отклонения от плана

- Fallback-шрифт для CJK, эмодзи и редкой латиницы (например U+0108) не
  делается: такие символы дают пустой глиф. Отложено в бэклог (решение
  архитектора, `docs/adr/0009-font-subsetting.md`).
- Проверка кириллицы через `pdftotext` выполняется только в CI: локально
  poppler нет. Тест без утилиты пропускается с сообщением, а не падает.
- Свой subsetting не писали — он уже в printpdf; `ttf-parser` из стека снят
  (RUSTSEC-2026-0192).
