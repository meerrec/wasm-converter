//! Подрезка TTF под использованные символы и PDF-объекты шрифта (ADR-0022).
//!
//! Разбор и метрики — `skrifa` (ADR-0002), подрезка — `subsetter`: в PDF
//! уходит только использованный набор глифов. Шрифт пишется составным,
//! Identity-H: код символа в потоке содержимого — это CID, он же gid
//! подрезанного шрифта, поэтому `ToUnicode` обязателен для извлечения текста,
//! а ширины `/W` пересчитываются из метрик исходного шрифта в тысячные em.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat::Literal};
use skrifa::instance::{LocationRef, Size};
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};
use subsetter::GlyphRemapper;

use crate::PdfError;

/// Ширина CID, для которого нет записи в `/W` (ISO 32000, таблица 116).
const DEFAULT_WIDTH: i64 = 1000;

/// Записей `beginbfchar` в одном блоке: больше 100 запрещает ISO 32000, 9.10.3.
const BFCHAR_PER_BLOCK: usize = 100;

/// Имя шрифта, если `name`-таблица не дала ни одного пригодного.
const FALLBACK_FACE_NAME: &str = "DocConverter";

/// Алфавит тега подмножества: только заглавные буквы — тег не требует
/// экранирования в Name-объекте (ISO 32000, 9.6.4).
const TAG_ALPHABET: &[u8; 16] = b"ABCDEFGHIJKLMNOP";

/// Подготовленный к записи шрифт: подрезанная программа и её PDF-метрики.
pub struct PreparedFont {
    /// Подрезанная программа TrueType — содержимое `FontFile2`.
    subset: Vec<u8>,
    /// Имя гарнитуры для `/BaseFont` и `/FontName` (без тега подмножества).
    face_name: String,
    /// Символ → CID подрезанного шрифта; символы без глифа кодируются нулём.
    cid_by_char: BTreeMap<char, u16>,
    /// CID → ширина в тысячных em; запись для CID 0 есть всегда.
    widths: BTreeMap<u16, i64>,
    /// CID → символ для `ToUnicode`, по возрастанию CID.
    to_unicode: BTreeMap<u16, char>,
    /// Линейные метрики для `FontDescriptor`, в тысячных em.
    ascent: i64,
    descent: i64,
    cap_height: i64,
    /// Рамка всех глифов `[x_min, y_min, x_max, y_max]`, в тысячных em.
    bbox: [i64; 4],
}

impl PreparedFont {
    /// Текст в 2-байтовые BE-глифы (CID Identity-H), без разделителей и скобок.
    ///
    /// Символ, которого не было в `used` или для которого нет глифа, кодируется
    /// нулевым CID — `.notdef` подмножества.
    #[must_use]
    pub(crate) fn encode(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() * 2);
        for ch in text.chars() {
            let cid = self.cid_by_char.get(&ch).copied().unwrap_or(0);
            out.extend_from_slice(&cid.to_be_bytes());
        }
        out
    }

    /// `CMap` для `ToUnicode`: CID подрезанного шрифта → Unicode.
    ///
    /// Блок `beginbfchar` несёт не более 100 записей (ISO 32000, 9.10.3);
    /// символы вне BMP записываются сурогатной парой UTF-16.
    fn to_unicode_cmap(&self) -> String {
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n\
             12 dict begin\n\
             begincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n\
             /CMapType 2 def\n\
             1 begincodespacerange\n\
             <0000> <FFFF>\n\
             endcodespacerange\n",
        );
        let entries: Vec<(u16, char)> = self
            .to_unicode
            .iter()
            .map(|(&cid, &ch)| (cid, ch))
            .collect();
        for chunk in entries.chunks(BFCHAR_PER_BLOCK) {
            let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
            for &(cid, ch) in chunk {
                let _ = write!(cmap, "<{cid:04X}> <");
                let mut units = [0_u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    let _ = write!(cmap, "{unit:04X}");
                }
                cmap.push_str(">\n");
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        cmap
    }
}

/// Разобрать TTF, подрезать по использованным символам и собрать данные для записи.
///
/// `used` — только те символы, что реально встретились в тексте. Символ без
/// глифа в шрифте в подмножество не попадает; [`PreparedFont::encode`] кодирует
/// его нулевым CID.
///
/// # Errors
///
/// [`PdfError::Font`], если байты не разбираются как TrueType или `subsetter`
/// отказал на таблицах шрифта.
pub(crate) fn prepare(bytes: &[u8], used: &BTreeSet<char>) -> Result<PreparedFont, PdfError> {
    let font = FontRef::new(bytes).map_err(|err| PdfError::Font(format!("parse: {err}")))?;
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    // Валидный TTF несёт `head`; нулевой upem — защита от деления на ноль.
    let units_per_em = if metrics.units_per_em == 0 {
        1000.0
    } else {
        f32::from(metrics.units_per_em)
    };
    let charmap = font.charmap();
    let glyph_metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());

    // `.notdef` остаётся CID 0: им кодируются символы, которых в подмножестве
    // нет, — в том числе те, для которых не нашлось глифа.
    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);

    // `BTreeSet` обходится по возрастанию символа — порядок CID детерминирован.
    let mut mapped: Vec<(char, skrifa::GlyphId, u16)> = Vec::with_capacity(used.len());
    for &ch in used {
        if let Some(glyph) = charmap.map(ch) {
            // У TTF глифов не больше `u16::MAX`: `numGlyphs` — u16.
            let old = u16::try_from(glyph.to_u32()).unwrap_or(0);
            let new = remapper.remap(old);
            mapped.push((ch, glyph, new));
        }
    }

    let subset = subsetter::subset(bytes, 0, &remapper)
        .map_err(|err| PdfError::Font(format!("subset: {err}")))?;

    let mut cid_by_char = BTreeMap::new();
    let mut widths = BTreeMap::new();
    let mut to_unicode = BTreeMap::new();
    widths.insert(
        0,
        glyph_width(&glyph_metrics, skrifa::GlyphId::NOTDEF, units_per_em),
    );
    for (ch, glyph, cid) in mapped {
        cid_by_char.insert(ch, cid);
        // Первый символ, пришедший на CID, и задаёт ширину и пару `ToUnicode`:
        // дубликаты вроде U+0020 и U+00A0 делят один глиф без потерь.
        widths
            .entry(cid)
            .or_insert_with(|| glyph_width(&glyph_metrics, glyph, units_per_em));
        to_unicode.entry(cid).or_insert(ch);
    }

    let ascent = to_thousandths(metrics.ascent, units_per_em);
    let descent = to_thousandths(metrics.descent, units_per_em);
    let cap_height = metrics
        .cap_height
        .map_or(ascent, |value| to_thousandths(value, units_per_em));
    let bbox = metrics
        .bounds
        .map_or([0, descent, DEFAULT_WIDTH, ascent], |bounds| {
            [
                to_thousandths(bounds.x_min, units_per_em),
                to_thousandths(bounds.y_min, units_per_em),
                to_thousandths(bounds.x_max, units_per_em),
                to_thousandths(bounds.y_max, units_per_em),
            ]
        });

    Ok(PreparedFont {
        subset,
        face_name: face_name(&font),
        cid_by_char,
        widths,
        to_unicode,
        ascent,
        descent,
        cap_height,
        bbox,
    })
}

/// Добавить в scratch-`lopdf::Document` объекты шрифта, вернуть id словаря Type0.
///
/// Пишутся `FontFile2` (с пометкой «сжимать при `compress`»), `FontDescriptor`,
/// `CIDFontType2` и сам Type0 с `ToUnicode`; вызывающий ссылается на
/// возвращённый id из `/Resources /Font`.
pub(crate) fn write_font(prepared: &PreparedFont, doc: &mut Document) -> ObjectId {
    let base_font = format!("{}+{}", subset_tag(&prepared.subset), prepared.face_name);

    let mut program = Dictionary::new();
    // Length1 — длина распакованной программы (ISO 32000, 9.9.2); сжатие
    // потока выполняет писатель по флагу `compress`.
    program.set(
        "Length1",
        i64::try_from(prepared.subset.len()).unwrap_or(i64::MAX),
    );
    let font_file =
        doc.add_object(Stream::new(program, prepared.subset.clone()).with_compression(true));

    let mut descriptor = Dictionary::new();
    descriptor.set("Type", "FontDescriptor");
    descriptor.set("FontName", Object::Name(base_font.clone().into_bytes()));
    descriptor.set("Flags", 32_i64);
    descriptor.set("ItalicAngle", 0_i64);
    descriptor.set("StemV", 80_i64);
    descriptor.set("Ascent", prepared.ascent);
    descriptor.set("Descent", prepared.descent);
    descriptor.set("CapHeight", prepared.cap_height);
    descriptor.set(
        "FontBBox",
        Object::Array(prepared.bbox.iter().copied().map(Object::Integer).collect()),
    );
    descriptor.set("FontFile2", Object::Reference(font_file));
    let descriptor_id = doc.add_object(descriptor);

    let mut system_info = Dictionary::new();
    system_info.set("Registry", Object::String(b"Adobe".to_vec(), Literal));
    system_info.set("Ordering", Object::String(b"Identity".to_vec(), Literal));
    system_info.set("Supplement", 0_i64);
    let mut cid_font = Dictionary::new();
    cid_font.set("Type", "Font");
    cid_font.set("Subtype", "CIDFontType2");
    cid_font.set("BaseFont", Object::Name(base_font.clone().into_bytes()));
    cid_font.set("CIDSystemInfo", system_info);
    cid_font.set("FontDescriptor", Object::Reference(descriptor_id));
    cid_font.set("CIDToGIDMap", "Identity");
    cid_font.set("DW", DEFAULT_WIDTH);
    cid_font.set("W", widths_array(&prepared.widths));
    let cid_font_id = doc.add_object(cid_font);

    let to_unicode_id = doc.add_object(Stream::new(
        Dictionary::new(),
        prepared.to_unicode_cmap().into_bytes(),
    ));

    let mut type0 = Dictionary::new();
    type0.set("Type", "Font");
    type0.set("Subtype", "Type0");
    type0.set("BaseFont", Object::Name(base_font.into_bytes()));
    type0.set("Encoding", "Identity-H");
    type0.set(
        "DescendantFonts",
        Object::Array(vec![Object::Reference(cid_font_id)]),
    );
    type0.set("ToUnicode", Object::Reference(to_unicode_id));
    doc.add_object(type0)
}

/// `/W`-массив: последовательные CID собираются в один подмассив ширин.
fn widths_array(widths: &BTreeMap<u16, i64>) -> Object {
    let mut items: Vec<Object> = Vec::new();
    let mut start: Option<u16> = None;
    let mut values: Vec<Object> = Vec::new();
    let mut previous: Option<u16> = None;
    for (&cid, &width) in widths {
        if previous.and_then(|prev| prev.checked_add(1)) != Some(cid) {
            flush_run(&mut items, start.take(), std::mem::take(&mut values));
        }
        start.get_or_insert(cid);
        values.push(Object::Integer(width));
        previous = Some(cid);
    }
    flush_run(&mut items, start, values);
    Object::Array(items)
}

/// Добавить накопленный подмассив ширин `[start [w…]]` к `/W`.
fn flush_run(items: &mut Vec<Object>, start: Option<u16>, values: Vec<Object>) {
    if let Some(start) = start {
        items.push(Object::Integer(i64::from(start)));
        items.push(Object::Array(values));
    }
}

/// Ширина глифа в тысячных em; глиф без записи в `hmtx` считается нулевым.
fn glyph_width(
    glyph_metrics: &skrifa::metrics::GlyphMetrics<'_>,
    glyph: skrifa::GlyphId,
    units_per_em: f32,
) -> i64 {
    to_thousandths(
        glyph_metrics.advance_width(glyph).unwrap_or(0.0),
        units_per_em,
    )
}

/// Значение метрики в тысячных em — единице измерения ширин PDF.
#[allow(clippy::cast_possible_truncation)] // метрики шрифта далеки от i64::MAX
fn to_thousandths(value: f32, units_per_em: f32) -> i64 {
    (value * 1000.0 / units_per_em).round() as i64
}

/// Имя гарнитуры из `name`-таблицы: PostScript-имя, иначе family + subfamily.
///
/// В Name-объект PDF не помещаются пробелы и не-ASCII без экранирования,
/// поэтому имя чистится до `[A-Za-z0-9_-]`; пустой результат заменяется
/// нейтральным именем.
fn face_name(font: &FontRef<'_>) -> String {
    let postscript = font
        .localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|name| name.to_string());
    let raw = postscript
        .or_else(|| {
            let family = font
                .localized_strings(StringId::FAMILY_NAME)
                .english_or_first()?;
            Some(
                match font
                    .localized_strings(StringId::SUBFAMILY_NAME)
                    .english_or_first()
                {
                    Some(subfamily) => format!("{family}-{subfamily}"),
                    None => family.to_string(),
                },
            )
        })
        .unwrap_or_else(|| String::from(FALLBACK_FACE_NAME));
    let cleaned: String = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .collect();
    if cleaned.is_empty() {
        String::from(FALLBACK_FACE_NAME)
    } else {
        cleaned
    }
}

/// Тег подмножества — шесть заглавных латинских букв (ISO 32000, 9.6.4).
///
/// Тег считается от байтов подмножества: разные наборы символов дают разные
/// имена, поэтому подмножества одной гарнитуры в документе не совпадут.
fn subset_tag(bytes: &[u8]) -> String {
    // FNV-1a: детерминирован и не требует зависимостей.
    let mut hash: u32 = 0x811C_9DC5;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let mut tag = String::with_capacity(6);
    for &byte in &hash.to_le_bytes()[..3] {
        tag.push(char::from(TAG_ALPHABET[usize::from(byte & 0x0F)]));
        tag.push(char::from(TAG_ALPHABET[usize::from(byte >> 4)]));
    }
    tag
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fmt::Write as _;
    use std::process::Command;

    use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

    use super::{prepare, write_font, PreparedFont};
    use crate::pdf::writer::StreamWriter;

    /// Символы строки — множество для подрезки.
    fn used_of(text: &str) -> BTreeSet<char> {
        text.chars().collect()
    }

    /// Мини-документ из одной страницы с `TEXT`, записанный своим писателем
    /// с `compress: true`: байты PDF, исходный текст и подготовленный шрифт.
    fn mini_pdf() -> (Vec<u8>, &'static str, PreparedFont) {
        const TEXT: &str = "Hello, мир! 123";
        let prepared = prepare(
            doc_converter_render::font::default_font_bytes(),
            &used_of(TEXT),
        )
        .unwrap();

        let mut doc = Document::new();
        let font_id = write_font(&prepared, &mut doc);

        let mut hex = String::new();
        for byte in prepared.encode(TEXT) {
            let _ = write!(hex, "{byte:02X}");
        }
        let content = format!("BT /F0 24 Tf 1 0 0 1 72 700 Tm <{hex}> Tj ET");
        let content_id = doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));

        let mut fonts = Dictionary::new();
        fonts.set("F0", Object::Reference(font_id));
        let mut resources = Dictionary::new();
        resources.set("Font", fonts);
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(595),
                Object::Integer(842),
            ]),
        );
        page.set("Resources", resources);
        page.set("Contents", Object::Reference(content_id));
        let page_id = doc.add_object(page);

        let mut page_tree = Dictionary::new();
        page_tree.set("Type", "Pages");
        page_tree.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        page_tree.set("Count", 1_i64);
        let tree_id = doc.add_object(page_tree);
        doc.objects
            .get_mut(&page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Parent", Object::Reference(tree_id));

        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", Object::Reference(tree_id));
        let catalog_id = doc.add_object(catalog);

        let mut bytes = Vec::new();
        {
            let mut writer = StreamWriter::new(&mut bytes, true);
            writer.begin("1.7", b"\xE2\xE3\xCF\xD3").unwrap();
            for (id, object) in &doc.objects {
                writer.write_indirect(id.0, id.1, object).unwrap();
            }
            let mut trailer = Dictionary::new();
            trailer.set("Size", i64::from(doc.max_id) + 1);
            trailer.set("Root", Object::Reference(catalog_id));
            writer.finish(doc.max_id, &trailer).unwrap();
        }
        (bytes, TEXT, prepared)
    }

    /// Id шрифта страницы: каталог → дерево → страница → `/Resources /Font /F0`.
    fn page_font_id(loaded: &Document) -> ObjectId {
        let root = loaded.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let catalog = loaded.get_dictionary(root).unwrap();
        let pages = catalog.get(b"Pages").unwrap().as_reference().unwrap();
        let pages = loaded.get_dictionary(pages).unwrap();
        let page = pages.get(b"Kids").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap();
        let page = loaded.get_dictionary(page).unwrap();
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        fonts.get(b"F0").unwrap().as_reference().unwrap()
    }

    /// Есть ли утилита в `PATH`: у `pdftotext` нет длинной формы `--version`,
    /// и ненулевой код на незнакомый флаг — не признак отсутствия утилиты.
    fn pdftotext_available() -> bool {
        Command::new("pdftotext").arg("--version").output().is_ok()
    }

    #[test]
    fn subsets_ascii_and_cyrillic() {
        let bytes = doc_converter_render::font::default_font_bytes();
        let text = "Hello, мир!";
        let prepared = prepare(bytes, &used_of(text)).unwrap();

        assert!(
            prepared.subset.len() < bytes.len(),
            "подрезка не уменьшила шрифт: {} из {}",
            prepared.subset.len(),
            bytes.len()
        );
        assert_eq!(prepared.encode(text).len(), text.chars().count() * 2);
        for ch in text.chars() {
            assert_ne!(prepared.cid_by_char[&ch], 0, "символ {ch:?} без глифа");
        }
        // Символ есть в исходном шрифте, но не в подмножестве, — нулевой CID.
        assert_eq!(prepared.encode("ж"), vec![0, 0]);
        // И символа нет вовсе — тоже нулевой CID.
        assert_eq!(prepared.encode("\u{1F600}"), vec![0, 0]);
    }

    #[test]
    fn embeds_font_as_flate_stream_with_length1() {
        let (bytes, _, prepared) = mini_pdf();
        let loaded = Document::load_mem(&bytes).unwrap();

        let font_id = page_font_id(&loaded);
        let type0 = loaded.get_dictionary(font_id).unwrap();
        assert_eq!(type0.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
        assert_eq!(
            type0.get(b"Encoding").unwrap().as_name().unwrap(),
            b"Identity-H"
        );
        let to_unicode = type0.get(b"ToUnicode").unwrap().as_reference().unwrap();
        let to_unicode = loaded.get_object(to_unicode).unwrap().as_stream().unwrap();
        let cmap = String::from_utf8(to_unicode.decompressed_content().unwrap()).unwrap();
        assert!(cmap.contains("beginbfchar"), "{cmap}");
        assert!(cmap.contains("endcmap"), "{cmap}");

        let cid_id = type0.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap();
        let cid = loaded.get_dictionary(cid_id).unwrap();
        assert_eq!(
            cid.get(b"Subtype").unwrap().as_name().unwrap(),
            b"CIDFontType2"
        );
        assert!(cid.get(b"W").unwrap().as_array().unwrap().len() >= 2);
        assert_eq!(cid.get(b"DW").unwrap().as_i64().unwrap(), 1000);

        let descriptor_id = cid.get(b"FontDescriptor").unwrap().as_reference().unwrap();
        let descriptor = loaded.get_dictionary(descriptor_id).unwrap();
        let font_file_id = descriptor
            .get(b"FontFile2")
            .unwrap()
            .as_reference()
            .unwrap();
        let font_file = loaded
            .get_object(font_file_id)
            .unwrap()
            .as_stream()
            .unwrap();
        assert_eq!(
            font_file.dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"FlateDecode"
        );
        assert_eq!(
            font_file.dict.get(b"Length1").unwrap().as_i64().unwrap(),
            i64::try_from(prepared.subset.len()).unwrap()
        );
        assert_eq!(font_file.decompressed_content().unwrap(), prepared.subset);
    }

    #[test]
    fn pdftotext_extracts_subset_text() {
        if !pdftotext_available() {
            eprintln!("пропуск: pdftotext не найден в PATH (в CI ставится шагом «pdf tooling»)");
            return;
        }
        let (bytes, text, _) = mini_pdf();
        let path = std::env::temp_dir().join(format!(
            "doc-converter-pdf-fonts-{}.pdf",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).unwrap();
        let output = Command::new("pdftotext")
            .arg(&path)
            .arg("-")
            .output()
            .expect("pdftotext запускается");
        std::fs::remove_file(&path).unwrap();

        let extracted = String::from_utf8_lossy(&output.stdout);
        // pdftotext расставляет переводы строк и пробелы по позициям глифов —
        // сравнивается текст без пробельных символов.
        let strip = |value: &str| {
            value
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        };
        assert_eq!(strip(&extracted), strip(text), "извлечено: {extracted:?}");
    }

    #[test]
    fn regular_and_bold_subset_independently() {
        let used = used_of("Bold и жирный");
        let regular = prepare(doc_converter_render::font::default_font_bytes(), &used).unwrap();
        let bold = prepare(doc_converter_render::font::bold_font_bytes(), &used).unwrap();

        assert_ne!(regular.subset, bold.subset);
        assert_ne!(regular.face_name, bold.face_name);
        assert_eq!(regular.encode("Bold").len(), 4 * 2);
        assert_eq!(bold.encode("Bold").len(), 4 * 2);
    }

    #[test]
    fn empty_used_set_stays_valid() {
        let empty = BTreeSet::new();
        let prepared = prepare(doc_converter_render::font::default_font_bytes(), &empty).unwrap();
        assert_eq!(prepared.encode("abc"), vec![0, 0, 0, 0, 0, 0]);

        let mut doc = Document::new();
        let font_id = write_font(&prepared, &mut doc);
        let type0 = doc.get_dictionary(font_id).unwrap();
        let cid_id = type0.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap();
        let cid = doc.get_dictionary(cid_id).unwrap();
        // Ширина `.notdef` пишется всегда, `/W` остаётся непустым.
        assert!(cid.get(b"W").unwrap().as_array().unwrap().len() >= 2);
    }
}
