# ADR-0034: Strict OOXML — поддержка для DOCX и XLSX

**Статус:** Принято
**Дата:** 21.10.2026
**Контекст:** Эпик 3, задача T3.2 (Strict OOXML)
**Связанные ADR:** [ADR-0017](0017-strict-vs-transitional.md) (пересмотрен в
части «только Transitional»), [ADR-0039](0039-cancel-v1-non-goals.md) (отмена
non-goal'а Strict), [ADR-0016](0016-docx-error-policy.md) (политика ошибок
DOCX), [ADR-0015](0015-ooxml-zip-limits.md) (лимиты ZIP — без изменений)

---

## Контекст

- OOXML существует в двух вариантах: **Transitional** (ECMA-376; Word по
  умолчанию) и **Strict** (ISO/IEC 29500-1 Strict) — с разными namespace'ами.
- ADR-0017 откладывал Strict в v1: парсер узнавал Strict по подстроке
  `http://purl.oclc.org/ooxml/` в начале частей пакета и отвергал документ
  ошибкой `Error::StrictNotSupported`.
- ADR-0039 снял non-goal-статус Strict; этот ADR — запланированный там
  пересмотр ADR-0017.
- Парсеры обеих крейтов (`crates/docx`, `crates/xlsx`) сравнивают локальные
  имена элементов и атрибутов, а не namespace: `w:p` и его Strict-аналог дают
  одно и то же `p`. Контентная модель обоих вариантов одинакова, поэтому
  поддержка Strict не требует ветвления по варианту внутри парсеров.
- Типы связей (`r:`) у обоих вариантов различаются префиксом URI, но хвосты
  (`/officeDocument`, `/styles`, `/worksheet`, `/image`, …) совпадают; всё
  существующее сопоставление связей — по хвостам — работает без изменений.

## Решение

### 1. Strict поддерживается для DOCX и XLSX

Оба формата принимают пакеты Strict наравне с Transitional — один парсер, одна
модель, один DisplayList. Механизм общий для двух крейтов: таблица
namespace'ов и определение варианта по корню главной части.

Namespace'ы (у SpreadsheetML Strict-вариант дополнительный):

| | Transitional | Strict |
|---|---|---|
| WordprocessingML | `http://schemas.openxmlformats.org/wordprocessingml/2006/main` | `http://purl.oclc.org/ooxml/wordprocessingml/main` |
| SpreadsheetML | `http://schemas.openxmlformats.org/spreadsheetml/2006/main` | `http://purl.oclc.org/ooxml/spreadsheetml/main` |
| DrawingML main | `http://schemas.openxmlformats.org/drawingml/2006/main` | `http://purl.oclc.org/ooxml/drawingml/main` |
| Relationships | `http://schemas.openxmlformats.org/officeDocument/2006/relationships` | `http://purl.oclc.org/ooxml/officeDocument/relationships` |

### 2. Вариант определяется по корню главной части

Решающий признак — namespace корневого элемента `word/document.xml` (DOCX) /
`xl/workbook.xml` (XLSX), а не `[Content_Types].xml`: content types у Strict и
Transitional совпадают, и признака варианта там нет. Проверка читает уже
прочитанный буфер главной части и смотрит её первый элемент: объявление
`xmlns` разрешается по префиксу корня (`w:document` → `xmlns:w`, беспрефиксный
корень → `xmlns`). Второй раз часть не читается.

Ранняя отбраковка Strict по подстроке из ADR-0017 удалена вместе с
`Error::StrictNotSupported` (вариант публичный, но ссылок вне `crates/docx`
grep не нашёл — снят без alias'а).

### 3. Таблицы namespace'ов

В каждом крейте — модуль `ns`:

- `crates/docx/src/ns.rs`: `NsTable { wordprocessingml, drawingml, relationships }`,
  константы `TRANSITIONAL`/`STRICT`, `OoxmlVariant::{Transitional, Strict}`,
  `detect(xml, part) -> Result<OoxmlVariant>`.
- `crates/xlsx/src/ns.rs`: `NsTable { spreadsheetml, drawingml, relationships }`
  — тот же механизм, detect по `xl/workbook.xml`.

Парсеры частей таблицу не получают: они сравнивают локальные имена, и от
варианта зависят только точки входа. Таблица публична как контракт для будущих
плагинов форматов (ADR-0030): она фиксирует, какие namespace'ы у варианта.

### 4. Незнакомый namespace — отказ с понятной ошибкой

Namespace корня, которого нет ни в Transitional, ни в Strict, — фатальная
ошибка `Error::UnknownVariant { part, namespace }` (DOCX) /
`XlsxError::UnknownVariant { part, namespace }` (XLSX): «unsupported OOXML
namespace `…` in `…`; expected Transitional or Strict». Молча разбирать такой
пакет — значит описывать несуществующий формат.

### 5. Фикстуры и оракул

- В `scripts/generate_docx_fixtures.ts` и `scripts/gen-fixtures.ts` — флаг
  `--strict`: те же фикстуры, но с purl-namespace'ами и Strict-типами связей
  (переписываются только XML/rels-части; namespace пакета и content types не
  трогаются). Выход детерминирован, как и обычный прогон.
- Коммитится подмножество: `test-fixtures/docx/strict/` (5 фикстур: тело,
  стили, рисунок, таблица, нумерация) и `test-fixtures/xlsx/strict/`
  (4 книги: значения, стили, диаграммы, картинка). Сайдкары strict-фикстур
  идентичны Transitional-оригиналам: модель от варианта не зависит.
- Оракул (`oracle.yml`): в джобы `ssim` и `docx` добавлен второй прогон по
  strict-подмножеству с отдельным каталогом отчёта. Эталон — тот же
  LibreOffice: он читает оба варианта.

## Последствия

### Положительные

- Strict-документы (государственные системы, ISO-экспортёры) открываются тем же
  путём, что Transitional: без второй модели, второй раскладки и второго
  DisplayList.
- Убран тонкий механизм отбраковки по подстроке — признак варианта теперь
  структурный (namespace корня), а не байтовый.
- Публичный контракт `NsTable`/`OoxmlVariant` — готовый ответ будущим плагинам
  форматов на вопрос «какие namespace'ы у варианта».

### Отрицательные

- Ошибки модели: парсер не видит разницы вариантов внутри частей. Если у
  Strict-документа встретится строго-специфичная конструкция (например,
  атрибуты, которых нет в Transitional), она разберётся по общим правилам или
  уйдёт в `Unknown` — как любой незнакомый элемент (ADR-0016).
- `Error::StrictNotSupported` удалён из публичного API без alias'а; коду,
  ловившему его, нужно перейти на `UnknownVariant` (внешних ссылок нет).

### Нейтральные

- Лимиты ZIP (ADR-0015) и бюджеты не меняются: разбор Strict не дороже
  Transitional — таблица namespace'ов статическая, аллокаций в горячем цикле
  нет.
- `[Content_Types].xml` и `_rels/.rels` парсятся как раньше: namespace пакета
  (`package/2006/…`) у обоих вариантов один.

## Альтернативы

- **Оставить Strict non-goal'ом.** Отклонено ADR-0039: To-Be документа
  архитектуры требует Strict.
- **Ветвить парсеры по варианту.** Отклонено: локальные имена одинаковы,
  ветвление удвоило бы парсеры без разницы в поведении.
- **Определять вариант по `[Content_Types].xml`.** Отклонено: content types у
  вариантов совпадают, признака там нет.

## Ссылки

- ISO/IEC 29500-1:2016 (Strict), ECMA-376.
- ADR-0017 (пересмотрен здесь), ADR-0039.
