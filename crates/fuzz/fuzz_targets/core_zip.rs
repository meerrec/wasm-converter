#![no_main]

//! ZIP-часть OOXML-пакета: открытие архива и чтение его part'ов.

use doc_converter_core::Archive;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(mut archive) = Archive::new(data.to_vec()) else {
        return;
    };
    // `names()` заимствует архив, поэтому имена сначала копируются в Vec:
    // читать part'ы можно только по `&mut`.
    let names: Vec<String> = archive.names().map(str::to_owned).take(64).collect();
    for name in names {
        let _ = archive.read(&name);
    }
    // TODO (Фаза 8, слайс S4): после ADR-0015 прогнать и `open_with_limits` —
    // лимиты на размер распакованной части проверяются только там.
});
