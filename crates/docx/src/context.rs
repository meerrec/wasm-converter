//! Контекст разбора: один аллокатор ID и один накопитель предупреждений на документ.

// Вызывающих у контекста ещё нет: их добавят парсеры частей (слайсы S6–S12), а
// сборка начнётся с `parse.rs` (S12). До тех пор `dead_code` срабатывал бы на
// каждом элементе модуля; `allow` снимается вместе с подключением.
#![allow(dead_code)]

use std::collections::BTreeSet;

use doc_converter_core::{NodeId, NodeIdAllocator, ParseWarning, WarningKind, Warnings};

use crate::error::{Error, Result};

/// Контекст разбора: один аллокатор и один накопитель предупреждений на документ.
///
/// Аллокатор общий на все части пакета: ID выдаются в порядке обхода XML, поэтому
/// один и тот же файл даёт одну и ту же нумерацию (ADR-0019 §2). Предупреждения
/// копятся в том же порядке, в каком их встретили парсеры частей.
pub(crate) struct ParseCtx {
    ids: NodeIdAllocator,
    warnings: Warnings,
    /// Идентификаторы начатых закладок (`w:bookmarkStart`).
    ///
    /// Набор на документ, а не на абзац: закладка вправе охватывать несколько
    /// абзацев, и `w:bookmarkEnd` встречается далеко от своего начала.
    bookmarks: BTreeSet<i64>,
}

impl ParseCtx {
    /// Пустой контекст: нумерация ID начинается с 1, предупреждений нет.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            ids: NodeIdAllocator::new(),
            warnings: Warnings::new(),
            bookmarks: BTreeSet::new(),
        }
    }

    /// Следующий идентификатор узла.
    #[must_use]
    pub(crate) fn id(&mut self) -> NodeId {
        self.ids.alloc()
    }

    /// Запомнить начало закладки (`w:bookmarkStart`).
    pub(crate) fn start_bookmark(&mut self, bookmark_id: i64) {
        self.bookmarks.insert(bookmark_id);
    }

    /// Закрыть закладку: `true`, если начало встретилось в этом же документе.
    ///
    /// Повторный `w:bookmarkEnd` с тем же `w:id` пары уже не находит — второй
    /// такой конец считается потерянным.
    #[must_use]
    pub(crate) fn end_bookmark(&mut self, bookmark_id: i64) -> bool {
        self.bookmarks.remove(&bookmark_id)
    }

    /// Добавляет предупреждение с именем части пакета.
    ///
    /// # Errors
    /// [`Error::TooManyWarnings`], когда предупреждений набралось уже
    /// [`Warnings::FATAL_THRESHOLD`]: разбор обязан остановиться, иначе
    /// патологический вход раздувает память (ADR-0016 §6).
    pub(crate) fn warn(
        &mut self,
        kind: WarningKind,
        part: &str,
        message: impl Into<String>,
    ) -> Result<()> {
        self.warnings
            .push(ParseWarning::at(kind, message, part))
            .map_err(Error::from)
    }

    /// Добавляет предупреждение с путём по XML внутри части.
    ///
    /// `xml_path` необязателен: часть парсеров знает узел, но не путь до него.
    ///
    /// # Errors
    /// То же, что у [`Self::warn`].
    pub(crate) fn warn_at(
        &mut self,
        kind: WarningKind,
        part: &str,
        xml_path: Option<&str>,
        message: impl Into<String>,
    ) -> Result<()> {
        let warning = ParseWarning::at(kind, message, part);
        let warning = match xml_path {
            Some(path) => warning.with_xml_path(path),
            None => warning,
        };
        self.warnings.push(warning).map_err(Error::from)
    }

    /// Добавляет предупреждение, привязанное к узлу модели.
    ///
    /// # Errors
    /// То же, что у [`Self::warn`].
    pub(crate) fn node_warn(
        &mut self,
        kind: WarningKind,
        part: &str,
        node: NodeId,
        message: impl Into<String>,
    ) -> Result<()> {
        self.warnings
            .push(ParseWarning::at(kind, message, part).with_node(node))
            .map_err(Error::from)
    }

    /// Предупреждения в порядке появления.
    #[must_use]
    pub(crate) fn warnings(&self) -> &[ParseWarning] {
        self.warnings.as_slice()
    }

    /// Отдаёт аллокатор и предупреждения для сборки [`crate::Document`].
    ///
    /// Аллокатор уходит вместе с предупреждениями, потому что сборка тела
    /// документа продолжается после разбора частей: узлы получают ID из того же
    /// счётчика, и нумерация остаётся сквозной по документу.
    #[must_use]
    pub(crate) fn into_parts(self) -> (NodeIdAllocator, Warnings) {
        (self.ids, self.warnings)
    }
}

impl Default for ParseCtx {
    /// Как [`Self::new`].
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_handed_out_in_order() {
        let mut ctx = ParseCtx::new();

        assert_eq!(ctx.id(), NodeId::new(1));
        assert_eq!(ctx.id(), NodeId::new(2));
        assert_eq!(ctx.id(), NodeId::new(3));
    }

    #[test]
    fn warn_keeps_the_part() {
        let mut ctx = ParseCtx::new();
        ctx.warn(
            WarningKind::UnknownElement,
            "word/styles.xml",
            "unknown element `w:foo`",
        )
        .expect("the warning budget is not exhausted");

        let warnings = ctx.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnknownElement);

        let location = warnings[0].location.as_ref().expect("location is set");
        assert_eq!(location.part, "word/styles.xml");
        assert_eq!(location.node_id, None);
        assert_eq!(location.xml_path, None);
    }

    #[test]
    fn warn_at_keeps_the_xml_path() {
        let mut ctx = ParseCtx::new();
        ctx.warn_at(
            WarningKind::InvalidAttribute,
            "word/document.xml",
            Some("w:document/w:body/w:p[3]/w:pPr/w:pStyle"),
            "invalid `w:val`",
        )
        .expect("the warning budget is not exhausted");

        let location = ctx.warnings()[0]
            .location
            .as_ref()
            .expect("location is set");
        assert_eq!(location.part, "word/document.xml");
        assert_eq!(
            location.xml_path.as_deref(),
            Some("w:document/w:body/w:p[3]/w:pPr/w:pStyle")
        );
    }

    #[test]
    fn node_warn_keeps_the_node() {
        let mut ctx = ParseCtx::new();
        ctx.node_warn(
            WarningKind::OrphanBookmark,
            "word/document.xml",
            NodeId::new(5),
            "`w:bookmarkEnd` without `w:bookmarkStart`",
        )
        .expect("the warning budget is not exhausted");

        let location = ctx.warnings()[0]
            .location
            .as_ref()
            .expect("location is set");
        assert_eq!(location.part, "word/document.xml");
        assert_eq!(location.node_id, Some(NodeId::new(5)));
    }

    #[test]
    fn into_parts_keeps_the_allocator_state() {
        let mut ctx = ParseCtx::new();
        assert_eq!(ctx.id(), NodeId::new(1));
        assert_eq!(ctx.id(), NodeId::new(2));
        assert_eq!(ctx.id(), NodeId::new(3));
        ctx.warn(WarningKind::DeepNesting, "word/document.xml", "too deep")
            .expect("the warning budget is not exhausted");

        let (allocator, warnings) = ctx.into_parts();
        assert_eq!(allocator.allocated(), 3);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn the_warning_after_the_limit_is_an_error() {
        let mut ctx = ParseCtx::new();
        for _ in 0..Warnings::FATAL_THRESHOLD {
            ctx.warn(
                WarningKind::UnknownElement,
                "word/document.xml",
                "unknown element",
            )
            .expect("the threshold itself is allowed");
        }

        let err = ctx
            .warn(
                WarningKind::UnknownElement,
                "word/document.xml",
                "unknown element",
            )
            .expect_err("the warning above the threshold is rejected");

        assert!(matches!(err, Error::TooManyWarnings(1000)), "{err}");
        assert_eq!(ctx.warnings().len(), Warnings::FATAL_THRESHOLD);
    }
}
