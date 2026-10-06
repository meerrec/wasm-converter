//! Аннотации `/Link` в готовом PDF: где лежат, куда ведут и как переживают
//! пагинацию. Проверки структурные — файл разбирается `lopdf`, как в
//! `tests/export.rs`.
//!
//! Часть книг — синтетические: фикстура с гиперссылками одна
//! (`layout-links.xlsx`), а битые ссылки и ссылки на уехавших на вторую
//! страницу ячейках нужны свои. Книга собирается публичными конструкторами
//! `doc-converter-xlsx` (`WorksheetBuilder`, `Sheet::new`, `Workbook::new`).

use std::path::Path;

use doc_converter_pdf::{Margins, PageConfig, PageSize, PdfExporter, PdfOptions};
use doc_converter_xlsx::layout::SheetLayout;
use doc_converter_xlsx::{
    Cell, CellValue, Hyperlink, HyperlinkTarget, Range, SharedStrings, Sheet, SheetContent,
    SheetState, StyleTable, Theme, Workbook, WorksheetBuilder, WorksheetMeta,
};
use lopdf::{Dictionary, Document};

/// Часть пакета синтетического листа; нужна только текстам ошибок.
const PART: &str = "xl/worksheets/sheet1.xml";

/// Открыть книгу из общего набора фикстур.
fn open_fixture(name: &str) -> Workbook {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{name}: {err}"))
}

/// Синтетическая книга: лист `name` с ячейками `(строка, столбец, текст)`
/// (по возрастанию) и гиперссылками `(диапазон, цель)`.
fn workbook(name: &str, cells: &[(u32, u32, &str)], links: &[(&str, HyperlinkTarget)]) -> Workbook {
    let mut builder = WorksheetBuilder::new(PART);
    for (row, col, text) in cells {
        builder
            .push(
                *row,
                Cell::new(*col, 0, CellValue::InlineString((*text).into())),
            )
            .expect("ячейка");
    }
    let content = SheetContent {
        cells: builder.finish(),
        hyperlinks: links
            .iter()
            .map(|(range, target)| Hyperlink {
                range: Range::parse_ref(range).expect("адрес ссылки"),
                target: target.clone(),
                display: None,
                tooltip: None,
            })
            .collect(),
        ..SheetContent::default()
    };
    let meta = WorksheetMeta {
        name: name.into(),
        part: PART.into(),
        state: SheetState::Visible,
    };
    Workbook::new(
        vec![Sheet::new(meta, content)],
        SharedStrings::default(),
        StyleTable::default(),
        Theme::default(),
        false,
    )
}

/// Экспортировать лист и разобрать результат.
fn export(book: &Workbook, options: PdfOptions) -> Document {
    let bytes = PdfExporter::new(options)
        .export_xlsx_sheet(book, 0)
        .unwrap_or_else(|err| panic!("экспорт не удался: {err}"));
    Document::load_mem(&bytes).expect("PDF разбирается lopdf")
}

/// Настройки страницы, в которую помещается ровно одна строка листа.
///
/// Высота области содержимого — полторы строки в пикселях раскладки: вторая
/// строка уже не влезает, но запас не даёт округлению `f32` сдвинуть разрыв.
fn one_row_page(sheet: &Sheet) -> PdfOptions {
    let layout = SheetLayout::new(sheet);
    let row_h = layout.row_y(1) - layout.row_y(0);
    let h_mm = row_h * 1.5 * 0.75 * 25.4 / 72.0;
    PdfOptions {
        page: PageConfig {
            size: PageSize::Custom { w_mm: 100.0, h_mm },
            margins: Margins {
                top_mm: 0.0,
                right_mm: 0.0,
                bottom_mm: 0.0,
                left_mm: 0.0,
            },
            ..PageConfig::default()
        },
        ..PdfOptions::default()
    }
}

/// Аннотации всех страниц в порядке номеров: `(страница с единицы, словарь)`.
fn annotations(doc: &Document) -> Vec<(u32, Dictionary)> {
    let mut found = Vec::new();
    for (number, page_id) in doc.get_pages() {
        let page = doc.get_dictionary(page_id).expect("словарь страницы");
        let Ok(annots) = page.get(b"Annots") else {
            continue;
        };
        let (_, annots) = doc.dereference(annots).expect("разыменование /Annots");
        for annot in annots.as_array().expect("/Annots — массив") {
            let (_, annot) = doc.dereference(annot).expect("разыменование аннотации");
            found.push((
                number,
                annot.as_dict().expect("аннотация — словарь").clone(),
            ));
        }
    }
    found
}

/// Имя из словаря (`/Subtype`, `/S`).
fn name(dict: &Dictionary, key: &[u8]) -> String {
    let value = dict
        .get(key)
        .unwrap_or_else(|_| panic!("нет ключа {}", String::from_utf8_lossy(key)));
    String::from_utf8(value.as_name().expect("имя").to_vec()).expect("имя — UTF-8")
}

/// Строка из словаря (`/URI`).
fn string(dict: &Dictionary, key: &[u8]) -> String {
    let value = dict.get(key).expect("строка в словаре");
    String::from_utf8(value.as_str().expect("строка").to_vec()).expect("строка — UTF-8")
}

/// Словарь действия `/A` аннотации.
fn action(annot: &Dictionary) -> &Dictionary {
    annot
        .get(b"A")
        .expect("действие аннотации")
        .as_dict()
        .expect("/A — словарь")
}

/// Номер страницы, на которую ведёт `/GoTo`-действие аннотации.
fn goto_page(doc: &Document, annot: &Dictionary) -> u32 {
    let action = action(annot);
    assert_eq!(name(action, b"S"), "GoTo", "действие не /GoTo");
    let dest = action
        .get(b"D")
        .expect("цель перехода")
        .as_array()
        .expect("/D — массив");
    let object = dest
        .first()
        .expect("первый элемент /D")
        .as_reference()
        .expect("/D ссылается на страницу");
    doc.get_pages()
        .into_iter()
        .find(|(_, id)| *id == object)
        .map(|(number, _)| number)
        .expect("страница цели есть в документе")
}

/// Прямоугольник `/Rect` как `[x0, y0, x1, y1]`.
fn rect(annot: &Dictionary) -> [f32; 4] {
    let values = annot
        .get(b"Rect")
        .expect("/Rect")
        .as_array()
        .expect("/Rect — массив");
    assert_eq!(values.len(), 4, "/Rect из четырёх чисел");
    std::array::from_fn(|i| values[i].as_float().expect("число в /Rect"))
}

/// DoD E1 (+ дефект B1): `/Annots` лежит в словаре страницы, а не в ресурсах —
/// в `Resources` просмотрщики аннотации не ищут.
#[test]
fn annots_live_on_page_not_in_resources() {
    let book = open_fixture("layout-links.xlsx");
    let doc = export(&book, PdfOptions::default());

    let mut pages = 0;
    for (_, page_id) in doc.get_pages() {
        let page = doc.get_dictionary(page_id).expect("словарь страницы");
        assert!(
            page.get(b"Annots").is_ok(),
            "/Annots нет в словаре страницы"
        );
        let (_, resources) = doc
            .dereference(page.get(b"Resources").expect("/Resources"))
            .expect("разыменование /Resources");
        let resources = resources.as_dict().expect("/Resources — словарь");
        assert!(
            resources.get(b"Annots").is_err(),
            "/Annots уехал в Resources — дефект printpdf 0.8.2"
        );
        pages += 1;
    }
    assert_eq!(pages, 1, "фикстура печатается на одну страницу");

    for (_, annot) in annotations(&doc) {
        assert_eq!(name(&annot, b"Type"), "Annot");
        assert_eq!(name(&annot, b"Subtype"), "Link");
    }
}

/// DoD E1: аннотаций ровно столько, сколько небитых ссылок листа, `/URI`
/// совпадает посимвольно, внешние аннотации — `/S /URI`, внутренняя — `/GoTo`.
#[test]
fn link_annotations_match_sheet_hyperlinks() {
    let book = open_fixture("layout-links.xlsx");
    let sheet = &book.sheets()[0];
    let doc = export(&book, PdfOptions::default());
    let annots = annotations(&doc);

    let links: Vec<&Hyperlink> = sheet
        .hyperlinks
        .iter()
        .filter(|link| !matches!(link.target, HyperlinkTarget::Broken(_)))
        .collect();
    assert_eq!(annots.len(), links.len(), "число аннотаций ≠ числу ссылок");

    let mut found: Vec<String> = Vec::new();
    for (_, annot) in &annots {
        match name(action(annot), b"S").as_str() {
            "URI" => found.push(string(action(annot), b"URI")),
            "GoTo" => {}
            other => panic!("неожиданный тип действия: {other}"),
        }
    }
    let mut expected: Vec<String> = links
        .iter()
        .filter_map(|link| match &link.target {
            HyperlinkTarget::External(uri) => Some(uri.clone()),
            _ => None,
        })
        .collect();
    found.sort();
    expected.sort();
    assert_eq!(found, expected, "URI аннотаций ≠ ссылкам листа");
}

/// `Broken` не даёт аннотации: цель не разрешилась, вести по ней некуда.
#[test]
fn broken_link_produces_no_annotation() {
    let book = workbook(
        "Ссылки",
        &[(0, 0, "r0"), (1, 0, "r1")],
        &[
            (
                "A1",
                HyperlinkTarget::External("https://example.com/".into()),
            ),
            ("A2", HyperlinkTarget::Broken("rId9".into())),
        ],
    );
    let doc = export(&book, PdfOptions::default());
    let annots = annotations(&doc);

    assert_eq!(annots.len(), 1, "битая ссылка аннотирована");
    assert_eq!(
        string(action(&annots[0].1), b"URI"),
        "https://example.com/",
        "выжила не та ссылка"
    );
}

/// Ссылка переживает пагинацию: аннотация остаётся на странице своей ячейки,
/// а не на первой.
#[test]
fn link_follows_cell_to_second_page() {
    let book = workbook(
        "Ссылки",
        &[(0, 0, "r0"), (1, 0, "r1")],
        &[
            (
                "A1",
                HyperlinkTarget::External("https://example.com/one".into()),
            ),
            (
                "A2",
                HyperlinkTarget::External("https://example.com/two".into()),
            ),
        ],
    );
    let sheet = &book.sheets()[0];
    let doc = export(&book, one_row_page(sheet));
    assert_eq!(doc.get_pages().len(), 2, "каждая строка — своя страница");

    let uris: Vec<(u32, String)> = annotations(&doc)
        .iter()
        .map(|(page, annot)| (*page, string(action(annot), b"URI")))
        .collect();
    assert_eq!(
        uris,
        vec![
            (1, "https://example.com/one".into()),
            (2, "https://example.com/two".into())
        ],
        "ссылка не уехала вместе с ячейкой"
    );
}

/// Внутренняя ссылка ведёт на страницу своей цели; цель вне печатаемого листа
/// (чужой лист, определённое имя) — переход в начало документа.
#[test]
fn internal_link_goes_to_page_of_target() {
    let book = workbook(
        "Лист",
        &[(0, 0, "r0"), (1, 0, "r1"), (2, 0, "r2")],
        &[
            ("A1", HyperlinkTarget::Internal("Лист!A3".into())),
            ("A2", HyperlinkTarget::Internal("Другой!A1".into())),
            ("A3", HyperlinkTarget::Internal("Итоги".into())),
        ],
    );
    let sheet = &book.sheets()[0];
    let doc = export(&book, one_row_page(sheet));
    let annots = annotations(&doc);

    assert_eq!(annots.len(), 3, "каждой ссылке — своя аннотация");
    assert_eq!(
        annots.iter().map(|(page, _)| *page).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "аннотация осталась не на странице ячейки"
    );
    assert_eq!(goto_page(&doc, &annots[0].1), 3, "цель на третьей странице");
    assert_eq!(goto_page(&doc, &annots[1].1), 1, "чужой лист — начало");
    assert_eq!(goto_page(&doc, &annots[2].1), 1, "имя области — начало");
}

/// Ссылка на диапазон накрывает обе ячейки: `/Rect` шире одиночной ссылки
/// и не вылезает за страницу.
#[test]
fn link_rect_covers_whole_range() {
    let book = workbook(
        "Ссылки",
        &[(0, 0, "r0"), (1, 0, "r1"), (1, 1, "r1b")],
        &[
            (
                "A1",
                HyperlinkTarget::External("https://example.com/single".into()),
            ),
            (
                "A2:B2",
                HyperlinkTarget::External("https://example.com/range".into()),
            ),
        ],
    );
    let doc = export(&book, PdfOptions::default());
    let annots = annotations(&doc);

    let single = rect(&annots[0].1);
    let range = rect(&annots[1].1);
    assert!(
        range[2] - range[0] > single[2] - single[0],
        "ссылка на A2:B2 уже одиночной: {range:?} против {single:?}"
    );
    let (width_pt, height_pt) = page_size(&doc);
    assert!(range[0] >= 0.0 && range[1] >= 0.0);
    assert!(
        range[2] <= width_pt && range[3] <= height_pt,
        "Rect за страницей"
    );
}

/// Размер первой страницы документа в точках.
fn page_size(doc: &Document) -> (f32, f32) {
    let page_id = *doc.get_pages().values().next().expect("страница есть");
    let page = doc.get_dictionary(page_id).expect("словарь страницы");
    let media = page
        .get(b"MediaBox")
        .expect("/MediaBox")
        .as_array()
        .expect("/MediaBox — массив");
    let value = |i: usize| media[i].as_float().expect("число");
    (value(2) - value(0), value(3) - value(1))
}
