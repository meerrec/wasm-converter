//! Рисунки DOCX в собранном PDF: `/Image`-`XObject`, прямоугольник раскладки
//! и пропуск рисунков без байтов.
//!
//! Фикстуры — те же из `test-fixtures/docx`, что читает `crates/docx`. Проверки
//! структурные: файл разбирается `lopdf`, прямоугольник вывода берётся из
//! `cm`-матрицы перед `Do`, а эталон считается от самой раскладки
//! ([`LayoutItem::Image::rect`]) — так же, как в `tests/docx_floats.rs`.
//!
//! `pedantic` включён здесь, чтобы линты совпадали с крейтовыми
//! (`#![deny(clippy::pedantic)]` в `lib.rs`).
#![deny(clippy::pedantic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use doc_converter_core::NodeId;
use doc_converter_docx::layout::{
    layout_document, FloatElement, LayoutItem, LayoutOptions, PageLayout, Rect,
};
use doc_converter_docx::{
    BlockItem, Cell, CellBorders, CellMargins, CellVAlign, Document, GridCol, Row, Table,
    TableBorders, TableLayout, TableLook, Twips,
};
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::FontRegistry;
use lopdf::content::Content;
use lopdf::{Document as Pdf, Object, Stream};

/// Пикселей раскладки в точке PDF: 96 dpi против 72 dpi.
const PX_PER_POINT: f32 = 96.0 / 72.0;
/// Допуск сверки координат, точки: `f32`-арифметика раскладки против PDF.
const TOLERANCE_PT: f32 = 0.1;
/// Размер LRU-кэша метрик — как у крейта (`painter::FONT_CACHE`).
const FONT_CACHE: usize = 4096;

/// Путь к фикстуре в `test-fixtures/docx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/docx")
        .join(name)
}

/// Открыть фикстуру; её отсутствие роняет тест, а не пропускает его.
fn document(name: &str) -> Document {
    let path = fixture(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    doc_converter_docx::open(bytes).unwrap_or_else(|e| panic!("{name} не открылся: {e}"))
}

/// Разложить документ так же, как это сделает экспорт.
fn layout(doc: &Document) -> PageLayout {
    layout_document(
        doc,
        &LayoutOptions::default(),
        &mut FontRegistry::new(FONT_CACHE),
    )
    .expect("раскладка собирается")
}

/// Экспортировать документ настройками по умолчанию.
fn export(doc: &Document) -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_docx(doc)
        .expect("документ экспортируется")
}

/// Операции страницы `index` (с нуля) собранного PDF.
fn page_ops(bytes: &[u8], index: usize) -> Content {
    let pdf = Pdf::load_mem(bytes).expect("PDF разбирается lopdf");
    let id = *pdf
        .get_pages()
        .values()
        .nth(index)
        .unwrap_or_else(|| panic!("страницы {index} нет"));
    let content = pdf.get_page_content(id);
    Content::decode(&content).expect("операции разбираются")
}

/// Операнд-число: координаты пишутся как `Real`.
#[allow(clippy::cast_precision_loss)] // координаты страницы далеки от 2^24
fn number(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("координата не число: {other:?}"),
    }
}

/// `Image`-`XObject`'ы документа (имя → поток) и число `Do`-ссылок на них.
///
/// `XObject`'ы глобальные: словарь один на документ, а страница ссылается на
/// него целиком. Имена поэтому собираются со всех страниц, а `Do` считается
/// по ним же — приём из `tests/image.rs`.
///
/// Свой бэкенд пишет `/Resources` страницы прямым словарём, вендоренный —
/// ссылкой на косвенный объект; принимаются оба (как `tests/image.rs`).
fn document_images(doc: &Pdf) -> (BTreeMap<Vec<u8>, &Stream>, usize) {
    let mut images: BTreeMap<Vec<u8>, &Stream> = BTreeMap::new();
    let mut draws = 0;
    for &page_id in doc.get_pages().values() {
        let (direct, indirect) = doc.get_page_resources(page_id).expect("ресурсы страницы");
        let resources = match (direct, indirect.first()) {
            (Some(dict), _) => dict,
            (None, Some(id)) => doc.get_dictionary(*id).expect("словарь ресурсов"),
            (None, None) => continue,
        };
        let Ok(xobjects) = resources.get(b"XObject") else {
            continue;
        };
        let xobjects = doc
            .get_object(xobjects.as_reference().expect("XObject — ссылка"))
            .and_then(Object::as_dict)
            .expect("XObject — словарь");
        for (name, value) in xobjects {
            let stream = doc
                .get_object(value.as_reference().expect("поток — ссылка"))
                .and_then(Object::as_stream)
                .expect("XObject — поток");
            if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image") {
                images.insert(name.clone(), stream);
            }
        }
        let bytes = doc.get_page_content(page_id);
        let content = Content::decode(&bytes).expect("операции разбираются");
        draws += content
            .operations
            .iter()
            .filter(|op| op.operator == "Do")
            .filter(|op| {
                op.operands
                    .first()
                    .and_then(|operand| operand.as_name().ok())
                    .is_some_and(|name| images.contains_key(name))
            })
            .count();
    }
    (images, draws)
}

/// Имя `XObject`'а и `cm`-матрица каждого `Do` в порядке потока.
///
/// `Op::UseXobject` пишется как `q` `cm` `Do` `Q`: матрица — это CTM,
/// действующая только на картинку. Складывается она из масштаба исходника
/// (1 px = 1 pt при `dpi = 72`) и переноса в прямоугольник якоря, поэтому
/// `a`/`d` — стороны рисунка в точках, `e`/`f` — его левый нижний угол.
fn do_matrices(content: &Content) -> Vec<(Vec<u8>, [f32; 6])> {
    let mut out = Vec::new();
    let mut matrix = None;
    for op in &content.operations {
        match op.operator.as_str() {
            "cm" => {
                let values: Vec<f32> = op.operands.iter().map(number).collect();
                matrix = Some(<[f32; 6]>::try_from(values).expect("`cm` — шесть чисел"));
            }
            "Do" => {
                let name = op
                    .operands
                    .first()
                    .and_then(|operand| operand.as_name().ok())
                    .expect("`Do` — имя XObject'а")
                    .to_vec();
                out.push((name, matrix.take().expect("перед `Do` есть `cm`")));
            }
            _ => matrix = None,
        }
    }
    out
}

/// Матрица вывода рисунка: стороны в точках и левый нижний угол страницы.
///
/// Эталон считается от прямоугольника раскладки (пиксели 96 dpi от левого
/// верхнего угла) — той же арифметикой, что у теста плейсхолдеров
/// (`expected_corners` в `tests/docx_floats.rs`).
fn expected_matrix(rect: Rect, page_height_px: f32) -> [f32; 6] {
    let left = rect.x / PX_PER_POINT;
    let bottom = (page_height_px - rect.y - rect.height) / PX_PER_POINT;
    [
        rect.width / PX_PER_POINT,
        0.0,
        0.0,
        rect.height / PX_PER_POINT,
        left,
        bottom,
    ]
}

/// Прямоугольник вывода `cm`-матрицы: `(x, y, ширина, высота)` в точках.
fn matrix_rect(matrix: [f32; 6]) -> (f32, f32, f32, f32) {
    (matrix[4], matrix[5], matrix[0], matrix[3])
}

/// Сверить `cm`-матрицу с прямоугольником раскладки.
fn assert_lands_on(rect: Rect, page_height_px: f32, matrix: [f32; 6], what: &str) {
    let expected = expected_matrix(rect, page_height_px);
    for (index, (got, want)) in matrix.iter().zip(expected).enumerate() {
        assert!(
            (got - want).abs() <= TOLERANCE_PT,
            "{what}: столбец {index} матрицы {matrix:?} не совпал с ожиданием {expected:?}"
        );
    }
}

/// Прямоугольники рисунков раскладки в порядке обхода: вложенные в ячейки
/// таблиц и плавающие — тоже.
fn layout_images(layout: &PageLayout) -> (Vec<(Option<u32>, Rect)>, Vec<&FloatElement>) {
    fn walk<'a>(
        items: &'a [LayoutItem],
        images: &mut Vec<(Option<u32>, Rect)>,
        floats: &mut Vec<&'a FloatElement>,
    ) {
        for item in items {
            match item {
                LayoutItem::Image { rect, image_id, .. } => images.push((*image_id, *rect)),
                LayoutItem::Table { cells, .. } => {
                    for cell in cells {
                        walk(&cell.content, images, floats);
                    }
                }
                LayoutItem::Float(float) => floats.push(float),
                LayoutItem::Paragraph { .. } | LayoutItem::PageBreak | LayoutItem::ColumnBreak => {}
            }
        }
    }

    let mut images = Vec::new();
    let mut floats = Vec::new();
    for page in &layout.pages {
        walk(&page.items, &mut images, &mut floats);
    }
    (images, floats)
}

/// Встроенный рисунок фикстуры доезжает до PDF как `Image`-`XObject` и
/// занимает прямоугольник раскладки.
///
/// До этого среза ветка `LayoutItem::Image` диспетчера была заглушкой
/// (коммит ccebc82): места в раскладке рисунок занимал, но в PDF не рисовался.
#[test]
fn inline_image_reaches_pdf_as_xobject() {
    let name = "images/inline.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let (images, floats) = layout_images(&layout);
    assert_eq!(
        images.len(),
        1,
        "{name}: у фикстуры один встроенный рисунок"
    );
    assert!(floats.is_empty(), "{name}: плавающих рисунков нет");
    let (image_id, rect) = images[0];
    assert!(
        image_id.is_some(),
        "{name}: байты рисунка нашлись в документе"
    );

    let bytes = export(&doc);
    let pdf = Pdf::load_mem(&bytes).expect("PDF разбирается lopdf");
    let (xobjects, draws) = document_images(&pdf);
    assert_eq!(xobjects.len(), 1, "{name}: одна media-часть — один XObject");
    assert_eq!(draws, 1, "{name}: страница выводит XObject один раз");

    let stream = xobjects.values().next().expect("XObject есть");
    assert_eq!(
        stream.dict.get(b"Subtype").and_then(Object::as_name).ok(),
        Some(b"Image" as &[u8]),
        "{name}: XObject должен быть изображением"
    );
    assert_eq!(
        stream.dict.get(b"Width").and_then(Object::as_i64).ok(),
        Some(8),
        "{name}: ширина исходника (`word/media/inline.png`)"
    );
    assert_eq!(
        stream.dict.get(b"Height").and_then(Object::as_i64).ok(),
        Some(8),
        "{name}: высота исходника"
    );

    let content = page_ops(&bytes, 0);
    let drawings = do_matrices(&content);
    let [(name_do, matrix)] = drawings.as_slice() else {
        panic!("{name}: ожидался ровно один `Do`, а их {}", drawings.len());
    };
    assert!(
        xobjects.contains_key(name_do),
        "{name}: `Do` ссылается на зарегистрированный XObject"
    );
    // `wp:extent` фикстуры — 914400 x 685800 EMU, то есть 96 x 72 px раскладки.
    assert!(
        (matrix[0] - 72.0).abs() <= TOLERANCE_PT && (matrix[3] - 54.0).abs() <= TOLERANCE_PT,
        "{name}: прямоугольник вывода {matrix:?} не 72 x 54 pt"
    );
    assert_lands_on(rect, layout.pages[0].height, *matrix, name);
}

/// У каждого рисунка фикстуры — свой `XObject` и свой прямоугольник.
#[test]
fn images_of_different_parts_get_their_own_xobjects() {
    let name = "images/multiple_sizes.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let (images, _) = layout_images(&layout);
    assert_eq!(images.len(), 3, "{name}: у фикстуры три рисунка");
    assert_eq!(
        layout.pages.len(),
        1,
        "{name}: все рисунки на одной странице"
    );

    let bytes = export(&doc);
    let pdf = Pdf::load_mem(&bytes).expect("PDF разбирается lopdf");
    let (xobjects, draws) = document_images(&pdf);
    assert_eq!(xobjects.len(), 3, "{name}: три части — три XObject'а");
    assert_eq!(draws, 3, "{name}: каждый XObject выведен один раз");

    // Размеры исходников: small 8x8, wide 24x24, tall 16x16 пикселей — разные
    // PNG, и по ним видно, что `Do` ссылается на свою часть, а не на первую.
    let mut sizes: Vec<(i64, i64)> = xobjects
        .values()
        .map(|stream| {
            (
                stream.dict.get(b"Width").and_then(Object::as_i64).unwrap(),
                stream.dict.get(b"Height").and_then(Object::as_i64).unwrap(),
            )
        })
        .collect();
    sizes.sort_unstable();
    assert_eq!(
        sizes,
        vec![(8, 8), (16, 16), (24, 24)],
        "{name}: размеры XObject'ов"
    );

    let content = page_ops(&bytes, 0);
    let matrices = do_matrices(&content);
    assert_eq!(matrices.len(), 3, "{name}: три `Do`");
    let page_height = layout.pages[0].height;
    for (index, (image_id, rect)) in images.iter().enumerate() {
        assert!(image_id.is_some(), "{name}: у рисунка {index} есть байты");
        // Порядок `Do` — порядок элементов раскладки: диспетчер дописывает
        // операции в конец, а плавающих рисунков у фикстуры нет.
        let (_, matrix) = matrices[index];
        let (x, y, w, h) = matrix_rect(matrix);
        let expected = expected_matrix(*rect, page_height);
        let (ex, ey, ew, eh) = matrix_rect(expected);
        assert!(
            (x - ex).abs() <= TOLERANCE_PT
                && (y - ey).abs() <= TOLERANCE_PT
                && (w - ew).abs() <= TOLERANCE_PT
                && (h - eh).abs() <= TOLERANCE_PT,
            "{name}: рисунок {index} лёг в ({x}, {y}, {w}, {h}) вместо ({ex}, {ey}, {ew}, {eh})"
        );
    }
}

/// Рисунок без байтов не рисуется и не ломает поток страницы.
///
/// `image_id: None` раскладка отдаёт, когда части нет в пакете (внешняя цель,
/// битая часть, часть выше предела). Поведение экспорта до среза — на месте
/// рисунка ничего: места в раскладке он уже занял, но показывать нечего.
#[test]
fn image_without_bytes_draws_nothing_and_keeps_the_page() {
    let name = "images/inline.docx";
    let mut doc = document(name);
    doc.media.clear();

    let layout = layout(&doc);
    let (images, _) = layout_images(&layout);
    assert_eq!(images.len(), 1, "{name}: рисунок в раскладке остался");
    assert_eq!(images[0].0, None, "{name}: байтов у рисунка больше нет");

    let bytes = export(&doc);
    let pdf = Pdf::load_mem(&bytes).expect("PDF разбирается lopdf");
    let (xobjects, draws) = document_images(&pdf);
    assert!(xobjects.is_empty(), "{name}: без байтов XObject'ов нет");
    assert_eq!(draws, 0, "{name}: без байтов нет и `Do`");
    assert_eq!(
        pdf.get_pages().len(),
        layout.pages.len(),
        "{name}: страницы на месте"
    );
    // Поток страницы разбирается (у абзаца без текста он пуст): пустая строка
    // под курсором структуру PDF не ломает.
    let content = page_ops(&bytes, 0);
    assert!(
        content.operations.iter().all(|op| op.operator != "Do"),
        "{name}: без байтов нет и `Do`"
    );
}

/// Рисунок в ячейке таблицы: экспорт рисует ровно то, что положила раскладка.
///
/// Фикстуры с рисунком внутри таблицы нет, поэтому абзац с рисунком переносится
/// в единственную ячейку собранной в тесте таблицы, а ячейки рисует тот же
/// диспетчер (`tables::draw` → `draw_items`). Сегодня `LayoutItem::Image` в
/// содержимое ячейки не попадает вовсе: блоки ячейки меряет `MeasuredBlock`, а
/// он знает только строки текста и вложенные таблицы (`layout::tables`) —
/// встроенный рисунок в ячейке раскладка теряет, и теряют его оба пути, canvas и
/// PDF, одинаково. Проверка сторожит инвариант экспорта — число `Do` равно
/// числу рисунков с байтами в раскладке, — и сработает, когда раскладка начнёт
/// рисунки в ячейках кладть: тогда к ней добавится сверка прямоугольника.
#[test]
fn cell_drawing_is_drawn_exactly_when_the_layout_has_it() {
    let name = "images/inline.docx";
    let mut doc = document(name);

    // Абзац с рисунком уходит в ячейку; всё остальное тело (в том числе
    // `SectPr`) остаётся на месте.
    let mut cell_items = Vec::new();
    let mut body = Vec::new();
    for item in std::mem::take(&mut doc.body.items) {
        match item {
            BlockItem::Paragraph(paragraph) if has_drawing(&paragraph) => {
                cell_items.push(BlockItem::Paragraph(paragraph));
            }
            other => body.push(other),
        }
    }
    assert_eq!(
        cell_items.len(),
        1,
        "{name}: у фикстуры один абзац с рисунком"
    );
    doc.body.items = body;
    doc.body.items.push(BlockItem::Table(a_table(cell_items)));

    let layout = layout(&doc);
    let (images, _) = layout_images(&layout);
    let with_bytes = images.iter().filter(|(id, _)| id.is_some()).count();

    let bytes = export(&doc);
    let pdf = Pdf::load_mem(&bytes).expect("PDF разбирается lopdf");
    // `XObject`'ы регистрируются по медиатаблице документа и остаются в
    // ресурсах, даже если раскладка рисунок потеряла, — поэтому счёт ведётся по
    // `Do`, а не по словарю ресурсов.
    let (xobjects, draws) = document_images(&pdf);
    assert_eq!(draws, with_bytes, "{name}: каждый рисунок выведен один раз");
    assert!(
        xobjects.len() >= with_bytes,
        "{name}: XObject'ов меньше, чем рисунков с байтами"
    );
    assert_eq!(
        pdf.get_pages().len(),
        layout.pages.len(),
        "{name}: страницы на месте"
    );

    let content = page_ops(&bytes, 0);
    let drawings = do_matrices(&content);
    assert_eq!(drawings.len(), with_bytes, "{name}: число `Do` на странице");
    for ((_, rect), (_, matrix)) in images
        .iter()
        .filter(|(id, _)| id.is_some())
        .zip(drawings.iter())
    {
        let (x, y, w, h) = matrix_rect(*matrix);
        let (ex, ey, ew, eh) = matrix_rect(expected_matrix(*rect, layout.pages[0].height));
        assert!(
            (x - ex).abs() <= TOLERANCE_PT
                && (y - ey).abs() <= TOLERANCE_PT
                && (w - ew).abs() <= TOLERANCE_PT
                && (h - eh).abs() <= TOLERANCE_PT,
            "{name}: рисунок лёг в ({x}, {y}, {w}, {h}) вместо ({ex}, {ey}, {ew}, {eh})"
        );
    }
}

/// Таблица с одной ячейкой, содержимое — `items`.
///
/// Поля повторяют `synthetic_table` движка раскладки: ширины сетки не заданы,
/// границы и оформление — умолчания `WordprocessingML`.
fn a_table(items: Vec<BlockItem>) -> Table {
    Table {
        id: NodeId::new(9000),
        style_ref: None,
        grid: vec![GridCol {
            width: Twips::new(6000),
        }],
        rows: vec![Row {
            id: NodeId::new(9001),
            cells: vec![Cell {
                id: NodeId::new(9002),
                grid_span: 1,
                v_merge: None,
                width: None,
                margins: CellMargins::default(),
                v_align: CellVAlign::Top,
                borders: Box::new(CellBorders::default()),
                shading: None,
                items,
            }],
            height: None,
            cant_split: false,
            header: false,
        }],
        layout: TableLayout::Autofit,
        width: None,
        borders: TableBorders::default(),
        look: TableLook::default(),
        jc: None,
        indent: None,
        cell_margins: CellMargins::default(),
        bidi_visual: false,
    }
}

/// Есть ли в абзаце рисунок (`w:drawing`) — на верхнем уровне или в прогоне.
fn has_drawing(paragraph: &doc_converter_docx::Paragraph) -> bool {
    fn walk(inlines: &[doc_converter_docx::Inline]) -> bool {
        inlines.iter().any(|inline| match inline {
            doc_converter_docx::Inline::Drawing(_) => true,
            doc_converter_docx::Inline::Run(run) => run
                .content
                .iter()
                .any(|content| matches!(content, doc_converter_docx::RunContent::Drawing(_))),
            doc_converter_docx::Inline::Hyperlink(link) => walk(&link.runs),
            _ => false,
        })
    }
    walk(&paragraph.runs)
}
