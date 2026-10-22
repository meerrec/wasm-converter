//! Базовое направление текста по UAX #9.
//!
//! Полный bidi-алгоритм для раскладки не нужен: браузер сам переупорядочивает
//! строку при рисовании и в DOM. Раскладке важно знать базу строки — от неё
//! зависят физическое выравнивание и `direction` в `TextLayer`.
//!
//! `w:bidi` задаёт базу явно; без него база определяется первым сильным
//! символом абзаца, как в UAX #9.
//!
//! Таблицу классов символов крейт `unicode-bidi` не тянет (фича
//! `hardcoded-data` выключена): она весит больше, чем весь бюджет wasm-docx.
//! Вместо неё — компактный классификатор ниже. Алгоритм UAX #9 остаётся
//! крейтовым; расходится только классификация редких письменностей, которых
//! нет в наших фикстурах, — это осознанная цена бюджета (ADR-0040).

use unicode_bidi::{BidiClass, BidiDataSource, BidiInfo, Level};

/// Классификатор направлений для `unicode-bidi` без таблицы UCD.
#[derive(Debug, Clone, Copy)]
struct BidiData;

impl BidiDataSource for BidiData {
    fn bidi_class(&self, ch: char) -> BidiClass {
        class(ch)
    }
}

/// База абзаца: `true` — справа налево.
///
/// `forced` — `w:bidi` (или унаследованный из стиля); он сильнее автоопределения.
#[must_use]
pub fn base_rtl(text: &str, forced: bool) -> bool {
    if forced {
        return true;
    }
    // `None` — база по первому сильному символу; `Some(level)` её фиксирует.
    let info = BidiInfo::new_with_data_source(&BidiData, text, None);
    info.paragraphs
        .first()
        .is_some_and(|paragraph| paragraph.level.is_rtl())
}

/// Уровни UAX #9 для строки с известной базой.
#[must_use]
pub fn levels(text: &str, rtl: bool) -> BidiInfo<'_> {
    let base = if rtl { Level::rtl() } else { Level::ltr() };
    BidiInfo::new_with_data_source(&BidiData, text, Some(base))
}

/// Класс направления символа — компактная замена таблицы UCD.
#[must_use]
pub fn class(ch: char) -> BidiClass {
    let code = ch as u32;
    match code {
        // Явные marks: LRM/RLM/ALM и встраивания.
        0x200E => BidiClass::L,
        0x202A | 0x202D => BidiClass::LRE,
        0x202B | 0x202E => BidiClass::RLE,
        0x202C => BidiClass::PDF,
        0x2066 => BidiClass::LRI,
        0x2067 => BidiClass::RLI,
        0x2068 => BidiClass::FSI,
        0x2069 => BidiClass::PDI,
        // Управляющие и нулевой ширины.
        0x0000..=0x0008 | 0x000E..=0x001B | 0x007F..=0x0084 | 0x0086..=0x009F | 0x00AD | 0x200B => {
            BidiClass::BN
        }
        0x0009
        | 0x000B
        | 0x000C
        | 0x0020
        | 0x00A0
        | 0x1680
        | 0x2000..=0x200A
        | 0x2028
        | 0x205F
        | 0x3000 => BidiClass::WS,
        0x000A | 0x000D | 0x001C..=0x001E | 0x0085 | 0x2029 => BidiClass::B,
        // Цифры.
        0x0030..=0x0039
        | 0x06F0..=0x06F9
        | 0x2070
        | 0x2074..=0x2079
        | 0x2080..=0x2089
        | 0x2150..=0x2189 => BidiClass::EN,
        0x0600..=0x0605
        | 0x0660..=0x0669
        | 0x066B..=0x066C
        | 0x06DD
        | 0x0890..=0x0891
        | 0x08E2
        | 0x10D30..=0x10D39
        | 0x10E60..=0x10E7E => BidiClass::AN,
        // Арабская пунктуация и знаки.
        0x0609..=0x060A | 0x066A | 0x06DE | 0x06E9 => BidiClass::ET,
        0x060C | 0x061B | 0x061D..=0x061F | 0x066D => BidiClass::CS,
        // Арабские/сирийские/еврейские огласовки.
        0x0300..=0x036F
        | 0x0483..=0x0489
        | 0x0591..=0x05BD
        | 0x05BF
        | 0x05C1..=0x05C2
        | 0x05C4..=0x05C5
        | 0x05C7
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED
        | 0x0711
        | 0x0730..=0x074A
        | 0x07A6..=0x07B0
        | 0x07EB..=0x07F3
        | 0x0816..=0x0819
        | 0x081B..=0x0823
        | 0x0825..=0x0827
        | 0x0829..=0x082D
        | 0x0859..=0x085B
        | 0x08D3..=0x08E1
        | 0x08E3..=0x0902 => BidiClass::NSM,
        // Иврит и родственные письменности.
        0x0590..=0x05FF | 0x07C0..=0x07FF | 0x200F | 0xFB1D..=0xFB4F | 0x1E800..=0x1EFFF => {
            BidiClass::R
        }
        // Арабский, сирийский, тана, самаритянский, мандейский.
        0x060B
        | 0x061C
        | 0x0620..=0x064A
        | 0x066E..=0x06D5
        | 0x06E5..=0x06E6
        | 0x06EE..=0x06EF
        | 0x06FA..=0x06FF
        | 0x0700..=0x070D
        | 0x070F
        | 0x0712..=0x072F
        | 0x074D..=0x074F
        | 0x0750..=0x077F
        | 0x0780..=0x07A5
        | 0x07B1
        | 0x0800..=0x0815
        | 0x0830..=0x083E
        | 0x0840..=0x0858
        | 0x0860..=0x086F
        | 0x0870..=0x088E
        | 0x08A0..=0x08C9
        | 0xFB50..=0xFDFF
        | 0xFE70..=0xFEFF
        | 0x10D00..=0x10D23
        | 0x10F30..=0x10F54 => BidiClass::AL,
        // Всё остальное: буквы — L, пунктуация и символы — ON.
        _ if ch.is_alphabetic() => BidiClass::L,
        _ => BidiClass::ON,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arabic_is_rtl_without_forced_bidi() {
        assert!(base_rtl("\u{0645}\u{0631}\u{062d}\u{0628}\u{0627}", false));
    }

    #[test]
    fn hebrew_is_rtl() {
        assert!(base_rtl("\u{05e9}\u{05dc}\u{05d5}\u{05dd}", false));
    }

    #[test]
    fn latin_is_ltr() {
        assert!(!base_rtl("Hello", false));
    }

    #[test]
    fn forced_bidi_wins() {
        assert!(base_rtl("Hello", true));
    }

    #[test]
    fn levels_follow_base() {
        let info = levels("Hello", true);
        assert!(info.paragraphs[0].level.is_rtl());
    }

    #[test]
    fn classes_cover_digits_and_marks() {
        assert_eq!(class('7'), BidiClass::EN);
        assert_eq!(class('\u{0667}'), BidiClass::AN);
        assert_eq!(class('\u{064e}'), BidiClass::NSM);
        assert_eq!(class('\u{200f}'), BidiClass::R);
    }

    #[test]
    fn numbers_in_rtl_get_even_level() {
        // «مرحبا 123»: цифры внутри RTL-абзаца должны остаться LTR-уровнем.
        let info = levels("\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} 123", true);
        let digits = info.text.find('1').expect("digit");
        assert_eq!(info.levels[digits].number() % 2, 0);
    }
}
