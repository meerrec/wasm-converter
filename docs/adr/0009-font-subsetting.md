# ADR 0009: Подрезка шрифтов — четыре подрезанных Carlito

**Статус:** принято 06.10.2026; реализуется в Спринте 6 (срезы C2 и C4).
**Контекст:** [`crates/render/src/fonts/README.md`](../../crates/render/src/fonts/README.md),
[`scripts/subset-fonts.sh`](../../scripts/subset-fonts.sh),
[`crates/render/src/font.rs`](../../crates/render/src/font.rs),
[`crates/pdf/src/fonts.rs`](../../crates/pdf/src/fonts.rs),
[`crates/pdf/src/text.rs`](../../crates/pdf/src/text.rs),
[`crates/xlsx/src/paint.rs`](../../crates/xlsx/src/paint.rs), [`deny.toml`](../../deny.toml),
[ADR-0002](0002-font-parsing.md), [ADR-0007](0007-pdf-layout-boundary.md)

## Контекст

ADR-0007 требует, чтобы экран и PDF считали текст одним и тем же шрифтом: иначе точки
переноса разойдутся. Canvas измеряет `carlito-subset.ttf` — `DEFAULT_FONT`
([`font.rs:30`](../../crates/render/src/font.rs)); PDF встраивает те же байты через
`render::font::default_font_bytes()`, своей копии TTF в `crates/pdf` нет.

Для DoD Спринта 6 этого мало: нужны кириллица (фикстуры `text-cyrillic-wrap.xlsx`,
`scale-ten-pages.xlsx` с «Привет» и «№») и начертания bold/italic по стилю ячейки.
В [`crates/render/src/fonts/`](../../crates/render/src/fonts/) лежат четыре подрезанных
Carlito и `OFL.txt`:

| Файл | Начертание | Размер |
|---|---|---|
| `carlito-subset.ttf` | Regular | 88 624 Б |
| `carlito-bold-subset.ttf` | Bold | 92 816 Б |
| `carlito-italic-subset.ttf` | Italic | 87 740 Б |
| `carlito-bolditalic-subset.ttf` | Bold Italic | 108 436 Б |

## Рассмотренные варианты

1. **Подрезать Carlito заранее и положить в репозиторий** (выбран). Плюсы: байт в байт
   воспроизводимо, набор символов проверяем, в бандле только нужное.
2. **Полагаться на подрезку printpdf/allsorts, встраивая полные шрифты.** Плюсы: не
   нужен свой скрипт. Минусы: полные файлы должны лежать в WASM-бандле, состав
   символов перестаёт быть проверяемым до экспорта.
3. **Свои subsetter'ы** (`subsetter`, `allsorts` напрямую). Минусы: `subsetter` уже
   приходит транзитивно (svg2pdf → printpdf), но прямой зависимости нет, а её
   подключение — та же работа, которую printpdf делает при сериализации.

## Решение

В PDF встраиваются четыре подрезанных Carlito из `crates/render/src/fonts/`. Источник —
официальный репозиторий github.com/googlefonts/carlito, ветка main, подпуть `fonts/ttf/`,
пришпилен коммитом `3a810cab78ebd6e2e4eed42af9e8453c4f9b850a`. Сборка воспроизводима:
`scripts/subset-fonts.sh` идемпотентен и ставит `pyftsubset` через
`uvx --from fonttools==4.66.1` с флагами `--no-hinting --name-IDs='*'`.

`--no-hinting` — не косметика: без него regular весит 140 864 Б против 88 624 Б,
хинтинг адресован растровому рендеру и в PDF не встраивается; прежний subset и так был
без него.

Набор — 465 кодпоинтов на файл: ASCII 95, кириллица U+0400–U+04FF (254 кодпоинта;
U+0476 и U+0487 в Carlito отсутствуют), пунктуация, № (U+2116). Замена regular
проверена: advance-ширины совпали на всех 464 общих кодпоинтах, upem 2048, добавлен
только U+2116.

Подрезка на экспорте — на стороне printpdf (allsorts, `SubsetProfile::Web`): в PDF
попадают только использованные глифы. Свои subsetter'ы не подключаем.

Политика начертаний: bold/italic выбираются по стилю ячейки; метрики и перенос
**всегда** считаются по regular (`DEFAULT_FONT_ID`) — ровно как в canvas-пути
([`paint.rs:1497-1498`](../../crates/xlsx/src/paint.rs)), иначе точки разрыва
разойдутся с экраном.

## Следствия

- Дыры в наборе: нет CJK, эмодзи, части редких латинских (например U+0108) — такие
  символы дают пустой глиф. Fallback-шрифт отложен в бэклог (решение архитектора).
- Три новых начертания — 288 992 Б в WASM-бандле; `.size-limit.json` этот рост не
  видит (там только JS-бандлы), поэтому он фиксируется замером.
- Лицензия: `deny.toml` разрешает Apache-2.0/MIT/BSD-2-Clause/BSD-3-Clause/ISC/
  Unicode-3.0/Zlib, а OFL — лицензия файла шрифта, не крейта; `OFL.txt` обязан
  лежать рядом со шрифтами.
- Скрипт — единственный путь пересборки: при изменении набора нужно заново сверять
  advance-ширины regular, иначе совпадение переносов с canvas теряется.
