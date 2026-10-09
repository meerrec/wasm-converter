//! Политика ошибок ADR-0016 на фикстурах `test-fixtures/docx/broken/`.
//!
//! Сайдкары фикстур (`metadata.fatal` и `metadata.expectedWarnings`) — эталон
//! того, что видит потребитель: фатальное нарушение возвращает `Err` своего
//! варианта, восстановимое — `Ok` с предупреждением своего вида. Тест держит
//! обе половины таблицы ADR-0016 §2 на реальных пакетах, а не на синтетике:
//! синтетика проверяется в модульных тестах `parse.rs`.

use std::path::PathBuf;

use doc_converter_core::{Error as CoreError, ParseWarning, WarningKind, Warnings};
use doc_converter_docx::{open, Document, Error};

/// Байты фикстуры из `test-fixtures/docx`.
fn fixture(relative: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/docx")
        .join(relative);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Разбор фикстуры через публичный вход крейта.
fn parse(relative: &str) -> Result<Document, Error> {
    open(fixture(relative))
}

/// Фикстура, которая обязана разбираться: `Ok` с подсказкой, какая упала.
fn parsed(relative: &str) -> Document {
    parse(relative).unwrap_or_else(|e| panic!("{relative} должен разбираться: {e}"))
}

/// Виды предупреждений документа в порядке появления.
fn kinds(document: &Document) -> Vec<WarningKind> {
    document.warnings().iter().map(|w| w.kind).collect()
}

// ---------------------------------------------------------------------------
// Фатальные: `Err` (ADR-0016 §2)
// ---------------------------------------------------------------------------

#[test]
fn truncated_zip_is_a_container_error() {
    // Обрезан EOCD — это отказ контейнера, а не разбора: ошибка `zip`
    // приходит прозрачно вложенной в `Error::Core`, разбирать нечего.
    let err = parse("broken/truncated_zip.docx").expect_err("обрезанный ZIP не разбирается");
    assert!(
        matches!(err, Error::Core(CoreError::Zip(_))),
        "ожидался Error::Core(Error::Zip), получено {err:?}"
    );
}

#[test]
fn macro_enabled_document_is_rejected() {
    let err = parse("broken/macro_enabled.docx").expect_err("макросный документ — non-goal v1");
    assert!(
        matches!(err, Error::MacroEnabledDocument),
        "получено {err:?}"
    );
}

#[test]
fn missing_document_xml_is_rejected() {
    let err = parse("broken/no_document_xml.docx").expect_err("главной части нет");
    assert!(matches!(err, Error::MissingDocumentXml), "получено {err:?}");
}

// ---------------------------------------------------------------------------
// Восстановимые: `Ok` + предупреждение (ADR-0016 §2)
// ---------------------------------------------------------------------------

#[test]
fn missing_root_rels_recovers_with_a_warning() {
    let document = parsed("broken/missing_root_rels.docx");
    assert!(
        document.warnings_by_kind(WarningKind::MissingRels).len() == 1,
        "ожидалось одно MissingRels, получено {:?}",
        kinds(&document)
    );
}

#[test]
fn cyclic_based_on_recovers_with_a_warning() {
    let document = parsed("broken/cyclic_based_on.docx");
    assert!(
        !document
            .warnings_by_kind(WarningKind::CyclicBasedOn)
            .is_empty(),
        "ожидался CyclicBasedOn, получено {:?}",
        kinds(&document)
    );
}

#[test]
fn missing_style_and_abstract_num_recover_with_warnings() {
    let document = parsed("broken/missing_style_and_abstract_num.docx");
    let kinds = kinds(&document);
    for expected in [
        WarningKind::MissingStyleRef,
        WarningKind::MissingAbstractNum,
    ] {
        assert!(
            kinds.contains(&expected),
            "ожидался {expected:?}, получено {kinds:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Чистый документ и выборки (ADR-0016 §5)
// ---------------------------------------------------------------------------

#[test]
fn clean_fixture_has_no_warnings() {
    let document = parsed("simple/one_paragraph.docx");
    assert!(
        document.warnings().is_empty(),
        "предупреждения на чистой фикстуре: {:?}",
        kinds(&document)
    );
    assert!(!document.has_fatal_warnings());
}

#[test]
fn warnings_by_kind_matches_a_manual_filter() {
    let document = parsed("broken/missing_style_and_abstract_num.docx");
    for kind in WarningKind::ALL {
        let selected = document.warnings_by_kind(kind);
        let manual: Vec<&ParseWarning> = document
            .warnings()
            .iter()
            .filter(|warning| warning.kind == kind)
            .collect();
        assert_eq!(selected, manual, "выборка {kind:?} разошлась с фильтром");
    }
}

#[test]
fn adr_0016_thresholds_are_visible_to_the_consumer() {
    // Поведение порогов (>100 → показ пользователю, >1000 → `TooManyWarnings`)
    // покрыто в `doc_converter_core::warning`; здесь — сверка значений §6,
    // чтобы потребитель видел их через тот же API, что и модель.
    assert_eq!(Warnings::UI_ALERT_THRESHOLD, 100);
    assert_eq!(Warnings::FATAL_THRESHOLD, 1000);
}
