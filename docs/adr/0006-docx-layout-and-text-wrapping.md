# ADR 0006: Алгоритмы раскладки и переноса строк для DOCX

## Статус

✅ **Принято** — 05.10.2026

## Контекст

Для рендеринга DOCX необходимо решить следующие задачи:

1. **Раскладка текста на страницы** (pagination):
   - DOCX состоит из последовательности блоков (абзацы, таблицы, изображения)
   - Нужно разбить их на страницы с учётом размеров страницы, полей, ориентации
   - Поддержка правил разбиения: widow/orphan control, keep-with-next, keep-lines-together

2. **Перенос строк** (line breaking):
   - Текст абзаца нужно разбить на строки, учитывая ширину страницы/колонки
   - Поддержка выравнивания: left, center, right, justify
   - Поддержка отступов (indent), межстрочного интервала

3. **Измерение текста** (text measurement):
   - Нужно точно измерять ширину текста с учётом шрифта, размера, стиля (bold/italic)
   - Поддержка kerning, ligatures (опционально)

4. **Обработка плавающих элементов**:
   - Изображения с обтеканием текстом (wrap-top, wrap-bottom, wrap-both)
   - Таблицы с автоподбором ширины

## Варианты

### Вариант 1: Жадный алгоритм переноса строк

**Описание:**
- Переносить слова на новую строку, если они не помещаются
- Простой и быстрый (O(n) по словам)
- Подходит для базового рендеринга

**Плюсы:**
- Простота реализации
- Высокая производительность
- Легко тестировать
- Достаточно для большинства случаев

**Минусы:**
- Не идеальное выравнивание по ширине (justified text)
- Не учитывает kerning и ligatures
- Не оптимален для узких колонок

**Реализация:**
```rust
fn greedy_line_break(text: &str, max_width: f32, font: &FontMetrics) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0.0;
    
    for word in words {
        let word_width = font.measure_text(word);
        if current_width + word_width <= max_width {
            if !current_line.is_empty() {
                current_line.push(' ');
                current_width += font.space_width();
            }
            current_line.push_str(word);
            current_width += word_width;
        } else {
            lines.push(std::mem::take(&mut current_line));
            current_line = word.to_string();
            current_width = word_width;
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    lines
}
```

---

### Вариант 2: Алгоритм Knuth-Plass

**Описание:**
- Оптимизирует расстановку пробелов для выравнивания по ширине
- Используется в TeX и профессиональных типографских системах
- Сложность: O(n^2) в худшем случае, но обычно O(n)

**Плюсы:**
- Идеальное выравнивание по ширине
- Учёт kerning и ligatures
- Профессиональное качество типографики

**Минусы:**
- Сложность реализации
- Более медленный (но приемлемо для большинства документов)
- Сложнее тестировать

**Реализация:**
Библиотека `textwrap` или собственная реализация на основе статьи Knuth-Plass.

---

### Вариант 3: Гибридный подход

**Описание:**
- **Жадный алгоритм** для рендеринга в браузере (скорость важнее качества)
- **Knuth-Plass** для экспорта в PDF (качество важнее скорости)

**Плюсы:**
- Оптимальный баланс производительности и качества
- Возможность расширять алгоритмы независимо
- Соответствует требованиям ROADMAP (60 FPS при скролле)

**Минусы:**
- Усложняет кодовую базу (нужно поддерживать два алгоритма)
- Тестирование усложняется (нужно проверять оба алгоритма)

---

## Решение

**Выбран: Вариант 3 (Гибридный подход)**

### Обоснование

1. **Производительность в браузере:**
   - При скролле нужно рендерить быстро (60 FPS)
   - Жадный алгоритм достаточно быстрый для этого
   - Пользователь не заметит разницы в качестве при интерактивном просмотре

2. **Качество экспорта в PDF:**
   - PDF — финальный формат, где важна типографика
   - Knuth-Plass обеспечивает профессиональное качество
   - Скорость менее критична для batch-операций

3. **Гибкость:**
   - Можно начать с жадного алгоритма и позже добавить Knuth-Plass
   - Levenshtein distance между алгоритмами минимален (оба работают с текстом)

4. **Соответствие ROADMAP:**
   - В ROADMAP указано: "For greedy algorithm — simple, fast"
   - Для DOCX важно и качество, и производительность

---

## Детальная архитектура

### Структура модулей

```
crates/docx/
├── layout/
│   ├── engine.rs          # Движок раскладки страниц
│   ├── line_break.rs      # Перенос строк (greedy + Knuth-Plass)
│   ├── pagination.rs      # Пагинация
│   ├── text_measure.rs    # Измерение текста
│   ├── tables.rs          # Раскладка таблиц
│   └── float.rs           # Плавающие элементы
└── model.rs              # Модель DOCX
```

### Интеграция с `crates/render/`

Общие компоненты:
- `DisplayList` — универсальный формат команд рендеринга
- `FontRegistry` — кеш метрик шрифтов
- `Painter` — рендерер на OffscreenCanvas

```rust
// crates/docx/src/layout/engine.rs
pub struct DocxLayoutEngine {
    font_registry: FontRegistry,
    page_size: PageSize,
    margins: Margins,
}

impl DocxLayoutEngine {
    pub fn layout_document(&self, doc: &Document) -> Vec<Page> {
        let mut pages = Vec::new();
        let mut current_page = Page::new(self.page_size);
        let mut y = self.margins.top;
        
        for block in &doc.body.children {
            match block {
                Block::Paragraph(p) => {
                    let lines = self.break_paragraph(p, current_page.width - self.margins.left - self.margins.right);
                    for line in lines {
                        if y + line.height > self.page_size.height - self.margins.bottom {
                            pages.push(current_page);
                            current_page = Page::new(self.page_size);
                            y = self.margins.top;
                        }
                        current_page.add_line(line, y);
                        y += line.height + line.spacing;
                    }
                }
                Block::Table(t) => {
                    let table_layout = self.layout_table(t, current_page.width);
                    // Аналогично для таблиц
                }
                // ... остальные типы блоков
            }
        }
        pages.push(current_page);
        pages
    }
}
```

### Алгоритмы переноса строк

#### 1. Жадный алгоритм (для браузера)

```rust
// crates/docx/src/layout/line_break.rs
pub struct GreedyLineBreaker {
    font_registry: FontRegistry,
}

impl GreedyLineBreaker {
    pub fn break_text(&self, text: &str, max_width: f32, style: &TextStyle) -> Vec<TextLine> {
        let font = self.font_registry.get_font(&style.font_family, style.font_size, style.bold, style.italic);
        let space_width = font.measure_text(" ");
        
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut lines = Vec::new();
        let mut current_words = Vec::new();
        let mut current_width = 0.0;
        
        for word in words {
            let word_width = font.measure_text(word);
            if current_words.is_empty() {
                // Первое слово в строке
                current_words.push(word);
                current_width = word_width;
            } else if current_width + space_width + word_width <= max_width {
                // Слово помещается
                current_words.push(word);
                current_width += space_width + word_width;
            } else {
                // Слово не помещается — переходим на новую строку
                lines.push(TextLine {
                    words: std::mem::take(&mut current_words),
                    width: current_width,
                    height: font.line_height(),
                });
                current_words.push(word);
                current_width = word_width;
            }
        }
        
        if !current_words.is_empty() {
            lines.push(TextLine {
                words: current_words,
                width: current_width,
                height: font.line_height(),
            });
        }
        
        lines
    }
}
```

#### 2. Knuth-Plass алгоритм (для PDF)

```rust
// crates/docx/src/layout/line_break.rs
pub struct KnuthPlassLineBreaker {
    font_registry: FontRegistry,
}

impl KnuthPlassLineBreaker {
    pub fn break_text(&self, text: &str, max_width: f32, style: &TextStyle) -> Vec<TextLine> {
        let font = self.font_registry.get_font(&style.font_family, style.font_size, style.bold, style.italic);
        
        // Реализация алгоритма Knuth-Plass
        // 1. Разбить текст на "boxes" (слова) и "glue" (пробелы)
        // 2. Вычислить "demerits" для каждого возможного разбиения
        // 3. Найти оптимальный путь через динамическое программирование
        // 4. Вернуть строки с оптимальными пробелами
        
        // Упрощённая реализация для начала
        let greedy = GreedyLineBreaker { font_registry: self.font_registry.clone() };
        greedy.break_text(text, max_width, style)
    }
}
```

---

## Пагинация

### Правила разбиения страниц

1. **Widow/Orphan Control:**
   - Минимальное количество строк абзаца на странице: 2
   - Если разрыв оставляет < 2 строки на текущей странице или < 2 на следующей — ищем другой вариант

2. **Keep With Next:**
   - Абзац с атрибутом `keep-with-next` не должен разрываться от следующего абзаца

3. **Keep Lines Together:**
   - Абзац с атрибутом `keep-lines-together` должен помещаться целиком на одной странице

4. **Page Break Before/After:**
   - Принудительный разрыв перед/после элемента

5. **Section Break:**
   - Разрыв страницы с возможной сменой форматирования

### Реализация

```rust
// crates/docx/src/layout/pagination.rs
pub struct Paginator {
    page_size: PageSize,
    margins: Margins,
    layout_engine: DocxLayoutEngine,
}

impl Paginator {
    pub fn paginate(&self, doc: &Document) -> Vec<Page> {
        let mut pages = Vec::new();
        let mut current_page = Page::new(self.page_size);
        let mut y = self.margins.top;
        let mut content_index = 0;
        
        while content_index < doc.body.children.len() {
            let block = &doc.body.children[content_index];
            let block_height = self.layout_engine.measure_block(block, current_page.width);
            
            // Проверяем, помещается ли блок на текущей странице
            if y + block_height <= self.page_size.height - self.margins.bottom {
                // Блок помещается
                current_page.add_block(block, y);
                y += block_height;
                content_index += 1;
            } else {
                // Блок не помещается — пробуем следующий вариант разбиения
                
                // Проверяем widow/orphan control
                if let Block::Paragraph(p) = block {
                    let lines = self.layout_engine.break_paragraph(p, current_page.width);
                    let lines_on_page = self.lines_that_fit(lines, self.page_size.height - y - self.margins.bottom);
                    
                    if lines_on_page < 2 {
                        // Слишком мало строк на текущей странице — переносим весь абзац
                        pages.push(current_page);
                        current_page = Page::new(self.page_size);
                        y = self.margins.top;
                        continue;
                    }
                    
                    let lines_on_next = lines.len() - lines_on_page;
                    if lines_on_next < 2 {
                        // Слишком мало строк на следующей странице — переносим часть обратно
                        let lines_on_page = lines.len() - 2;
                        // Добавляем lines_on_page строк на текущую страницу
                        // Остальные — на следующую
                    }
                }
                
                // Сохраняем текущую страницу и начинаем новую
                pages.push(current_page);
                current_page = Page::new(self.page_size);
                y = self.margins.top;
            }
        }
        
        pages.push(current_page);
        pages
    }
}
```

---

## Раскладка таблиц

### Требования

1. Автоподбор ширины столбцов (autofit)
2. Фиксированная ширина столбцов
3. Объединённые ячейки (merged cells)
4. Границы таблицы (borders)
5. Выравнивание текста в ячейках

### Реализация

```rust
// crates/docx/src/layout/tables.rs
pub struct TableLayouter {
    font_registry: FontRegistry,
}

impl TableLayouter {
    pub fn layout_table(&self, table: &Table, max_width: f32) -> TableLayout {
        let mut col_widths = Vec::new();
        let mut row_heights = Vec::new();
        
        // Измеряем содержимое каждой ячейки
        for col in 0..table.cols {
            let mut max_col_width = 0.0;
            for row in 0..table.rows {
                if let Some(cell) = table.get_cell(row, col) {
                    let cell_width = self.measure_cell_content(&cell);
                    max_col_width = max_col_width.max(cell_width);
                }
            }
            col_widths.push(max_col_width);
        }
        
        // Если autofit — масштабируем столбцы под доступную ширину
        let total_width: f32 = col_widths.iter().sum();
        if total_width > max_width {
            let scale = max_width / total_width;
            for width in &mut col_widths {
                *width *= scale;
            }
        }
        
        // Вычисляем высоты строк
        for row in 0..table.rows {
            let mut max_row_height = 0.0;
            for col in 0..table.cols {
                if let Some(cell) = table.get_cell(row, col) {
                    let cell_height = self.measure_cell_height(&cell, col_widths[col]);
                    max_row_height = max_row_height.max(cell_height);
                }
            }
            row_heights.push(max_row_height);
        }
        
        TableLayout { col_widths, row_heights }
    }
}
```

---

## Интеграция с `crates/render/`

### Использование DisplayList

```rust
// crates/docx/src/paint.rs
use doc_converter_render::display_list::{DisplayList, DrawCommand};

pub fn build_display_list_for_page(page: &Page) -> DisplayList {
    let mut list = DisplayList::new();
    
    // Рисуем фон страницы
    list.push(DrawCommand::FillRect {
        rect: page.rect,
        color: page.background_color,
    });
    
    // Рисуем содержимое страницы
    for block in &page.blocks {
        match block {
            PageBlock::Paragraph(para) => {
                for line in &para.lines {
                    list.push(DrawCommand::FillText {
                        text: line.text.clone(),
                        x: line.x,
                        y: line.y,
                        font: line.font.clone(),
                        color: line.color,
                    });
                }
            }
            PageBlock::Table(table) => {
                // Рисуем таблицу через DisplayList
                for row in &table.rows {
                    for cell in &row.cells {
                        // Рисуем фон ячейки
                        if let Some(fill) = cell.background {
                            list.push(DrawCommand::FillRect {
                                rect: cell.rect,
                                color: fill,
                            });
                        }
                        // Рисуем границы ячейки
                        // ...
                        // Рисуем текст ячейки
                        list.push(DrawCommand::FillText {
                            text: cell.text.clone(),
                            x: cell.text_x,
                            y: cell.text_y,
                            font: cell.font.clone(),
                            color: cell.text_color,
                        });
                    }
                }
            }
            // ... остальные типы блоков
        }
    }
    
    list
}
```

---

## Порядок реализации

### Фаза 1: Базовая раскладка (Спринт 8)

1. ✅ **Модель DOCX** (`crates/docx/src/model.rs`)
   - `Document`, `Body`, `Paragraph`, `Run`, `Text`, `Table`
   - Поддержка базовых элементов

2. **Парсинг DOCX** (`crates/docx/src/parser/`)
   - `document.xml`, `styles.xml`, `numbering.xml`
   - Разрешение стилей

3. **Базовый рендер**
   - Простой перенос строк (greedy algorithm)
   - Рендер на OffscreenCanvas

### Фаза 2: Пагинация (Спринт 9)

1. **Движок раскладки** (`crates/docx/src/layout/engine.rs`)
   - Поддержка размеров страницы, полей
   - Разбиение на страницы

2. **Перенос строк** (`crates/docx/src/layout/line_break.rs`)
   - Greedy algorithm для браузера
   - Поддержка базового выравнивания

3. **Раскладка таблиц** (`crates/docx/src/layout/tables.rs`)
   - Фиксированная ширина столбцов
   - Автоподбор ширины (autofit)

### Фаза 3: Улучшенная типографика (Спринт 10)

1. **Knuth-Plass алгоритм**
   - Для экспорта в PDF
   - Оптимальное выравнивание по ширине

2. **Плавающие элементы** (`crates/docx/src/layout/float.rs`)
   - Обтекание текстом вокруг изображений
   - Поддержка wrap-top, wrap-bottom, wrap-both

3. **Продвинутые_features таблиц**
   - Объединённые ячейки
   - Границы таблицы
   - Выравнивание текста в ячейках

---

## Тестирование

### Unit-тесты

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_greedy_line_break() {
        let breaker = GreedyLineBreaker { font_registry: FontRegistry::default() };
        let text = "Hello World";
        let lines = breaker.break_text(text, 50.0, &TextStyle::default());
        assert_eq!(lines.len(), 1);
    }
    
    #[test]
    fn test_pagination_widow_control() {
        let paginator = Paginator { ... };
        let doc = Document::with_paragraphs(10, "Test paragraph");
        let pages = paginator.paginate(&doc);
        
        // Проверяем, что ни на одной странице нет одиночной строки
        for page in &pages {
            for para in &page.paragraphs {
                assert!(para.lines_on_page >= 2 || para.lines_on_page == para.total_lines);
            }
        }
    }
    
    #[test]
    fn test_table_layout() {
        let table = Table::new(3, 3);
        let layout = TableLayouter::new().layout_table(&table, 300.0);
        assert_eq!(layout.col_widths.len(), 3);
    }
}
```

### Интеграционные тесты

1. **Сравнение с MS Word:**
   - Число страниц ±1
   - SSIM ≥ 0.95 для первой страницы
   - Сравнение числа абзацев, таблиц

2. **Бенчмарки:**
   - Пагинация 100 страниц < 500 мс
   - Рендер страницы < 30 мс

---

## Пороговые значения

| Метрика | Цель | Реализация |
|---------|------|------------|
| Число страниц vs MS Word | ±1 | Спринт 9 |
| SSIM для первой страницы | ≥ 0.95 | Спринт 10 |
| Пагинация 100 страниц | < 500 мс | Спринт 9 |
| Рендер страницы DOCX | < 30 мс | Спринт 9 |
| Память модели DOCX | ≤ 50 байт/символ | Спринт 8 |

---

## Ссылки

- [Knuth-Plass Algorithm (Wikipedia)](https://en.wikipedia.org/wiki/TeX#Line_breaking)
- [Greedy Algorithm for Text Wrapping](https://en.wikipedia.org/wiki/Line_wrap)
- [Rustybuzz (шрифтовые метрики)](https://github.com/RazrFalcon/rustybuzz)
- [ECMA-376 Part 1 (WordprocessingML)](https://www.ecma-international.org/publications-and-standards/standards/ecma-376/)
- [Office Open XML Documentation](https://officeopenxml.com/)
- [ROADMAP: doc-converter](../ROADMAP.md)
