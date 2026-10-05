//! Условное форматирование: эффективное оформление ячейки.
//!
//! Модель хранит правила как есть ([`Sheet::conditional_formatting`]), а этот
//! слой отвечает на вопрос «как выглядит ячейка с учётом сработавших правил».
//! Сработавшее правило накладывает дифференциальный формат (`dxf`) — частичный
//! набор полей; поля, которых в `dxf` нет, остаются от базового стиля ячейки.
//! Правила применяются в порядке `priority` (меньшее число раньше), а `dxf`
//! разных правил складываются: позднее правило перекрывает только заданные им
//! поля.
//!
//! Здесь считаются только правила, меняющие оформление, — `cellIs` и
//! `expression`. Шкалы, гистограммы и значки рисует следующий слайс; в файле они
//! записаны без `dxf`, и в индекс таких правил нет вовсе.
//!
//! Формулы `cellIs` и `expression` разбирает намеренно узкий вычислитель:
//! полноценного вычислителя формул в крейте нет. Поддержаны сравнение, ссылки
//! на ячейки, числа, строки, логические константы и функции `MOD`, `LEN`,
//! `ROW` — ровно то, что встречается в правилах оформления. Формула за
//! пределами этого набора правило не зажигает: потерять оформление безопаснее,
//! чем показать неверное.

use std::borrow::Cow;
use std::cmp::Ordering;

use crate::cellref::{CellRef, Range, MAX_COL, MAX_ROW};
use crate::model::{
    Border, BorderSide, BorderStyle, CellFormat, CellIsOperator, CellValue, Color, ConditionalRule,
    Dxf, DxfNumberFormat, Fill, FillPattern, Font, RuleKind, Sheet, StyleTable, Workbook,
};

/// Высота полосы строк в индексе правил.
///
/// Правило попадает во все полосы, которые пересекает его диапазон; ячейка
/// смотрит только свою полосу и проверяет попадание в диапазон. Так чужие
/// строки не перебираются — бюджет условного форматирования меньше 5 мс на 10k
/// ячеек.
const BAND_ROWS: u32 = 1024;

/// Индекс правил листа: «что проверить для этой ячейки».
///
/// Строится на кадр в [`crate::paint::build`]: правила листа неизменны, а
/// пересборка дешевле кэша в модели. Ссылки внутри ведут на лист и его `dxfs`,
/// поэтому индекс живёт не дольше книги.
#[derive(Debug)]
pub struct RuleIndex<'a> {
    sheet: &'a Sheet,
    /// Записи «правило × диапазон» в порядке применения: `priority`, затем
    /// порядок файла.
    entries: Vec<Entry<'a>>,
    /// Для каждой полосы строк — записи, пересекающие её, в том же порядке.
    bands: Vec<Vec<usize>>,
}

/// Одно правило на одном из своих диапазонов.
#[derive(Debug)]
struct Entry<'a> {
    range: Range,
    condition: Condition,
    overlay: Overlay<'a>,
    stop_if_true: bool,
}

impl<'a> RuleIndex<'a> {
    /// Собрать индекс по правилам листа.
    #[must_use]
    pub fn new(book: &'a Workbook, sheet: &'a Sheet) -> Self {
        let styles = book.styles();
        let mut ordered = Vec::new();
        for block in &sheet.conditional_formatting {
            for rule in &block.rules {
                // Правила без `dxf` оформление не меняют: так записаны шкалы,
                // гистограммы и значки, и так же выглядит потерянный `dxfId`.
                let Some(dxf) = rule.dxf_id.and_then(|id| styles.dxf(id)) else {
                    continue;
                };
                let Some(condition) = Condition::parse(rule) else {
                    continue;
                };
                let overlay = Overlay::new(dxf);
                for range in &block.ranges {
                    ordered.push((
                        rule.priority,
                        Entry {
                            range: range.normalized(),
                            condition: condition.clone(),
                            overlay: overlay.clone(),
                            stop_if_true: rule.stop_if_true,
                        },
                    ));
                }
            }
        }
        // Сортировка стабильная: при равных `priority` правила остаются в
        // порядке файла.
        ordered.sort_by_key(|(priority, _)| *priority);
        let entries: Vec<Entry<'a>> = ordered.into_iter().map(|(_, entry)| entry).collect();
        let bands = Self::bands(&entries, sheet);

        Self {
            sheet,
            entries,
            bands,
        }
    }

    /// Разложить записи по полосам строк.
    fn bands(entries: &[Entry<'_>], sheet: &Sheet) -> Vec<Vec<usize>> {
        let Some(last_row) = sheet.cells.last_row() else {
            return Vec::new();
        };
        let max_row = entries
            .iter()
            .map(|entry| entry.range.last.row.min(last_row))
            .max()
            .unwrap_or(0);
        let mut bands = vec![Vec::new(); (max_row / BAND_ROWS) as usize + 1];
        for (index, entry) in entries.iter().enumerate() {
            // За последней строкой с данными ячеек нет — правило там некому
            // применять.
            if entry.range.first.row > last_row {
                continue;
            }
            let first = (entry.range.first.row / BAND_ROWS) as usize;
            let last = (entry.range.last.row.min(last_row) / BAND_ROWS) as usize;
            for slot in &mut bands[first..=last] {
                slot.push(index);
            }
        }
        bands
    }

    /// Оформление ячейки: базовый формат плюс наложения сработавших правил.
    #[must_use]
    pub fn style_at(&self, book: &Workbook, at: CellRef) -> EffectiveStyle<'a> {
        let base = self.sheet.cells.cell(at).map_or(0, |cell| cell.style);
        let mut style = EffectiveStyle::new(book.styles().resolve(base));
        for entry in self.matching(at) {
            let eval = Eval {
                book,
                sheet: self.sheet,
                at,
                anchor: entry.range.first,
            };
            if entry.condition.matches(&eval) {
                style.apply(&entry.overlay);
                if entry.stop_if_true {
                    break;
                }
            }
        }
        style
    }

    /// Записи, чьи диапазоны покрывают ячейку, в порядке применения.
    fn matching(&self, at: CellRef) -> impl Iterator<Item = &Entry<'a>> + '_ {
        let band = (at.row / BAND_ROWS) as usize;
        self.bands
            .get(band)
            .into_iter()
            .flatten()
            .map(move |&index| &self.entries[index])
            .filter(move |entry| entry.range.contains(at))
    }
}

/// Оформление ячейки после сработавших правил.
///
/// Базовый формат берётся из [`StyleTable`], наложения — из дифференциальных
/// форматов. Группы (шрифт, заливка, рамка, формат числа) складываются
/// независимо: правило, задавшее только заливку, шрифт не трогает.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveStyle<'a> {
    base: CellFormat,
    applied: Overlay<'a>,
}

impl<'a> EffectiveStyle<'a> {
    /// Оформление без наложений — формат ячейки как он записан в файле.
    #[must_use]
    pub fn new(base: CellFormat) -> Self {
        Self {
            base,
            applied: Overlay::default(),
        }
    }

    /// Базовый формат ячейки из `cellXfs`.
    #[must_use]
    pub const fn base(&self) -> CellFormat {
        self.base
    }

    /// Шрифт: базовый с наложениями `dxf`.
    #[must_use]
    pub fn font(&self, styles: &StyleTable) -> Font {
        let mut font = styles.font(self.base.font).cloned().unwrap_or_default();
        if let Some(overlay) = &self.applied.font {
            if let Some(name) = &overlay.name {
                font.name.clone_from(name);
            }
            if let Some(size) = overlay.size {
                font.size = size;
            }
            font.bold |= overlay.bold;
            font.italic |= overlay.italic;
            font.underline |= overlay.underline;
            font.strike |= overlay.strike;
            if let Some(color) = overlay.color {
                font.color = color;
            }
        }
        font
    }

    /// Заливка: `dxf` задаёт её целиком, иначе берётся заливка базового формата.
    #[must_use]
    pub fn fill(&self, styles: &StyleTable) -> Option<Fill> {
        self.applied
            .fill
            .or_else(|| styles.fill(self.base.fill).copied())
    }

    /// Рамка: стороны из `dxf` накладываются на базовые по отдельности.
    #[must_use]
    pub fn border(&self, styles: &StyleTable) -> Border {
        let mut border = styles.border(self.base.border).copied().unwrap_or_default();
        if let Some(overlay) = &self.applied.border {
            if let Some(side) = overlay.left {
                border.left = side;
            }
            if let Some(side) = overlay.right {
                border.right = side;
            }
            if let Some(side) = overlay.top {
                border.top = side;
            }
            if let Some(side) = overlay.bottom {
                border.bottom = side;
            }
            if let Some(side) = overlay.diagonal {
                border.diagonal = side;
            }
            border.diagonal_up |= overlay.diagonal_up;
            border.diagonal_down |= overlay.diagonal_down;
        }
        border
    }

    /// Код формата числа: сперва `dxf`, затем базовый формат.
    #[must_use]
    pub fn format_code<'b>(&'b self, styles: &'b StyleTable) -> Option<&'b str> {
        let code = self.applied.number_format.and_then(|format| {
            format
                .code
                .as_deref()
                .or_else(|| format.id.and_then(|id| styles.format_code(id)))
        });
        code.or_else(|| styles.format_code(self.base.num_fmt))
    }

    /// Наложить `dxf` сработавшего правила.
    fn apply(&mut self, overlay: &Overlay<'a>) {
        if let Some(font) = &overlay.font {
            match &mut self.applied.font {
                Some(current) => current.merge(font),
                None => self.applied.font = Some(font.clone()),
            }
        }
        if overlay.fill.is_some() {
            self.applied.fill = overlay.fill;
        }
        if let Some(border) = &overlay.border {
            match &mut self.applied.border {
                Some(current) => current.merge(border),
                None => self.applied.border = Some(*border),
            }
        }
        if overlay.number_format.is_some() {
            self.applied.number_format = overlay.number_format;
        }
    }
}

/// Наложения одного `dxf`, разложенные по группам.
#[derive(Debug, Clone, Default, PartialEq)]
struct Overlay<'a> {
    font: Option<FontOverlay>,
    fill: Option<Fill>,
    border: Option<BorderOverlay>,
    number_format: Option<&'a DxfNumberFormat>,
}

impl<'a> Overlay<'a> {
    fn new(dxf: &'a Dxf) -> Self {
        // Пустой `<fill/>` без узора и цветов ничего не задаёт.
        let fill = dxf.fill.filter(|fill| {
            fill.pattern != FillPattern::None
                || fill.foreground != Color::None
                || fill.background != Color::None
        });
        Self {
            font: dxf.font.as_ref().map(FontOverlay::new),
            fill,
            border: dxf.border.as_ref().map(BorderOverlay::new),
            number_format: dxf.number_format.as_ref(),
        }
    }
}

/// Наложение шрифта: только поля, заданные в `dxf`.
///
/// Значения по умолчанию модели (`""`, 11 pt, `false`, [`Color::None`]) здесь
/// означают «не задано»: отличить их от явно записанных в файле нельзя. Так же
/// описывает `dxf` и [`Font`].
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq)]
struct FontOverlay {
    name: Option<String>,
    size: Option<f32>,
    /// Флаги только включаются: выключение (`false`) неотличимо от «не задано».
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: Option<Color>,
}

impl FontOverlay {
    // 11 pt — кегль [`Font`] по умолчанию; сравнение точное: кегль приходит из
    // файла литералом, а не вычисляется.
    #[allow(clippy::float_cmp)]
    fn new(font: &Font) -> Self {
        Self {
            name: (!font.name.is_empty()).then(|| font.name.clone()),
            size: (font.size != Font::default().size).then_some(font.size),
            bold: font.bold,
            italic: font.italic,
            underline: font.underline,
            strike: font.strike,
            color: (font.color != Color::None).then_some(font.color),
        }
    }

    fn merge(&mut self, other: &Self) {
        if other.name.is_some() {
            self.name.clone_from(&other.name);
        }
        if other.size.is_some() {
            self.size = other.size;
        }
        self.bold |= other.bold;
        self.italic |= other.italic;
        self.underline |= other.underline;
        self.strike |= other.strike;
        if other.color.is_some() {
            self.color = other.color;
        }
    }
}

/// Наложение рамки: стороны накладываются по отдельности.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct BorderOverlay {
    left: Option<BorderSide>,
    right: Option<BorderSide>,
    top: Option<BorderSide>,
    bottom: Option<BorderSide>,
    diagonal: Option<BorderSide>,
    diagonal_up: bool,
    diagonal_down: bool,
}

impl BorderOverlay {
    fn new(border: &Border) -> Self {
        Self {
            left: side(border.left),
            right: side(border.right),
            top: side(border.top),
            bottom: side(border.bottom),
            diagonal: side(border.diagonal),
            diagonal_up: border.diagonal_up,
            diagonal_down: border.diagonal_down,
        }
    }

    fn merge(&mut self, other: &Self) {
        for (own, other) in [
            (&mut self.left, other.left),
            (&mut self.right, other.right),
            (&mut self.top, other.top),
            (&mut self.bottom, other.bottom),
            (&mut self.diagonal, other.diagonal),
        ] {
            if other.is_some() {
                *own = other;
            }
        }
        self.diagonal_up |= other.diagonal_up;
        self.diagonal_down |= other.diagonal_down;
    }
}

/// Сторона, которую `dxf` действительно задаёт.
fn side(side: BorderSide) -> Option<BorderSide> {
    (side.style != BorderStyle::None || side.color != Color::None).then_some(side)
}

/// Условие правила, по которому оно срабатывает.
#[derive(Debug, Clone)]
enum Condition {
    /// `cellIs`: значение ячейки против формул правила.
    CellIs {
        operator: CellIsOperator,
        first: Formula,
        second: Option<Formula>,
    },
    /// `expression`: истинность формулы.
    Expression(Formula),
}

impl Condition {
    /// Разобрать условие; `None` — правило вычислить нечем, и оно пропускается.
    fn parse(rule: &ConditionalRule) -> Option<Self> {
        match &rule.kind {
            RuleKind::CellIs { operator, formulas } => {
                let first = Formula::parse(formulas.first()?)?;
                let second = match formulas.get(1) {
                    Some(formula) => Some(Formula::parse(formula)?),
                    None => None,
                };
                // `between` без второй формулы вычислить нечем.
                if second.is_none()
                    && matches!(
                        operator,
                        CellIsOperator::Between | CellIsOperator::NotBetween
                    )
                {
                    return None;
                }
                Some(Self::CellIs {
                    operator: *operator,
                    first,
                    second,
                })
            }
            RuleKind::Expression { formulas } => {
                Some(Self::Expression(Formula::parse(formulas.first()?)?))
            }
            _ => None,
        }
    }

    fn matches(&self, eval: &Eval<'_>) -> bool {
        match self {
            Self::CellIs {
                operator,
                first,
                second,
            } => eval.cell_is(*operator, first, second.as_ref()),
            Self::Expression(formula) => eval.truthy(formula),
        }
    }
}

/// Значение, с которым работает вычислитель.
#[derive(Debug, Clone, PartialEq)]
enum Value<'a> {
    /// Пустая ячейка: в числовых сравнениях это ноль, в текстовых — `""`.
    Empty,
    Number(f64),
    Text(Cow<'a, str>),
    Bool(bool),
    /// Ошибка или неподдержанное выражение: сравнение с ним не истинно.
    Error,
}

/// Контекст вычисления одного правила на одной ячейке.
struct Eval<'a> {
    book: &'a Workbook,
    sheet: &'a Sheet,
    /// Ячейка, к которой применяется правило.
    at: CellRef,
    /// Верхний левый угол диапазона, от которого отсчитываются относительные
    /// ссылки формулы, — так их понимает Excel.
    anchor: CellRef,
}

impl<'a> Eval<'a> {
    /// Значение ячейки листа.
    fn cell_value(&self, at: CellRef) -> Value<'a> {
        let sheet = self.sheet;
        let Some(cell) = sheet.cells.cell(at) else {
            return Value::Empty;
        };
        match &cell.value {
            CellValue::Empty => Value::Empty,
            CellValue::Number(value) => Value::Number(*value),
            CellValue::Bool(value) => Value::Bool(*value),
            CellValue::Error(_) => Value::Error,
            CellValue::SharedString(index) => self
                .book
                .shared_strings()
                .get(*index)
                .map_or(Value::Error, |text| Value::Text(Cow::Borrowed(text))),
            CellValue::InlineString(text) => Value::Text(Cow::Borrowed(text)),
        }
    }

    /// Правило `cellIs`: сравнение значения ячейки с формулой по оператору.
    fn cell_is(&self, operator: CellIsOperator, first: &Formula, second: Option<&Formula>) -> bool {
        let value = self.cell_value(self.at);
        let Some(low) = compare(&value, &self.eval(first)) else {
            return false;
        };
        match operator {
            CellIsOperator::LessThan => low == Ordering::Less,
            CellIsOperator::LessThanOrEqual => low != Ordering::Greater,
            CellIsOperator::Equal => low == Ordering::Equal,
            CellIsOperator::NotEqual => low != Ordering::Equal,
            CellIsOperator::GreaterThanOrEqual => low != Ordering::Less,
            CellIsOperator::GreaterThan => low == Ordering::Greater,
            CellIsOperator::Between | CellIsOperator::NotBetween => {
                let Some(second) = second else {
                    return false;
                };
                let Some(high) = compare(&value, &self.eval(second)) else {
                    return false;
                };
                // Между — включительно с обеих сторон.
                let between = low != Ordering::Less && high != Ordering::Greater;
                between == matches!(operator, CellIsOperator::Between)
            }
        }
    }

    /// Истинность формулы правила `expression`.
    fn truthy(&self, formula: &Formula) -> bool {
        match self.eval(formula) {
            Value::Bool(value) => value,
            Value::Number(value) => value != 0.0,
            // Текст и пустая ячейка логического смысла не несут.
            _ => false,
        }
    }

    /// Вычислить формулу.
    fn eval<'v>(&'v self, formula: &'v Formula) -> Value<'v> {
        match formula {
            Formula::Number(value) => Value::Number(*value),
            Formula::Text(text) => Value::Text(Cow::Borrowed(text.as_str())),
            Formula::Bool(value) => Value::Bool(*value),
            Formula::Cell(reference) => reference
                .resolve(self.anchor, self.at)
                .map_or(Value::Error, |at| self.cell_value(at)),
            Formula::Compare {
                operator,
                left,
                right,
            } => {
                let left = self.eval(left);
                let right = self.eval(right);
                compare(&left, &right).map_or(Value::Error, |ordering| {
                    Value::Bool(operator.matches(ordering))
                })
            }
            Formula::Call { function, args } => self.call(*function, args),
        }
    }

    /// Числовое значение формулы; текст Excel приводит к числу, если может.
    fn number<'v>(&'v self, formula: &'v Formula) -> Option<f64> {
        match self.eval(formula) {
            Value::Empty => Some(0.0),
            Value::Number(value) => Some(value),
            Value::Bool(value) => Some(if value { 1.0 } else { 0.0 }),
            Value::Text(text) => text.trim().parse().ok(),
            Value::Error => None,
        }
    }

    /// Функции, поддержанные вычислителем.
    fn call<'v>(&'v self, function: Function, args: &'v [Formula]) -> Value<'v> {
        match function {
            Function::Mod => {
                let [left, right] = args else {
                    return Value::Error;
                };
                let (Some(left), Some(right)) = (self.number(left), self.number(right)) else {
                    return Value::Error;
                };
                if right == 0.0 {
                    return Value::Error;
                }
                // Excel: знак результата — знак делителя, а не делимого.
                Value::Number(left - right * (left / right).floor())
            }
            Function::Len => {
                let [arg] = args else {
                    return Value::Error;
                };
                text_of(&self.eval(arg)).map_or(Value::Error, |text| {
                    let length = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
                    Value::Number(f64::from(length))
                })
            }
            Function::Row => {
                if args.is_empty() {
                    Value::Number(f64::from(self.at.row.saturating_add(1)))
                } else {
                    Value::Error
                }
            }
        }
    }
}

/// Сравнить значения по правилам Excel.
///
/// Числа меньше текста, текст меньше логических значений — это порядок типов
/// Excel. Текст сравнивается без учёта регистра (для регистрозависимого
/// сравнения в Excel есть отдельная функция `EXACT`, но её тут нет). Пустая
/// ячейка равна нулю и пустой строке. Сравнение с ошибкой не истинно.
///
/// Допущение: порядок типов и поведение пустой ячейки взяты из модели
/// вычислений Excel и на файлах не проверялись.
fn compare(left: &Value<'_>, right: &Value<'_>) -> Option<Ordering> {
    if matches!(left, Value::Error) || matches!(right, Value::Error) {
        return None;
    }
    match (left, right) {
        (Value::Empty, Value::Text(text)) | (Value::Text(text), Value::Empty) => {
            let ordering = if text.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            };
            Some(if matches!(left, Value::Empty) {
                ordering
            } else {
                ordering.reverse()
            })
        }
        _ => match rank(left).cmp(&rank(right)) {
            Ordering::Equal => same_rank(left, right),
            ordering => Some(ordering),
        },
    }
}

/// Место типа значения в порядке типов Excel.
const fn rank(value: &Value<'_>) -> u8 {
    match value {
        Value::Empty | Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error => 3,
    }
}

/// Сравнить значения одного типа.
fn same_rank(left: &Value<'_>, right: &Value<'_>) -> Option<Ordering> {
    match (left, right) {
        (Value::Empty | Value::Number(_), Value::Empty | Value::Number(_)) => {
            number_of(left)?.partial_cmp(&number_of(right)?)
        }
        (Value::Text(left), Value::Text(right)) => Some(text_cmp(left, right)),
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn number_of(value: &Value<'_>) -> Option<f64> {
    match value {
        Value::Empty => Some(0.0),
        Value::Number(number) => Some(*number),
        _ => None,
    }
}

/// Текст как в Excel: без учёта регистра.
fn text_cmp(left: &str, right: &str) -> Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

/// Текстовое представление значения — для `LEN`.
fn text_of<'v>(value: &Value<'v>) -> Option<Cow<'v, str>> {
    match value {
        Value::Text(text) => Some(text.clone()),
        Value::Empty => Some(Cow::Borrowed("")),
        Value::Number(number) => Some(Cow::Owned(number.to_string())),
        Value::Bool(value) => Some(Cow::Borrowed(if *value { "TRUE" } else { "FALSE" })),
        Value::Error => None,
    }
}

/// Формула правила: ровно то, что нужно `cellIs` и `expression`.
///
/// Полноценного вычислителя формул в крейте нет: здесь только сравнение,
/// константы, ссылки и три функции. Формула за пределами набора не разбирается —
/// [`Parser::parse`] вернёт `None`, и правило не сработает.
#[derive(Debug, Clone, PartialEq)]
enum Formula {
    Number(f64),
    Text(String),
    Bool(bool),
    Cell(CellRefFormula),
    Call {
        function: Function,
        args: Vec<Formula>,
    },
    Compare {
        operator: CompareOp,
        left: Box<Formula>,
        right: Box<Formula>,
    },
}

impl Formula {
    /// Разобрать формулу целиком; `None` — синтаксис вне поддержанного набора.
    fn parse(input: &str) -> Option<Self> {
        Parser::parse(input)
    }
}

/// Ссылка на ячейку в формуле: абсолютная или относительная по строке/столбцу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Absolute(u32),
    Relative(u32),
}

impl Part {
    /// Координата при применении к ячейке: относительная отсчитывается от
    /// верхнего левого угла диапазона правила.
    fn resolve(self, anchor: u32, at: u32, limit: u32) -> Option<u32> {
        match self {
            Self::Absolute(value) => (value <= limit).then_some(value),
            Self::Relative(value) => {
                let offset = i64::from(at) - i64::from(anchor);
                let resolved = u32::try_from(i64::from(value) + offset).ok()?;
                (resolved <= limit).then_some(resolved)
            }
        }
    }
}

/// Ссылка на ячейку из формулы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CellRefFormula {
    column: Part,
    row: Part,
}

impl CellRefFormula {
    fn resolve(self, anchor: CellRef, at: CellRef) -> Option<CellRef> {
        Some(CellRef::new(
            self.row.resolve(anchor.row, at.row, MAX_ROW)?,
            self.column.resolve(anchor.col, at.col, MAX_COL)?,
        ))
    }
}

/// Функции, которые вычислитель умеет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Function {
    Mod,
    Len,
    Row,
}

impl Function {
    fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().as_str() {
            "MOD" => Some(Self::Mod),
            "LEN" => Some(Self::Len),
            "ROW" => Some(Self::Row),
            _ => None,
        }
    }
}

/// Оператор сравнения в формуле.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareOp {
    Less,
    LessEqual,
    NotEqual,
    GreaterEqual,
    Greater,
    Equal,
}

impl CompareOp {
    const fn matches(self, ordering: Ordering) -> bool {
        match self {
            Self::Less => matches!(ordering, Ordering::Less),
            Self::LessEqual => !matches!(ordering, Ordering::Greater),
            Self::NotEqual => !matches!(ordering, Ordering::Equal),
            Self::GreaterEqual => !matches!(ordering, Ordering::Less),
            Self::Greater => matches!(ordering, Ordering::Greater),
            Self::Equal => matches!(ordering, Ordering::Equal),
        }
    }
}

/// Разбор формулы: `сравнение = операнд [оператор операнд]`.
struct Parser<'f> {
    input: &'f str,
    pos: usize,
}

impl<'f> Parser<'f> {
    fn parse(input: &'f str) -> Option<Formula> {
        let mut parser = Self { input, pos: 0 };
        let formula = parser.comparison()?;
        parser.skip_ws();
        (parser.pos == input.len()).then_some(formula)
    }

    fn comparison(&mut self) -> Option<Formula> {
        let left = self.operand()?;
        let Some(operator) = self.compare_op() else {
            return Some(left);
        };
        let right = self.operand()?;
        Some(Formula::Compare {
            operator,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn operand(&mut self) -> Option<Formula> {
        self.skip_ws();
        match self.peek()? {
            b'(' => {
                self.pos += 1;
                let inner = self.comparison()?;
                self.skip_ws();
                if !self.take(b')') {
                    return None;
                }
                Some(inner)
            }
            b'"' => self.string(),
            b'$' => self.reference(),
            byte if byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-') => self.number(),
            byte if byte.is_ascii_alphabetic() => self.name(),
            _ => None,
        }
    }

    /// Ссылка с `$` в столбце: `$B2`, `$B$2`.
    fn reference(&mut self) -> Option<Formula> {
        let column = self.column_part()?;
        let row = self.row_part()?;
        Some(Formula::Cell(CellRefFormula { column, row }))
    }

    /// Имя: функция, логическая константа или ссылка без `$` в столбце.
    fn name(&mut self) -> Option<Formula> {
        let start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        let letters = &self.input[start..self.pos];
        if self.peek() == Some(b'(') {
            return self.call(Function::parse(letters)?);
        }
        if matches!(self.peek(), Some(b'$' | b'0'..=b'9')) {
            return Some(Formula::Cell(CellRefFormula {
                column: Part::Relative(column_of(letters)?),
                row: self.row_part()?,
            }));
        }
        match letters.to_ascii_uppercase().as_str() {
            "TRUE" => Some(Formula::Bool(true)),
            "FALSE" => Some(Formula::Bool(false)),
            _ => None,
        }
    }

    fn call(&mut self, function: Function) -> Option<Formula> {
        self.pos += 1; // открывающая скобка
        let mut args = Vec::new();
        self.skip_ws();
        if self.take(b')') {
            return Some(Formula::Call { function, args });
        }
        loop {
            args.push(self.comparison()?);
            self.skip_ws();
            if self.take(b',') {
                continue;
            }
            if self.take(b')') {
                return Some(Formula::Call { function, args });
            }
            return None;
        }
    }

    fn string(&mut self) -> Option<Formula> {
        self.pos += 1; // открывающая кавычка
        let mut value = String::new();
        loop {
            let rest = &self.input[self.pos..];
            let end = rest.find('"')?;
            value.push_str(&rest[..end]);
            self.pos += end + 1;
            // Удвоенная кавычка внутри строки — один её символ.
            if self.peek() == Some(b'"') {
                value.push('"');
                self.pos += 1;
            } else {
                return Some(Formula::Text(value));
            }
        }
    }

    fn number(&mut self) -> Option<Formula> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let digits = self.pos;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            self.pos += 1;
        }
        if self.pos == digits {
            return None;
        }
        self.input[start..self.pos]
            .parse()
            .ok()
            .map(Formula::Number)
    }

    fn column_part(&mut self) -> Option<Part> {
        let absolute = self.take(b'$');
        let start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        let column = column_of(&self.input[start..self.pos])?;
        Some(if absolute {
            Part::Absolute(column)
        } else {
            Part::Relative(column)
        })
    }

    fn row_part(&mut self) -> Option<Part> {
        let absolute = self.take(b'$');
        let start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        let row: u32 = self.input[start..self.pos].parse().ok()?;
        let row = row.checked_sub(1)?;
        Some(if absolute {
            Part::Absolute(row)
        } else {
            Part::Relative(row)
        })
    }

    fn compare_op(&mut self) -> Option<CompareOp> {
        self.skip_ws();
        let rest = &self.input[self.pos..];
        let (operator, len) = if rest.starts_with("<=") {
            (CompareOp::LessEqual, 2)
        } else if rest.starts_with(">=") {
            (CompareOp::GreaterEqual, 2)
        } else if rest.starts_with("<>") {
            (CompareOp::NotEqual, 2)
        } else if rest.starts_with('<') {
            (CompareOp::Less, 1)
        } else if rest.starts_with('>') {
            (CompareOp::Greater, 1)
        } else if rest.starts_with('=') {
            (CompareOp::Equal, 1)
        } else {
            return None;
        };
        self.pos += len;
        Some(operator)
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|byte| matches!(byte, b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

/// Номер столбца из букв (`A` → 0).
fn column_of(letters: &str) -> Option<u32> {
    if letters.is_empty() {
        return None;
    }
    let mut column: u32 = 0;
    for byte in letters.bytes() {
        let digit = u32::from(byte.to_ascii_uppercase().saturating_sub(b'A')) + 1;
        column = column.checked_mul(26)?.checked_add(digit)?;
    }
    column.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Cell, ConditionalFormatting, SheetContent, WorksheetBuilder, WorksheetMeta,
    };
    use crate::{SharedStrings, SheetState, Theme};
    use std::collections::BTreeMap;

    const PART: &str = "xl/worksheets/sheet1.xml";

    /// Книга с одним листом: правила на `A1:A10`, `dxfs` и числа в ячейках.
    fn book_with_cells(
        rules: Vec<ConditionalRule>,
        dxfs: Vec<Dxf>,
        cells: &[(u32, u32, f64)],
    ) -> Workbook {
        book_with_range(rules, dxfs, cells, "A1:A10")
    }

    /// Книга с правилами на заданном диапазоне.
    fn book_with_range(
        rules: Vec<ConditionalRule>,
        dxfs: Vec<Dxf>,
        cells: &[(u32, u32, f64)],
        range: &str,
    ) -> Workbook {
        let mut builder = WorksheetBuilder::new(PART);
        for (row, col, value) in cells {
            builder
                .push(*row, Cell::new(*col, 0, CellValue::Number(*value)))
                .unwrap();
        }
        let content = SheetContent {
            cells: builder.finish(),
            conditional_formatting: vec![ConditionalFormatting {
                ranges: vec![Range::parse(range).unwrap()],
                rules,
            }],
            ..SheetContent::default()
        };
        let sheet = Sheet::new(
            WorksheetMeta {
                name: "Лист1".into(),
                part: PART.into(),
                state: SheetState::Visible,
            },
            content,
        );
        Workbook::new(
            vec![sheet],
            SharedStrings::default(),
            styles(dxfs),
            Theme::default(),
            false,
        )
    }

    fn styles(dxfs: Vec<Dxf>) -> StyleTable {
        StyleTable::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            BTreeMap::new(),
        )
        .with_dxfs(dxfs)
    }

    fn rule(
        priority: u32,
        dxf_id: u32,
        operator: CellIsOperator,
        formulas: &[&str],
    ) -> ConditionalRule {
        ConditionalRule {
            priority,
            stop_if_true: false,
            dxf_id: Some(dxf_id),
            kind: RuleKind::CellIs {
                operator,
                formulas: formulas.iter().map(|text| (*text).to_owned()).collect(),
            },
        }
    }

    fn red() -> Dxf {
        Dxf {
            fill: Some(Fill {
                pattern: FillPattern::Solid,
                foreground: Color::Rgb(0xFFFF_0000),
                background: Color::None,
            }),
            ..Dxf::default()
        }
    }

    fn filled(book: &Workbook, index: &RuleIndex<'_>, row: u32) -> bool {
        index
            .style_at(book, CellRef::new(row, 0))
            .fill(book.styles())
            .is_some()
    }

    #[test]
    fn cell_is_matches_only_its_side_of_the_comparison() {
        let book = book_with_cells(
            vec![rule(1, 0, CellIsOperator::GreaterThan, &["30"])],
            vec![red()],
            &[(0, 0, 10.0), (1, 0, 40.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Ячейка с 10 не проходит, с 40 — проходит.
        assert!(!filled(&book, &index, 0));
        assert!(filled(&book, &index, 1));
    }

    #[test]
    fn empty_cell_counts_as_zero() {
        let book = book_with_cells(
            vec![rule(1, 0, CellIsOperator::Equal, &["0"])],
            vec![red()],
            &[(1, 0, 40.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // A1 в модели нет — для правила это ноль.
        assert!(filled(&book, &index, 0));
    }

    #[test]
    fn between_needs_both_bounds() {
        let book = book_with_cells(
            vec![rule(1, 0, CellIsOperator::Between, &["25", "45"])],
            vec![red()],
            &[(0, 0, 25.0), (1, 0, 45.0), (2, 0, 24.9), (3, 0, 45.1)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Границы включительно, всё вне них — нет.
        assert!(filled(&book, &index, 0) && filled(&book, &index, 1));
        assert!(!filled(&book, &index, 2) && !filled(&book, &index, 3));
    }

    #[test]
    fn later_priority_overrides_only_the_fields_it_sets() {
        let bold = Dxf {
            font: Some(Font {
                bold: true,
                ..Font::default()
            }),
            ..Dxf::default()
        };
        let italic = Dxf {
            font: Some(Font {
                italic: true,
                ..Font::default()
            }),
            ..Dxf::default()
        };
        let book = book_with_cells(
            vec![
                rule(1, 0, CellIsOperator::GreaterThan, &["0"]),
                rule(2, 1, CellIsOperator::GreaterThan, &["0"]),
            ],
            vec![bold, italic],
            &[(0, 0, 5.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);
        let font = index
            .style_at(&book, CellRef::new(0, 0))
            .font(book.styles());

        // Оба правила сработали: полужирный от первого, курсив от второго.
        assert!(font.bold && font.italic);
    }

    #[test]
    fn stop_if_true_breaks_the_chain() {
        let mut stop = rule(1, 0, CellIsOperator::GreaterThan, &["0"]);
        stop.stop_if_true = true;
        let bold = Dxf {
            font: Some(Font {
                bold: true,
                ..Font::default()
            }),
            ..Dxf::default()
        };
        let book = book_with_cells(
            vec![stop, rule(2, 1, CellIsOperator::GreaterThan, &["0"])],
            vec![red(), bold],
            &[(0, 0, 5.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);
        let style = index.style_at(&book, CellRef::new(0, 0));

        assert!(style.fill(book.styles()).is_some());
        assert!(!style.font(book.styles()).bold);
    }

    #[test]
    fn expression_with_relative_reference_follows_the_cell() {
        let blue = Dxf {
            fill: Some(Fill {
                pattern: FillPattern::Solid,
                foreground: Color::Rgb(0xFF00_00FF),
                background: Color::None,
            }),
            ..Dxf::default()
        };
        let mut formula = rule(1, 0, CellIsOperator::Equal, &["1"]);
        formula.kind = RuleKind::Expression {
            formulas: vec!["MOD($A2,2)=0".to_owned()],
        };
        let book = book_with_range(
            vec![formula],
            vec![blue],
            &[(1, 0, 14.0), (2, 0, 15.0), (3, 0, 10.0)],
            "A2:A10",
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // `$A2` отсчитывается от угла диапазона: A2 — 14 (чётное), A3 — 15,
        // A4 — 10.
        assert!(filled(&book, &index, 1));
        assert!(!filled(&book, &index, 2));
        assert!(filled(&book, &index, 3));
    }

    #[test]
    fn formula_parser_reads_the_fixture_formulas() {
        for formula in [
            "MOD($B2,2)=0",
            "$B2=0",
            "LEN($A2)>10",
            "MOD(ROW(),2)=0",
            "30",
            "\"текст\"",
        ] {
            assert!(
                Formula::parse(formula).is_some(),
                "формула не разобрана: {formula}"
            );
        }
        // Формулы вне набора правило не зажигают, а не роняют кадр.
        for formula in ["AND(A1,B1)", "SUM(A1:A3)", "A1+1", ""] {
            assert!(
                Formula::parse(formula).is_none(),
                "формула разобрана, хотя не поддержана: {formula}"
            );
        }
    }

    #[test]
    fn text_comparison_is_case_insensitive() {
        assert_eq!(
            compare(
                &Value::Text(Cow::Borrowed("Ключ")),
                &Value::Text(Cow::Borrowed("ключ"))
            ),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn dxf_without_any_group_changes_nothing() {
        let book = book_with_cells(
            vec![rule(1, 0, CellIsOperator::GreaterThan, &["0"])],
            vec![Dxf::default()],
            &[(0, 0, 5.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);
        let style = index.style_at(&book, CellRef::new(0, 0));

        assert_eq!(style, EffectiveStyle::new(style.base()));
    }
}
