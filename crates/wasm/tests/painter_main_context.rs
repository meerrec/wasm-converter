#![cfg(target_arch = "wasm32")]

//! Main-thread fallback: painter обязан принимать обычный
//! `CanvasRenderingContext2d`, а не только OffscreenCanvas-контекст воркера.
//!
//! Проверяются два контракта `OffscreenPainter::install_js`: настоящий
//! `<canvas>` из документа действительно получает кадр (пиксель красный), а
//! посторонние объекты отвергаются duck-типизацией до `unchecked_into`.

use doc_converter_render::canvas::OffscreenPainter;
use doc_converter_render::display_list::{Color, DisplayList, DrawCommand};
use doc_converter_wasm::painter_api::{
    dispose_painter, init_painter_main, paint_display_list_bytes,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

wasm_bindgen_test_configure!(run_in_browser);

const SIDE: u32 = 32;

/// `get_context("2d")` на `<canvas>` возвращает `Object`; приводим его к
/// типизированному контексту, чтобы вызвать `get_image_data`.
fn context_2d(canvas: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    canvas
        .get_context("2d")
        .expect("getContext не должен бросать")
        .expect("для 2d-контекста getContext возвращает объект")
        .dyn_into::<CanvasRenderingContext2d>()
        .expect("getContext(\"2d\") даёт CanvasRenderingContext2d")
}

#[wasm_bindgen_test]
fn main_thread_canvas_gets_painted() {
    // Painter — singleton на wasm-модуль: если соседний тест в этом файле
    // (или повторный запуск в той же странице) оставил экземпляр, install
    // вернул бы «already initialised» и упал бы по чужой вине.
    dispose_painter();

    let document = web_sys::window()
        .expect("в браузере есть window")
        .document()
        .expect("у window есть document");
    let canvas = document
        .create_element("canvas")
        .expect("create_element(canvas)")
        .dyn_into::<HtmlCanvasElement>()
        .expect("createElement(\"canvas\") даёт HtmlCanvasElement");
    canvas.set_width(SIDE);
    canvas.set_height(SIDE);

    let ctx = context_2d(&canvas);
    init_painter_main(ctx).expect("main-thread контекст должен пройти duck-типизацию");

    // Красный прямоугольник на весь холст. Холст создан пустым (прозрачный),
    // поэтому после заливки единственный проверяемый пиксель обязан быть
    // красным и непрозрачным.
    let mut dl = DisplayList::with_capacity(1);
    dl.push(DrawCommand::Rect {
        x: 0.0,
        y: 0.0,
        w: SIDE as f32,
        h: SIDE as f32,
        fill: Color::rgba(255, 0, 0, 255),
        stroke: Color::TRANSPARENT,
        stroke_w: 0.0,
        radius: [0.0; 4],
    });
    paint_display_list_bytes(&dl.to_bytes()).expect("кадр должен нарисоваться на main-контексте");

    let pixel = context_2d(&canvas)
        .get_image_data(0.0, 0.0, 1.0, 1.0)
        .expect("getImageData")
        .data()
        .0;
    assert_eq!(pixel.len(), 4, "один пиксель — четыре канала");
    let (r, g, b, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);

    // Порядок каналов в `ImageData` по спецификации RGBA, но проверка через
    // доминирование красного не зависит от того, какую раскладку вернул
    // браузер: у чистого красного r заведомо больше g и b при любой из них.
    assert!(
        u32::from(r) > 200
            && u32::from(r) > u32::from(g) + 100
            && u32::from(r) > u32::from(b) + 100,
        "пиксель должен быть красным, получено RGBA({r}, {g}, {b}, {a})"
    );
    assert!(
        a > 200,
        "пиксель должен быть непрозрачным, получено RGBA({r}, {g}, {b}, {a})"
    );

    // Singleton не должен переживать тест: следующие тесты модуля ставят
    // painter заново.
    dispose_painter();
}

#[wasm_bindgen_test]
fn install_js_rejects_non_canvas_objects() {
    // Обычный объект без свойства canvas: до unchecked_into дело доходить
    // не должно.
    let err = OffscreenPainter::install_js(JsValue::from(js_sys::Object::new()))
        .expect_err("посторонний объект обязан быть отвергнут");
    let msg = err.as_string().unwrap_or_default();
    assert!(
        msg.contains("Canvas2D"),
        "ошибка должна объяснять, что это не Canvas2D-контекст, получено: {msg}"
    );

    // Объект с полем canvas, но без методов контекста: duck-типизация обязана
    // назвать первый отсутствующий метод, а не молча принять объект.
    let fake = js_sys::Object::new();
    js_sys::Reflect::set(&fake, &JsValue::from_str("canvas"), &js_sys::Object::new())
        .expect("Reflect::set");
    let err = OffscreenPainter::install_js(fake.into())
        .expect_err("объект без методов контекста обязан быть отвергнут");
    let msg = err.as_string().unwrap_or_default();
    assert!(
        msg.contains("is not a function"),
        "ошибка должна назвать отсутствующий метод, получено: {msg}"
    );
}
