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

#[test]
fn a_thousand_distinct_warnings_are_fatal() {
    // Порог считает уникальные предупреждения (ADR-0016 §6, дополнение
    // 12.10.2026): тысяча разных классов проблем — патологическое разнообразие,
    // и разбор обязан остановиться.
    let mut warnings = Warnings::new();
    for index in 0..Warnings::FATAL_THRESHOLD {
        warnings
            .push(ParseWarning::new(
                WarningKind::UnknownElement,
                format!("unknown element #{index}"),
            ))
            .expect("the threshold itself is allowed");
    }
    assert_eq!(warnings.len(), Warnings::FATAL_THRESHOLD);

    let err = warnings
        .push(ParseWarning::new(
            WarningKind::UnknownElement,
            "unknown element #1000",
        ))
        .expect_err("the 1001st distinct warning is rejected");

    assert!(matches!(err, CoreError::TooManyWarnings(1000)), "{err}");
}

#[test]
fn a_million_identical_warnings_are_deduplicated() {
    // Word пишет `w:lang`, `w:bCs`, `w:iCs` в каждый `w:rPr`, поэтому реальный
    // документ приносит тысячи дословно одинаковых `unknown_element`: повторы
    // не должны ни приближать порог, ни раздувать память.
    const REPEATS: usize = 1_000_000;
    let mut warnings = Warnings::new();
    let repeated = ParseWarning::new(WarningKind::UnknownElement, "unknown element `w:lang`");
    for _ in 0..REPEATS {
        warnings
            .push(repeated.clone())
            .expect("repeats never reach the threshold");
    }

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings.total(), REPEATS);
    assert!(!warnings.needs_ui_alert());
}

#[test]
fn warnings_deduplicate_by_kind_message_and_location() {
    let mut warnings = Warnings::new();
    let warning = ParseWarning::at(
        WarningKind::MissingNumId,
        "num_id 7 is not defined",
        "word/document.xml",
    );

    warnings
        .push(warning.clone())
        .expect("below the fatal threshold");
    warnings.push(warning).expect("below the fatal threshold");
    assert_eq!(warnings.len(), 1, "повтор — тот же класс проблемы");
    assert_eq!(warnings.total(), 2, "но total считает все вхождения");

    // Другая часть пакета — другое место, значит другой класс.
    warnings
        .push(ParseWarning::at(
            WarningKind::MissingNumId,
            "num_id 7 is not defined",
            "word/numbering.xml",
        ))
        .expect("below the fatal threshold");
    // Другое сообщение — тоже другой класс.
    warnings
        .push(ParseWarning::at(
            WarningKind::MissingNumId,
            "num_id 8 is not defined",
            "word/document.xml",
        ))
        .expect("below the fatal threshold");

    assert_eq!(warnings.len(), 3);
    assert_eq!(warnings.total(), 4);
}
