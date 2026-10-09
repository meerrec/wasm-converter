#![no_main]

//! ZIP-часть OOXML-пакета: открытие архива и чтение его part'ов.

use doc_converter_core::{Archive, ZipLimits};
use libfuzzer_sys::fuzz_target;

/// Тесные лимиты ADR-0015: с умолчаниями (64 МиБ на часть) фаззерный вход до
/// порогов не дотягивается, и ветки отказа остались бы непокрытыми.
const TIGHT_LIMITS: ZipLimits = ZipLimits {
    per_part_uncompressed: 4096,
    per_archive_uncompressed: 16 * 1024,
    max_ratio: 4,
    max_parts: 16,
    max_name_len: 64,
    max_depth: 8,
};

/// Прочитать не больше 64 part'ов: имён во входе может быть сколько угодно,
/// а каждая распаковка — работа на итерации.
fn read_parts(archive: &mut Archive) {
    // `names()` заимствует архив, поэтому имена сначала копируются в Vec:
    // читать part'ы можно только по `&mut`.
    let names: Vec<String> = archive.names().map(str::to_owned).take(64).collect();
    for name in names {
        let _ = archive.read(&name);
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(mut archive) = Archive::new(data.to_vec()) else {
        return;
    };
    read_parts(&mut archive);

    // Лимиты на размер распакованной части проверяются только здесь.
    let Ok(mut archive) = Archive::open_with_limits(data.to_vec(), TIGHT_LIMITS) else {
        return;
    };
    read_parts(&mut archive);
});
