#![cfg(target_arch = "wasm32")]

//! Проверки wasm-моста DOCX: открытие фикстуры, кадр в ring'е, закрытие.
//!
//! Фикстура вкомпилирована (`include_bytes!`): у теста под wasm нет файловой
//! системы, а путь от `CARGO_MANIFEST_DIR` тот же, что у нативных тестов.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Одностраничная фикстура: `w:pgSz` 11906×16838 твипов, то есть A4.
const ONE_PARAGRAPH: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/docx/simple/one_paragraph.docx"
));

/// Размер фикстуры в пикселях раскладки: 1 px = 15 твипов при 96 dpi.
const PAGE_W: f64 = 11906.0 / 15.0;
const PAGE_H: f64 = 16838.0 / 15.0;

/// Ёмкость слота ring'а: кадр страницы много меньше мебибайта.
const SLOT_CAPACITY: u32 = 1 << 20;

/// Окно в том же виде, в каком его шлёт воркер.
fn viewport() -> JsValue {
    let object = js_sys::Object::new();
    let fields = [
        ("scrollX", 0.0),
        ("scrollY", 0.0),
        ("width", 800.0),
        ("height", 600.0),
        ("scale", 2.0),
    ];
    for (name, value) in fields {
        js_sys::Reflect::set(&object, &JsValue::from_str(name), &JsValue::from_f64(value))
            .expect("поле окна записывается");
    }
    object.into()
}

/// Числовое поле ответа; отсутствие поля — ошибка теста, а не ноль.
fn number(value: &JsValue, key: &str) -> f64 {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .expect("поле читается")
        .as_f64()
        .unwrap_or_else(|| panic!("{key} — не число"))
}

fn boolean(value: &JsValue, key: &str) -> bool {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .expect("поле читается")
        .as_bool()
        .unwrap_or_else(|| panic!("{key} — не bool"))
}

/// Размеры страниц приходят в пикселях раскладки: по ним интерфейс строит
/// прокрутку, как по размерам листа XLSX.
#[wasm_bindgen_test]
fn open_reports_page_sizes() {
    let pages = doc_converter_docx_wasm::docx_api::docx_open(ONE_PARAGRAPH.to_vec())
        .expect("фикстура открывается");
    assert!(doc_converter_docx_wasm::docx_api::docx_is_open());

    let pages = js_sys::Array::from(&pages);
    assert_eq!(pages.length(), 1, "в фикстуре одна страница");

    let page = pages.get(0);
    let width = number(&page, "width");
    let height = number(&page, "height");
    assert!((width - PAGE_W).abs() < 0.01, "ширина страницы: {width}");
    assert!((height - PAGE_H).abs() < 0.01, "высота страницы: {height}");

    doc_converter_docx_wasm::docx_api::docx_close();
    assert!(!doc_converter_docx_wasm::docx_api::docx_is_open());
}

/// `docx_open_bytes` — тот же разбор, но байты приезжают `Uint8Array`.
#[wasm_bindgen_test]
fn open_bytes_accepts_a_uint8_array() {
    let pages =
        doc_converter_docx_wasm::docx_api::docx_open_bytes(js_sys::Uint8Array::from(ONE_PARAGRAPH))
            .expect("фикстура открывается");
    assert_eq!(js_sys::Array::from(&pages).length(), 1, "одна страница");
    doc_converter_docx_wasm::docx_api::docx_close();
}

/// Fallback без SAB: байты кадра уезжают в воркер копией.
#[wasm_bindgen_test]
fn build_without_sab_returns_the_frame_bytes() {
    doc_converter_docx_wasm::docx_api::docx_open(ONE_PARAGRAPH.to_vec())
        .expect("фикстура открывается");

    let bytes = doc_converter_docx_wasm::docx_api::docx_build_display_list(0, viewport())
        .expect("кадр собирается");
    assert!(bytes.len() > doc_converter_render::display_list::HEADER_SIZE);
    assert_eq!(
        &bytes[..4],
        doc_converter_render::display_list::DL_MAGIC.to_le_bytes(),
        "кадр начинается с магии DisplayList"
    );

    doc_converter_docx_wasm::docx_api::docx_close();
}

/// Кадр ложится в слот ring'а целиком: статистика непустая, а в слоте именно
/// DisplayList, а не мусор.
#[wasm_bindgen_test]
fn builds_a_frame_into_the_sab_slot() {
    doc_converter_docx_wasm::docx_api::docx_open(ONE_PARAGRAPH.to_vec())
        .expect("фикстура открывается");

    // SAB аллоцируется здесь, а не через `sab_api` основного модуля: у моста
    // в этом крейте нет доступа к аллокатору, да он и не нужен — ring работает
    // с любым буфером подходящего размера.
    let total = doc_converter_render::sab::HEADER_BYTES + (SLOT_CAPACITY as usize) * 2;
    let sab = js_sys::SharedArrayBuffer::new(total as u32);
    let stats = doc_converter_docx_wasm::docx_api::docx_build_display_list_sab(
        sab.clone(),
        SLOT_CAPACITY,
        0,
        viewport(),
    )
    .expect("кадр собирается");

    assert!(boolean(&stats, "written"), "кадр уложился в слот");
    assert!(number(&stats, "cmds") > 0.0, "в кадре есть команды");
    assert!(number(&stats, "buildMs") >= 0.0);

    let ring = doc_converter_render::sab::SabRing::from_sab(sab, SLOT_CAPACITY as usize)
        .expect("ring поверх того же SAB");
    let mut frame = Vec::new();
    assert!(
        ring.copy_current_into(&mut frame).expect("слот читается"),
        "в ring'е лежит кадр"
    );
    assert_eq!(
        &frame[..4],
        doc_converter_render::display_list::DL_MAGIC.to_le_bytes(),
        "в слоте — DisplayList"
    );

    doc_converter_docx_wasm::docx_api::docx_close();
}

/// После закрытия собирать кадр не из чего: мост отвечает ошибкой, а не
/// пустым кадром.
#[wasm_bindgen_test]
fn build_after_close_is_an_error() {
    doc_converter_docx_wasm::docx_api::docx_open(ONE_PARAGRAPH.to_vec())
        .expect("фикстура открывается");
    doc_converter_docx_wasm::docx_api::docx_close();

    let error = doc_converter_docx_wasm::docx_api::docx_build_display_list(0, viewport())
        .expect_err("после закрытия кадр не собирается");
    let message = error.as_string().unwrap_or_default();
    assert!(message.contains("no document"), "сообщение: {message}");
}
