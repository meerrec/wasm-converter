# ADR-0016: Политика ошибок парсинга DOCX

**Статус:** Принято
**Дата:** 05.10.2026
**Контекст:** проектирование парсера DOCX перед Спринтом 8
**Связанные ADR:** ADR-0013, ADR-0014, ADR-0015

## Контекст

DOCX из реального мира часто невалиден: отсутствующие rels, циклические `basedOn`, ссылки на несуществующие стили, битые части, неполный `document.xml`. Просмотрщик должен показывать как можно больше содержимого, а не падать на первой ошибке.

Возможные стратегии:

- **fail-fast** — первая ошибка прерывает парсинг;
- **recover-and-continue** — парсер накапливает warnings, отдаёт частичную модель;
- **гибрид** — фатальные ошибки прерывают, восстановимые — warning.

## Решение

### 1. Recover-and-continue

Парсер **всегда** возвращает `Document` и `Vec<ParseWarning>`, если возможно. Фатальные ошибки — только на уровне ZIP и отсутствия `document.xml`.

```rust
pub fn parse_docx(bytes: &[u8], limits: ZipLimits)
    -> Result<Document, ParseError>;

pub struct Document {
    // ...
    pub warnings: Vec<ParseWarning>,
}
```

### 2. Категории ошибок

#### Фатальные (возвращают `Err`)

| Ошибка | Когда |
|---|---|
| `ParseError::Zip(ZipError)` | Лимиты ZIP, path traversal, ratio |
| `ParseError::MissingContentTypes` | Нет `[Content_Types].xml` |
| `ParseError::MissingDocumentXml` | Нет `word/document.xml` |
| `ParseError::XmlFatal` | XML-парсер не может продолжить (невалидная кодировка, обрыв потока) |
| `ParseError::EncryptedDocument` | Файл зашифрован |
| `ParseError::MacroEnabledDocument` | `.docm` (non-goal v1) |
| `ParseError::LegacyFormat` | `.doc` (бинарный, non-goal v1) |

#### Восстановимые (warning + продолжаем)

| Warning | Когда | Поведение |
|---|---|---|
| `MissingRels` | Нет `.rels` для части | Часть без rels, ссылки игнорируются |
| `CyclicBasedOn` | Цикл в `basedOn` | Стиль применяется как есть |
| `MissingStyleRef` | `style_ref` не резолвится | Стиль игнорируется, используется default |
| `MissingAbstractNum` | `numId` → нет `abstractNum` | Список рендерится как обычный абзац |
| `MissingNumId` | `numId` не резолвится в `Num` | То же |
| `MissingPart` | Часть, на которую ссылаются, отсутствует | Ссылка игнорируется |
| `InvalidAttribute` | Атрибут невалиден (например, `w:val="abc"` для числа) | Дефолт, warning |
| `UnknownElement` | Неизвестный элемент в известном контексте | Сохраняется как `Unknown` |
| `SymlinkIgnored` | Часть — symlink | Игнорируется |
| `DeepNesting` | Глубина > 32 (для `basedOn`), > 16 (для `mc`) | Обрезка |
| `DuplicateStyleId` | Два стиля с одним ID | Первый побеждает |
| `OrphanBookmark` | `bookmarkEnd` без `Start` | Игнорируется |

### 3. `ParseWarning`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseWarning {
    pub kind: WarningKind,
    pub message: String,
    pub location: Option<WarningLocation>,
}

pub struct WarningLocation {
    pub part: String,       // "word/document.xml"
    pub node_id: Option<NodeId>,
    pub xml_path: Option<String>,  // "w:document/w:body/w:p[3]/w:pPr/w:pStyle"
}
```

### 4. Логирование

Warnings логируются через `tracing` на уровне `WARN`. Не влияют на возврат.

### 5. API для потребителя

```rust
impl Document {
    pub fn warnings(&self) -> &[ParseWarning];
    pub fn warnings_by_kind(&self, kind: WarningKind) -> Vec<&ParseWarning>;
    pub fn has_fatal_warnings(&self) -> bool; // true, если warnings > 100
}
```

### 6. Пороговые значения

- `warnings.len() > 1000` → возврат `Err(ParseError::TooManyWarnings)`. Защита от патологического входа.
- Отдельно: если `warnings.len() > 100`, UI показывает предупреждение пользователю.

## Последствия

### Положительные

- Пользователь видит максимум содержимого даже из битых файлов.
- Fuzz-friendly: парсер не падает на мусоре.
- Дифференциальный тест стабилен: warnings не влияют на exit code.

### Отрицательные

- Частичная модель может ввести пользователя в заблуждение.
- Warnings нужно отображать в UI (Спринт 11 — расширение).
- Код парсера сложнее: каждая ветка обрабатывает warning.

### Нейтральные

- Fatal-ошибок мало — их легко тестировать.

## Альтернативы

- **A. Fail-fast.** Отклонено: пользователь не увидит содержимое битого файла.
- **B. Silent recovery без warnings.** Отклонено: невозможно отладить.
- **C. Гибрид без порога.** Отклонено: патологический вход даст миллион warnings.

## Ссылки

- ADR-0013, ADR-0014, ADR-0015.
