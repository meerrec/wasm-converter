//! Предупреждения парсера и политика восстановления (ADR-0016).

use serde::{Deserialize, Serialize};

use crate::{Error, NodeId, Result};

/// Причина предупреждения — таблица «Восстановимые» из ADR-0016 §2.
///
/// Парсер продолжает работу: каждое такое нарушение стоит в модели как
/// [`ParseWarning`], а не как отказ разбора.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    MissingRels,
    CyclicBasedOn,
    MissingStyleRef,
    MissingAbstractNum,
    MissingNumId,
    MissingPart,
    InvalidAttribute,
    UnknownElement,
    SymlinkIgnored,
    DeepNesting,
    DuplicateStyleId,
    OrphanBookmark,
}

impl WarningKind {
    /// Все варианты: полнота перечня и [`Self::as_str`] проверяются тестом.
    pub const ALL: [Self; 12] = [
        Self::MissingRels,
        Self::CyclicBasedOn,
        Self::MissingStyleRef,
        Self::MissingAbstractNum,
        Self::MissingNumId,
        Self::MissingPart,
        Self::InvalidAttribute,
        Self::UnknownElement,
        Self::SymlinkIgnored,
        Self::DeepNesting,
        Self::DuplicateStyleId,
        Self::OrphanBookmark,
    ];

    /// Имя варианта так, как его видят JSON и логи.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingRels => "missing_rels",
            Self::CyclicBasedOn => "cyclic_based_on",
            Self::MissingStyleRef => "missing_style_ref",
            Self::MissingAbstractNum => "missing_abstract_num",
            Self::MissingNumId => "missing_num_id",
            Self::MissingPart => "missing_part",
            Self::InvalidAttribute => "invalid_attribute",
            Self::UnknownElement => "unknown_element",
            Self::SymlinkIgnored => "symlink_ignored",
            Self::DeepNesting => "deep_nesting",
            Self::DuplicateStyleId => "duplicate_style_id",
            Self::OrphanBookmark => "orphan_bookmark",
        }
    }
}

/// Где именно сработало предупреждение (ADR-0016 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarningLocation {
    /// Часть пакета: `word/document.xml`, `word/styles.xml`, …
    pub part: String,
    pub node_id: Option<NodeId>,
    /// Путь по XML: `w:document/w:body/w:p[3]/w:pPr/w:pStyle`.
    pub xml_path: Option<String>,
}

/// Одно восстановимое нарушение.
///
/// Парсер не прерывает разбор, а накапливает предупреждения в [`Warnings`];
/// пустая часть означает, что место не названо (ADR-0016 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWarning {
    pub kind: WarningKind,
    pub message: String,
    pub location: Option<WarningLocation>,
}

impl ParseWarning {
    /// Предупреждение без привязки к месту.
    #[must_use]
    pub fn new(kind: WarningKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            location: None,
        }
    }

    /// Предупреждение с частью пакета; узел и путь по XML уточняют
    /// [`Self::with_node`] и [`Self::with_xml_path`].
    #[must_use]
    pub fn at(kind: WarningKind, message: impl Into<String>, part: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            location: Some(WarningLocation {
                part: part.into(),
                node_id: None,
                xml_path: None,
            }),
        }
    }

    /// Привязывает предупреждение к узлу модели.
    ///
    /// Если место ещё не заведено [`Self::at`], оно появляется с пустой частью:
    /// у [`WarningLocation`] часть обязательна, но потерять узел из-за порядка
    /// вызовов хуже, чем показать предупреждение без имени части.
    #[must_use]
    pub fn with_node(self, id: NodeId) -> Self {
        self.map_location(|location| WarningLocation {
            node_id: Some(id),
            ..location
        })
    }

    /// Привязывает предупреждение к пути по XML; место заводится так же, как
    /// в [`Self::with_node`].
    #[must_use]
    pub fn with_xml_path(self, path: impl Into<String>) -> Self {
        let path = path.into();
        self.map_location(|location| WarningLocation {
            xml_path: Some(path),
            ..location
        })
    }

    fn map_location(mut self, edit: impl FnOnce(WarningLocation) -> WarningLocation) -> Self {
        let location = self.location.take().unwrap_or(WarningLocation {
            part: String::new(),
            node_id: None,
            xml_path: None,
        });
        self.location = Some(edit(location));
        self
    }
}

/// Накопитель предупреждений парсера.
///
/// Пороги из ADR-0016 §6: больше [`Self::UI_ALERT_THRESHOLD`] предупреждений —
/// повод показать их пользователю, [`Self::FATAL_THRESHOLD`] — предел, после
/// которого разбор признаётся патологическим и [`Warnings::push`] отказывает.
#[derive(Debug, Default)]
pub struct Warnings {
    items: Vec<ParseWarning>,
}

impl Warnings {
    pub const UI_ALERT_THRESHOLD: usize = 100;
    pub const FATAL_THRESHOLD: usize = 1000;

    #[must_use]
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Добавляет предупреждение.
    ///
    /// # Errors
    /// [`Error::TooManyWarnings`], когда накоплено уже [`Self::FATAL_THRESHOLD`]:
    /// предупреждение не добавляется и длина не растёт, так что патологический
    /// вход не раздувает память (ADR-0016 §6). Число в ошибке — сколько
    /// предупреждений уже есть.
    pub fn push(&mut self, warning: ParseWarning) -> Result<()> {
        if self.items.len() >= Self::FATAL_THRESHOLD {
            return Err(Error::TooManyWarnings(self.items.len()));
        }
        self.items.push(warning);
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Пора показать предупреждения пользователю.
    #[must_use]
    pub fn needs_ui_alert(&self) -> bool {
        self.items.len() > Self::UI_ALERT_THRESHOLD
    }

    pub fn iter(&self) -> impl Iterator<Item = &ParseWarning> {
        self.items.iter()
    }

    #[must_use]
    pub fn as_slice(&self) -> &[ParseWarning] {
        &self.items
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<ParseWarning> {
        self.items
    }
}

impl Extend<ParseWarning> for Warnings {
    /// Добавляет предупреждения **без** проверки порога: `Extend` не умеет
    /// вернуть ошибку, а нужен он для слияния уже собранных списков — вход
    /// раздувает память через [`Warnings::push`], не через слияние.
    fn extend<T: IntoIterator<Item = ParseWarning>>(&mut self, iter: T) {
        self.items.extend(iter);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;

    fn warning(kind: WarningKind) -> ParseWarning {
        ParseWarning::new(kind, "test warning")
    }

    fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
        let wire = serde_json::to_string(value).expect("serializes");
        serde_json::from_str(&wire).expect("deserializes")
    }

    #[test]
    fn all_kinds_are_listed_once() {
        assert_eq!(WarningKind::ALL.len(), 12);

        let names: HashSet<&str> = WarningKind::ALL.iter().map(|kind| kind.as_str()).collect();
        let kinds: HashSet<WarningKind> = WarningKind::ALL.iter().copied().collect();

        assert_eq!(names.len(), 12);
        assert_eq!(kinds.len(), 12);
        assert!(names.contains("missing_rels"));
        assert!(names.contains("orphan_bookmark"));
    }

    #[test]
    fn kinds_keep_snake_case_names_on_the_wire() {
        for kind in WarningKind::ALL {
            assert_eq!(
                serde_json::to_value(kind).expect("serializes"),
                json!(kind.as_str())
            );
            let restored: WarningKind =
                serde_json::from_str(&serde_json::to_string(&kind).expect("serializes"))
                    .expect("deserializes");
            assert_eq!(restored, kind);
        }
    }

    #[test]
    fn warning_round_trips_without_location() {
        let warning = warning(WarningKind::MissingRels);
        assert_eq!(
            serde_json::to_value(&warning).expect("serializes"),
            json!({
                "kind": "missing_rels",
                "message": "test warning",
                "location": null,
            })
        );
        assert_eq!(round_trip(&warning), warning);
    }

    #[test]
    fn warning_round_trips_with_location() {
        let warning = ParseWarning::at(
            WarningKind::MissingStyleRef,
            "style is not defined",
            "word/document.xml",
        )
        .with_node(NodeId::new(3))
        .with_xml_path("w:document/w:body/w:p[3]/w:pPr/w:pStyle");

        assert_eq!(
            serde_json::to_value(&warning).expect("serializes"),
            json!({
                "kind": "missing_style_ref",
                "message": "style is not defined",
                "location": {
                    "part": "word/document.xml",
                    "node_id": 3,
                    "xml_path": "w:document/w:body/w:p[3]/w:pPr/w:pStyle",
                },
            })
        );
        assert_eq!(round_trip(&warning), warning);
    }

    #[test]
    fn node_id_inside_a_location_is_a_number() {
        assert_eq!(
            serde_json::to_value(NodeId::new(7)).expect("serializes"),
            json!(7)
        );

        let warning = warning(WarningKind::MissingRels).with_node(NodeId::new(7));
        assert_eq!(
            serde_json::to_value(&warning).expect("serializes"),
            json!({
                "kind": "missing_rels",
                "message": "test warning",
                "location": {
                    "part": "",
                    "node_id": 7,
                    "xml_path": null,
                },
            })
        );
    }

    #[test]
    fn node_without_a_part_keeps_the_node() {
        // Место не задано — часть остаётся пустой, но узел не теряется.
        let warning = warning(WarningKind::OrphanBookmark).with_node(NodeId::new(9));
        let location = warning.location.expect("location is created");

        assert_eq!(location.part, "");
        assert_eq!(location.node_id, Some(NodeId::new(9)));
    }

    #[test]
    fn accessors_see_everything_pushed() {
        let mut warnings = Warnings::new();
        assert!(warnings.is_empty());

        warnings
            .push(warning(WarningKind::InvalidAttribute))
            .expect("below the fatal threshold");

        assert!(!warnings.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings.as_slice().len(), 1);
        assert_eq!(warnings.iter().count(), 1);
        assert_eq!(warnings.into_vec().len(), 1);
    }

    #[test]
    fn ui_alert_fires_above_the_threshold() {
        assert_eq!(Warnings::UI_ALERT_THRESHOLD, 100);
        assert_eq!(Warnings::FATAL_THRESHOLD, 1000);

        let mut warnings = Warnings::new();
        for _ in 0..Warnings::UI_ALERT_THRESHOLD {
            warnings
                .push(warning(WarningKind::MissingPart))
                .expect("below the fatal threshold");
        }
        assert_eq!(warnings.len(), 100);
        assert!(!warnings.needs_ui_alert());

        warnings
            .push(warning(WarningKind::MissingPart))
            .expect("below the fatal threshold");
        assert!(warnings.needs_ui_alert());
    }

    #[test]
    fn fatal_threshold_rejects_the_next_warning() {
        let mut warnings = Warnings::new();
        for _ in 0..Warnings::FATAL_THRESHOLD {
            warnings
                .push(warning(WarningKind::UnknownElement))
                .expect("the threshold itself is allowed");
        }
        assert_eq!(warnings.len(), Warnings::FATAL_THRESHOLD);

        let err = warnings
            .push(warning(WarningKind::UnknownElement))
            .expect_err("the 1001st warning is rejected");

        assert!(matches!(err, Error::TooManyWarnings(1000)), "{err}");
        assert_eq!(warnings.len(), Warnings::FATAL_THRESHOLD);
    }

    #[test]
    fn extend_merges_without_the_threshold() {
        let mut warnings = Warnings::new();
        warnings.extend(vec![
            warning(WarningKind::DuplicateStyleId);
            Warnings::FATAL_THRESHOLD + 500
        ]);

        assert_eq!(warnings.len(), 1500);
    }
}
