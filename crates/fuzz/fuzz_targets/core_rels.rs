#![no_main]

//! Разбор `_rels/*.rels` — карты relationships.

use doc_converter_core::rels::RelMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = RelMap::parse(data);
});
