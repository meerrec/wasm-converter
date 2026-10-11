#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Прогревающие проходы: первый кадр в headless Chrome рисуется холодным
/// (canvas, JIT), их результат в замер не идёт.
const WARMUP_PASSES: usize = 2;
/// Замеров для медианы: одиночный замер ловит разброс среды раннера, медиана
/// его гасит.
const MEASURED_PASSES: usize = 5;
/// Мягкий гейт; худшее наблюдённое — 138 мс (подробности у ассерта).
const SOFT_GATE_MS: f64 = 300.0;

#[wasm_bindgen_test]
fn paint_1000_rects_under_soft_gate() {
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

    let paint_ms = || {
        let stats = doc_converter_wasm::painter_api::paint_display_list_bytes(&bytes).unwrap();
        js_sys::Reflect::get(&stats, &"paintMs".into())
            .unwrap()
            .as_f64()
            .unwrap()
    };

    for _ in 0..WARMUP_PASSES {
        let _ = paint_ms();
    }
    let mut samples: Vec<f64> = (0..MEASURED_PASSES).map(|_| paint_ms()).collect();
    samples.sort_by(f64::total_cmp);
    let median = samples[MEASURED_PASSES / 2];
    // Вывод консоли tests-runner'а виден в логе CI — по следующим прогонам будет
    // судить о разбросе и, если он уляжется, ужимать порог.
    web_sys::console::log_1(
        &format!("painter_perf: paintMs samples = {samples:?}, median = {median}").into(),
    );

    // §9 написан для поставляемой сборки на живой машине, а headless Chrome на
    // раннере рисует программно: здесь меряется среда, а не сборка — одно и то
    // же число даёт 32 мс в debug, 82 мс в release (b40771e) и 138 мс на
    // прогоне 02c8de2, где менялся только crates/pdf, то есть разброс среды
    // ~×1.7. Разрыв debug → release показывает, что в одиночный замер попадал
    // холодный первый кадр: отсюда прогрев и медиана пяти проходов. 300 мс —
    // мягкий гейт, а не бюджет (~×2 к худшему наблюдённому): он ловит регрессию
    // на порядок, а не проценты, а бюджет < 16 мс остаётся целью для живой
    // машины в ROADMAP §9.
    assert!(
        median < SOFT_GATE_MS,
        "paintMs median = {median} (samples = {samples:?})"
    );
}
