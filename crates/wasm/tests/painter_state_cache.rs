#![cfg(target_arch = "wasm32")]

//! Регрессия: кэш стилей painter'а обязан совпадать с состоянием контекста
//! после `restore()`.
//!
//! Кадр страницы DOCX (`docx::render::build`) открывается клипом видимой части,
//! внутри клипа идёт белая заливка страницы и текст; `PopClip` откатывает
//! `fillStyle` к состоянию на `save()`. Если кэш об этом не знает, второй кадр
//! заливает страницу цветом текста первого.

use doc_converter_render::display_list::{
    Color, DisplayList, DrawCommand, TextAlign, TextBaseline,
};
use doc_converter_render::painter::Painter2D;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SIDE: u32 = 64;

/// Кадр в том же порядке, в каком его собирает рендер страницы DOCX.
fn page() -> Vec<u8> {
    let mut dl = DisplayList::with_capacity(5);
    let font = dl.intern("Calibri");
    let text = dl.intern("Aa");
    dl.push(DrawCommand::Clear);
    dl.push(DrawCommand::PushClip {
        x: 0.0,
        y: 0.0,
        w: SIDE as f32,
        h: SIDE as f32,
    });
    dl.push(DrawCommand::Rect {
        x: 0.0,
        y: 0.0,
        w: SIDE as f32,
        h: SIDE as f32,
        fill: Color::WHITE,
        stroke: Color::TRANSPARENT,
        stroke_w: 0.0,
        radius: [0.0; 4],
    });
    dl.push(DrawCommand::Text {
        x: 16.0,
        y: 8.0,
        text,
        font,
        size: 12.0,
        color: Color::BLACK,
        align: TextAlign::Left,
        baseline: TextBaseline::Top,
        bold: false,
        italic: false,
        underline: false,
    });
    dl.push(DrawCommand::PopClip);
    dl.to_bytes()
}

/// Пиксель `(x, y)` как RGBA.
fn pixel(painter: &Painter2D, x: f64, y: f64) -> [u8; 4] {
    let data = painter
        .context()
        .get_image_data(x, y, 1.0, 1.0)
        .unwrap()
        .data()
        .0;
    [data[0], data[1], data[2], data[3]]
}

#[wasm_bindgen_test]
fn repainted_page_stays_white() {
    let canvas = web_sys::OffscreenCanvas::new(SIDE, SIDE).unwrap();
    let ctx = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::OffscreenCanvasRenderingContext2d>()
        .unwrap();
    let mut painter = Painter2D::new(ctx);

    // Пиксель левого верхнего угла страницы: текст рисуется правее и ниже.
    let (x, y) = (2.0, 2.0);
    let bytes = page();
    painter.paint_bytes(&bytes).unwrap();
    assert_eq!(pixel(&painter, x, y), [255, 255, 255, 255], "первый кадр");
    painter.paint_bytes(&bytes).unwrap();
    assert_eq!(pixel(&painter, x, y), [255, 255, 255, 255], "второй кадр");
}
