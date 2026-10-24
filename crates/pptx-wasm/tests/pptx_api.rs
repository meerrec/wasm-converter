#![cfg(target_arch = "wasm32")]

//! Проверки wasm-моста PPTX: открытие фикстуры, кадр в ring'е, закрытие.
//!
//! Фикстуры вкомпилированы (`include_bytes!`): у теста под wasm нет файловой
//! системы, а путь от `CARGO_MANIFEST_DIR` тот же, что у нативных тестов.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Два слайда с текстом — базовая фикстура просмотра.
const TWO_SLIDES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/pptx/simple/two_slides.pptx"
));

/// Слайд с картинкой: в модели есть хотя бы одна медиачасть.
const PICTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/pptx/images/picture.pptx"
));

/// Размер слайда генератора в пикселях раскладки: 12192000×6858000 EMU.
const SLIDE_W: f64 = 1280.0;
const SLIDE_H: f64 = 720.0;

/// Ёмкость слота ring'а: кадр слайда много меньше мебибайта.
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

/// Строковое поле ответа; отсутствие поля — ошибка теста, а не пустая строка.
fn string(value: &JsValue, key: &str) -> String {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .expect("поле читается")
        .as_string()
        .unwrap_or_else(|| panic!("{key} — не строка"))
}

/// Флаг, взведённый до открытия, обрывает раскладку на первом слайде
/// (ADR-0032), а снятый — возвращает обычное открытие: отмена не портит
/// состояние воркера.
#[wasm_bindgen_test]
fn cancel_stops_open_and_worker_stays_healthy() {
    doc_converter_pptx_wasm::pptx_api::set_cancel(true);
    let err = doc_converter_pptx_wasm::pptx_api::pptx_open(TWO_SLIDES.to_vec())
        .expect_err("взведённый флаг отменяет открытие");
    assert!(
        err.as_string()
            .is_some_and(|m| m.starts_with("[cancelled]")),
        "ошибка: {err:?}"
    );

    doc_converter_pptx_wasm::pptx_api::set_cancel(false);
    let slides = doc_converter_pptx_wasm::pptx_api::pptx_open(TWO_SLIDES.to_vec())
        .expect("после снятия флага открытие проходит");
    assert!(slides.is_array(), "ответ — массив слайдов");
}

/// Слайды приходят индексом, именем и размерами в пикселях раскладки.
#[wasm_bindgen_test]
fn open_reports_slide_infos() {
    let slides = doc_converter_pptx_wasm::pptx_api::pptx_open(TWO_SLIDES.to_vec())
        .expect("фикстура открывается");
    assert!(doc_converter_pptx_wasm::pptx_api::pptx_is_open());

    let slides = js_sys::Array::from(&slides);
    assert_eq!(slides.length(), 2, "в фикстуре два слайда");

    let slide = slides.get(0);
    assert_eq!(number(&slide, "index"), 0.0, "индекс первого слайда");
    assert_eq!(string(&slide, "name"), "Deck 1", "имя слайда");
    assert!(
        (number(&slide, "width") - SLIDE_W).abs() < 0.01,
        "ширина слайда"
    );
    assert!(
        (number(&slide, "height") - SLIDE_H).abs() < 0.01,
        "высота слайда"
    );

    doc_converter_pptx_wasm::pptx_api::pptx_close();
    assert!(!doc_converter_pptx_wasm::pptx_api::pptx_is_open());
}

/// `pptx_open_bytes` — тот же разбор, но байты приезжают `Uint8Array`.
#[wasm_bindgen_test]
fn open_bytes_accepts_a_uint8_array() {
    let slides =
        doc_converter_pptx_wasm::pptx_api::pptx_open_bytes(js_sys::Uint8Array::from(TWO_SLIDES))
            .expect("фикстура открывается");
    assert_eq!(js_sys::Array::from(&slides).length(), 2, "два слайда");
    doc_converter_pptx_wasm::pptx_api::pptx_close();
}

/// Fallback без SAB: байты кадра уезжают в буфер вызывающего.
#[wasm_bindgen_test]
fn build_without_sab_writes_the_frame_into_the_caller_buffer() {
    doc_converter_pptx_wasm::pptx_api::pptx_open(TWO_SLIDES.to_vec())
        .expect("фикстура открывается");

    let out = js_sys::Uint8Array::new_with_length(SLOT_CAPACITY);
    let stats =
        doc_converter_pptx_wasm::pptx_api::pptx_build_display_list_into(&out, viewport(), 1)
            .expect("кадр собирается");
    assert!(
        js_sys::Reflect::get(&stats, &JsValue::from_str("written"))
            .expect("поле читается")
            .as_bool()
            .expect("written — bool"),
        "кадр уложился в буфер"
    );
    let len = number(&stats, "bytes") as usize;
    assert!(len > doc_converter_render::display_list::HEADER_SIZE);

    let bytes = out.to_vec();
    assert_eq!(
        &bytes[..4],
        doc_converter_render::display_list::DL_MAGIC.to_le_bytes(),
        "кадр начинается с магии DisplayList"
    );
    let frame = doc_converter_render::display_list::DisplayList::from_bytes(&bytes[..len])
        .expect("кадр декодируется как DisplayList");
    assert!(frame.cmd_count() > 0, "слайд с текстом даёт команды");

    doc_converter_pptx_wasm::pptx_api::pptx_close();
}

/// Кадр ложится в слот ring'а целиком: статистика непустая, а в слоте именно
/// DisplayList, а не мусор.
#[wasm_bindgen_test]
fn builds_a_frame_into_the_sab_slot() {
    doc_converter_pptx_wasm::pptx_api::pptx_open(TWO_SLIDES.to_vec())
        .expect("фикстура открывается");

    let total = doc_converter_render::sab::HEADER_BYTES + (SLOT_CAPACITY as usize) * 2;
    let sab = js_sys::SharedArrayBuffer::new(total as u32);
    let stats = doc_converter_pptx_wasm::pptx_api::pptx_build_display_list_sab(
        sab.clone(),
        SLOT_CAPACITY,
        viewport(),
        0,
    )
    .expect("кадр собирается");

    let written = js_sys::Reflect::get(&stats, &JsValue::from_str("written"))
        .expect("поле читается")
        .as_bool()
        .expect("written — bool");
    assert!(written, "кадр уложился в слот");

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

    doc_converter_pptx_wasm::pptx_api::pptx_close();
}

/// После закрытия собирать кадр не из чего: мост отвечает ошибкой, а не
/// пустым кадром.
#[wasm_bindgen_test]
fn build_after_close_is_an_error() {
    doc_converter_pptx_wasm::pptx_api::pptx_open(TWO_SLIDES.to_vec())
        .expect("фикстура открывается");
    doc_converter_pptx_wasm::pptx_api::pptx_close();

    // До записи кадр не собирается: буфер на ошибку не влияет, хватит минимума.
    let out = js_sys::Uint8Array::new_with_length(16);
    let error =
        doc_converter_pptx_wasm::pptx_api::pptx_build_display_list_into(&out, viewport(), 0)
            .expect_err("после закрытия кадр не собирается");
    let message = error.as_string().unwrap_or_default();
    assert!(message.contains("no presentation"), "сообщение: {message}");
}

/// Индекса слайда нет в презентации — ошибка, а не пустой кадр.
#[wasm_bindgen_test]
fn slide_out_of_range_is_an_error() {
    doc_converter_pptx_wasm::pptx_api::pptx_open(TWO_SLIDES.to_vec())
        .expect("фикстура открывается");

    let out = js_sys::Uint8Array::new_with_length(16);
    let error =
        doc_converter_pptx_wasm::pptx_api::pptx_build_display_list_into(&out, viewport(), 99)
            .expect_err("слайда с таким индексом нет");
    let message = error.as_string().unwrap_or_default();
    assert!(message.contains("out of range"), "сообщение: {message}");

    doc_converter_pptx_wasm::pptx_api::pptx_close();
}

/// Список картинок несёт id, MIME и длину, а `pptx_image_bytes` отдаёт байты
/// ровно этой длины: по ним интерфейс собирает `Blob` и регистрирует битмап.
#[wasm_bindgen_test]
fn images_report_the_media_of_the_presentation() {
    doc_converter_pptx_wasm::pptx_api::pptx_open(PICTURE.to_vec()).expect("фикстура открывается");

    let images = js_sys::Array::from(
        &doc_converter_pptx_wasm::pptx_api::pptx_images().expect("список картинок"),
    );
    assert_eq!(images.length(), 1, "в фикстуре одна картинка");

    let image = images.get(0);
    let id = number(&image, "id") as u32;
    let mime = string(&image, "mime");
    assert!(mime.starts_with("image/"), "MIME: {mime}");
    let byte_length = number(&image, "byteLength") as u32;
    assert!(byte_length > 0, "длина байтов: {byte_length}");

    let bytes = doc_converter_pptx_wasm::pptx_api::pptx_image_bytes(id).expect("байты картинки");
    assert_eq!(
        bytes.length(),
        byte_length,
        "длина байтов совпадает со списком"
    );

    doc_converter_pptx_wasm::pptx_api::pptx_close();
}

/// id, которого нет среди прочитанных частей, — ошибка, а не пустой массив.
#[wasm_bindgen_test]
fn unknown_image_id_is_an_error() {
    doc_converter_pptx_wasm::pptx_api::pptx_open(PICTURE.to_vec()).expect("фикстура открывается");

    let error = doc_converter_pptx_wasm::pptx_api::pptx_image_bytes(u32::MAX)
        .expect_err("картинки с таким id нет");
    let message = error.as_string().unwrap_or_default();
    assert!(message.contains("does not exist"), "сообщение: {message}");

    doc_converter_pptx_wasm::pptx_api::pptx_close();
}
