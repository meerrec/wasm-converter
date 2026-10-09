#![no_main]

//! Разбор книги XLSX: листы, строки, стили, изображения.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // У крейта xlsx свои лимиты (32 МиБ на картинку, 128 МиБ на книгу), но
    // прогон всё равно идёт с `-timeout=30`: на медленном входе падение по
    // таймауту — это тоже находка.
    let _ = doc_converter_xlsx::open(data.to_vec());
});
