#![no_main]

//! Разбор части слайда PPTX — `ppt/slides/slide1.xml`.

use std::io::{Cursor, Write};

use doc_converter_core::ZipLimits;
use libfuzzer_sys::fuzz_target;

/// Пакет PPTX с одной подставленной частью слайда: мусорные байты
/// подставляются в `ppt/slides/slide1.xml` валидного каркаса.
fn package_with(slide: &[u8]) -> Vec<u8> {
    let parts: &[(&str, &[u8])] = &[
        ("[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>"#),
        ("_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#),
        ("ppt/presentation.xml", br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldMasterIdLst/><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#),
        ("ppt/_rels/presentation.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#),
        ("ppt/slides/slide1.xml", slide),
    ];
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in parts {
            if writer.start_file(*name, options).is_err() {
                return Vec::new();
            }
            if writer.write_all(data).is_err() {
                return Vec::new();
            }
        }
        if writer.finish().is_err() {
            return Vec::new();
        }
    }
    buf
}

fuzz_target!(|data: &[u8]| {
    let bytes = package_with(data);
    // Ошибка разбора — легальный исход на мусоре: панику внутри библиотеки
    // ловит сам либфаззер, и только она здесь находка.
    let _ = doc_converter_pptx::parse(&bytes, &ZipLimits::default());
});
