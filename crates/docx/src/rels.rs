//! Разбор `_rels/*.rels` (слайс S10b): связи частей пакета и цели ссылок.
//!
//! Саму карту разбирает `doc-converter-core` ([`Archive::rels_for`]); здесь она
//! приводится к модели и превращается в типизированный поиск целей: колонтитулы,
//! сноски, комментарии, картинки и гиперссылки различаются суффиксом типа связи.
//! Отсутствие `_rels`-части — предупреждение [`WarningKind::MissingRels`], а не
//! отказ: часть без rels теряет только ссылки (ADR-0016 §2).

// Вызывающих у парсера ещё нет: их добавит сборка `parse.rs` (S12). До тех пор
// `dead_code` срабатывал бы на каждом элементе модуля; `allow` снимается вместе
// с подключением — как в `context.rs` и `xml.rs`.
#![allow(dead_code)]

use doc_converter_core::rels::{rels_part, Relationship};
use doc_converter_core::{Archive, WarningKind};

use crate::context::ParseCtx;
use crate::error::Result;
use crate::model::Relationships;
use crate::settings::xml_error;

/// Загрузить relationships части, например `word/_rels/document.xml.rels`.
///
/// # Errors
/// [`crate::Error::XmlFatal`] — XML rels не читается; [`crate::Error::Core`] —
/// часть не распаковывается; [`crate::Error::TooManyWarnings`] — предупреждений
/// стало больше порога (ADR-0016 §6).
pub(crate) fn load(
    archive: &mut Archive,
    source_part: &str,
    ctx: &mut ParseCtx,
) -> Result<Relationships> {
    // Имя `.rels`-части — и для ошибки, и для предупреждения: ядро разбирает
    // её под своим именем («_rels»), которое ничего не говорит вызывающему.
    let name = rels_part(source_part);

    let Some(map) = archive
        .rels_for(source_part)
        .map_err(|e| xml_error(&name, e))?
    else {
        ctx.warn(
            WarningKind::MissingRels,
            source_part,
            format!("no `{name}`; relationships of `{source_part}` are ignored"),
        )?;
        return Ok(Relationships::default());
    };

    Ok(Relationships::from_map(map))
}

/// Цели связей с данным суффиксом типа, приведённые к пути части: `(rId, часть, связь)`.
///
/// Суффикс (`/header`, `/footer`, `/footnotes`, `/comments`, `/image`, `/hyperlink`)
/// сравнивается с концом типа связи: полный URI версионно-зависим, а хвост у
/// Transitional и Strict один и тот же. Внешняя цель (`TargetMode="External"`,
/// например гиперссылка) даёт `None`: это ссылка, а не часть пакета.
///
/// Обёртка над [`Relationships::by_type`]: та отдаёт сами связи, а потребителю
/// (S12) нужен ещё и путь части, который считается от части-источника.
pub(crate) fn targets<'a>(
    rels: &'a Relationships,
    source_part: &str,
    type_suffix: &str,
) -> Vec<(&'a str, Option<String>, &'a Relationship)> {
    rels.by_type(type_suffix)
        .map(|(id, rel)| (id, rel.part(source_part), rel))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};
    use std::path::PathBuf;

    use doc_converter_core::rels::RelMap;
    use doc_converter_core::Archive;
    use serde_json::Value;

    use super::*;
    use crate::error::Error;

    const DOCUMENT: &str = "word/document.xml";
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

    fn archive(name: &str) -> Archive {
        let path = fixtures_dir().join(format!("{name}.docx"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        Archive::new(bytes).expect("пакет открывается")
    }

    fn sidecar(name: &str) -> Value {
        let path = fixtures_dir().join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        serde_json::from_str(&text).expect("сайдкар — JSON")
    }

    /// Все значения `part` в сайдкаре — рекурсивно по `content`.
    fn sidecar_parts(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, item) in map {
                    match (key.as_str(), item.as_str()) {
                        ("part", Some(part)) => out.push(part.to_owned()),
                        _ => sidecar_parts(item, out),
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    sidecar_parts(item, out);
                }
            }
            _ => {}
        }
    }

    /// Части колонтитулов по сайдкару: `headers[]`, `footers[]` и `fields[]`
    /// называют их одинаково — путём `word/header*.xml`/`word/footer*.xml`.
    fn sidecar_headers_and_footers(name: &str) -> Vec<String> {
        let content = &sidecar(name)["content"];
        let mut parts = Vec::new();
        sidecar_parts(content, &mut parts);
        parts.retain(|part| part.starts_with("word/header") || part.starts_with("word/footer"));
        parts.sort();
        parts.dedup();
        parts
    }

    fn parts_of(targets: &[(&str, Option<String>, &Relationship)]) -> Vec<String> {
        let mut parts: Vec<String> = targets
            .iter()
            .map(|(_, part, _)| part.clone().expect("цель — часть пакета"))
            .collect();
        parts.sort();
        parts
    }

    /// Каждая фикстура объявляет ровно те колонтитулы, что записаны в сайдкаре.
    #[test]
    fn header_and_footer_targets_match_the_sidecar() {
        for name in FIXTURES {
            let mut archive = archive(name);
            let mut ctx = ParseCtx::new();
            let rels = load(&mut archive, DOCUMENT, &mut ctx).expect("rels разбираются");

            let mut found = parts_of(&targets(&rels, DOCUMENT, "/header"));
            found.extend(parts_of(&targets(&rels, DOCUMENT, "/footer")));
            found.sort();

            assert_eq!(found, sidecar_headers_and_footers(name), "{name}");
            assert!(ctx.warnings().is_empty(), "{name}: {:?}", ctx.warnings());
        }
    }

    /// `first` и `even` — такие же связи, как `default`: все три находятся.
    #[test]
    fn even_odd_fixture_gives_three_distinct_headers() {
        let mut archive = archive("even_odd");
        let mut ctx = ParseCtx::new();
        let rels = load(&mut archive, DOCUMENT, &mut ctx).expect("rels разбираются");

        let found = targets(&rels, DOCUMENT, "/header");
        let ids: Vec<&str> = found.iter().map(|(id, _, _)| *id).collect();
        let parts: Vec<&str> = found
            .iter()
            .map(|(_, part, _)| part.as_deref().expect("цель — часть пакета"))
            .collect();

        assert_eq!(ids, ["rIdHeader1", "rIdHeader2", "rIdHeader3"]);
        assert_eq!(
            parts,
            ["word/header1.xml", "word/header2.xml", "word/header3.xml"]
        );
        assert!(
            targets(&rels, DOCUMENT, "/footer").is_empty(),
            "нижних колонтитулов в фикстуре нет"
        );
    }

    /// Цели считаются от части-источника: у колонтитула свои rels и своя картинка.
    #[test]
    fn a_header_part_resolves_its_own_image() {
        let mut archive = archive("header_with_image");
        let mut ctx = ParseCtx::new();
        let rels = load(&mut archive, "word/header1.xml", &mut ctx).expect("rels разбираются");

        let found = targets(&rels, "word/header1.xml", "/image");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "rIdImg1");
        assert_eq!(found[0].1.as_deref(), Some("word/media/image1.png"));
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    #[test]
    fn a_part_without_rels_warns_and_gives_an_empty_map() {
        let mut archive = Archive::new(zip(&[("word/document.xml", "<w:document/>")]))
            .expect("пакет открывается");
        let mut ctx = ParseCtx::new();

        let rels = load(&mut archive, DOCUMENT, &mut ctx).expect("отсутствие rels не фатально");

        assert!(rels.items.is_empty());
        let warnings = ctx.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::MissingRels);
        let location = warnings[0].location.as_ref().expect("часть указана");
        assert_eq!(location.part, DOCUMENT);
        assert!(
            warnings[0].message.contains("word/_rels/document.xml.rels"),
            "{}",
            warnings[0].message
        );
    }

    #[test]
    fn broken_rels_xml_is_fatal() {
        // Битый XML приходит из ядра как `Malformed`: часть есть, но не читается.
        let mut archive = Archive::new(zip(&[(
            "word/_rels/document.xml.rels",
            "<Relationships><Relationship Id=\"rId1\" Type=\"x\" Target=\"styles.xml\"</Relationships>",
        )]))
        .expect("пакет открывается");
        let mut ctx = ParseCtx::new();

        let err = load(&mut archive, DOCUMENT, &mut ctx).expect_err("битый XML — фатально");

        match err {
            Error::Malformed { part, .. } => assert_eq!(part, "word/_rels/document.xml.rels"),
            other => panic!("ожидалась `Malformed`, пришло `{other}`"),
        }
    }

    #[test]
    fn a_mismatched_tag_in_rels_is_fatal() {
        let mut archive = Archive::new(zip(&[(
            "word/_rels/document.xml.rels",
            "<Relationships><Relationship Id=\"rId1\" Type=\"x\" Target=\"y\"></Wrong></Relationships>",
        )]))
        .expect("пакет открывается");
        let mut ctx = ParseCtx::new();

        let err = load(&mut archive, DOCUMENT, &mut ctx).expect_err("битый XML — фатально");

        match err {
            Error::XmlFatal { part, .. } => assert_eq!(part, "word/_rels/document.xml.rels"),
            other => panic!("ожидался `XmlFatal`, пришло `{other}`"),
        }
    }

    /// Внешние цели — ссылки, а не части пакета: путь им не строится.
    #[test]
    fn external_targets_have_no_part() {
        let map = RelMap::parse(
            br#"<Relationships><Relationship Id="rId1" Type="http://x/hyperlink"
                    Target="https://example.com/a%20b" TargetMode="External"/></Relationships>"#,
        )
        .expect("rels разбираются");
        let rels = Relationships::from_map(map);

        let found = targets(&rels, DOCUMENT, "/hyperlink");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "rId1");
        assert_eq!(found[0].1, None);
        assert_eq!(found[0].2.target, "https://example.com/a%20b");
    }

    #[test]
    fn a_target_is_resolved_against_the_source_part() {
        let map = RelMap::parse(
            br#"<Relationships><Relationship Id="rId1" Type="http://x/footnotes"
                    Target="footnotes.xml"/></Relationships>"#,
        )
        .expect("rels разбираются");
        let rels = Relationships::from_map(map);

        let found = targets(&rels, "word/glossary/document.xml", "/footnotes");

        assert_eq!(found[0].1.as_deref(), Some("word/glossary/footnotes.xml"));
    }

    /// ZIP в памяти: тесту хватает одной части — остальные ему не нужны.
    fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
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
