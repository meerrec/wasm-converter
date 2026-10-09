//! Разбор `docProps/core.xml` и `docProps/app.xml` (слайс S10b): свойства пакета.
//!
//! Обе части необязательны, и их отсутствие — не нарушение: на `docProps/*` не
//! ссылается ни один relationship, поэтому [`Metadata::default`] отдаётся без
//! предупреждения (в отличие от отсутствующих rels, где ссылки терялись бы).
//! Битый XML уже фатален: часть есть, но прочитать её нельзя.

// Вызывающих у парсера ещё нет: их добавит сборка `parse.rs` (S12). До тех пор
// `dead_code` срабатывал бы на каждом элементе модуля; `allow` снимается вместе
// с подключением — как в `context.rs` и `xml.rs`.
#![allow(dead_code)]

use doc_converter_core::xml::XmlReader;
use doc_converter_core::{Archive, WarningKind};
use quick_xml::events::Event;

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::Metadata;
use crate::settings::xml_error;
use crate::xml::{local_name, resolve_reference};

/// Свойства пакета (`dc:title`, `cp:lastModifiedBy`, …).
const CORE_PART: &str = "docProps/core.xml";
/// Свойства приложения (`Application`).
const APP_PART: &str = "docProps/app.xml";

/// Разобрать свойства пакета.
///
/// # Errors
/// [`Error::XmlFatal`] — XML части не читается; [`Error::Malformed`] — часть
/// оборвана; [`Error::Core`] — часть не распаковывается.
pub(crate) fn parse(archive: &mut Archive, ctx: &mut ParseCtx) -> Result<Metadata> {
    let mut metadata = Metadata::default();

    if archive.contains(CORE_PART) {
        let bytes = archive
            .part(CORE_PART)
            .map_err(|e| xml_error(CORE_PART, e))?;
        parse_core(&bytes, ctx, &mut metadata, CORE_PART)?;
    }
    if archive.contains(APP_PART) {
        let bytes = archive.part(APP_PART).map_err(|e| xml_error(APP_PART, e))?;
        parse_app(&bytes, &mut metadata, APP_PART)?;
    }

    Ok(metadata)
}

/// Разобрать `docProps/core.xml`: значения свойств — текст элементов.
fn parse_core(bytes: &[u8], ctx: &mut ParseCtx, metadata: &mut Metadata, part: &str) -> Result<()> {
    // `preserving`, а не `new`: значения свойств — данные, и пробелы по краям
    // текстовых узлов (`Old &quot;quoted&quot; title`) обрезать нельзя.
    let mut reader = XmlReader::preserving(bytes, part);

    // Первый `Start` — корень части (`cp:coreProperties`): свойства идут его
    // детьми, а на самом корне текст не собирается.
    while let Some(event) = next_event(&mut reader, part)? {
        if matches!(event, Event::Start(_)) {
            break;
        }
    }

    while let Some(event) = next_event(&mut reader, part)? {
        // Пустой элемент (`<dc:title/>`) значит отсутствие свойства.
        let Event::Start(element) = event else {
            continue;
        };
        let name = local_name(element.name().as_ref()).to_vec();
        let text = element_text(&mut reader, part)?;

        match name.as_slice() {
            b"title" => metadata.title = property(text),
            b"subject" => metadata.subject = property(text),
            b"creator" => metadata.creator = property(text),
            b"keywords" => metadata.keywords = property(text),
            b"description" => metadata.description = property(text),
            b"lastModifiedBy" => metadata.last_modified_by = property(text),
            b"category" => metadata.category = property(text),
            // Дата остаётся строкой: значение `dcterms:W3CDTF` в модели не разбирается.
            b"created" => metadata.created = property(text),
            b"modified" => metadata.modified = property(text),
            b"revision" => metadata.revision = revision(&text, ctx, part)?,
            // Часть свойств ядру неизвестна (`cp:contentStatus`, `dc:language`, …).
            _ => {}
        }
    }

    Ok(())
}

/// Разобрать `docProps/app.xml`: из расширенных свойств модель несёт `Application`.
fn parse_app(bytes: &[u8], metadata: &mut Metadata, part: &str) -> Result<()> {
    // Текст `Application` — тоже данные: имя приложения может кончаться пробелом.
    let mut reader = XmlReader::preserving(bytes, part);

    while let Some(event) = next_event(&mut reader, part)? {
        let Event::Start(element) = event else {
            continue;
        };
        if local_name(element.name().as_ref()) == b"Application" {
            metadata.application = property(element_text(&mut reader, part)?);
        }
    }

    Ok(())
}

/// Значение свойства: пустая строка — свойства нет.
fn property(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// Номер ревизии: `cp:revision` — число, невалидное значение становится предупреждением.
fn revision(text: &str, ctx: &mut ParseCtx, part: &str) -> Result<Option<u32>> {
    if let Ok(value) = text.trim().parse::<u32>() {
        return Ok(Some(value));
    }
    ctx.warn(
        WarningKind::InvalidAttribute,
        part,
        format!("`cp:revision`: `{text}` is not a number, ignored"),
    )?;
    Ok(None)
}

/// Текст элемента до его парного `End`.
///
/// Ссылки на сущности `quick-xml` отдаёт отдельным событием, поэтому
/// разворачиваются здесь же — иначе `&quot;` в заголовке доехал бы до модели
/// эскейпленной формой.
fn element_text(reader: &mut XmlReader<'_>, part: &str) -> Result<String> {
    let mut text = String::new();
    let mut depth: u32 = 0;

    while let Some(event) = next_event(reader, part)? {
        match event {
            Event::Text(chunk) => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| Error::malformed(part, format!("bad text: {e}")))?;
                text.push_str(&decoded);
            }
            Event::CData(chunk) => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| Error::malformed(part, format!("bad CDATA: {e}")))?;
                text.push_str(&decoded);
            }
            Event::GeneralRef(reference) => text.push_str(&resolve_reference(&reference, part)?),
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => return Ok(text),
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }

    Err(Error::malformed(part, "unexpected end of input"))
}

/// Следующее значимое событие; `None` — конец части.
fn next_event(reader: &mut XmlReader<'_>, part: &str) -> Result<Option<Event<'static>>> {
    reader.next_significant().map_err(|e| xml_error(part, e))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use doc_converter_core::Archive;
    use serde_json::Value;

    use super::*;

    const FIXTURES: [&str; 5] = [
        "default_header_footer",
        "even_odd",
        "header_with_image",
        "page_number_field",
        "title_pg_first",
    ];

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx/headers_footers")
    }

    fn fixture(name: &str) -> Archive {
        let path = fixtures_dir().join(format!("{name}.docx"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        Archive::new(bytes).expect("пакет открывается")
    }

    fn sidecar(name: &str) -> Value {
        let path = fixtures_dir().join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        serde_json::from_str(&text).expect("сайдкар — JSON")
    }

    /// Заголовок каждой фикстуры совпадает с сайдкаром: `dc:title` доходит
    /// до модели целиком, вместе с развёрнутыми сущностями (`&quot;`).
    #[test]
    fn titles_of_every_fixture_match_the_sidecar() {
        for name in FIXTURES {
            let mut archive = fixture(name);
            let mut ctx = ParseCtx::new();
            let metadata = parse(&mut archive, &mut ctx).expect("свойства разбираются");

            let expected = sidecar(name)["metadata"]["docTitle"]
                .as_str()
                .expect("docTitle — строка")
                .to_owned();
            assert_eq!(metadata.title.as_deref(), Some(expected.as_str()), "{name}");
            assert_eq!(ctx.warnings(), [], "{name}");
        }
    }

    /// Сгенерированные фикстуры несут один и тот же набор свойств.
    #[test]
    fn generated_fixtures_carry_the_full_property_set() {
        let mut archive = fixture("default_header_footer");
        let mut ctx = ParseCtx::new();
        let metadata = parse(&mut archive, &mut ctx).expect("свойства разбираются");

        assert_eq!(metadata.creator.as_deref(), Some("doc-converter fixtures"));
        assert_eq!(
            metadata.last_modified_by.as_deref(),
            Some("doc-converter fixtures")
        );
        assert_eq!(metadata.revision, Some(1));
        assert_eq!(metadata.created.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(metadata.modified.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(
            metadata.application.as_deref(),
            Some("doc-converter fixture generator")
        );
        assert_eq!(metadata.subject, None, "свойства нет в части");
    }

    #[test]
    fn a_package_without_doc_props_yields_the_default_without_warnings() {
        let mut empty = Archive::new(zip(&[("word/document.xml", "<w:document/>")]))
            .expect("пакет открывается");
        let mut ctx = ParseCtx::new();

        let metadata = parse(&mut empty, &mut ctx).expect("отсутствие свойств не фатально");

        assert_eq!(metadata, Metadata::default());
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    #[test]
    fn properties_are_read_by_shape_not_by_prefix() {
        let mut ctx = ParseCtx::new();
        let mut metadata = Metadata::default();
        let xml = br#"<cp:coreProperties xmlns:cp="http://x" xmlns:dc="http://y">
                <dc:title>Old &quot;quoted&quot; title</dc:title>
                <cp:lastModifiedBy>pad &amp; pencil</cp:lastModifiedBy>
                <cp:revision>12</cp:revision>
                <dc:subject/>
            </cp:coreProperties>"#;

        parse_core(xml, &mut ctx, &mut metadata, CORE_PART).expect("часть разбирается");

        assert_eq!(
            metadata.title.as_deref(),
            Some("Old \"quoted\" title"),
            "сущности разворачиваются"
        );
        assert_eq!(metadata.last_modified_by.as_deref(), Some("pad & pencil"));
        assert_eq!(metadata.revision, Some(12));
        assert_eq!(metadata.subject, None, "пустой элемент — свойства нет");
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    #[test]
    fn an_invalid_revision_is_a_warning() {
        let mut ctx = ParseCtx::new();
        let mut metadata = Metadata::default();
        let xml = br"<cp:coreProperties><cp:revision>many</cp:revision></cp:coreProperties>";

        parse_core(xml, &mut ctx, &mut metadata, CORE_PART).expect("часть разбирается");

        assert_eq!(metadata.revision, None);
        assert_eq!(ctx.warnings().len(), 1);
        assert_eq!(ctx.warnings()[0].kind, WarningKind::InvalidAttribute);
    }

    #[test]
    fn broken_part_xml_is_fatal() {
        for xml in [
            b"<cp:coreProperties><dc:title>broken".as_slice(),
            b"<cp:coreProperties><dc:title></cp:x></cp:coreProperties>".as_slice(),
        ] {
            let mut ctx = ParseCtx::new();
            let mut metadata = Metadata::default();
            let err = parse_core(xml, &mut ctx, &mut metadata, CORE_PART)
                .expect_err("битый XML — фатально");

            assert!(
                matches!(err, Error::XmlFatal { .. } | Error::Malformed { .. }),
                "{err}"
            );
        }
    }

    /// Минимальный ZIP: свойствам пакета довольно одного места в архиве.
    fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
        use std::io::{Cursor, Write};

        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in parts {
                zip.start_file(*name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }
}
