//! Разбор `word/settings.xml` (слайс S10b): параметры документа и флаги совместимости.
//!
//! Часть необязательна: нет `word/settings.xml` — берётся [`Settings::default`].
//! Незнакомый элемент внутри `w:settings` — не ошибка (ADR-0016 §1): из файла
//! читаются только те элементы, что есть в модели, остальные пропускаются молча.
//!
//! Модуль несёт общий для частей S10b маппер ошибок XML ([`xml_error`]): `error.rs`
//! закрыт для правок слайса, а дублировать его в трёх файлах хуже, чем держать в одном.

// Вызывающих у парсера ещё нет: их добавит сборка `parse.rs` (S12). До тех пор
// `dead_code` срабатывал бы на каждом элементе модуля; `allow` снимается вместе
// с подключением — как в `context.rs` и `xml.rs`.
#![allow(dead_code)]

use doc_converter_core::xml::XmlReader;
use quick_xml::events::{BytesStart, Event};

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::numbering::NumFmt;
use crate::model::raw::Toggle;
use crate::model::settings::{
    CharacterSpacingControl, CompatSettings, EndnotePos, EndnotePr, FootnotePos, FootnotePr,
    NumRestart, Settings,
};
use crate::xml::{
    attr_i32, attr_toggle, attr_u32, attributes, capture_element, find, local_name, Attr,
};

/// Ошибка чтения XML становится своим вариантом парсера (ADR-0016 §2).
///
/// `quick-xml` отдаёт её как `core::Error::Xml` с частью и позицией, а
/// relationships — как `core::Error::Malformed` без части; без переноса эти
/// поля терялись бы внутри [`Error::Core`], а вызывающий не отличил бы порчу
/// XML от, например, ошибки распаковки.
pub(crate) fn xml_error(part: &str, err: doc_converter_core::Error) -> Error {
    match err {
        doc_converter_core::Error::Xml {
            position, message, ..
        } => Error::XmlFatal {
            part: part.to_owned(),
            position,
            message,
        },
        doc_converter_core::Error::Malformed(reason) => Error::malformed(part, reason),
        other => Error::from(other),
    }
}

/// Разобрать `word/settings.xml`.
///
/// # Errors
/// [`Error::XmlFatal`] — XML не читается; [`Error::TooManyWarnings`] —
/// предупреждений стало больше порога (ADR-0016 §6).
pub(crate) fn parse(bytes: &[u8], ctx: &mut ParseCtx, part: &str) -> Result<Settings> {
    let mut settings = Settings::default();
    let mut reader = XmlReader::new(bytes, part);

    while let Some(event) = next_event(&mut reader, part)? {
        let (element, empty) = match event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            _ => continue,
        };
        let attrs = attributes(&element, part)?;

        match local_name(element.name().as_ref()) {
            b"defaultTabStop" => {
                settings.default_tab_stop = attr_i32(&attrs, "val", ctx, part)?.map(Into::into);
            }
            b"evenAndOddHeaders" => {
                settings.even_and_odd_headers =
                    toggle_value(&attrs, ctx, part, "w:evenAndOddHeaders")?;
            }
            // Параметры сносок заданы детьми, у пустого элемента их нет.
            b"footnotePr" if !empty => {
                settings.footnote_pr = parse_note_props(&mut reader, ctx, part)?.into();
            }
            b"endnotePr" if !empty => {
                settings.endnote_pr = parse_note_props(&mut reader, ctx, part)?.into();
            }
            b"compat" if !empty => parse_compat(&mut reader, ctx, part, &mut settings.compat)?,
            b"characterSpacingControl" => {
                settings.character_spacing_control = character_spacing_control(find(&attrs, "val"));
            }
            _ => {}
        }
    }

    Ok(settings)
}

/// Следующее значимое событие; `None` — конец части.
fn next_event(reader: &mut XmlReader<'_>, part: &str) -> Result<Option<Event<'static>>> {
    reader.next_significant().map_err(|e| xml_error(part, e))
}

/// Тумблер как `bool`: элемент без `w:val` включён — так это читает Word, и
/// `Inherit` для параметра документа тоже значит «включено», наследовать ему не от кого.
fn toggle_value(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str, element: &str) -> Result<bool> {
    Ok(!matches!(
        attr_toggle(attrs, ctx, part, element)?,
        Some(Toggle::Off)
    ))
}

/// Поля `w:footnotePr` и `w:endnotePr` до приведения к модельным типам.
///
/// Оба элемента различаются только типом `w:pos` (`ST_FtnPos` и `ST_EdnPos`),
/// поэтому разбираются одним проходом.
#[derive(Default)]
struct NoteProps {
    pos: Option<String>,
    num_fmt: Option<NumFmt>,
    num_start: Option<u32>,
    num_restart: Option<NumRestart>,
}

impl From<NoteProps> for FootnotePr {
    fn from(props: NoteProps) -> Self {
        Self {
            pos: props.pos.as_deref().map(footnote_pos),
            num_fmt: props.num_fmt,
            num_start: props.num_start,
            num_restart: props.num_restart,
        }
    }
}

impl From<NoteProps> for EndnotePr {
    fn from(props: NoteProps) -> Self {
        Self {
            pos: props.pos.as_deref().map(endnote_pos),
            num_fmt: props.num_fmt,
            num_start: props.num_start,
            num_restart: props.num_restart,
        }
    }
}

/// Разобрать `w:footnotePr`/`w:endnotePr`: события до парного `End`.
fn parse_note_props(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<NoteProps> {
    let mut props = NoteProps::default();
    let mut depth: u32 = 0;

    while let Some(event) = next_event(reader, part)? {
        match event {
            Event::Empty(element) => read_note_prop(&element, ctx, part, &mut props)?,
            Event::Start(element) => {
                read_note_prop(&element, ctx, part, &mut props)?;
                depth += 1;
            }
            Event::End(_) if depth == 0 => return Ok(props),
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }

    Err(Error::malformed(part, "unexpected end of input"))
}

/// Прочитать один параметр сноски; незнакомый элемент пропускается.
fn read_note_prop(
    element: &BytesStart<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    props: &mut NoteProps,
) -> Result<()> {
    let attrs = attributes(element, part)?;
    match local_name(element.name().as_ref()) {
        b"pos" => props.pos = find(&attrs, "val").map(str::to_owned),
        b"numFmt" => props.num_fmt = find(&attrs, "val").map(NumFmt::from_ooxml),
        b"numStart" => props.num_start = attr_u32(&attrs, "val", ctx, part)?,
        b"numRestart" => props.num_restart = find(&attrs, "val").map(num_restart),
        _ => {}
    }
    Ok(())
}

/// Разобрать `w:compat`: флаги модели, остальные дети — в [`CompatSettings::unknown`].
fn parse_compat(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    compat: &mut CompatSettings,
) -> Result<()> {
    let mut depth: u32 = 0;

    while let Some(event) = next_event(reader, part)? {
        match event {
            Event::Empty(element) if depth == 0 => {
                if !read_compat(&element, ctx, part, compat)? {
                    let name = element_name(&element);
                    let xml = empty_element_xml(&element, part)?;
                    compat.unknown.push((name, xml));
                }
            }
            Event::Start(element) if depth == 0 => {
                if read_compat(&element, ctx, part, compat)? {
                    // Содержимое тумблера (CT_OnOff) не значимо, но дочитать его надо.
                    depth += 1;
                } else {
                    let name = element_name(&element);
                    let xml = capture_element(reader, &element, ctx, part)?;
                    compat.unknown.push((name, xml));
                }
            }
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => return Ok(()),
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }

    Err(Error::malformed(part, "unexpected end of input"))
}

/// Прочитать один флаг `w:compat`; `false` — элемент незнаком и его надо сохранить.
fn read_compat(
    element: &BytesStart<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    compat: &mut CompatSettings,
) -> Result<bool> {
    let attrs = attributes(element, part)?;
    let name = element_name(element);

    if let Some(flag) = compat_flag(name.as_bytes()) {
        let on = toggle_value(&attrs, ctx, part, &name)?;
        compat.apply(flag, on);
        return Ok(true);
    }

    // Признак перекрытия табличного стиля Word пишет не тумблером, а
    // compatSetting с именем `overrideTableStyleFontSizeAndJustification` (MS-DOCX §2.3.1).
    if name == "compatSetting"
        && find(&attrs, "name") == Some("overrideTableStyleFontSizeAndJustification")
    {
        compat.compat_setting_override_table_style = toggle_value(&attrs, ctx, part, &name)?;
        return Ok(true);
    }

    Ok(false)
}

/// Флаги `w:compat`, которые есть в [`CompatSettings`].
#[derive(Clone, Copy)]
enum CompatFlag {
    DoNotExpandShiftReturn,
    DoNotUseHtmlParagraphAutoSpacing,
    DoNotAutofitTables,
    DoNotBreakWrappedTables,
    DoNotVertAlignInCellWithSp,
    DoNotUseEastAsianBreakRules,
    UseSingleBorderForContiguousCells,
}

/// Флаг по имени элемента `w:compat`.
///
/// `doNotAutofitTables` в `CT_Compat` нет (ECMA-376 §17.15.1.19): полю модели
/// отвечает ближайший по смыслу `doNotAutofitConstrainedTables`.
fn compat_flag(name: &[u8]) -> Option<CompatFlag> {
    Some(match name {
        b"doNotExpandShiftReturn" => CompatFlag::DoNotExpandShiftReturn,
        b"doNotUseHTMLParagraphAutoSpacing" => CompatFlag::DoNotUseHtmlParagraphAutoSpacing,
        b"doNotAutofitConstrainedTables" => CompatFlag::DoNotAutofitTables,
        b"doNotBreakWrappedTables" => CompatFlag::DoNotBreakWrappedTables,
        b"doNotVertAlignCellWithSp" => CompatFlag::DoNotVertAlignInCellWithSp,
        b"doNotUseEastAsianBreakRules" => CompatFlag::DoNotUseEastAsianBreakRules,
        b"useSingleBorderforContiguousCells" => CompatFlag::UseSingleBorderForContiguousCells,
        _ => return None,
    })
}

impl CompatSettings {
    /// Записать разобранный флаг; имена вариантов совпадают с полями.
    fn apply(&mut self, flag: CompatFlag, on: bool) {
        match flag {
            CompatFlag::DoNotExpandShiftReturn => self.do_not_expand_shift_return = on,
            CompatFlag::DoNotUseHtmlParagraphAutoSpacing => {
                self.do_not_use_html_paragraph_auto_spacing = on;
            }
            CompatFlag::DoNotAutofitTables => self.do_not_autofit_tables = on,
            CompatFlag::DoNotBreakWrappedTables => self.do_not_break_wrapped_tables = on,
            CompatFlag::DoNotVertAlignInCellWithSp => self.do_not_vert_align_in_cell_wi = on,
            CompatFlag::DoNotUseEastAsianBreakRules => self.do_not_use_east_asian_break = on,
            CompatFlag::UseSingleBorderForContiguousCells => {
                self.use_single_border_for_contiguous_cells = on;
            }
        }
    }
}

/// Локальное имя элемента строкой — для предупреждений и [`CompatSettings::unknown`].
fn element_name(element: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(local_name(element.name().as_ref())).into_owned()
}

/// XML пустого элемента: `<имя атрибуты/>` — как в исходной части.
fn empty_element_xml(element: &BytesStart<'_>, part: &str) -> Result<String> {
    let raw = std::str::from_utf8(element)
        .map_err(|e| Error::malformed(part, format!("event bytes are not UTF-8: {e}")))?;
    Ok(format!("<{raw}/>"))
}

/// Правила сжатия интервалов (`ST_CharacterSpacing`); незнакомое значение — как есть.
fn character_spacing_control(raw: Option<&str>) -> CharacterSpacingControl {
    match raw {
        // Элемента нет — умолчание `WordprocessingML`, оно же `doNotCompress`.
        None | Some("doNotCompress") => CharacterSpacingControl::DoNotCompress,
        Some("compressPunctuation") => CharacterSpacingControl::CompressPunctuation,
        Some("compressPunctuationAndJapaneseKana") => {
            CharacterSpacingControl::CompressPunctuationAndJapaneseKana
        }
        Some(other) => CharacterSpacingControl::Other(other.to_owned()),
    }
}

/// Положение сносок (`ST_FtnPos`).
fn footnote_pos(raw: &str) -> FootnotePos {
    match raw {
        "pageBottom" => FootnotePos::PageBottom,
        "beneathText" => FootnotePos::BeneathText,
        "sectEnd" => FootnotePos::SectionEnd,
        "docEnd" => FootnotePos::DocEnd,
        other => FootnotePos::Other(other.to_owned()),
    }
}

/// Положение концевых сносок (`ST_EdnPos`).
fn endnote_pos(raw: &str) -> EndnotePos {
    match raw {
        "sectEnd" => EndnotePos::SectionEnd,
        "docEnd" => EndnotePos::DocEnd,
        other => EndnotePos::Other(other.to_owned()),
    }
}

/// Перезапуск нумерации (`ST_RestartNumber`).
fn num_restart(raw: &str) -> NumRestart {
    match raw {
        "continuous" => NumRestart::Continuous,
        "eachSect" => NumRestart::EachSection,
        "eachPage" => NumRestart::EachPage,
        other => NumRestart::Other(other.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use doc_converter_core::Archive;

    use super::*;
    use crate::model::raw::Twips;
    use crate::model::settings::{EndnotePos, FootnotePos, NumRestart};

    const PART: &str = "word/settings.xml";

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx/headers_footers")
    }

    fn parse_str(xml: &str) -> Result<(Settings, Vec<doc_converter_core::ParseWarning>)> {
        let mut ctx = ParseCtx::new();
        let settings = parse(xml.as_bytes(), &mut ctx, PART)?;
        Ok((settings, ctx.warnings().to_vec()))
    }

    /// Настройки фикстуры `even_odd`: разметка колонтитулов задана в документе,
    /// а не в самой части, поэтому она несёт только флаги.
    #[test]
    fn even_odd_fixture_settings_are_read() {
        let path = fixtures_dir().join("even_odd.docx");
        let bytes = std::fs::read(&path).expect("фикстура читается");
        let mut archive = Archive::new(bytes).expect("пакет открывается");
        let bytes = archive.part(PART).expect("часть есть в пакете");

        let mut ctx = ParseCtx::new();
        let settings = parse(&bytes, &mut ctx, PART).expect("часть разбирается");

        assert_eq!(settings.default_tab_stop, Some(Twips::new(708)));
        assert!(settings.even_and_odd_headers);
        assert_eq!(
            settings.character_spacing_control,
            CharacterSpacingControl::DoNotCompress
        );
        // `w:compat` фикстуры содержит только compatSetting режима совместимости.
        assert_eq!(
            settings.compat.unknown,
            [(
                "compatSetting".to_owned(),
                r#"<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>"#
                    .to_owned()
            )]
        );
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    #[test]
    fn note_props_are_read_for_both_kinds() {
        let (settings, warnings) = parse_str(
            r#"<w:settings xmlns:w="http://x">
                <w:footnotePr>
                    <w:pos w:val="beneathText"/>
                    <w:numFmt w:val="lowerRoman"/>
                    <w:numStart w:val="3"/>
                    <w:numRestart w:val="eachSect"/>
                </w:footnotePr>
                <w:endnotePr>
                    <w:pos w:val="docEnd"/>
                    <w:numFmt w:val="decimal"/>
                    <w:numStart w:val="1"/>
                    <w:numRestart w:val="continuous"/>
                </w:endnotePr>
            </w:settings>"#,
        )
        .expect("настройки разбираются");

        assert_eq!(settings.footnote_pr.pos, Some(FootnotePos::BeneathText));
        assert_eq!(settings.footnote_pr.num_fmt, Some(NumFmt::LowerRoman));
        assert_eq!(settings.footnote_pr.num_start, Some(3));
        assert_eq!(
            settings.footnote_pr.num_restart,
            Some(NumRestart::EachSection)
        );
        assert_eq!(settings.endnote_pr.pos, Some(EndnotePos::DocEnd));
        assert_eq!(settings.endnote_pr.num_fmt, Some(NumFmt::Decimal));
        assert_eq!(settings.endnote_pr.num_start, Some(1));
        assert_eq!(
            settings.endnote_pr.num_restart,
            Some(NumRestart::Continuous)
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn an_empty_note_props_element_keeps_the_defaults() {
        let (settings, _) = parse_str("<w:settings><w:footnotePr/><w:endnotePr/></w:settings>")
            .expect("настройки разбираются");

        assert_eq!(settings.footnote_pr, FootnotePr::default());
        assert_eq!(settings.endnote_pr, EndnotePr::default());
    }

    #[test]
    fn unknown_note_values_are_kept_as_is() {
        let (settings, warnings) = parse_str(
            r#"<w:settings><w:footnotePr>
                <w:pos w:val="marginTop"/><w:numFmt w:val="bogus"/><w:numRestart w:val="never"/>
            </w:footnotePr></w:settings>"#,
        )
        .expect("настройки разбираются");

        assert_eq!(
            settings.footnote_pr.pos,
            Some(FootnotePos::Other("marginTop".to_owned()))
        );
        assert_eq!(
            settings.footnote_pr.num_fmt,
            Some(NumFmt::Other("bogus".to_owned()))
        );
        assert_eq!(
            settings.footnote_pr.num_restart,
            Some(NumRestart::Other("never".to_owned()))
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn compat_flags_are_read_and_the_rest_goes_to_unknown() {
        let (settings, warnings) = parse_str(
            r#"<w:settings xmlns:w="http://x">
                <w:compat>
                    <w:doNotExpandShiftReturn/>
                    <w:doNotUseHTMLParagraphAutoSpacing w:val="false"/>
                    <w:doNotAutofitConstrainedTables/>
                    <w:doNotVertAlignCellWithSp/>
                    <w:doNotUseEastAsianBreakRules/>
                    <w:useSingleBorderforContiguousCells/>
                    <w:compatSetting w:name="overrideTableStyleFontSizeAndJustification"
                                     w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>
                    <w:doNotLeaveBackslashAlone/>
                </w:compat>
            </w:settings>"#,
        )
        .expect("настройки разбираются");

        let compat = settings.compat;
        assert!(compat.do_not_expand_shift_return);
        assert!(
            !compat.do_not_use_html_paragraph_auto_spacing,
            "`w:val=\"false\"` — выключено"
        );
        assert!(compat.do_not_autofit_tables);
        assert!(compat.do_not_vert_align_in_cell_wi);
        assert!(compat.do_not_use_east_asian_break);
        assert!(compat.use_single_border_for_contiguous_cells);
        assert!(compat.compat_setting_override_table_style);
        assert!(!compat.do_not_break_wrapped_tables, "флага нет в файле");
        assert_eq!(
            compat.unknown,
            [(
                "doNotLeaveBackslashAlone".to_owned(),
                "<w:doNotLeaveBackslashAlone/>".to_owned()
            )]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_non_override_compat_setting_stays_unknown() {
        let (settings, _) = parse_str(
            r#"<w:settings><w:compat>
                <w:compatSetting w:name="compatibilityMode" w:uri="http://x" w:val="15"/>
            </w:compat></w:settings>"#,
        )
        .expect("настройки разбираются");

        assert!(!settings.compat.compat_setting_override_table_style);
        assert_eq!(settings.compat.unknown.len(), 1);
        assert_eq!(settings.compat.unknown[0].0, "compatSetting");
    }

    #[test]
    fn even_and_odd_headers_accepts_both_spellings_of_a_toggle() {
        let (on, _) = parse_str("<w:settings><w:evenAndOddHeaders/></w:settings>").expect("разбор");
        let (off, _) = parse_str(r#"<w:settings><w:evenAndOddHeaders w:val="0"/></w:settings>"#)
            .expect("разбор");

        assert!(on.even_and_odd_headers);
        assert!(!off.even_and_odd_headers);
    }

    #[test]
    fn a_broken_tab_stop_is_a_warning_not_a_failure() {
        let (settings, warnings) =
            parse_str(r#"<w:settings><w:defaultTabStop w:val="wide"/></w:settings>"#)
                .expect("настройки разбираются");

        assert_eq!(settings.default_tab_stop, None);
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].kind,
            doc_converter_core::WarningKind::InvalidAttribute
        );
    }

    #[test]
    fn unknown_settings_elements_are_ignored_silently() {
        let (settings, warnings) = parse_str(
            r#"<w:settings xmlns:w="http://x" xmlns:wc="http://y">
                <w:zoom w:percent="100"/>
                <w:rsids><w:rsid w:val="00AB12CD"/></w:rsids>
                <w:characterSpacingControl w:val="compressPunctuation"/>
                <wc:docId w:val="{7C1E}"/>
            </w:settings>"#,
        )
        .expect("настройки разбираются");

        assert_eq!(
            settings.character_spacing_control,
            CharacterSpacingControl::CompressPunctuation
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn an_unknown_character_spacing_value_is_kept_as_is() {
        let (settings, _) =
            parse_str(r#"<w:settings><w:characterSpacingControl w:val="squeeze"/></w:settings>"#)
                .expect("настройки разбираются");

        assert_eq!(
            settings.character_spacing_control,
            CharacterSpacingControl::Other("squeeze".to_owned())
        );
    }

    #[test]
    fn broken_xml_is_fatal() {
        for xml in [
            "<w:settings><w:defaultTabStop w:val></w:settings>",
            "<w:settings><w:compat><w:doNotExpandShiftReturn></w:settings>",
        ] {
            let mut ctx = ParseCtx::new();
            let err = parse(xml.as_bytes(), &mut ctx, PART).expect_err("битый XML — фатально");
            assert!(
                matches!(err, Error::XmlFatal { .. } | Error::Malformed { .. }),
                "{xml}: {err}"
            );
        }
    }
}
