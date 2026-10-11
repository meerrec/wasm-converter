//! Экспорт в HTML: точные фрагменты вывода на фикстурах, экранирование, структура
//! самостоятельного документа и обход всего корпуса.
//!
//! Первая половина проверяет разметку на конкретике (заголовки, начертания, ссылки,
//! таблицы, списки, разрывы, выравнивание), вторая — что экспорт проходит по всем
//! фикстурам `test-fixtures/docx` без паник и даёт документ целиком.

use std::path::{Path, PathBuf};

use doc_converter_docx::export::html::to_html;
use doc_converter_docx::open;

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

/// Разбирает фикстуру по имени и экспортирует её в HTML.
fn html_of(name: &str) -> String {
    let path = fixtures_dir().join(name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} не читается: {error}", path.display()));
    let document = open(bytes).unwrap_or_else(|error| panic!("{name} не разбирается: {error}"));
    to_html(&document)
}

/// Проверяет, что ожидаемая строка есть в выводе; при расхождении печатает весь вывод.
#[track_caller]
fn assert_has(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "в выводе нет {needle:?}:\n{haystack}"
    );
}

/// Тело страницы: вывод без шапки и стилей — то, что видно в браузере.
#[track_caller]
fn page_body(html: &str) -> &str {
    let open = "<div class=\"page\">\n";
    let start = html.find(open).expect("нет полосы набора") + open.len();
    let end = html[start..]
        .find("</div>")
        .expect("полоса набора не закрыта")
        + start;
    html[start..end].trim_end_matches('\n')
}

/// Содержимое первого абзаца вывода: от `<p>` до `</p>`.
#[track_caller]
fn first_paragraph(html: &str) -> &str {
    let start = html.find("<p").expect("в выводе нет абзаца");
    let start = html[start..].find('>').expect("тег абзаца не закрыт") + start + 1;
    let end = html[start..].find("</p>").expect("абзац не закрыт") + start;
    &html[start..end]
}

#[test]
fn heading_style_becomes_a_heading_element() {
    // `Heading1`/`Heading2` в фикстурах — полужирные стили (начертание приходит из
    // каскада): начертания внутри заголовка сохраняются, как и в Markdown.
    assert_eq!(
        page_body(&html_of("formatting/heading_1.docx")),
        "<h1><strong>Heading 1</strong></h1>"
    );
    assert_eq!(
        page_body(&html_of("formatting/heading_2.docx")),
        "<h2><strong>Heading 2</strong></h2>"
    );
    // Уровень берётся из `w:outlineLvl`, а не из имени стиля: здесь он наследуется
    // по цепочке `basedOn`.
    assert_has(
        &html_of("styles/based_on_chain.docx"),
        "<h3><strong><em>Заголовок конца цепочки</em></strong></h3>",
    );
}

#[test]
fn run_marks_become_html_elements() {
    // Прогоны одного абзаца: `w:b` → `<strong>`, `w:i` → `<em>`, `w:u` → `<u>`,
    // `Emphasis` — знаковый стиль → `<em>`. Пробелы остаются внутри элементов:
    // в HTML, в отличие от разметки Markdown, `**bold **` ничего не ломает.
    // Цвет и кегль прогона (`colored`, `sized`) через `Segment` не доезжают —
    // в разметке есть только начертания.
    let html = html_of("basic/multiple_runs.docx");
    assert_has(
        &html,
        "<p>Plain <strong>bold </strong><em>italic </em><u>underlined </u>colored sized<em> emphasized</em></p>",
    );
}

#[test]
fn hyperlink_becomes_an_anchor_with_href() {
    // В фикстуре два вида ссылок: `w:hyperlink` и составное поле HYPERLINK;
    // у обеих цель разрешена через relationships.
    let html = html_of("fields/hyperlink_field.docx");
    assert_has(
        &html,
        "<p><a href=\"https://example.com/docs\"><u>example.com/docs</u></a></p>",
    );
    assert_has(
        &html,
        "<p>Link: <a href=\"https://example.com/docs\"><u>example.com/docs</u></a></p>",
    );
}

#[test]
fn image_becomes_an_img_with_the_package_part() {
    let html = html_of("images/inline.docx");
    assert_has(
        &html,
        "<img src=\"word/media/inline.png\" alt=\"inline.png\">",
    );
}

#[test]
fn table_becomes_rows_and_cells() {
    let html = html_of("tables/simple_2x2.docx");
    assert_has(
        &html,
        "<table>\n<tr><td><p>A1</p></td><td><p>A2</p></td></tr>\n\
         <tr><td><p>B1</p></td><td><p>B2</p></td></tr>\n</table>",
    );
    assert_eq!(html.matches("<table").count(), 1);
    assert_eq!(html.matches("</table>").count(), 1);
}

#[test]
fn header_rows_become_th_cells() {
    // Шапка таблицы — строки `w:tblHeader`: у них `<th>`, у остальных `<td>`.
    let html = html_of("tables/header_repeat.docx");
    assert_has(
        &html,
        "<tr><th><p>Заголовок A</p></th><th><p>Заголовок B</p></th></tr>",
    );
    assert_has(
        &html,
        "<tr><td><p>Строка 1</p></td><td><p>Значение 1</p></td></tr>",
    );
    // В `simple_2x2` `w:tblHeader` нет: первая строка там не шапка, и `<th>` не появляется.
    assert!(!html_of("tables/simple_2x2.docx").contains("<th"));
}

#[test]
fn merged_columns_and_cell_alignment_become_attributes_and_classes() {
    // `w:gridSpan` — `colspan`; `w:vAlign` и `w:jc` таблицы — классы.
    let html = html_of("tables/grid_span.docx");
    assert_has(
        &html,
        "<tr><td colspan=\"3\"><p>Строка во всю ширину</p></td></tr>",
    );
    let aligned = html_of("tables/alignment_widths.docx");
    assert_has(&aligned, "<table class=\"align-center\">");
    assert_has(
        &aligned,
        "<td class=\"valign-middle\"><p>По центру</p></td>",
    );
    assert_has(&aligned, "<td class=\"valign-bottom\"><p>Снизу</p></td>");
}

#[test]
fn nested_table_stays_a_table() {
    // Внутри ячейки таблица остаётся таблицей: HTML это позволяет, в отличие от
    // GFM-таблицы Markdown, где вложенность пришлось разворачивать в текст.
    let html = html_of("tables/nested.docx");
    assert_has(
        &html,
        "<td><p>Текст до вложенной</p>\n<table>\n\
         <tr><td><p>Внутренняя A1</p></td><td><p>Внутренняя B1</p></td></tr>",
    );
    assert_eq!(html.matches("<table").count(), 2);
    assert_eq!(html.matches("</table>").count(), 2);
}

#[test]
fn list_items_become_li_in_ul_and_ol() {
    // `w:numFmt="decimal"` — нумерованный список; номер расставляет браузер.
    assert_has(
        &html_of("numbering/decimal_basic.docx"),
        "<ol><li>Первый пункт</li><li>Второй пункт</li><li>Третий пункт</li></ol>",
    );
    // `w:startOverride` сдвигает начало: без `start` HTML начал бы список с единицы.
    assert_has(
        &html_of("numbering/start_override.docx"),
        "<ol start=\"5\"><li>Пункт с номером 5</li>",
    );
    // `w:numFmt="bullet"` — маркированный список, вложенный уровень вкладывается в `<li>`.
    assert_has(
        &html_of("numbering/bullet_symbols.docx"),
        "<ul><li>Первый маркер<ul><li>Вложенный маркер<ul><li>Третий уровень</li></ul></li>\
         <li>Снова второй уровень</li></ul></li></ul>",
    );
    // `w:numFmt="lowerLetter"` даёт маркер без цифр: `start` не ставится, `type` — тоже
    // (HTML-нумерация начнётся с единицы, но структура списка сохранена).
    assert_has(
        &html_of("numbering/custom_format.docx"),
        "<ol><li>Первый пункт<ol><li>Первый подпункт</li></ol></li><li>Второй пункт</li></ol>",
    );
}

#[test]
fn line_break_becomes_br() {
    // `w:br` внутри run'а — `<br>`; разрыв страницы отдельного абзаца в потоке HTML
    // тоже перенос: страниц здесь нет.
    let html = html_of("basic/breaks_and_tabs.docx");
    assert_has(&html, "<p>First line<br>second line</p>");
    assert_has(&html, "<p>Col1<span class=\"tab\"></span>Col2</p>");
    assert_has(&html, "<p>After the page break</p>");
}

#[test]
fn paragraph_alignment_becomes_a_class() {
    // Выравнивание из `w:jc` — класс, а не инлайн-стиль: по левому краю это умолчание
    // HTML и класса не получает.
    let html = html_of("basic/paragraph_alignment.docx");
    assert_has(&html, "<p>Aligned left</p>");
    assert_has(&html, "<p class=\"align-center\">Aligned center</p>");
    assert_has(&html, "<p class=\"align-right\">Aligned right</p>");
    assert_has(&html, "<p class=\"align-justify\">Aligned both</p>");
}

#[test]
fn document_defaults_become_the_stylesheet_and_the_paragraph_class() {
    // `docDefaults` фикстуры: Georgia 13pt, `w:color 1A1A1A`, `w:jc both` в `w:pPrDefault`.
    let html = html_of("styles/doc_defaults.docx");
    assert_has(&html, "font-family: \"Georgia\";");
    assert_has(&html, "font-size: 13pt;");
    assert_has(&html, "color: #1a1a1a;");
    assert_has(
        &html,
        "<p class=\"align-justify\">Абзац наследует docDefaults</p>",
    );
}

#[test]
fn page_setup_comes_from_the_section() {
    // `w:pgSz` 11906×16838 twips и поля 1134 twips: пункты — двадцатые доли twips.
    let html = html_of("tables/simple_2x2.docx");
    assert_has(&html, "width: 595.3pt;");
    assert_has(&html, "min-height: 841.9pt;");
    assert_has(&html, "padding: 56.7pt 56.7pt 56.7pt 56.7pt;");
}

#[test]
fn output_is_a_standalone_html_document() {
    let html = html_of("tables/simple_2x2.docx");
    assert!(
        html.starts_with("<!DOCTYPE html>\n<html lang=\"en\">\n"),
        "{html}"
    );
    for part in [
        "<head>",
        "<meta charset=\"utf-8\">",
        "<title>",
        "</title>",
        "<style>",
        "</style>",
        "</head>",
        "<body>",
        "</body>",
        "</html>",
    ] {
        assert_has(&html, part);
    }
    // Документ закрыт целиком: открывающих и закрывающих тегов таблицы поровну.
    assert!(html.ends_with("</html>\n"), "{html}");
}

#[test]
fn html_special_characters_in_text_are_escaped() {
    // Текст фикстуры: `<&">'Test&'"<>`; в разметку он должен попасть только
    // экранированным, иначе `<` открыл бы тег, а `&` — сущность.
    let html = html_of("edge_cases/special_chars.docx");
    let paragraph = first_paragraph(&html);
    assert_eq!(paragraph, "&lt;&gt;&amp;&quot;'Test&amp;'&quot;&lt;&gt;");
    let raw = paragraph
        .replace("&amp;", "")
        .replace("&lt;", "")
        .replace("&gt;", "")
        .replace("&quot;", "");
    assert!(
        !raw.contains(['<', '>', '&']),
        "в тексте остались неэкранированные символы: {paragraph}"
    );
}

#[test]
fn every_fixture_exports_to_html() {
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
                // не падает и отдаёт закрытый документ.
                let html = to_html(&document);
                assert!(
                    html.starts_with("<!DOCTYPE html>") && html.ends_with("</html>\n"),
                    "{}: вывод не документ",
                    path.display()
                );
                assert_eq!(
                    html.matches("<table").count(),
                    html.matches("</table>").count(),
                    "{}: таблица не закрыта",
                    path.display()
                );
                exported += 1;
            }
            Err(_) => skipped += 1,
        }
    }
    println!(
        "HTML: {exported} фикстур экспортировано, {skipped} не разбирается из {}",
        paths.len()
    );
    assert_eq!(exported + skipped, paths.len());
    assert!(exported >= 90, "экспортировано всего {exported} фикстур");
}
