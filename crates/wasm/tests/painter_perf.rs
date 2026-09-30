#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn paint_1000_rects_under_16ms() {
    let canvas = web_sys::OffscreenCanvas::new(1200, 800).unwrap();
    let ctx = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::OffscreenCanvasRenderingContext2d>()
        .unwrap();

    doc_converter_wasm::painter_api::init_painter(ctx).unwrap();

    use doc_converter_render::display_list::*;
    let mut dl = DisplayList::with_capacity(1000);
    for i in 0..1000 {
        dl.push(DrawCommand::Rect {
            x: (i % 100) as f32 * 10.0,
            y: (i / 100) as f32 * 10.0,
            w: 10.0,
            h: 10.0,
            fill: Color::rgba(0, 0, 0, 255),
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        });
    }
    let bytes = dl.to_bytes();
    let stats = doc_converter_wasm::painter_api::paint_display_list_bytes(&bytes).unwrap();
    let ms = js_sys::Reflect::get(&stats, &"paintMs".into())
        .unwrap()
        .as_f64()
        .unwrap();
    assert!(ms < 16.0, "paintMs = {ms}");
}
