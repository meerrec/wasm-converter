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
