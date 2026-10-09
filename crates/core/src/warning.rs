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

/// Запись и чтение serde-формы значения.
///
/// `serde_json` в dev-зависимостях крейта нет, а `Cargo.lock` этого слайса
/// заморожен, поэтому форму повторяет своя пара сериализатора и десериализатора:
/// `Number` — число, `Text` — строка, `Null` — `null`, `Object` — объект.
/// Тесты сверяют с ней раскладку [`NodeId`] (просто число), [`WarningKind`]
/// (`snake_case`) и [`ParseWarning`].
#[cfg(test)]
pub(crate) mod wire {
    use std::fmt;

    use serde::de::value::StringDeserializer;
    use serde::de::{self, DeserializeOwned, DeserializeSeed, Deserializer, MapAccess, Visitor};
    use serde::ser::{self, Impossible, SerializeStruct, Serializer};
    use serde::Serialize;

    /// Значение в той форме, в какой его записал бы JSON.
    #[derive(Debug, PartialEq)]
    pub(crate) enum Value {
        Number(u64),
        Text(String),
        Null,
        Object(Vec<(String, Value)>),
    }

    /// Ошибка записи и чтения: тест падает, если раскладка не совпала.
    #[derive(Debug)]
    pub(crate) struct Error(String);

    impl Error {
        fn unsupported() -> Self {
            Self("wire: unexpected type in the checked value".into())
        }
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for Error {}

    impl ser::Error for Error {
        fn custom<T: fmt::Display>(msg: T) -> Self {
            Self(msg.to_string())
        }
    }

    impl de::Error for Error {
        fn custom<T: fmt::Display>(msg: T) -> Self {
            Self(msg.to_string())
        }
    }

    /// Записывает serde-форму значения.
    pub(crate) fn capture<T: ?Sized + Serialize>(value: &T) -> Value {
        value
            .serialize(Capture)
            .expect("wire: value has a JSON form")
    }

    /// Записывает значение и читает его обратно.
    pub(crate) fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        T::deserialize(capture(value)).expect("wire: value survives its own form")
    }

    struct Capture;

    impl Serializer for Capture {
        type Ok = Value;
        type Error = Error;
        type SerializeSeq = Impossible<Value, Error>;
        type SerializeTuple = Impossible<Value, Error>;
        type SerializeTupleStruct = Impossible<Value, Error>;
        type SerializeTupleVariant = Impossible<Value, Error>;
        type SerializeMap = Impossible<Value, Error>;
        type SerializeStruct = Fields;
        type SerializeStructVariant = Impossible<Value, Error>;

        fn serialize_u64(self, value: u64) -> Result<Value, Error> {
            Ok(Value::Number(value))
        }

        fn serialize_str(self, value: &str) -> Result<Value, Error> {
            Ok(Value::Text(value.to_owned()))
        }

        fn serialize_unit_variant(
            self,
            _name: &'static str,
            _index: u32,
            variant: &'static str,
        ) -> Result<Value, Error> {
            // Вариант без полей JSON пишет строкой.
            Ok(Value::Text(variant.to_owned()))
        }

        fn serialize_none(self) -> Result<Value, Error> {
            Ok(Value::Null)
        }

        fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Value, Error> {
            value.serialize(self)
        }

        fn serialize_unit(self) -> Result<Value, Error> {
            Ok(Value::Null)
        }

        fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, Error> {
            Ok(Value::Null)
        }

        fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Fields, Error> {
            Ok(Fields::default())
        }

        // Ниже — типы, которых в проверяемых значениях быть не должно: их
        // появление означает, что раскладка перестала быть JSON-подобной.
        fn serialize_bool(self, _value: bool) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_i8(self, _value: i8) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_i16(self, _value: i16) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_i32(self, _value: i32) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_i64(self, _value: i64) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_u8(self, _value: u8) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_u16(self, _value: u16) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_u32(self, _value: u32) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_f32(self, _value: f32) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_f64(self, _value: f64) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_char(self, _value: char) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_bytes(self, _value: &[u8]) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_newtype_struct<T: ?Sized + Serialize>(
            self,
            _name: &'static str,
            _value: &T,
        ) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_newtype_variant<T: ?Sized + Serialize>(
            self,
            _name: &'static str,
            _index: u32,
            _variant: &'static str,
            _value: &T,
        ) -> Result<Value, Error> {
            Err(Error::unsupported())
        }

        fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Error> {
            Err(Error::unsupported())
        }

        fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
            Err(Error::unsupported())
        }

        fn serialize_tuple_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleStruct, Error> {
            Err(Error::unsupported())
        }

        fn serialize_tuple_variant(
            self,
            _name: &'static str,
            _index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleVariant, Error> {
            Err(Error::unsupported())
        }

        fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
            Err(Error::unsupported())
        }

        fn serialize_struct_variant(
            self,
            _name: &'static str,
            _index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStructVariant, Error> {
            Err(Error::unsupported())
        }
    }

    /// Поля объекта в порядке записи.
    #[derive(Default)]
    struct Fields(Vec<(String, Value)>);

    impl SerializeStruct for Fields {
        type Ok = Value;
        type Error = Error;

        fn serialize_field<T: ?Sized + Serialize>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Error> {
            self.0.push((key.to_owned(), capture(value)));
            Ok(())
        }

        fn end(self) -> Result<Value, Error> {
            Ok(Value::Object(self.0))
        }
    }

    impl<'de> Deserializer<'de> for Value {
        type Error = Error;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self {
                Self::Number(value) => visitor.visit_u64(value),
                Self::Text(text) => visitor.visit_string(text),
                Self::Null => visitor.visit_none(),
                Self::Object(fields) => visitor.visit_map(ObjectMap::new(fields)),
            }
        }

        fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self {
                Self::Null => visitor.visit_none(),
                value => visitor.visit_some(value),
            }
        }

        fn deserialize_enum<V: Visitor<'de>>(
            self,
            name: &'static str,
            variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Error> {
            match self {
                // Вариант без полей записан строкой — её же читает
                // `StringDeserializer`, вместе со своей проверкой имени.
                Self::Text(text) => {
                    StringDeserializer::<Error>::new(text).deserialize_enum(name, variants, visitor)
                }
                _ => Err(Error("wire: enum value must be a string".into())),
            }
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
            map struct identifier ignored_any
        }
    }

    /// Объект, читающий поля в порядке записи.
    struct ObjectMap {
        fields: std::vec::IntoIter<(String, Value)>,
        pending: Option<Value>,
    }

    impl ObjectMap {
        fn new(fields: Vec<(String, Value)>) -> Self {
            Self {
                fields: fields.into_iter(),
                pending: None,
            }
        }
    }

    impl<'de> MapAccess<'de> for ObjectMap {
        type Error = Error;

        fn next_key_seed<K: DeserializeSeed<'de>>(
            &mut self,
            seed: K,
        ) -> Result<Option<K::Value>, Error> {
            let Some((key, value)) = self.fields.next() else {
                return Ok(None);
            };
            self.pending = Some(value);
            seed.deserialize(StringDeserializer::<Error>::new(key))
                .map(Some)
        }

        fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
            let value = self
                .pending
                .take()
                .ok_or_else(|| Error("wire: value without a key".into()))?;
            seed.deserialize(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::warning::wire;

    fn text(value: &str) -> wire::Value {
        wire::Value::Text(value.to_owned())
    }

    fn warning(kind: WarningKind) -> ParseWarning {
        ParseWarning::new(kind, "test warning")
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
            assert_eq!(wire::capture(&kind), text(kind.as_str()));
            assert_eq!(wire::round_trip(&kind), kind);
        }
    }

    #[test]
    fn warning_round_trips_without_location() {
        let warning = warning(WarningKind::MissingRels);
        assert_eq!(
            wire::capture(&warning),
            wire::Value::Object(vec![
                ("kind".to_owned(), text("missing_rels")),
                ("message".to_owned(), text("test warning")),
                ("location".to_owned(), wire::Value::Null),
            ])
        );
        assert_eq!(wire::round_trip(&warning), warning);
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
            wire::capture(&warning),
            wire::Value::Object(vec![
                ("kind".to_owned(), text("missing_style_ref")),
                ("message".to_owned(), text("style is not defined")),
                (
                    "location".to_owned(),
                    wire::Value::Object(vec![
                        ("part".to_owned(), text("word/document.xml")),
                        ("node_id".to_owned(), wire::Value::Number(3)),
                        (
                            "xml_path".to_owned(),
                            text("w:document/w:body/w:p[3]/w:pPr/w:pStyle")
                        ),
                    ])
                ),
            ])
        );
        assert_eq!(wire::round_trip(&warning), warning);
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
