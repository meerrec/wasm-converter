# План интеграционных тестов DOCX vs MS Word

**Дата:** 05.10.2026  
**Версия:** 1.0  
**Статус:** Черновик

---

## 📋 Обзор

Документ описывает **стратегию и план интеграционного тестирования** для DOCX-парсера и рендерера. Цель — убедиться, что рендер DOCX в `doc-converter` **совпадает с MS Word** с допустимой погрешностью.

---

## 🎯 Цели тестирования

|Цель|Метрика|Целевое значение|Спринт|
|----|-------|-----------------|-------|
|Число страниц|Разница с MS Word|±1|9|
|Визуальное сходство|SSIM (первая страница)|≥ 0.95|10|
|Число абзацев|Совпадение|100%|8|
|Число таблиц|Совпадение|100%|8|
|Число изображений|Совпадение|100%|10|
|Размеры страницы|Совпадение|100%|9|
|Шрифты|Совпадение Familien|100%|8|

---

## 🏗️ Структура тестов

```
test-fixtures/docx/
├── simple/           # Базовые документы
├── formatting/       # Форматирование текста
├── tables/           # Таблицы
├── images/           # Изображения
├── complex/          # Сложные документы
└── edge_cases/       # Крайние случаи
```

Каждый DOCX-файл сопровождается:
1. **`.docx`** — тестируемый файл
2. **`.json`** — эталонные метаданные (структура документа)
3. **`.pdf`** — эталонный рендер из MS Word (для визуальных тестов)
4. **`_page1.png`** — скриншот первой страницы из MS Word (для SSIM)

---

## 📊 Виды тестов

### 1. Структурные тесты (Structural Tests)

**Цель:** Проверить, что парсинг DOCX даёт правильную структуру.

#### Тестируемые аспекты
- Число абзацев
- Число таблиц
- Число изображений
- Число страниц
- Иерархия заголовков
- Стили текста

#### Пример теста
```rust
// tests/integration/docx_structural_test.rs

#[test]
fn test_paragraph_count_matches() {
    let fixture_path = "test-fixtures/docx/simple/one_paragraph.docx";
    
    // 1. Парсим через наш движок
    let bytes = std::fs::read(fixture_path).unwrap();
    let doc = doc_converter_docx::open(&bytes).unwrap();
    let our_paragraphs = doc.body.paragraph_count();
    
    // 2. Загружаем эталон из JSON
    let expected: serde_json::Value = serde_json::from_reader(
        std::fs::File::open(fixture_path.replace(".docx", ".json")).unwrap()
    ).unwrap();
    let expected_paragraphs = expected["metadata"]["expectedParagraphs"].as_u64().unwrap();
    
    // 3. Проверяем
    assert_eq!(our_paragraphs, expected_paragraphs as usize,
        "Paragraph count mismatch for {}", fixture_path);
}
```

#### matrices
|Фикстура|Число абзацев|Число таблиц|Приоритет|
|--------|---------------|------------|---------|
|simple/empty.docx|0|0|⭐⭐⭐|
|simple/one_paragraph.docx|1|0|⭐⭐⭐|
|simple/multiple_paragraphs_*.docx|5|0|⭐⭐⭐|
|formatting/bold.docx|1|0|⭐⭐⭐|
|formatting/heading_*.docx|1|0|⭐⭐⭐|
|tables/simple_2x2.docx|0|1|⭐⭐⭐|
|complex/text_and_table.docx|2|1|⭐⭐⭐|

---

### 2. Визуальные тесты (Visual Tests)

**Цель:** Проверить, что рендер DOCX визуально совпадает с MS Word.

#### Метрики
- **SSIM** (Structural Similarity Index) — сравнение изображений
  - 1.0 = идентично
  - 0.95+ = отлично
  - 0.90-0.95 = хорошо
  - < 0.90 = нужно исправлять

#### Инструменты
- `image` crate для работы с PNG
- `ssim-rs` для вычисления SSIM

#### Пример теста
```rust
// tests/integration/docx_visual_test.rs

use image::{DynamicImage, io::Reader as ImageReader};
use ssim::ssim;

#[test]
fn test_visual_match_ms_word_simple() {
    test_visual_match("simple/one_paragraph");
}

#[test]
fn test_visual_match_ms_word_formatting() {
    test_visual_match("formatting/bold");
}

fn test_visual_match(fixture_name: &str) {
    let base_path = format!("test-fixtures/docx/{}", fixture_name);
    
    // 1. Рендерим наш DOCX в PNG
    let docx_bytes = std::fs::read(format!("{} .docx", base_path)).unwrap();
    let doc = doc_converter_docx::open(&docx_bytes).unwrap();
    let png_bytes = doc_converter_docx::render_to_png(&doc, 0).unwrap(); // 0 = первая страница
    let our_img = ImageReader::new(std::io::Cursor::new(png_bytes))
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
        .into_luma8();
    
    // 2. Загружаем эталонный PNG из MS Word
    let expected_img = ImageReader::new(std::io::Cursor::new(
        std::fs::read(format!("{}_page1.png", base_path)).unwrap()
    ))
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
        .into_luma8();
    
    // 3. Вычисляем SSIM
    let score = ssim(&our_img, &expected_img).unwrap();
    assert!(score >= 0.95, 
        "SSIM too low for {}: {}", fixture_name, score);
}
```

#### matrices
|Фикстура|SSIM цель|Приоритет|
|--------|----------|---------|
|simple/one_paragraph.docx|≥ 0.95|⭐⭐⭐|
|formatting/bold.docx|≥ 0.95|⭐⭐⭐|
|formatting/heading_1.docx|≥ 0.95|⭐⭐⭐|
|tables/simple_2x2.docx|≥ 0.95|⭐⭐⭐|
|complex/text_and_table.docx|≥ 0.95|⭐⭐⭐|

---

### 3. Пагинационные тесты (Pagination Tests)

**Цель:** Проверить, что число страниц совпадает с MS Word (±1).

#### Пример теста
```rust
// tests/integration/docx_pagination_test.rs

#[test]
fn test_pagination_matches_ms_word() {
    // Тестируем все фикстуры из complex/
    let fixtures = [
        "complex/text_and_table",
        "complex/long_document_1",
        "complex/long_document_2",
        "complex/long_document_3",
    ];
    
    for fixture in fixtures {
        test_pagination(fixture);
    }
}

fn test_pagination(fixture_name: &str) {
    let base_path = format!("test-fixtures/docx/{}", fixture_name);
    
    // 1. Парсим и пагинируем через наш движок
    let docx_bytes = std::fs::read(format!("{} .docx", base_path)).unwrap();
    let doc = doc_converter_docx::open(&docx_bytes).unwrap();
    let pages = doc_converter_docx::paginate(&doc).unwrap();
    let our_pages = pages.len();
    
    // 2. Загружаем эталон из JSON
    let expected: serde_json::Value = serde_json::from_reader(
        std::fs::File::open(format!("{}.json", base_path)).unwrap()
    ).unwrap();
    let expected_pages = expected["metadata"]["expectedPages"].as_u64().unwrap();
    
    // 3. Проверяем
    let diff = (our_pages as i32) - (expected_pages as i32);
    assert!(diff.abs() <= 1, 
        "Page count mismatch for {}: our={}, expected={}", 
        fixture_name, our_pages, expected_pages);
}
```

#### matrices
|Фикстура|Ожидаемое число страниц|Приоритет|
|--------|------------------------|---------|
|complex/text_and_table.docx|1|⭐⭐⭐|
|complex/long_document_1.docx|≥2|⭐⭐⭐|
|complex/long_document_2.docx|≥3|⭐⭐⭐|
|complex/long_document_3.docx|≥4|⭐⭐⭐|

---

### 4. Тесты экспорта в PDF

**Цель:** Проверить, что экспорт DOCX в PDF работает корректно.

#### Проверяемые аспекты
- PDF открывается в Adobe Reader / Chrome / Firefox
- Текст читаемый и корректный
- Шрифты встроены
- Изображения отображаются
- Таблицы отображаются корректно

#### Пример теста
```rust
// tests/integration/docx_pdf_export_test.rs

#[test]
fn test_pdf_export_opens() {
    let fixtures = [
        "simple/one_paragraph",
        "formatting/bold",
        "tables/simple_2x2",
    ];
    
    for fixture in fixtures {
        test_pdf_opens(fixture);
    }
}

fn test_pdf_opens(fixture_name: &str) {
    let docx_bytes = std::fs::read(format!(
        "test-fixtures/docx/{}.docx", fixture_name
    )).unwrap();
    
    let doc = doc_converter_docx::open(&docx_bytes).unwrap();
    let pdf_bytes = doc_converter_docx::export_to_pdf(&doc).unwrap();
    
    // Проверяем, что PDF валидный
    assert!(pdf_bytes.len() > 100, "PDF too short for {}", fixture_name);
    
    // Проверяем сигнатуру PDF (%PDF-...)
    assert!(pdf_bytes.starts_with(b"%PDF"), 
        "Invalid PDF signature for {}", fixture_name);
    
    // Проверяем через qpdf (если доступен)
    if std::process::Command::new("qpdf")
        .arg("--check")
        .stdin(std::process::Stdio::piped())
        .status()
        .is_ok()
    {
        let mut child = std::process::Command::new("qpdf")
            .arg("--check")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn qpdf");
        
        std::process::Command::new("qpdf")
            .stdin(std::process::Stdio::from(pdf_bytes.as_slice()))
            .status()
            .expect("qpdf check failed for {}", fixture_name);
    }
}
```

---

### 5. Бенчмарки

**Цель:** Проверить производительность парсинга и рендеринга.

#### Метрики из ROADMAP
|Операция|Цель|Инструмент|
|--------|----|----------|
|`openDocx` 50 МБ|< 1.5 с (native)|criterion|
|Пагинация 100 страниц|< 500 мс|criterion|
|Рендер страницы DOCX A4|< 30 мс|criterion|

#### Пример бенчмарка
```rust
// benches/docx_bench.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_open_docx(c: &mut Criterion) {
    let docx_50mb = std::fs::read("test-fixtures/docx/edge_cases/huge_file.docx").unwrap();
    
    c.bench_function("open_docx_50mb", |b| {
        b.iter(|| {
            let _ = doc_converter_docx::open(black_box(&docx_50mb)).unwrap();
        })
    });
}

fn bench_pagination(c: &mut Criterion) {
    let docx_bytes = std::fs::read("test-fixtures/docx/complex/long_document_3.docx").unwrap();
    let doc = doc_converter_docx::open(&docx_bytes).unwrap();
    
    c.bench_function("pagination_100_pages", |b| {
        b.iter(|| {
            let _ = doc_converter_docx::paginate(black_box(&doc)).unwrap();
        })
    });
}

criterion_group!(benches, bench_open_docx, bench_pagination);
criterion_main!(benches);
```

---

## 📁 Генерация эталонных данных

### из MS Word

Для каждой фикстуры нужно сгенерировать эталонные данные:

```bash
# 1. Создать DOCX в MS Word с нужным содержимым
# 2. Сохранить как DOCX
# 3. Экспортировать в PDF
# 4. Сделать скриншот первой страницы

# Пример для генерации всех эталонов:
for file in test-fixtures/docx/**/*.docx; do
    # Конвертировать в PDF
    libreoffice --headless --convert-to pdf "$file"
    
    # Сделать скриншот первой страницы PDF
    # (используя инструменты вроде pdftoppm + convert)
    pdftoppm -f 1 -l 1 "${file/.docx/.pdf}" page1
    convert page1-1.ppm "${file/.docx/_page1.png}"
    rm page1-1.ppm
    
    # Обновить JSON с числом страниц
    page_count=$(pdfinfo "${file/.docx/.pdf}" | grep Pages | awk '{print $2}')
    jq --arg pc "$page_count" '.metadata.expectedPages = ($pc | tonumber)' \
        "${file/.docx/.json}" > "${file/.docx/.json}.tmp" && \
        mv "${file/.docx/.json}.tmp" "${file/.docx/.json}"
done
```

### Автоматизация через Python

```python
# scripts/generate_docx_oracles.py
import subprocess
import json
import os
from pathlib import Path

def generate_oracles(docx_dir: str):
    """Генерировать эталонные PDF и PNG для всех DOCX фикстур."""
    docx_dir = Path(docx_dir)
    
    for docx_file in docx_dir.glob("**/*.docx"):
        pdf_file = docx_file.with_suffix(".pdf")
        png_file = docx_file.with_name(docx_file.stem + "_page1.png")
        json_file = docx_file.with_suffix(".json")
        
        # Конвертировать в PDF через libreoffice
        subprocess.run([
            "libreoffice", "--headless", "--convert-to", "pdf",
            str(docx_file), "--outdir", str(docx_dir)
        ], check=True)
        
        # Сделать скриншот первой страницы
        subprocess.run([
            "pdftoppm", "-f", "1", "-l", "1", "-png", str(pdf_file), str(png_file)
        ], check=True)
        
        # Получить число страниц
        result = subprocess.run([
            "pdfinfo", str(pdf_file)
        ], capture_output=True, text=True)
        
        page_count = None
        for line in result.stdout.split('\n'):
            if line.startswith('Pages:'):
                page_count = int(line.split()[1])
                break
        
        # Обновить JSON
        if json_file.exists():
            with open(json_file) as f:
                data = json.load(f)
            data['metadata']['expectedPages'] = page_count
            with open(json_file, 'w') as f:
                json.dump(data, f, indent=2)
        else:
            # Создать новый JSON
            data = {
                "metadata": {
                    "name": docx_file.stem,
                    "expectedPages": page_count,
                    "expectedParagraphs": 0,
                    "expectedTables": 0,
                    "expectedImages": 0
                },
                "content": {
                    "paragraphs": [],
                    "tables": [],
                    "images": []
                }
            }
            with open(json_file, 'w') as f:
                json.dump(data, f, indent=2)

if __name__ == "__main__":
    generate_oracles("test-fixtures/docx")
```

---

## 🚀 Порядок реализации тестов

### Спринт 8 (Парсинг DOCX)

1. **Структурные тесты:**
   - Число абзацев
   - Число таблиц
   - Число изображений
   - Стили текста

2. **unit-тесты для парсинга:**
   - Тесты отдельных XML-элементов
   - Тесты разрешения стилей

### Спринт 9 (Раскладка DOCX)

1. **Пагинационные тесты:**
   - Число страниц ±1
   - Размеры страницы
   - Поля страницы

2. **Тесты рендеринга:**
   - Базовый рендер на OffscreenCanvas
   - Сравнение с эталоном (SSIM)

### Спринт 10 (Рендер + PDF DOCX)

1. **Визуальные тесты:**
   - SSIM ≥ 0.95 для всех фикстур
   - Тесты экспорта в PDF

2. **Интеграционные тесты:**
   - Сквозные тесты в браузере
   - Тесты в 3 браузерах (Chrome, Firefox, Safari)

---

## 📊 Отчётность

### Формат отчёта

Каждый запуск CI генерирует отчёт в формате:

```json
{
  "date": "2026-10-05",
  "total_tests": 45,
  "passed": 42,
  "failed": 3,
  "by_category": {
    "structural": {"passed": 15, "failed": 0},
    "visual": {"passed": 12, "failed": 2},
    "pagination": {"passed": 10, "failed": 1},
    "pdf_export": {"passed": 5, "failed": 0}
  },
  "failed_tests": [
    {"name": "visual/tables/simple_2x2", "ssim": 0.92, "expected": 0.95},
    {"name": "pagination/complex/long_document_3", "pages": 5, "expected": 4},
    {"name": "visual/formatting/heading_1", "ssim": 0.91, "expected": 0.95}
  ]
}
```

### Интеграция с CI

Добавить в `.github/workflows/ci.yml`:

```yaml
- name: Run DOCX integration tests
  run: |
    # Устанавливаем зависимости
    sudo apt-get install -y qpdf pdftoppm imagemagick
    
    # Запускаем структурные тесты
    cargo test --test docx_structural
    
    # Запускаем визуальные тесты
    cargo test --test docx_visual
    
    # Запускаем пагинационные тесты
    cargo test --test docx_pagination
    
    # Запускаем тесты экспорта в PDF
    cargo test --test docx_pdf_export
    
    # Запускаем бенчмарки
    cargo bench --bench docx_bench
```

---

## 📌 Чек-лист

- [x] Создать фикстуры DOCX (15 файлов)
- [ ] Сгенерировать эталонные PDF и PNG для всех фикстур
- [ ] Добавить metadata в JSON-файлы (expectedPages, etc.)
- [ ] Реализовать структурные тесты
- [ ] Реализовать визуальные тесты (SSIM)
- [ ] Реализовать пагинационные тесты
- [ ] Реализовать тесты экспорта в PDF
- [ ] Добавить бенчмарки
- [ ] Интегрировать тесты в CI
- [ ] Настроить отчётность

---

## 🔗 Полезные ссылки

- [SSIM на Wikipedia](https://en.wikipedia.org/wiki/Structural_similarity)
- [Crates: image](https://github.com/image-rs/image)
- [Crates: ssim-rs](https://crates.io/crates/ssim)
- [LibreOffice (конвертация в PDF)](https://www.libreoffice.org/)
- [qpdf (валидация PDF)](https://qpdf.sourceforge.io/)
- [ROADMAP: doc-converter](../../ROADMAP.md)
- [ADR 0003: Алгоритмы раскладки DOCX](../adr/0003-docx-layout-and-text-wrapping.md)
