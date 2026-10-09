#![no_main]

//! Стриминговый XML-читатель: разбор событий вплоть до `Eof`.

use doc_converter_core::xml::XmlReader;
use libfuzzer_sys::fuzz_target;

/// Потолок на число событий: у документа без ошибок их число не ограничено
/// входом, и на длинном входе прогон упирался бы в `-timeout` ложно.
const MAX_EVENTS: usize = 100_000;

fuzz_target!(|data: &[u8]| {
    let mut reader = XmlReader::new(data, "fuzz");
    for _ in 0..MAX_EVENTS {
        match reader.next_significant() {
            // Ошибка разбора — легальный исход на мусоре: она не паника,
            // и проверять после неё нечего.
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
});
