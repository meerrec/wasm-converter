#![deny(clippy::pedantic)]
// модуль включается сюда `#[path]`-ом — пусть CI держит его pedantic-чистым
//! Тесты задачи F: разбор кодов колонтитулов, геометрия полос, водяной знак.
//!
//! Модуль подключается `#[path]`-ом: регистрация `mod overlay;` в `lib.rs` —
//! часть интеграции (следующая волна), а тесты обязаны исполняться уже сейчас.
//! Модуль от остального крейта не зависит, поэтому такое включение безопасно.

#[path = "../src/overlay.rs"]
mod overlay;

use overlay::{
    anchor_x, clamp_opacity, estimate_text_width, footer_band, footer_baseline, footer_rule,
    header_band, header_baseline, header_rule, needs_extgstate, parse_header_footer,
    rotation_matrix, watermark_draw, Align, Band, HeaderFooterParts, OverlayConfig, OverlayMargins,
    PageContext, Watermark, WatermarkLayer,
};

/// Точность сравнения координат — доли точки.
const EPS: f32 = 1e-3;

/// A4 книжной ориентации в точках.
const A4_W: f32 = 595.275_6;
const A4_H: f32 = 841.889_8;

/// Поля по умолчанию из `PageConfig`: 20/15/20/15 мм.
fn default_margins() -> OverlayMargins {
    OverlayMargins::from_mm(20.0, 15.0, 20.0, 15.0)
}

#[track_caller]
fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < EPS,
        "ожидалось {expected}, получено {actual}"
    );
}

// --- разбор строки колонтитула ---

#[test]
fn parses_all_codes() {
    let ctx = PageContext {
        date: Some("06.10.2026"),
        file_name: Some("book.xlsx"),
        ..PageContext::new(3, 7)
    };
    let parts = parse_header_footer("&Lстр. &P из &N&C&F&R&D", ctx);
    assert_eq!(
        parts,
        HeaderFooterParts {
            left: "стр. 3 из 7".to_string(),
            center: "book.xlsx".to_string(),
            right: "06.10.2026".to_string(),
        }
    );
}

#[test]
fn empty_string_gives_empty_parts() {
    let parts = parse_header_footer("", PageContext::new(1, 1));
    assert!(parts.is_empty());
    assert_eq!(parts, HeaderFooterParts::default());
}

#[test]
fn text_without_codes_goes_to_center() {
    let parts = parse_header_footer("Отчёт за квартал", PageContext::new(1, 2));
    assert_eq!(parts.center, "Отчёт за квартал");
    assert!(parts.left.is_empty() && parts.right.is_empty());
}

#[test]
fn only_center_section() {
    let parts = parse_header_footer("&CЛист 1", PageContext::new(1, 2));
    assert_eq!(parts.center, "Лист 1");
}

#[test]
fn sections_switch_and_accumulate() {
    let parts = parse_header_footer("&Lслева&Rсправа&Cцентр&L ещё", PageContext::new(1, 1));
    assert_eq!(parts.left, "слева ещё");
    assert_eq!(parts.center, "центр");
    assert_eq!(parts.right, "справа");
}

#[test]
fn ampersand_is_escaped_by_doubling() {
    let ctx = PageContext::new(4, 9);
    assert_eq!(parse_header_footer("&L&&", ctx).left, "&");
    assert_eq!(parse_header_footer("&C&&", ctx).center, "&");
    assert_eq!(parse_header_footer("&R&&", ctx).right, "&");
    // `&&P` — амперсанд и литера `P`, а не номер страницы.
    assert_eq!(parse_header_footer("&L&&P", ctx).left, "&P");
}

#[test]
fn unknown_code_and_trailing_ampersand_stay_literal() {
    let parts = parse_header_footer("&L&Z&", PageContext::new(1, 1));
    assert_eq!(parts.left, "&Z&");
}

#[test]
fn page_number_and_total_are_substituted() {
    for page_no in 1..=3 {
        let parts = parse_header_footer("&C&P/&N", PageContext::new(page_no, 3));
        assert_eq!(parts.center, format!("{page_no}/3"));
    }
}

#[test]
fn missing_date_and_file_expand_to_empty() {
    let parts = parse_header_footer("&L&D&F", PageContext::new(1, 1));
    assert!(parts.left.is_empty());
}

// --- геометрия полос ---

#[test]
fn header_band_sits_in_top_margin() {
    let margins = default_margins();
    let band = header_band(A4_W, A4_H, margins);
    close(band.x, margins.left);
    close(band.width, A4_W - margins.left - margins.right);
    // Низ полосы — верх области содержимого, верх полосы — край бумаги.
    close(band.y, A4_H - margins.top);
    close(band.y + band.height, A4_H);
}

#[test]
fn footer_band_sits_in_bottom_margin() {
    let margins = default_margins();
    let band = footer_band(A4_W, margins);
    close(band.y, 0.0);
    close(band.height, margins.bottom);
    close(band.x, margins.left);
    close(band.width, A4_W - margins.left - margins.right);
}

#[test]
fn header_text_stays_off_the_content() {
    let band = header_band(A4_W, A4_H, default_margins());
    let size = 9.0;
    let baseline = header_baseline(band, size);
    // Коробка строки (кегль) целиком внутри полосы, то есть выше содержимого.
    assert!(baseline - size * 0.8 >= band.y - EPS);
    assert!(baseline + size * 0.2 <= band.y + band.height + EPS);
}

#[test]
fn footer_text_stays_off_the_content() {
    let band = footer_band(A4_W, default_margins());
    let size = 9.0;
    let baseline = footer_baseline(band, size);
    assert!(baseline - size * 0.8 >= band.y - EPS);
    assert!(baseline + size * 0.2 <= band.y + band.height + EPS);
}

#[test]
fn oversized_font_is_pinned_to_the_paper_edge() {
    let header = header_band(A4_W, A4_H, default_margins());
    let size = header.height + 20.0;
    let baseline = header_baseline(header, size);
    // Верх коробки прижат к краю бумаги, а не к области содержимого.
    close(baseline - size * 0.8 + size, header.y + header.height);

    let footer = footer_band(A4_W, default_margins());
    let baseline = footer_baseline(footer, size);
    // Низ коробки — край бумаги.
    close(baseline - size * 0.8, 0.0);
}

#[test]
fn rules_sit_on_the_content_border() {
    let margins = default_margins();
    let header = header_band(A4_W, A4_H, margins);
    let footer = footer_band(A4_W, margins);
    let rule = header_rule(header);
    close(rule.y, header.y);
    close(rule.x0, margins.left);
    close(rule.x1, A4_W - margins.right);
    close(footer_rule(footer).y, footer.y + footer.height);
}

#[test]
fn anchor_respects_alignment() {
    let band = Band {
        x: 100.0,
        y: 0.0,
        width: 400.0,
        height: 50.0,
    };
    close(anchor_x(band, Align::Left, 50.0), 100.0);
    close(anchor_x(band, Align::Center, 50.0), 275.0);
    close(anchor_x(band, Align::Right, 50.0), 450.0);
}

#[test]
fn overflowing_text_is_not_shifted_left() {
    let band = Band {
        x: 100.0,
        y: 0.0,
        width: 400.0,
        height: 50.0,
    };
    close(anchor_x(band, Align::Center, 500.0), 100.0);
    close(anchor_x(band, Align::Right, 500.0), 100.0);
}

// --- водяной знак ---

#[test]
fn watermark_box_center_coincides_with_page_center() {
    let watermark = Watermark {
        text: "ЧЕРНОВИК".to_string(),
        ..Watermark::default()
    };
    let draw = watermark_draw(600.0, 800.0, &watermark).expect("текст задан");
    let [a, b, _, _, tx, ty] = draw.matrix;
    let half = estimate_text_width(&watermark.text, watermark.font_size_pt) / 2.0;
    let half_cap = watermark.font_size_pt * 0.35;
    // Обратное преобразование: центр коробки текста в системе страницы.
    close(tx + half * a - half_cap * b, 300.0);
    close(ty + half * b + half_cap * a, 400.0);
}

#[test]
fn watermark_matrix_encodes_the_angle() {
    let watermark = Watermark {
        text: "DRAFT".to_string(),
        angle_deg: 90.0,
        ..Watermark::default()
    };
    let draw = watermark_draw(400.0, 400.0, &watermark).expect("текст задан");
    // 90° против часовой стрелки: локальная ось X смотрит вверх.
    close(draw.matrix[0], 0.0);
    close(draw.matrix[1], 1.0);
    close(draw.matrix[2], -1.0);
    close(draw.matrix[3], 0.0);
}

#[test]
fn rotation_matrix_at_zero_is_translation() {
    let m = rotation_matrix(0.0, 5.0, 7.0);
    close(m[0], 1.0);
    close(m[1], 0.0);
    close(m[2], 0.0);
    close(m[3], 1.0);
    close(m[4], 5.0);
    close(m[5], 7.0);
}

#[test]
fn opacity_is_clamped_and_drives_extgstate() {
    close(clamp_opacity(-0.5), 0.0);
    close(clamp_opacity(1.5), 1.0);
    close(clamp_opacity(f32::NAN), 1.0);
    assert!(needs_extgstate(0.15));
    assert!(!needs_extgstate(1.0));
    assert!(needs_extgstate(-1.0));

    let watermark = Watermark {
        text: "X".to_string(),
        opacity: 2.0,
        ..Watermark::default()
    };
    let draw = watermark_draw(100.0, 100.0, &watermark).expect("текст задан");
    close(draw.opacity, 1.0);
    assert!(!needs_extgstate(draw.opacity));
}

#[test]
fn empty_watermark_text_draws_nothing() {
    assert!(watermark_draw(100.0, 100.0, &Watermark::default()).is_none());
    let whitespace = Watermark {
        text: "   ".to_string(),
        ..Watermark::default()
    };
    assert!(watermark_draw(100.0, 100.0, &whitespace).is_none());
}

#[test]
fn watermark_layer_is_carried_to_the_draw() {
    assert_eq!(Watermark::default().layer, WatermarkLayer::Under);
    let watermark = Watermark {
        text: "X".to_string(),
        layer: WatermarkLayer::Over,
        ..Watermark::default()
    };
    let draw = watermark_draw(100.0, 100.0, &watermark).expect("текст задан");
    assert_eq!(draw.layer, WatermarkLayer::Over);
}

#[test]
fn width_estimate_scales_with_font_and_length() {
    close(estimate_text_width("abc", 10.0), 18.0);
    close(estimate_text_width("", 10.0), 0.0);
    close(estimate_text_width("ab", 20.0), 24.0);
}

// --- конфигурация ---

#[test]
fn config_is_empty_until_something_is_set() {
    let mut config = OverlayConfig::default();
    assert!(config.is_empty());
    config.header = "&C&P".to_string();
    assert!(!config.is_empty());

    let only_empty_watermark = OverlayConfig {
        watermark: Some(Watermark::default()),
        ..OverlayConfig::default()
    };
    assert!(only_empty_watermark.is_empty());

    let with_watermark = OverlayConfig {
        watermark: Some(Watermark {
            text: "CONFIDENTIAL".to_string(),
            ..Watermark::default()
        }),
        ..OverlayConfig::default()
    };
    assert!(!with_watermark.is_empty());
}

// --- сквозные проверки: колонтитулы и водяной знак в собранном PDF ---

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use doc_converter_pdf::overlay::Watermark as CrateWatermark;
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_xlsx::Workbook;
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object};

/// Имя `/GS`, которым painter выражает прозрачность знака.
const WATERMARK_GS: &[u8] = b"WatermarkAlpha";

/// Путь к книге в общем наборе фикстур.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Открыть книгу из фикстур.
fn open_book(name: &str) -> Workbook {
    let path = fixture(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{name}: {err}"))
}

/// Экспортировать лист 0 книги с настройками, которые правит `configure`.
fn export_with(name: &str, configure: impl FnOnce(&mut PdfOptions)) -> Vec<u8> {
    let book = open_book(name);
    let mut options = PdfOptions::default();
    configure(&mut options);
    PdfExporter::new(options)
        .export_xlsx_sheet(&book, 0)
        .unwrap_or_else(|err| panic!("{name} не экспортировалась: {err}"))
}

/// PDF разбирается `lopdf`.
fn parse(bytes: &[u8]) -> Document {
    Document::load_mem(bytes).expect("PDF разбирается lopdf")
}

/// Номера страниц документа по возрастанию.
fn pages(doc: &Document) -> Vec<u32> {
    doc.get_pages().keys().copied().collect()
}

/// Операции страницы (номер — с единицы).
fn page_operations(doc: &Document, number: u32) -> Vec<Operation> {
    let page = *doc
        .get_pages()
        .get(&number)
        .unwrap_or_else(|| panic!("страницы {number} нет"));
    let bytes = doc.get_page_content(page).expect("поток содержимого");
    Content::decode(&bytes)
        .expect("операции разбираются")
        .operations
}

/// Число из операнда lopdf.
#[allow(
    clippy::cast_precision_loss,
    reason = "единственный вызов — альфа /GS фикстур в 0..1, где i64 → f32 точен"
)]
fn operand_number(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("операнд не число: {other:?}"),
    }
}

/// Текст страницы (с единицы), восстановленный картами `ToUnicode` её шрифтов.
///
/// `lopdf::extract_text` на PDF printpdf падает (см. `tests/export.rs`), поэтому
/// здесь повторяется то, что делает `pdftotext`: hex-строки перед `Tj`
/// переводятся картой шрифта обратно в символы.
fn page_text(doc: &Document, number: u32) -> String {
    let page = *doc
        .get_pages()
        .get(&number)
        .unwrap_or_else(|| panic!("страницы {number} нет"));
    let cmaps = page_cmaps(doc, page);
    let content = doc.get_page_content(page).expect("поток содержимого");
    let content = String::from_utf8_lossy(&content).into_owned();
    let mut events: Vec<(usize, &str)> = Vec::new();
    events.extend(content.match_indices("Tf"));
    events.extend(content.match_indices("Tj"));
    events.sort_by_key(|(at, _)| *at);

    let mut font = String::new();
    let mut out = String::new();
    for (at, token) in events {
        if token == "Tf" {
            if let Some(slash) = content[..at].rfind('/') {
                content[slash + 1..at]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .clone_into(&mut font);
            }
            continue;
        }
        let Some(open) = content[..at].rfind('<') else {
            continue;
        };
        let Some(close) = content[open..at].find('>') else {
            continue;
        };
        for pair in content.as_bytes()[open + 1..open + close].chunks(4) {
            let Ok(code) = u16::from_str_radix(&String::from_utf8_lossy(pair), 16) else {
                continue;
            };
            if let Some(ch) = cmaps.get(&font).and_then(|map| map.get(&code)) {
                out.push(*ch);
            }
        }
        out.push(' ');
    }
    out
}

/// Карта `ToUnicode` каждого шрифта страницы: имя ресурса → «код → символ».
fn page_cmaps(doc: &Document, page: lopdf::ObjectId) -> HashMap<String, HashMap<u16, char>> {
    let mut cmaps = HashMap::new();
    for (name, font) in doc.get_page_fonts(page).expect("шрифты страницы") {
        let Ok(to_unicode) = font.get(b"ToUnicode") else {
            continue;
        };
        let Ok(id) = to_unicode.as_reference() else {
            continue;
        };
        let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
            continue;
        };
        let Ok(content) = stream.get_plain_content() else {
            continue;
        };
        cmaps.insert(
            String::from_utf8_lossy(&name).into_owned(),
            parse_bfchar(&String::from_utf8_lossy(&content)),
        );
    }
    cmaps
}

/// Пары «код → символ» из `beginbfchar`-секции `CMap`.
fn parse_bfchar(cmap: &str) -> HashMap<u16, char> {
    let mut map = HashMap::new();
    let mut inside = false;
    for line in cmap.lines() {
        let line = line.trim();
        if line.ends_with("beginbfchar") {
            inside = true;
        } else if line == "endbfchar" {
            inside = false;
        } else if inside {
            let Some((source, target)) = line.split_once(' ') else {
                continue;
            };
            let code = u16::from_str_radix(source.trim_matches(['<', '>']), 16);
            let value = u16::from_str_radix(target.trim_matches(['<', '>']), 16);
            if let (Ok(code), Ok(value)) = (code, value) {
                if let Some(ch) = char::from_u32(u32::from(value)) {
                    map.insert(code, ch);
                }
            }
        }
    }
    map
}

/// Словарь `/ExtGState` страницы, если он есть.
fn extgstates(doc: &Document, number: u32) -> Option<&lopdf::Dictionary> {
    let page = *doc.get_pages().get(&number).expect("страница есть");
    let (direct, indirect) = doc.get_page_resources(page).expect("ресурсы страницы");
    let resources = match (direct, indirect.first()) {
        (Some(dict), _) => dict,
        (None, Some(id)) => doc.get_dictionary(*id).expect("словарь ресурсов"),
        (None, None) => panic!("у страницы нет ресурсов"),
    };
    let mut value = resources.get(b"ExtGState").ok()?;
    while let Object::Reference(id) = value {
        value = doc.get_object(*id).expect("ссылка разрешается");
    }
    value.as_dict().ok()
}

/// Колонтитул доходит до текста страницы, а `&P`/`&N` разворачиваются после
/// пагинации: номер в шапке совпадает с номером страницы в PDF.
#[test]
fn header_footer_reach_page_text() {
    let bytes = export_with("scale-ten-pages.xlsx", |options| {
        options.overlay.header = "&Lстр. &P из &N".to_string();
        options.overlay.footer = "&CСвод &P".to_string();
    });
    let doc = parse(&bytes);
    let numbers = pages(&doc);
    let total = numbers.len();
    assert!(
        total >= 3,
        "фикстура должна быть многостраничной, а страниц {total}"
    );

    let first = page_text(&doc, numbers[0]);
    assert!(
        first.contains(&format!("стр. {} из {total}", numbers[0])),
        "шапка первой страницы: {first:?}"
    );
    assert!(
        first.contains(&format!("Свод {}", numbers[0])),
        "подвал первой страницы: {first:?}"
    );

    let last_number = *numbers.last().expect("страницы есть");
    let last = page_text(&doc, last_number);
    assert!(
        last.contains(&format!("стр. {last_number} из {total}")),
        "шапка последней страницы: {last:?}"
    );
}

/// Колонтитулы самой книги (`oddHeader`/`oddFooter`) печатаются и без
/// настроек: экспорт не теряет то, что задано в файле.
#[test]
fn workbook_header_reaches_page_text() {
    let book = open_book("content-mixed-types.xlsx");
    let mut sheet = book.sheets()[0].clone();
    sheet.print.odd_header = Some("&CОтчёт &P из &N".to_string());
    sheet.print.odd_footer = Some("&Lиз книги".to_string());
    let book = Workbook::new(
        vec![sheet],
        book.shared_strings().clone(),
        book.styles().clone(),
        book.theme().clone(),
        book.date1904(),
    )
    .with_images(book.images().to_vec());

    let bytes = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("PDF собирается");
    let doc = parse(&bytes);
    let text = page_text(&doc, 1);
    assert!(text.contains("Отчёт 1 из 1"), "шапка книги: {text:?}");
    assert!(text.contains("из книги"), "подвал книги: {text:?}");
}

/// Водяной знак присутствует на каждой странице: текст, `Tm`-матрица и `/GS`
/// при прозрачности.
#[test]
fn watermark_reaches_every_page_with_extgstate() {
    let bytes = export_with("content-mixed-types.xlsx", |options| {
        options.overlay.watermark = Some(CrateWatermark {
            text: "ЧЕРНОВИК".to_string(),
            opacity: 0.25,
            ..CrateWatermark::default()
        });
    });
    let doc = parse(&bytes);
    let numbers = pages(&doc);
    assert!(!numbers.is_empty(), "страницы есть");

    for &number in &numbers {
        let ops = page_operations(&doc, number);
        let matrix = ops
            .iter()
            .find(|op| op.operator == "Tm")
            .unwrap_or_else(|| panic!("на странице {number} нет матрицы Tm"));
        assert_eq!(matrix.operands.len(), 6, "Tm — шесть чисел");

        let gs = ops
            .iter()
            .find(|op| {
                op.operator == "gs"
                    && op.operands.first().and_then(|o| o.as_name().ok()) == Some(WATERMARK_GS)
            })
            .unwrap_or_else(|| panic!("на странице {number} нет /GS водяного знака"));

        assert!(
            page_text(&doc, number).contains("ЧЕРНОВИК"),
            "текста знака нет на странице {number}"
        );

        let extgstates = extgstates(&doc, number).expect("словарь /ExtGState");
        let name = gs.operands[0].as_name().expect("имя /GS");
        let state = extgstates.get(name).expect("ресурс /GS есть в словаре");
        let mut state = state;
        while let Object::Reference(id) = state {
            state = doc.get_object(*id).expect("ссылка разрешается");
        }
        let state = state.as_dict().expect("/GS — словарь");
        let alpha = state
            .get(b"CA")
            .map(operand_number)
            .expect("альфа заливки в /GS");
        assert!(
            (alpha - 0.25).abs() < 1e-6,
            "альфа знака 0.25, а в /GS {alpha}"
        );
    }
}

/// Непрозрачный водяной знак не заводит `/GS`: сплошной заливке он не нужен.
#[test]
fn opaque_watermark_needs_no_extgstate() {
    let bytes = export_with("content-mixed-types.xlsx", |options| {
        options.overlay.watermark = Some(CrateWatermark {
            text: "ЧЕРНОВИК".to_string(),
            opacity: 1.0,
            ..CrateWatermark::default()
        });
    });
    let doc = parse(&bytes);
    let ops = page_operations(&doc, 1);
    assert!(
        ops.iter().any(|op| op.operator == "Tm"),
        "матрица поворота есть"
    );
    assert!(
        !ops.iter().any(|op| op.operator == "gs"),
        "непрозрачному знаку /GS не нужен"
    );
    assert!(page_text(&doc, 1).contains("ЧЕРНОВИК"));
}
