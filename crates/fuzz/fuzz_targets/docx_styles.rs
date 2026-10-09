#![no_main]

//! Разбор таблицы стилей DOCX — `word/styles.xml`.

#[path = "docx_package.rs"]
mod docx_package;

use doc_converter_core::ZipLimits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let bytes = docx_package::package_with("word/styles.xml", data);
    let _ = doc_converter_docx::parse_docx(&bytes, ZipLimits::default());
});
