# ADR-0014: mc:AlternateContent

**Статус:** Принято
**Дата:** 05.10.2026
**Контекст:** проектирование парсера DOCX перед Спринтом 8
**Связанные ADR:** ADR-0013, ADR-0016

## Контекст

Word оборачивает фигуры, текстовые поля, SmartArt и некоторые таблицы в `mc:AlternateContent`:

```xml
<mc:AlternateContent>
  <mc:Choice Requires="wps">...</mc:Choice>
  <mc:Fallback>...</mc:Fallback>
</mc:AlternateContent>
```

`mc:Choice` содержит современное представление (DrawingML, WordprocessingShape), `mc:Fallback` — legacy (VML). Без обработки парсер либо пропускает содержимое, либо дублирует его.

## Решение

### 1. Берём `mc:Choice`, игнорируем `mc:Fallback`

`resolve_alternate_content()`:

1. Перебираем `mc:Choice` в порядке появления.
2. Берём первую, чьи `Requires` поддерживаются (пустой список = поддерживается всегда).
3. Если ни одна не поддержана — сохраняем весь блок как `Unknown` с исходным XML.
4. `mc:Fallback` не парсим никогда.

### 2. Поддерживаемые `Requires`

| Namespace | Поддержка |
|---|---|
| `wps` (WordprocessingShape) | Да |
| `wpg` (WordprocessingGroup) | Да |
| `wpc` (WordprocessingCanvas) | Да |
| `w14` (Word 2010) | Да |
| `wp14` (Word 2010 Drawing) | Да |
| `v` (VML) | Нет (non-goal) |
| `o` (Office) | Нет (non-goal) |

### 3. Что сохраняется как `Unknown`

```rust
BlockItem::Unknown { id: NodeId, xml: String }
Inline::Unknown { id: NodeId, xml: String }
```

XML сохраняется для отладки и будущей обработки. В модель не входит.

### 4. Вложенность

`mc:AlternateContent` может быть вложен. Глубина ≤ 16. При превышении — warning, блок сохраняется как `Unknown` без рекурсии.

## Последствия

### Положительные

- Часть DOCX парсится корректно (фигуры, текстовые поля).
- Нет дублирования Choice + Fallback.
- VML не тянет legacy-код.

### Отрицательные

- Документы со сложным SmartArt теряют часть содержимого.
- Отладка `Unknown` требует ручного просмотра XML.

### Нейтральные

- VML — non-goal v1, но XML сохраняется.

## Альтернативы

- **A. Берём Fallback.** Отклонено: VML — legacy, non-goal.
- **B. Обе ветки.** Отклонено: дублирование.
- **C. Игнорировать весь блок.** Отклонено: потеря фигур и текстовых полей.

## Ссылки

- ECMA-376 Part 1 §10.1.2 (Markup Compatibility).
- ISO/IEC 29500-3 (Markup Compatibility and Extensibility).
- ADR-0013, ADR-0016.
