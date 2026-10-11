//! Вычисление полей `PAGE` и `NUMPAGES` на раскладке.
//!
//! Раскладка собирает текст абзаца из run'ов (`segments` в `paragraph.rs`),
//! а поле — не run: `w:fldSimple` и сложное поле (`begin`…`end`) без подстановки
//! не дают ни одного знака, и колонтитул «Page {PAGE} of {NUMPAGES}» теряет оба
//! числа. Word пересчитывает поля при печати и кэшированный результат между
//! `separate` и `end` игнорирует — раскладка делает так же.
//!
//! Подстановка идёт до раскладки абзаца, а не при отрисовке строки: номер
//! страницы участвует в метриках строки (перенос, ширина), а `PAGE` у каждой
//! страницы свой — подставить его один раз в модель документа нельзя.
//! `NUMPAGES` — число страниц всего документа, и на первом проходе раскладки
//! оно ещё не известно: [`FieldPass`] хранит признак «поле встретилось», по
//! которому `layout_document` решает, нужен ли проход с числом страниц.

use crate::model::{Field, Inline, Paragraph, Run, RunContent};

/// Поля, которые раскладка вычисляет сама.
///
/// Прочие поля (`DATE`, `REF`, `TOC`, `HYPERLINK`, `MERGEFIELD`…) раскладке
/// неизвестны и остаются как есть: их результат — отдельный долг, а не паника.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Computed {
    /// `PAGE` — номер страницы, на которой стоит поле.
    Page,
    /// `NUMPAGES` — число страниц всего документа.
    NumPages,
}

/// Вычисляемое поле в инструкции; `None` — поле раскладке неизвестно.
///
/// Инструкция — первый знак `w:instr`/`w:instrText`: у `PAGE` с ключами
/// (`PAGE \* MERGEFORMAT`) имя всё равно первое слово, а `PAGEREF` — другое
/// слово целиком и вычисляемым не становится.
fn computed(instruction: &str) -> Option<Computed> {
    let keyword = instruction.split_whitespace().next()?;
    if keyword.eq_ignore_ascii_case("PAGE") {
        Some(Computed::Page)
    } else if keyword.eq_ignore_ascii_case("NUMPAGES") {
        Some(Computed::NumPages)
    } else {
        None
    }
}

/// Что раскладка знает о полях на текущем проходе.
#[derive(Debug, Clone)]
pub(crate) struct FieldPass {
    /// Номер страницы, на которой раскладывается текущий блок.
    page: u32,
    /// Число страниц всего документа; `None` — первый проход, оно ещё не посчитано.
    num_pages: Option<u32>,
    /// Встретилось ли `NUMPAGES`: без второго прохода его нечем вычислить.
    saw_num_pages: bool,
}

impl FieldPass {
    /// Проход раскладки; `num_pages` — число страниц из прошлого прохода, если оно уже есть.
    pub(crate) fn new(num_pages: Option<u32>) -> Self {
        Self {
            page: 1,
            num_pages,
            saw_num_pages: false,
        }
    }

    /// Страница, на которой раскладывается следующий блок.
    ///
    /// Номер берётся в начале блока, поэтому абзац, растянувшийся на две
    /// страницы, несёт номер первой из них: строки режутся уже после подстановки.
    pub(crate) fn on_page(&mut self, page: u32) {
        self.page = page;
    }

    /// Встретилось ли `NUMPAGES` — нужен ли раскладке проход с числом страниц.
    pub(crate) fn saw_num_pages(&self) -> bool {
        self.saw_num_pages
    }

    fn context(&self) -> FieldContext {
        FieldContext {
            page: self.page,
            num_pages: self.num_pages,
        }
    }
}

/// Значения полей одной подстановки.
#[derive(Debug, Clone, Copy)]
struct FieldContext {
    /// Номер страницы, на которой стоит поле.
    page: u32,
    /// Число страниц документа; `None` — ещё не известно (первый проход).
    num_pages: Option<u32>,
}

/// Подставить вычисленные `PAGE`/`NUMPAGES` в абзац.
///
/// `None` — вычисляемых полей в абзаце нет: копия абзаца не делается, и
/// документ без номеров страниц платит только за обход run'ов. Иначе — абзац
/// с подставленными значениями; [`FieldPass`] запоминает, встретилось ли
/// `NUMPAGES` (по этому признаку `layout_document` считает второй проход).
pub(crate) fn resolve_paragraph(paragraph: &Paragraph, pass: &mut FieldPass) -> Option<Paragraph> {
    if !has_computed(&paragraph.runs) {
        return None;
    }
    let context = pass.context();
    let mut resolved = paragraph.clone();
    let mut saw_num_pages = false;
    substitute(&mut resolved.runs, context, &mut saw_num_pages);
    if saw_num_pages {
        pass.saw_num_pages = true;
    }
    Some(resolved)
}

/// Есть ли в списке inline-элементов поле, которое раскладка вычисляет.
fn has_computed(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inline| match inline {
        Inline::Field(field) => computed(&field.instruction).is_some(),
        _ => false,
    })
}

/// Заменить вычисляемые поля их значением; прочие поля и элементы не трогаются.
fn substitute(inlines: &mut [Inline], context: FieldContext, saw_num_pages: &mut bool) {
    for inline in inlines.iter_mut() {
        let Inline::Field(field) = inline else {
            continue;
        };
        let Some(field_kind) = computed(&field.instruction) else {
            continue;
        };
        if field_kind == Computed::NumPages {
            *saw_num_pages = true;
        }
        *inline = value_inline(field, context.value(field_kind, &cached_text(field)));
    }
}

impl FieldContext {
    /// Значение поля: `PAGE` — текущая страница, `NUMPAGES` — число страниц.
    ///
    /// Кэш документа идёт в дело, лишь пока число страниц не посчитано: на
    /// первом проходе показывается он, на втором вычисленное значение замещает
    /// его целиком — как пересчёт полей при печати в Word.
    fn value(&self, field: Computed, cached: &str) -> String {
        match field {
            Computed::Page => self.page.to_string(),
            Computed::NumPages => self
                .num_pages
                .map_or_else(|| cached.to_owned(), |total| total.to_string()),
        }
    }
}

/// Run со значением поля.
///
/// Форматирование берётся у первого текстового run'а кэшированного результата:
/// так номер печатается тем же кеглем и начертанием, что и результат в самом
/// документе. Результата нет (пустое поле) — значение идёт с умолчаниями
/// абзаца, как у run'а без `w:rPr`.
fn value_inline(field: &Field, value: String) -> Inline {
    let content = vec![RunContent::Text(value)];
    match field.result.iter().find_map(text_run) {
        Some(run) => Inline::Run(Run {
            content,
            ..run.clone()
        }),
        None => Inline::Run(Run {
            id: field.id,
            content,
            ..Run::default()
        }),
    }
}

/// Run, у которого есть текст: он и отдаёт форматирование значению поля.
fn text_run(inline: &Inline) -> Option<&Run> {
    let Inline::Run(run) = inline else {
        return None;
    };
    run.content
        .iter()
        .any(|content| matches!(content, RunContent::Text(_)))
        .then_some(run)
}

/// Кэшированный результат поля: текст `separate`…`end`, склеенный по run'ам.
///
/// Между первым и вторым проходом он играет роль заглушки для `NUMPAGES`;
/// для `PAGE` не нужен, но стоит копейки и держит обе ветки одинаковыми.
fn cached_text(field: &Field) -> String {
    let mut text = String::new();
    for inline in &field.result {
        collect_text(inline, &mut text);
    }
    text
}

/// Текст inline-элемента; вложенные контейнеры (гиперссылка, поле) обходятся.
fn collect_text(inline: &Inline, out: &mut String) {
    match inline {
        Inline::Run(run) => {
            for content in &run.content {
                if let RunContent::Text(text) = content {
                    out.push_str(text);
                }
            }
        }
        Inline::Hyperlink(hyperlink) => {
            for inline in &hyperlink.runs {
                collect_text(inline, out);
            }
        }
        Inline::Field(field) => {
            for inline in &field.result {
                collect_text(inline, out);
            }
        }
        _ => {}
    }
}
