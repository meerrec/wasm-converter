# ADR-0017: Strict vs Transitional OOXML

**Статус:** Принято
**Дата:** 05.10.2026
**Контекст:** проектирование парсера DOCX перед Спринтом 8
**Связанные ADR:** ADR-0013

## Контекст

OOXML существует в двух вариантах:

- **Transitional** (ISO/IEC 29500-1:2016, ECMA-376) — используется Word по умолчанию, Google Docs, LibreOffice, большинством экспортёров.
- **Strict** (ISO/IEC 29500-1:2016, Part 1, Strict) — более строгий, убирает legacy (VML, некоторые атрибуты). Namespaces разные.

Namespaces:

| Transitional | Strict |
|---|---|
| `http://schemas.openxmlformats.org/wordprocessingml/2006/main` | `http://purl.oclc.org/ooxml/wordprocessingml/main` |
| `http://schemas.openxmlformats.org/drawingml/2006/main` | `http://purl.oclc.org/ooxml/drawingml/main` |
| `http://schemas.openxmlformats.org/officeDocument/2006/relationships` | `http://purl.oclc.org/ooxml/officeDocument/relationships` |

Strict встречается редко: некоторые государственные системы, старые экспортёры, документы из ISO-стандартов.

## Решение

### 1. Поддерживаем только Transitional в v1

Strict — non-goal v1. Парсер:

- распознаёт Transitional namespaces;
- при обнаружении Strict возвращает `ParseError::StrictNotSupported` с понятным сообщением;
- фиксирует в README как non-goal.

### 2. Определение варианта

Проверка при открытии `document.xml`:

```rust
fn detect_ooxml_variant(reader: &XmlReader) -> OoxmlVariant {
    // Смотрим namespace корневого элемента
    match root_namespace {
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main" => Transitional,
        "http://purl.oclc.org/ooxml/wordprocessingml/main" => Strict,
        _ => Unknown,
    }
}
```

### 3. Что это значит для кода

- Парсеры используют namespace prefix `w`, но сравнивают не строку `"w:p"`, а `(namespace, local_name)`.
- Namespace URL фиксирован: Transitional.
- Все фикстуры — Transitional.

### 4. Триггер для пересмотра

ADR пересматривается, если:

- появляется требование поддержки Strict (клиент, регулятор);
- появляется фикстура Strict, которая должна открываться.

Пересмотр — отдельный ADR, отдельный спринт.

## Последствия

### Положительные

- Код парсера проще: один набор namespaces.
- Тесты на 100 фикстурах Transitional.
- Нет ложной поддержки, которая всё равно не работает.

### Отрицательные

- Пользователь со Strict-документом получает ошибку. Решение: понятное сообщение + предложение сохранить в Word как Transitional.
- README должен явно указать non-goal.

### Нейтральные

- Расширение до Strict возможно позже, без изменения модели.

## Альтернативы

- **A. Поддерживать оба.** Отклонено: удваивает тесты и ветвление, спрос неизвестен.
- **B. Игнорировать namespace.** Отклонено: конфликты с `mc:` и другими.

## Ссылки

- ISO/IEC 29500-1:2016.
- ECMA-376 Part 1.
- ADR-0013.
