//! Экспорт в HTML: точные фрагменты вывода на фикстурах, экранирование, структура
//! самостоятельного документа и обход всего корпуса.
//!
//! Первая половина проверяет разметку на конкретике (заголовки, начертания, ссылки,
//! таблицы, списки, разрывы, выравнивание), вторая — что экспорт проходит по всем
//! фикстурам `test-fixtures/docx` без паник и даёт документ целиком.

use std::path::{Path, PathBuf};

use doc_converter_docx::export::html::to_html;
use doc_converter_docx::{open, BlockItem, Document, TableLayout, TableWidth, Twips, VMerge};

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

/// Разбирает фикстуру по имени и экспортирует её в HTML.
fn html_of(name: &str) -> String {
    to_html(&document_of(name))
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

/// Инлайновый стиль ячейки, обведённой со всех сторон одной и той же линией:
/// так выглядит большинство таблиц корпуса (`w:tblBorders` с `w:sz="4"`, `auto`).
fn bordered(width: &str, color: &str) -> String {
    format!(
        " style=\"border-top: {width} solid {color}; border-right: {width} solid {color}; \
         border-bottom: {width} solid {color}; border-left: {width} solid {color}\""
    )
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
    // Границы таблицы (`w:tblBorders`) доезжают до каждой ячейки инлайновым стилем:
    // в CSS границы ячейкам не наследуются, а схлопывание (`border-collapse`) уже
    // стоит в таблице стилей.
    let html = html_of("tables/simple_2x2.docx");
    let border = bordered("0.67px", "#000000");
    assert_eq!(
        page_body(&html),
        format!(
            "<table>\n\
             <tr><td{border}><p>A1</p></td><td{border}><p>A2</p></td></tr>\n\
             <tr><td{border}><p>B1</p></td><td{border}><p>B2</p></td></tr>\n\
             </table>"
        )
    );
    assert_eq!(html.matches("<table").count(), 1);
    assert_eq!(html.matches("</table>").count(), 1);
}

#[test]
fn header_rows_become_th_cells() {
    // Шапка таблицы — строки `w:tblHeader`: у них `<th>`, у остальных `<td>`.
    let html = html_of("tables/header_repeat.docx");
    let border = bordered("0.67px", "#000000");
    assert_has(
        &html,
        &format!("<tr><th{border}><p>Заголовок A</p></th><th{border}><p>Заголовок B</p></th></tr>"),
    );
    assert_has(
        &html,
        &format!("<tr><td{border}><p>Строка 1</p></td><td{border}><p>Значение 1</p></td></tr>"),
    );
    // В `simple_2x2` `w:tblHeader` нет: первая строка там не шапка, и `<th>` не появляется.
    assert!(!html_of("tables/simple_2x2.docx").contains("<th"));
}

#[test]
fn merged_columns_and_cell_alignment_become_attributes_and_classes() {
    // `w:gridSpan` — `colspan`; `w:vAlign` и `w:jc` таблицы — классы.
    let html = html_of("tables/grid_span.docx");
    let border = bordered("0.67px", "#000000");
    assert_has(
        &html,
        &format!("<tr><td colspan=\"3\"{border}><p>Строка во всю ширину</p></td></tr>"),
    );
    let aligned = html_of("tables/alignment_widths.docx");
    assert_has(&aligned, "<table class=\"align-center\">");
    assert_has(
        &aligned,
        &format!("<td{border} class=\"valign-middle\"><p>По центру</p></td>"),
    );
    assert_has(
        &aligned,
        &format!("<td{border} class=\"valign-bottom\"><p>Снизу</p></td>"),
    );
}

#[test]
fn alignment_classes_are_defined_in_the_stylesheet() {
    // Класс сам по себе ничего не значит: за каждым должно стоять правило, иначе
    // выравнивание таблицы, ячейки и абзаца молча терялось бы.
    let html = html_of("tables/alignment_widths.docx");
    for rule in [
        "th, td { border: 0; padding: 0 5.4pt; text-align: left; vertical-align: top; }",
        ".align-center { text-align: center; }",
        ".align-right { text-align: right; }",
        ".align-justify { text-align: justify; }",
        "table.align-center { margin-left: auto; margin-right: auto; }",
        "table.align-right { margin-left: auto; margin-right: 0; }",
        ".valign-middle { vertical-align: middle; }",
        ".valign-bottom { vertical-align: bottom; }",
    ] {
        assert_has(&html, rule);
    }
}

#[test]
fn explicit_table_width_and_fixed_layout_become_a_style() {
    // `w:tblW w:type="dxa"` — ширина в twips: 14400 twips это 720pt, больше страницы
    // (595.3pt); страница не расширяется, таблица выходит за поля. `w:tblLayout
    // w:type="fixed"` держит ширины столбцов из сетки, а не по содержимому.
    // В корпусе явного `w:tblW` нет, поэтому модель правится прямо в тесте.
    let mut document = document_of("tables/alignment_widths.docx");
    let BlockItem::Table(table) = &mut document.body.items[0] else {
        panic!("в фикстуре нет таблицы");
    };
    table.width = Some(TableWidth::Dxa(Twips::new(14_400)));
    table.layout = TableLayout::Fixed;
    assert_has(
        &to_html(&document),
        "style=\"width: 720pt; table-layout: fixed\"",
    );
    // `w:tblW w:type="auto"` ширины не задаёт: стиля у таблицы нет вовсе.
    assert!(!html_of("tables/simple_2x2.docx").contains("<table style="));
}

#[test]
fn nested_table_stays_a_table() {
    // Внутри ячейки таблица остаётся таблицей: HTML это позволяет, в отличие от
    // GFM-таблицы Markdown, где вложенность пришлось разворачивать в текст.
    let html = html_of("tables/nested.docx");
    let outer = bordered("1px", "#2f5496");
    let inner = bordered("0.67px", "#000000");
    assert_has(
        &html,
        &format!(
            "<td{outer}><p>Текст до вложенной</p>\n<table>\n\
             <tr><td{inner}><p>Внутренняя A1</p></td><td{inner}><p>Внутренняя B1</p></td></tr>"
        ),
    );
    assert_eq!(html.matches("<table").count(), 2);
    assert_eq!(html.matches("</table>").count(), 2);
}

#[test]
fn vertical_merge_becomes_rowspan() {
    // `w:vMerge w:val="restart"` в первой строке и два продолжения ниже: одна ячейка
    // на три строки. Ячейки-продолжения своего `<td>` не получают вовсе — их место
    // занято `rowspan`-ом, поэтому на шесть исходных ячеек в таблице четыре.
    let html = html_of("tables/v_merge.docx");
    let border = bordered("0.67px", "#000000");
    assert_eq!(
        page_body(&html),
        format!(
            "<table>\n\
             <colgroup><col style=\"width: 50%\"><col style=\"width: 50%\"></colgroup>\n\
             <tr><td rowspan=\"3\"{border}><p>Объединено по вертикали</p></td>\
             <td{border}><p>Строка 1</p></td></tr>\n\
             <tr><td{border}><p>Строка 2</p></td></tr>\n\
             <tr><td{border}><p>Строка 3</p></td></tr>\n\
             </table>"
        )
    );
    assert_eq!(html.matches("<td").count(), 4);
}

#[test]
fn a_merge_with_a_horizontal_span_keeps_both_attributes() {
    // Ячейка, объединённая и по горизонтали, и по вертикали: `colspan` и `rowspan`
    // вместе. В корпусе таблицы с таким сочетанием нет, поэтому модель правится
    // прямо в тесте: объединение начинается во второй строке, а не в первой.
    let mut document = document_of("tables/grid_span.docx");
    let BlockItem::Table(table) = &mut document.body.items[0] else {
        panic!("в фикстуре нет таблицы");
    };
    let border = bordered("0.67px", "#000000");
    table.rows[1].cells[0].v_merge = Some(VMerge::Restart);
    table.rows[2].cells[0].v_merge = Some(VMerge::Continue);
    // Продолжение занимает те же два столбца, что и начало: соседняя ячейка строки
    // уходит под объединение.
    table.rows[2].cells[0].grid_span = 2;
    table.rows[2].cells.remove(1);
    assert_eq!(
        page_body(&to_html(&document)),
        format!(
            "<table>\n\
             <colgroup><col style=\"width: 33.33%\"><col style=\"width: 33.33%\">\
             <col style=\"width: 33.33%\"></colgroup>\n\
             <tr><td colspan=\"3\"{border}><p>Строка во всю ширину</p></td></tr>\n\
             <tr><td colspan=\"2\" rowspan=\"2\"{border}><p>Две ячейки</p></td>\
             <td{border}><p>C2</p></td></tr>\n\
             <tr><td{border}><p>C3</p></td></tr>\n\
             </table>"
        )
    );
}

#[test]
fn a_row_fully_covered_by_a_merge_stays_an_empty_row() {
    // Если объединение занимает строку целиком, `<tr>` всё равно выводится пустым:
    // `rowspan` считает строки разметки, и без неё объединение уехало бы ниже.
    let mut document = document_of("tables/grid_span.docx");
    let BlockItem::Table(table) = &mut document.body.items[0] else {
        panic!("в фикстуре нет таблицы");
    };
    table.rows[0].cells[0].v_merge = Some(VMerge::Restart);
    table.rows[1].cells[0].v_merge = Some(VMerge::Continue);
    table.rows[1].cells.remove(1);
    assert_has(&to_html(&document), "<tr></tr>\n");
}

#[test]
fn cell_borders_and_shading_become_inline_styles() {
    // `w:tblBorders` — одиночная линия в 8 восьмых пункта (1.33px) цвета 2F5496 —
    // у каждой ячейки; `w:tcBorders` ячейки A2 перекрывает нижнюю границу двойной
    // линией в 6 восьмых (1px), `w:shd` даёт заливку.
    let html = html_of("tables/borders_shading.docx");
    assert_has(&html, "background-color: #2f5496");
    assert_has(
        &html,
        "<td style=\"border-top: 1.33px solid #2f5496; border-right: 1.33px solid #2f5496; \
         border-bottom: 1px double #c00000; border-left: 1.33px solid #2f5496; \
         background-color: #deeaf6\"><p>Ячейка A2</p></td>",
    );
    // Ячейка B2 своей границы не задаёт — остаются все четыре табличные, а
    // `w:fill="FFFFFF"` такая же заливка, как и любая другая.
    assert_has(
        &html,
        "<td style=\"border-top: 1.33px solid #2f5496; border-right: 1.33px solid #2f5496; \
         border-bottom: 1.33px solid #2f5496; border-left: 1.33px solid #2f5496; \
         background-color: #ffffff\"><p>Ячейка B2</p></td>",
    );
}

#[test]
fn column_widths_become_a_colgroup() {
    // `w:tblGrid` 2000/5000/2000 twips — проценты от суммы: сетка задаёт доли
    // столбцов, а ширину всей таблицы задаёт `w:tblW`.
    let html = html_of("tables/alignment_widths.docx");
    assert_has(
        &html,
        "<colgroup><col style=\"width: 22.22%\"><col style=\"width: 55.56%\">\
         <col style=\"width: 22.22%\"></colgroup>",
    );
    // Сетки в таблице нет — полосы столбцов тоже: ширины размечает браузер.
    assert!(!html_of("tables/simple_2x2.docx").contains("<colgroup>"));
}

#[test]
fn cell_margins_become_padding() {
    // `w:tblCellMar` фикстуры — умолчание `WordprocessingML` (108 twips по
    // горизонтали, ноль по вертикали): оно уже стоит в таблице стилей, поэтому
    // `padding` появляется только там, где `w:tcMar` ячейки его меняет.
    let html = html_of("tables/cell_margins_valign.docx");
    assert_has(&html, "padding: 24pt 5.4pt 24pt 5.4pt");
    assert_has(&html, "padding: 0pt 36pt 0pt 36pt");
    // Поля ячейки перекрывают табличные по каждой стороне отдельно.
    assert_has(&html, "padding: 0pt 0pt 12pt 0pt");
    assert!(!html.contains("padding: 0pt 5.4pt 0pt 5.4pt"));
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
