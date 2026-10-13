#![cfg(target_arch = "wasm32")]

//! Экспорт сабсетов Carlito: JS-регистрация `FontFace` должна получать
//! настоящие TrueType-байты каждого лица, а не заглушку и не одно лицо на всех.

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Проверка головы sfnt: TrueType-шрифт начинается с версии `0x00010000`
/// (или тега `true`/`OTTO` — у сабсетов Carlito это TrueType).
fn is_sfnt(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x00, 0x01, 0x00, 0x00])
        || bytes.starts_with(b"true")
        || bytes.starts_with(b"OTTO")
}

/// Сабсеты весят десятки килобайт: пустой или обрезанный ответ видно сразу.
const MIN_SUBSET_BYTES: u32 = 1024;

#[wasm_bindgen_test]
fn font_bytes_returns_all_four_faces() {
    let mut faces: Vec<Vec<u8>> = Vec::with_capacity(4);
    for face in 0..4 {
        let bytes = doc_converter_wasm::font_api::font_bytes(face).unwrap();
        assert!(
            bytes.length() > MIN_SUBSET_BYTES,
            "face {face}: only {} bytes",
            bytes.length()
        );
        let raw = bytes.to_vec();
        assert!(is_sfnt(&raw), "face {face}: not an sfnt font");
        faces.push(raw);
    }

    // Лица — разные файлы: один и тот же сабсет на все четыре номера означал бы
    // потерянный match и неверное начертание в предпросмотре.
    for (a, first) in faces.iter().enumerate() {
        for (b, second) in faces.iter().enumerate().skip(a + 1) {
            assert_ne!(first, second, "faces {a} and {b} are the same file");
        }
    }
}

#[wasm_bindgen_test]
fn font_bytes_rejects_unknown_face() {
    let error = doc_converter_wasm::font_api::font_bytes(4).unwrap_err();
    let message = error.as_string().unwrap_or_default();
    assert!(
        message.contains("font face 4"),
        "unexpected message: {message}"
    );
}
