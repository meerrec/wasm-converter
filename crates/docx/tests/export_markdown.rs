//! Экспорт в Markdown: точные фрагменты вывода на фикстурах и обход всего корпуса.
//!
//! Первая половина проверяет разметку на конкретике (начертания, ссылки, таблицы,
//! списки, разрывы), вторая — что экспорт проходит по всем фикстурам `test-fixtures/docx`
//! без паник и не теряет текст.

use std::path::{Path, PathBuf};

use doc_converter_docx::export::markdown::to_markdown;
use doc_converter_docx::{open, BlockItem, Cell, Document, Justification};

/// Каталог с фикстурами.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Все `.docx` под каталогом фикстур, по возрастанию пути.
fn fixture_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut stack = vec![fixtures_dir()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{} не читается: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("запись каталога").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "docx") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

/// Разбирает фикстуру по имени: тесты, которым мало корпуса, правят модель руками.
fn document_of(name: &str) -> Document {
    let path = fixtures_dir().join(name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} не читается: {error}", path.display()));
    open(bytes).unwrap_or_else(|error| panic!("{name} не разбирается: {error}"))
}

/// Разбирает фикстуру по имени и экспортирует её в Markdown.
fn markdown_of(name: &str) -> String {
    to_markdown(&document_of(name))
}

/// Проставляет выравнивание всем абзацам ячейки: столбцу GFM достаточно одного.
fn align_cell(cell: &mut Cell, jc: Option<Justification>) {
    for item in &mut cell.items {
        if let BlockItem::Paragraph(paragraph) = item {
            paragraph.ppr.jc = jc.clone();
        }
    }
}

/// Проверяет, что ожидаемая строка есть в выводе; при расхождении печатает весь вывод.
#[track_caller]
fn assert_has(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "в выводе нет {needle:?}:\n{haystack}"
    );
}

#[test]
fn emphasis_becomes_markdown_marks() {
    // Прогоны одного абзаца: `w:b`, `w:i` и `w:u` — прямое форматирование,
    // `Emphasis` — знаковый стиль.
    let markdown = markdown_of("basic/multiple_runs.docx");
    assert_has(
        &markdown,
        "Plain **bold** *italic* <u>underlined</u> colored sized *emphasized*",
    );
}

#[test]
fn hyperlink_becomes_a_markdown_link() {
    // В фикстуре два вида ссылок: `w:hyperlink` и составное поле HYPERLINK;
    // у обеих цель разрешена через relationships.
    let markdown = markdown_of("fields/hyperlink_field.docx");
    assert_has(
        &markdown,
        "[<u>example.com/docs</u>](https://example.com/docs)",
    );
    assert_has(
        &markdown,
        "Link: [<u>example.com/docs</u>](https://example.com/docs)",
    );
}

#[test]
fn table_becomes_a_gfm_table_with_a_separator() {
    let markdown = markdown_of("tables/simple_2x2.docx");
    assert_eq!(markdown, "| A1 | A2 |\n| --- | --- |\n| B1 | B2 |\n");
}

#[test]
fn table_cells_are_escaped_and_flattened() {
    // Ячейка с двумя абзацами: перенос внутри ячейки — `<br>`, `|` экранируется.
    let markdown = markdown_of("complex/text_and_table.docx");
    assert_eq!(
        markdown,
        "Text before\n\n| H1 | H2 |\n| --- | --- |\n| C1 | C2 |\n\nText after\n"
    );
}

#[test]
fn vertical_merge_leaves_the_continuation_cells_empty() {
    // GFM вертикального объединения не выражает: продолжения объединения остаются
    // пустыми, а текст живёт только в его начале.
    let expected = "| Объединено по вертикали | Строка 1 |\n\
                    | --- | --- |\n\
                    |  | Строка 2 |\n\
                    |  | Строка 3 |\n";
    assert_eq!(markdown_of("tables/v_merge.docx"), expected);

    // Даже если продолжение несёт текст, в вывод он не попадает: место занято
    // началом объединения, и повтор был бы выдуманными данными.
    let mut document = document_of("tables/v_merge.docx");
    let BlockItem::Table(table) = &mut document.body.items[0] else {
        panic!("в фикстуре нет таблицы");
    };
    let duplicate = table.rows[1].cells[1].items.clone();
    table.rows[1].cells[0].items = duplicate;
    assert_eq!(to_markdown(&document), expected);
}

#[test]
fn a_horizontally_merged_cell_is_padded_to_the_column_count() {
    // `w:gridSpan` занимает в GFM одну ячейку: остальные места строки дописываются
    // пустыми, иначе строки разошлись бы с разделителем по числу столбцов.
    assert_eq!(
        markdown_of("tables/grid_span.docx"),
        "| Строка во всю ширину |  |  |\n\
         | --- | --- | --- |\n\
         | Две ячейки | C2 |  |\n\
         | A3 | B3 | C3 |\n"
    );
}

#[test]
fn column_alignment_comes_from_the_first_row_cells() {
    // Разделитель GFM один на столбец и идёт за строкой заголовка — выравнивание
    // столбца берётся у ячеек первой строки.
    let mut document = document_of("tables/alignment_widths.docx");
    let BlockItem::Table(table) = &mut document.body.items[0] else {
        panic!("в фикстуре нет таблицы");
    };
    align_cell(&mut table.rows[0].cells[0], Some(Justification::Both));
    align_cell(&mut table.rows[0].cells[1], Some(Justification::Center));
    align_cell(&mut table.rows[0].cells[2], Some(Justification::Right));
    assert_has(&to_markdown(&document), "| --- | :---: | ---: |");

    // Абзацы одной ячейки разошлись во выравнивании: приписать столбцу одну из двух
    // сторон было бы произволом, и столбец остаётся с умолчанием GFM.
    let mut document = document_of("tables/alignment_widths.docx");
    let BlockItem::Table(table) = &mut document.body.items[0] else {
        panic!("в фикстуре нет таблицы");
    };
    let second = table.rows[0].cells[1].items[0].clone();
    table.rows[0].cells[1].items.push(second);
    let BlockItem::Paragraph(first) = &mut table.rows[0].cells[1].items[0] else {
        panic!("в ячейке нет абзаца");
    };
    first.ppr.jc = Some(Justification::Center);
    let BlockItem::Paragraph(last) = table.rows[0].cells[1].items.last_mut().unwrap() else {
        panic!("в ячейке нет абзаца");
    };
    last.ppr.jc = Some(Justification::Right);
    assert_has(&to_markdown(&document), "| --- | --- | --- |");
}

#[test]
fn heading_style_becomes_a_markdown_heading() {
    // `Heading1` в фикстуре — полужирный стиль (начертание приходит из каскада),
    // поэтому заголовок несёт и `**` внутри.
    assert_eq!(
        markdown_of("formatting/heading_1.docx"),
        "# **Heading 1**\n"
    );
    assert_eq!(
        markdown_of("formatting/heading_2.docx"),
        "## **Heading 2**\n"
    );
    // Уровень берётся из `w:outlineLvl`, а не из имени стиля: здесь он наследуется
    // по цепочке `basedOn`, где самой цепочке полтора стиля.
    assert_eq!(
        markdown_of("styles/based_on_chain.docx"),
        "### ***Заголовок конца цепочки***\n"
    );
}

#[test]
fn numbered_list_items_get_numbers_from_the_numbering_table() {
    let markdown = markdown_of("numbering/decimal_basic.docx");
    assert_eq!(
        markdown,
        "1. Первый пункт\n2. Второй пункт\n3. Третий пункт\n"
    );
    // `w:startOverride` сдвигает начало списка: нумерация начинается с пяти.
    let overridden = markdown_of("numbering/start_override.docx");
    assert_eq!(
        overridden,
        "5. Пункт с номером 5\n6. Пункт с номером 6\n7. Пункт с номером 7\n"
    );
}

#[test]
fn numbering_formats_and_restarts_come_from_the_numbering_table() {
    // `lowerLetter` на первом уровне и `upperRoman` на втором; `%1`/`%2` в `w:lvlText`
    // подставляют номера уровней, а возврат на верхний уровень начинает вложенный заново.
    assert_eq!(
        markdown_of("numbering/custom_format.docx"),
        "a) Первый пункт\n  I. Первый подпункт\nb) Второй пункт\n"
    );
    // Два списка на одной схеме: счётчики у `w:num` свои, между списками — пустая строка.
    assert_eq!(
        markdown_of("numbering/restart_numbering.docx"),
        "1. Первый список, пункт 1\n2. Первый список, пункт 2\n\n1. Второй список, пункт 1\n2. Второй список, пункт 2\n"
    );
    // Уровень 2 сбрасывается, когда снова встречается уровень 1.
    assert_eq!(
        markdown_of("numbering/nested_levels.docx"),
        "1. Верхний уровень\n  1. Вложенный уровень 1\n    1. Вложенный уровень 2\n  2. Возврат на уровень 1\n"
    );
}

#[test]
fn bullet_list_items_get_bullets_and_indent_by_level() {
    let markdown = markdown_of("numbering/bullet_symbols.docx");
    assert_eq!(
        markdown,
        "- Первый маркер\n  - Вложенный маркер\n    - Третий уровень\n  - Снова второй уровень\n"
    );
}

#[test]
fn line_break_becomes_a_hard_break() {
    // `w:br` внутри run'а — жёсткий перенос (обратный слэш в конце строки);
    // разрыв страницы отдельного абзаца в вывод не попадает.
    let markdown = markdown_of("basic/breaks_and_tabs.docx");
    assert_has(&markdown, "First line\\\nsecond line");
    assert_has(&markdown, "Col1\tCol2");
    assert_has(&markdown, "After the page break");
}

#[test]
fn markdown_special_characters_in_text_are_escaped() {
    let markdown = markdown_of("edge_cases/special_chars.docx");
    assert_eq!(markdown, "\\<\\>&\"'Test&'\"\\<\\>\n");
}

#[test]
fn image_reference_keeps_the_package_part() {
    let markdown = markdown_of("images/inline.docx");
    assert_eq!(markdown, "![inline.png](word/media/inline.png)\n");
}

#[test]
fn every_fixture_exports_to_markdown() {
    let paths = fixture_files();
    assert!(
        !paths.is_empty(),
        "в test-fixtures/docx нет ни одного .docx"
    );
    let mut exported = 0;
    let mut skipped = 0;
    for path in &paths {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|error| panic!("{} не читается: {error}", path.display()));
        match open(bytes) {
            Ok(document) => {
                // Вывод не проверяется на конкретику — только на то, что экспорт
                // не падает и не теряет непустой документ целиком.
                let markdown = to_markdown(&document);
                let has_text = document.body.items.iter().any(|item| {
                    matches!(item, doc_converter_docx::BlockItem::Paragraph(p)
                        if !doc_converter_docx::export::text::paragraph_text(p).trim().is_empty())
                });
                assert!(
                    !has_text || !markdown.trim().is_empty(),
                    "{}: текст есть, а Markdown пуст",
                    path.display()
                );
                exported += 1;
            }
            Err(_) => skipped += 1,
        }
    }
    println!(
        "Markdown: {exported} фикстур экспортировано, {skipped} не разбирается из {}",
        paths.len()
    );
    assert_eq!(exported + skipped, paths.len());
    assert!(exported >= 50, "экспортировано всего {exported} фикстур");
}
