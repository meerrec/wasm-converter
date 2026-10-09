#![no_main]

//! Разбор главной части DOCX — `word/document.xml`.

#[path = "docx_package.rs"]
mod docx_package;

use doc_converter_core::ZipLimits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let bytes = docx_package::package_with("word/document.xml", data);
    // Ошибка разбора — легальный исход на мусоре: панику внутри библиотеки
    // ловит сам либфаззер, и только она здесь находка.
    let _ = doc_converter_docx::parse_docx(&bytes, ZipLimits::default());
});
