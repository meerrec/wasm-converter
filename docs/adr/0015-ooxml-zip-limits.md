# ADR-0015: Лимиты ZIP для OOXML

**Статус:** Принято
**Дата:** 05.10.2026
**Контекст:** безопасность парсинга OOXML перед Спринтом 8
**Связанные ADR:** ADR-0016

## Контекст

DOCX и XLSX — это ZIP-архивы (OPC-пакеты). Просмотрщик открывает файлы из недоверенных источников: почта, чаты, публичные ссылки. Модель угроз — просмотрщик чужих документов.

Векторы атаки:

- **zip bomb** — маленький файл, огромный uncompressed size;
- **zip slip** — имена частей с `..`, ведущие за пределы архива;
- **ratio bomb** — экстремальное соотношение сжатия;
- **OOM** — суммарный uncompressed size, больше доступной памяти;
- **symlink** — части, помеченные как symlink.

В XLSX-пути лимиты уже есть (32 МиБ на картинку, 128 МиБ на книгу). Для DOCX лимитов нет.

## Решение

### 1. Лимиты по умолчанию

| Параметр | Значение | Обоснование |
|---|---|---|
| Per-part uncompressed | 64 МиБ | Документ с 100k абзацев — ~30 МиБ XML; 64 с запасом |
| Per-archive uncompressed | 256 МиБ | 4× per-part; покрывает вложенные таблицы и изображения |
| Compression ratio | ≤ 200:1 | Типичный DOCX — 5–20:1; 200 — аномалия |
| Количество частей | ≤ 4096 | Типичный DOCX — 20–50 частей |
| Длина имени части | ≤ 255 байт | Стандарт файловых систем |
| Глубина пути | ≤ 16 | OPC не использует глубокую вложенность |

### 2. Path traversal

- Имена частей нормализуются.
- `..` в любом сегменте → ошибка `ZipError::InvalidPath`.
- Абсолютные пути (`/...`) → ошибка.
- Backslash (`\`) → ошибка.
- Null byte (`\0`) → ошибка.

### 3. Symlink

Части с атрибутом symlink (external attributes) игнорируются. Ошибки нет, warning `ParseWarning::SymlinkIgnored`.

### 4. Ratio check

Проверяется при открытии каждой части:

```rust
let ratio = uncompressed_size / compressed_size;
if ratio > 200 { return Err(ZipError::RatioExceeded); }
```

Проверка выполняется до распаковки, по заголовкам центрального каталога.

### 5. Конфигурация

```rust
pub struct ZipLimits {
    pub per_part_uncompressed: u64,   // default 64 МиБ
    pub per_archive_uncompressed: u64, // default 256 МиБ
    pub max_ratio: u32,                // default 200
    pub max_parts: u32,                // default 4096
    pub max_name_len: u32,             // default 255
    pub max_depth: u32,                // default 16
}

impl Default for ZipLimits { ... }
```

API:

```rust
OoxmlArchive::open_with_limits(path, ZipLimits::default())?;
OoxmlArchive::open_with_limits(path, ZipLimits::unlimited())?; // для тестов
```

### 6. Ошибки

```rust
enum ZipError {
    InvalidPath,
    RatioExceeded,
    PartTooLarge { name: String, size: u64 },
    ArchiveTooLarge { size: u64 },
    TooManyParts { count: u32 },
    NameTooLong { name: String },
    PathTooDeep { name: String },
    SymlinkIgnored { name: String },   // warning, не error
}
```

### 7. Область применения

Лимиты применяются ко **всем** OOXML-архивам: DOCX, XLSX, DOCM (если бы поддерживался), POTX и т.д. Не только к DOCX.

## Последствия

### Положительные

- Закрыт вектор zip bomb / zip slip.
- Память ограничена сверху.
- Лимиты конфигурируемы для тестов.

### Отрицательные

- Легитимные документы с > 64 МиБ одной части будут отклонены. Таких на практике не бывает (documents.xml редко > 30 МиБ).
- Пользователь не может открыть повреждённый файл с подозрительными заголовками.

### Нейтральные

- Лимиты можно поднять через API для внутренних сценариев.

## Альтернативы

- **A. Без лимитов.** Отклонено: OOM и zip bomb.
- **B. Лимит только на архив.** Отклонено: не покрывает zip bomb внутри одной части.
- **C. Ratio без per-part.** Отклонено: ratio можно обойти, распределив данные.

## Ссылки

- OPC (ECMA-376 Part 2).
- ZIP APPNOTE 6.3.10.
- ADR-0016.
