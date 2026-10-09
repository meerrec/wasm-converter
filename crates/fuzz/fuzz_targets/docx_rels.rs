#![no_main]

//! Разбор связей главной части DOCX — `word/_rels/document.xml.rels`.

#[path = "docx_package.rs"]
mod docx_package;

use doc_converter_core::ZipLimits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let bytes = docx_package::package_with("word/_rels/document.xml.rels", data);
    let _ = doc_converter_docx::parse_docx(&bytes, ZipLimits::default());
});
