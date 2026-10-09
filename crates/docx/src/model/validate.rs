//! Валидация ссылок модели после разбора (ADR-0016 §2, ADR-0013 §3).
//!
//! Ссылки вида `w:basedOn` и `w:abstractNumId` разрешаются парсерами по таблицам
//! стилей и нумерации, но файл может назвать стиль или схему, которых в пакете
//! нет. Такие дефекты не фатальны: обход не паникует и не зацикливается, а копит
//! предупреждения (ADR-0016 §2).

// Вызовет обход сборка документа (слайс S12), после того как парсеры частей
// (S6–S11) наполнят таблицы стилей и нумерации. До тех пор `dead_code` срабатывал
// бы на всём модуле; `allow` снимается вместе с подключением.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use doc_converter_core::WarningKind;

use super::numbering::NumberingTable;
use super::raw::StyleId;
use super::style::StyleTable;
use crate::context::ParseCtx;
use crate::error::Result;

/// Предел глубины цепочки `basedOn`: длиннее — признак сломанного файла (ADR-0016 §2).
const MAX_BASED_ON_DEPTH: usize = 32;

impl StyleTable {
    /// Обходит цепочки `basedOn` и предупреждает о циклах и пропавших целях.
    ///
    /// Каждый вид стилей обходится отдельно: `w:basedOn` ссылается на стиль того
    /// же вида, и таблица другого вида тут ничего не значит. Стиль, чья цепочка
    /// уже разобрана, повторно не обходится — иначе цикл из `n` стилей дал бы `n`
    /// предупреждений об одном и том же дефекте. `next` и `link` в каскаде не
    /// участвуют и не проверяются (ADR-0013 §3).
    ///
    /// # Errors
    /// [`crate::Error::TooManyWarnings`], если предупреждений стало слишком много.
    pub(crate) fn validate(&mut self, part: &str, ctx: &mut ParseCtx) -> Result<()> {
        validate_based_on(part, &self.paragraph, |style| style.based_on.clone(), ctx)?;
        validate_based_on(part, &self.character, |style| style.based_on.clone(), ctx)?;
        validate_based_on(part, &self.table, |style| style.based_on.clone(), ctx)?;
        validate_based_on(part, &self.numbering, |style| style.based_on.clone(), ctx)?;
        Ok(())
    }
}

/// Идёт по цепочкам `basedOn` одной таблицы стилей.
///
/// `based_on` отдаёт основу конкретного вида стиля: виды отличаются только типом,
/// а обход у них общий. Основа возвращается во владение, чтобы замыкание не
/// связывало времена жизни таблицы и её элементов.
///
/// # Errors
/// [`crate::Error::TooManyWarnings`], если предупреждений стало слишком много.
fn validate_based_on<T>(
    part: &str,
    table: &BTreeMap<StyleId, T>,
    based_on: impl Fn(&T) -> Option<StyleId>,
    ctx: &mut ParseCtx,
) -> Result<()> {
    // Цепочки, уже доведённые до конца: их звенья не обходятся второй раз,
    // поэтому цикл `a → b → a` даёт одно предупреждение, а не по одному на
    // каждый стиль цикла.
    let mut done: BTreeSet<StyleId> = BTreeSet::new();

    for start in table.keys() {
        if done.contains(start) {
            continue;
        }

        // Звенья текущей цепочки: повтор встреченного здесь — это цикл,
        // а не ссылка на уже проверенную ветку.
        let mut chain: BTreeSet<StyleId> = BTreeSet::new();
        let mut current = start.clone();
        let mut depth = 0usize;

        loop {
            chain.insert(current.clone());

            let Some(parent) = table.get(&current).and_then(&based_on) else {
                break;
            };

            if !table.contains_key(&parent) {
                ctx.warn(
                    WarningKind::MissingStyleRef,
                    part,
                    format!("style `{current}` is based on undefined style `{parent}`"),
                )?;
                break;
            }

            if chain.contains(&parent) {
                ctx.warn(
                    WarningKind::CyclicBasedOn,
                    part,
                    format!("style `{current}` closes a `basedOn` cycle through `{parent}`"),
                )?;
                break;
            }

            if done.contains(&parent) {
                // Дальше по цепочке всё уже проверено и дефектов не содержит.
                break;
            }

            depth += 1;
            if depth > MAX_BASED_ON_DEPTH {
                ctx.warn(
                    WarningKind::DeepNesting,
                    part,
                    format!(
                        "`basedOn` chain of `{start}` is deeper than {MAX_BASED_ON_DEPTH} styles"
                    ),
                )?;
                break;
            }

            current = parent;
        }

        done.extend(chain);
    }

    Ok(())
}

impl NumberingTable {
    /// Проверяет, что у каждого `num` есть `abstractNum`.
    ///
    /// # Errors
    /// [`crate::Error::TooManyWarnings`] — как у [`StyleTable::validate`].
    pub(crate) fn validate(&self, part: &str, ctx: &mut ParseCtx) -> Result<()> {
        for (id, num) in &self.nums {
            if !self.abstract_nums.contains_key(&num.abstract_id) {
                ctx.warn(
                    WarningKind::MissingAbstractNum,
                    part,
                    format!(
                        "numbering `{}` refers to undefined abstract numbering `{}`",
                        id.value(),
                        num.abstract_id.value()
                    ),
                )?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::numbering::{AbstractNum, AbstractNumId, MultiLevelType, Num, NumId};
    use crate::model::raw::{RawPPr, RawRPr};
    use crate::model::style::{CharacterStyle, NumberingStyle, ParagraphStyle, TableStyle};

    /// Стиль абзаца без свойств: заполнено только то, что важно валидации.
    fn paragraph(id: &str, based_on: Option<&str>) -> ParagraphStyle {
        ParagraphStyle {
            id: StyleId::new(id),
            name: None,
            based_on: based_on.map(StyleId::new),
            next: None,
            link: None,
            is_default: false,
            hidden: false,
            custom: false,
            aliases: Vec::new(),
            ppr: RawPPr::default(),
            rpr: RawRPr::default(),
        }
    }

    /// Стиль знака без свойств.
    fn character(id: &str, based_on: Option<&str>) -> CharacterStyle {
        CharacterStyle {
            id: StyleId::new(id),
            name: None,
            based_on: based_on.map(StyleId::new),
            link: None,
            is_default: false,
            hidden: false,
            custom: false,
            aliases: Vec::new(),
            rpr: RawRPr::default(),
        }
    }

    /// Стиль таблицы без свойств и условных форматов.
    fn table_style(id: &str, based_on: Option<&str>) -> TableStyle {
        TableStyle {
            id: StyleId::new(id),
            name: None,
            based_on: based_on.map(StyleId::new),
            is_default: false,
            hidden: false,
            custom: false,
            aliases: Vec::new(),
            ppr: RawPPr::default(),
            rpr: RawRPr::default(),
            conditional: Vec::new(),
        }
    }

    /// Стиль нумерации без свойств.
    fn numbering_style(id: &str, based_on: Option<&str>) -> NumberingStyle {
        NumberingStyle {
            id: StyleId::new(id),
            name: None,
            based_on: based_on.map(StyleId::new),
            is_default: false,
            hidden: false,
            custom: false,
            aliases: Vec::new(),
            ppr: RawPPr::default(),
            rpr: RawRPr::default(),
        }
    }

    /// Виды предупреждений в порядке появления.
    fn kinds(ctx: &ParseCtx) -> Vec<WarningKind> {
        ctx.warnings().iter().map(|warning| warning.kind).collect()
    }

    #[test]
    fn a_cycle_between_two_styles_warns_once() {
        let mut styles = StyleTable::default();
        styles
            .paragraph
            .insert(StyleId::new("a"), paragraph("a", Some("b")));
        styles
            .paragraph
            .insert(StyleId::new("b"), paragraph("b", Some("a")));
        let mut ctx = ParseCtx::new();

        styles
            .validate("word/styles.xml", &mut ctx)
            .expect("the warning budget is not exhausted");

        // Обход завершается, а не зацикливается, и предупреждение об одном
        // цикле выдаётся один раз, хотя в цикле два стиля.
        assert_eq!(kinds(&ctx), vec![WarningKind::CyclicBasedOn]);
    }

    #[test]
    fn a_self_reference_is_a_cycle() {
        let mut styles = StyleTable::default();
        styles
            .paragraph
            .insert(StyleId::new("a"), paragraph("a", Some("a")));
        let mut ctx = ParseCtx::new();

        styles
            .validate("word/styles.xml", &mut ctx)
            .expect("the warning budget is not exhausted");

        assert_eq!(kinds(&ctx), vec![WarningKind::CyclicBasedOn]);
    }

    #[test]
    fn a_missing_base_is_reported_for_every_style_kind() {
        let mut styles = StyleTable::default();
        styles
            .paragraph
            .insert(StyleId::new("p"), paragraph("p", Some("gone")));
        styles
            .character
            .insert(StyleId::new("c"), character("c", Some("gone")));
        styles
            .table
            .insert(StyleId::new("t"), table_style("t", Some("gone")));
        styles
            .numbering
            .insert(StyleId::new("n"), numbering_style("n", Some("gone")));
        let mut ctx = ParseCtx::new();

        styles
            .validate("word/styles.xml", &mut ctx)
            .expect("the warning budget is not exhausted");

        assert_eq!(kinds(&ctx), vec![WarningKind::MissingStyleRef; 4]);
    }

    #[test]
    fn a_shared_base_is_not_a_cycle() {
        let mut styles = StyleTable::default();
        styles
            .paragraph
            .insert(StyleId::new("base"), paragraph("base", None));
        styles
            .paragraph
            .insert(StyleId::new("left"), paragraph("left", Some("base")));
        styles
            .paragraph
            .insert(StyleId::new("right"), paragraph("right", Some("base")));
        let mut ctx = ParseCtx::new();

        styles
            .validate("word/styles.xml", &mut ctx)
            .expect("the warning budget is not exhausted");

        // Два стиля, ссылающиеся на одну основу, — это «ромб», а не цикл.
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    #[test]
    fn a_too_deep_chain_is_reported_once() {
        let mut styles = StyleTable::default();
        for index in 0..40u32 {
            let id = format!("s{index}");
            let base = (index < 39).then_some(format!("s{}", index + 1));
            styles
                .paragraph
                .insert(StyleId::new(id.clone()), paragraph(&id, base.as_deref()));
        }
        let mut ctx = ParseCtx::new();

        styles
            .validate("word/styles.xml", &mut ctx)
            .expect("the warning budget is not exhausted");

        assert_eq!(kinds(&ctx), vec![WarningKind::DeepNesting]);
    }

    #[test]
    fn a_missing_abstract_num_is_reported() {
        let mut numbering = NumberingTable::default();
        numbering.abstract_nums.insert(
            AbstractNumId::new(6),
            AbstractNum {
                id: AbstractNumId::new(6),
                multi_level_type: MultiLevelType::SingleLevel,
                levels: BTreeMap::new(),
                num_style_link: None,
                style_link: None,
                nsid: None,
                tmpl: None,
            },
        );
        numbering.nums.insert(
            NumId::new(1),
            Num {
                id: NumId::new(1),
                abstract_id: AbstractNumId::new(5),
                overrides: BTreeMap::new(),
                picture_bullet_id: None,
            },
        );
        numbering.nums.insert(
            NumId::new(2),
            Num {
                id: NumId::new(2),
                abstract_id: AbstractNumId::new(6),
                overrides: BTreeMap::new(),
                picture_bullet_id: None,
            },
        );
        let mut ctx = ParseCtx::new();

        numbering
            .validate("word/numbering.xml", &mut ctx)
            .expect("the warning budget is not exhausted");

        assert_eq!(kinds(&ctx), vec![WarningKind::MissingAbstractNum]);
    }

    #[test]
    fn empty_tables_are_valid() {
        let mut styles = StyleTable::default();
        let numbering = NumberingTable::default();
        let mut ctx = ParseCtx::new();

        styles
            .validate("word/styles.xml", &mut ctx)
            .expect("the warning budget is not exhausted");
        numbering
            .validate("word/numbering.xml", &mut ctx)
            .expect("the warning budget is not exhausted");

        assert!(ctx.warnings().is_empty());
    }
}
