# ODF-фикстуры

Эталоны для будущего крейта `crates/odf` (ODF 1.2: `.odt`, `.ods`, `.odp`).
Парсера пока нет — фикстуры фиксируют вход, который ему предстоит разбирать,
и собираются генератором, а не руками в LibreOffice, чтобы изменения были
видны в диффе.

## Команда

```bash
pnpm gen:odf-fixtures          # то же, что npx tsx scripts/generate_odf_fixtures.ts
```

Генератор пишет `test-fixtures/odf/{odt,ods,odp}/` и не трогает этот README:
при прогоне удаляются только каталоги форматов.

## Раскладка

```text
test-fixtures/odf/
  README.md
  odt/simple.odt
  odt/table_image.odt
  odt/page_styles.odt
  ods/simple.ods
  ods/repeated.ods
  odp/simple.odp
  odp/text_styles.odp
```

Каждый пакет — ODF 1.2 со всеми обязательными частями:

| Часть | Что внутри |
|---|---|
| `mimetype` | Первая запись архива, **без сжатия**, без extra-поля; определяет формат |
| `META-INF/manifest.xml` | `manifest:version="1.2"`, запись `/` и перечень всех частей пакета |
| `content.xml` | `office:body` — текст, электронная таблица или презентация |
| `styles.xml` | `office:styles`, `office:automatic-styles`, `office:master-styles` |
| `meta.xml` | Фиксированные `dc:title`, `meta:generator`, даты создания/изменения |
| `settings.xml` | Минимальные настройки просмотра |
| `Pictures/pixel.png` | 1×1 PNG фиксированными байтами (где есть картинка) |

## Фикстуры

| Файл | Покрытие |
|---|---|
| `odt/simple.odt` | `text:h` двух уровней, абзацы, прогоны bold/italic, гиперссылка `text:a`, маркированный список с вложенным уровнем, значимые пробелы (`xml:space="preserve"`, `text:s`, `text:tab`) |
| `odt/table_image.odt` | Таблица 2×2 с объединённой шапкой (`number-columns-spanned` + `covered-table-cell`), границы/заливка/отступы ячеек, встроенный `draw:frame` с `svg:width/height` и `Pictures/pixel.png` |
| `odt/page_styles.odt` | Два `style:page-layout` (A4 портрет и A4 альбом) с `fo:page-width/height` и полями, `style:master-page` с колонтитулами, разрыв страницы `fo:break-before="page"`, смена мастер-страницы через `style:master-page-name` |
| `ods/simple.ods` | Два листа, строки/числа/boolean/дата, формулы (`of:=SUM([.B2:.B3])` и межлистовой `of:=SUM([Data.B2:Data.B3])`), объединение A4:B4, ширины столбцов и высота строк, стили (bold, заливка, границы) и числовые форматы (`number:number`, `number:percentage`, `number:date`) |
| `ods/repeated.ods` | `table:number-rows-repeated` (4 строки), `table:number-columns-repeated` (пустые ячейки), `table:number-columns-spanned`/`rows-spanned` (2×2), покрытые ячейки |
| `odp/simple.odp` | Два `draw:page`, текст-бокс с абзацами и прогонами, геометрия `svg:x/y/width/height`, `draw:rect` с заливкой и контуром, картинка `Pictures/pixel.png` |
| `odp/text_styles.odp` | Заголовок и абзацы разных кеглей/цветов (40/24/18/14/11 pt), вложенный список с разными стилями текста |

## Детерминированность

Два прогона подряд дают побайтово одинаковые файлы:

- фиксированные DOS-времена `1980-01-01 00:00` (как в генераторах DOCX/PPTX);
- порядок записей архива задан массивом, без обхода файловой системы;
- CRC32 считается собственной таблицей, без зависимости от версии Node;
- `mimetype` всегда пишется хранимым (store), остальные части — deflate
  level 9, если он реально уменьшает размер (несжимаемый PNG хранится как есть);
- никаких `Date.now()`, случайных id и времени генерации: `meta.xml` содержит
  фиксированные дату `2024-01-01T00:00:00` и статичный `meta:generator`.

Проверка:

```bash
pnpm gen:odf-fixtures
find test-fixtures/odf -name '*.od?' | sort | xargs shasum -a 256 > /tmp/odf.sha
pnpm gen:odf-fixtures
find test-fixtures/odf -name '*.od?' | sort | xargs shasum -a 256 | diff /tmp/odf.sha -
```

## Проверка пакетов

```bash
# список частей; mimetype должен быть первым
unzip -l test-fixtures/odf/odt/simple.odt

# mimetype лежит без сжатия и совпадает с форматом
unzip -p test-fixtures/odf/odt/simple.odt mimetype

# без unzip: python3 -m zipfile -l test-fixtures/odf/odt/simple.odt
```

XML во всех частях — валидный UTF-8 без BOM; пробелы в тексте значимы, поэтому
там, где они важны, стоит `xml:space="preserve"`. Для формул объявлено
пространство имён `of` (`urn:oasis:names:tc:opendocument:xmlns:of:1.2`) — без
него LibreOffice показывает `Err:510`. Атрибут `style:master-page-name` лежит
на самом `style:style`, а не внутри `style:paragraph-properties` (так его
понимает LibreOffice).

## Правка фикстур

Меняйте только `scripts/generate_odf_fixtures.ts` и перегенерируйте файлы;
ручные правки `.odt/.ods/.odp` потеряются при следующем прогоне. Сгенерированные
пакеты коммитятся вместе с генератором.
