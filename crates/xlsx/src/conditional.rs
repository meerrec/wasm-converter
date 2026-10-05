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
//! Кроме правил, меняющих оформление (`cellIs`, `expression`), здесь считаются
//! и те, что рисуют сами: цветовые шкалы, гистограммы и наборы значков. У них
//! нет `dxf` — оформление задано внутри правила, и его значения (`min`/`max`,
//! проценты, процентили) зависят от содержимого диапазона. Эти значения
//! считаются один раз на правило и только если правило попало в кадр.
//!
//! Формулы `cellIs` и `expression` разбирает намеренно узкий вычислитель:
//! полноценного вычислителя формул в крейте нет. Поддержаны сравнение, ссылки
//! на ячейки, числа, строки, логические константы и функции `MOD`, `LEN`,
//! `ROW` — ровно то, что встречается в правилах оформления. Формула за
//! пределами этого набора правило не зажигает: потерять оформление безопаснее,
//! чем показать неверное.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::cmp::Ordering;

use doc_converter_render::display_list::Color as Rgba;

use crate::cellref::{CellRef, Range, MAX_COL, MAX_ROW};
use crate::model::{
    Border, BorderSide, BorderStyle, CellFormat, CellIsOperator, CellValue, Color, ColorScale,
    ConditionalRule, DataBar, Dxf, DxfNumberFormat, Fill, FillPattern, Font, IconSet, RuleKind,
    Sheet, StyleTable, Theme, Threshold, ThresholdKind, Workbook,
};
use crate::paint::resolve_color;

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
    /// Есть ли правила, рисующие сами: без них `paint` не добавляет слой
    /// изображений, и шкалы не стоят ничего листу без условного оформления.
    has_visuals: bool,
}

/// Одно правило на одном из своих диапазонов.
#[derive(Debug)]
struct Entry<'a> {
    range: Range,
    /// Условие правила `cellIs`/`expression`; `None` — правило визуальное.
    condition: Option<Condition>,
    overlay: Overlay<'a>,
    /// Визуальная часть правила (шкала, гистограмма, значки).
    visual: Option<VisualRule<'a>>,
    stop_if_true: bool,
    /// Разрешённые пороги правила; считаются по содержимому диапазона лениво —
    /// диапазон бывает во весь лист, а видно из него несколько ячеек.
    thresholds: OnceCell<Thresholds>,
}

impl<'a> RuleIndex<'a> {
    /// Собрать индекс по правилам листа.
    #[must_use]
    pub fn new(book: &'a Workbook, sheet: &'a Sheet) -> Self {
        let styles = book.styles();
        let mut ordered = Vec::new();
        for block in &sheet.conditional_formatting {
            for rule in &block.rules {
                let condition = Condition::parse(rule);
                let visual = VisualRule::parse(&rule.kind);
                // Правило без `dxf` и без визуальной части не изображает ничего:
                // так выглядит потерянный `dxfId` и вид правила, который мы не
                // разбираем (`top10`, `aboveAverage`, …).
                if condition.is_none() && visual.is_none() {
                    continue;
                }
                let overlay = rule.dxf_id.and_then(|id| styles.dxf(id)).map(Overlay::new);
                for range in &block.ranges {
                    ordered.push((
                        rule.priority,
                        Entry {
                            range: range.normalized(),
                            condition: condition.clone(),
                            overlay: overlay.clone().unwrap_or_default(),
                            visual,
                            stop_if_true: rule.stop_if_true,
                            thresholds: OnceCell::new(),
                        },
                    ));
                }
            }
        }
        // Сортировка стабильная: при равных `priority` правила остаются в
        // порядке файла.
        ordered.sort_by_key(|(priority, _)| *priority);
        let entries: Vec<Entry<'a>> = ordered.into_iter().map(|(_, entry)| entry).collect();
        let has_visuals = entries.iter().any(|entry| entry.visual.is_some());
        let bands = Self::bands(&entries, sheet);

        Self {
            sheet,
            entries,
            bands,
            has_visuals,
        }
    }

    /// Есть ли на листе правила, рисующие сами, — тогда `paint` добавляет слой
    /// изображений (полосы данных и значки).
    #[must_use]
    pub(crate) const fn has_visuals(&self) -> bool {
        self.has_visuals
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
            let visual = match &entry.visual {
                // Шкалы, гистограммы и значки считаются только по числу: тексту
                // и пустой ячейке они не рисуют ничего (допущение: так же ведёт
                // себя Excel — шкала на текстовую ячейку не ложится).
                Some(rule) => match eval.cell_value(at) {
                    Value::Number(value) => self.resolve_visual(book, entry, *rule, value),
                    _ => None,
                },
                None => None,
            };
            let fired = match &entry.visual {
                Some(_) => visual.is_some(),
                None => entry
                    .condition
                    .as_ref()
                    .is_some_and(|condition| condition.matches(&eval)),
            };
            if !fired {
                continue;
            }
            style.apply(&entry.overlay);
            if let Some(visual) = visual {
                style.set_visual(visual);
            }
            if entry.stop_if_true {
                break;
            }
        }
        style
    }

    /// Изображение визуального правила для значения ячейки.
    fn resolve_visual(
        &self,
        book: &Workbook,
        entry: &Entry<'a>,
        rule: VisualRule<'a>,
        value: f64,
    ) -> Option<Visual> {
        // Пороги зависят только от содержимого диапазона, а не от ячейки, и
        // считаются один раз — при первом видимом значении этого правила.
        let thresholds = entry
            .thresholds
            .get_or_init(|| Thresholds::new(self.sheet, entry.range, rule.thresholds()));
        match rule {
            VisualRule::ColorScale(scale) => {
                color_scale(book.theme(), scale, &thresholds.values, value)
            }
            VisualRule::DataBar(bar) => data_bar(book.theme(), bar, &thresholds.values, value),
            VisualRule::IconSet(set, shape) => icon_set(set, shape, &thresholds.values, value),
        }
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
    /// Изображение визуального правила; последнее по порядку применения
    /// побеждает, как и у полей `dxf`.
    visual: Option<Visual>,
}

impl<'a> EffectiveStyle<'a> {
    /// Оформление без наложений — формат ячейки как он записан в файле.
    #[must_use]
    pub fn new(base: CellFormat) -> Self {
        Self {
            base,
            applied: Overlay::default(),
            visual: None,
        }
    }

    /// Базовый формат ячейки из `cellXfs`.
    #[must_use]
    pub const fn base(&self) -> CellFormat {
        self.base
    }

    /// Изображение, которое рисует само правило: полоса данных или значок.
    ///
    /// Цветовая шкала сюда не попадает — её цвет отдан заливке (см.
    /// [`Self::background`]).
    #[must_use]
    pub(crate) const fn visual(&self) -> Option<Visual> {
        match self.visual {
            Some(Visual::ColorScale(_)) | None => None,
            Some(visual) => Some(visual),
        }
    }

    /// Фон ячейки из цветовой шкалы: он перекрывает заливку формата.
    #[must_use]
    pub(crate) const fn background(&self) -> Option<Rgba> {
        match self.visual {
            Some(Visual::ColorScale(color)) => Some(color),
            _ => None,
        }
    }

    /// Прячет ли правило значение ячейки (`showValue="0"`).
    #[must_use]
    pub(crate) const fn hides_value(&self) -> bool {
        match self.visual {
            Some(Visual::DataBar { show_value, .. } | Visual::Icon { show_value, .. }) => {
                !show_value
            }
            _ => false,
        }
    }

    /// Принять изображение сработавшего визуального правила.
    fn set_visual(&mut self, visual: Visual) {
        self.visual = Some(visual);
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
            // Заливка `dxf` перекрывает фон цветовой шкалы — как и любое
            // позднее правило. Полоса и значок остаются: они не фон.
            if matches!(self.visual, Some(Visual::ColorScale(_))) {
                self.visual = None;
            }
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

/// Визуальное правило: шкала, гистограмма или набор значков.
#[derive(Debug, Clone, Copy)]
enum VisualRule<'a> {
    ColorScale(&'a ColorScale),
    DataBar(&'a DataBar),
    /// Набор значков и разобранная форма глифов: имя набора ищется подстрокой,
    /// и в горячем пути (оформление ячейки спрашивается по нескольку раз на
    /// ячейку) этот поиск стоил дороже всей раскладки листа.
    IconSet(&'a IconSet, IconShape),
}

/// Форма значков набора — категория имени `iconSet`, разобранная один раз.
#[derive(Debug, Clone, Copy)]
enum IconShape {
    /// Наборы со стрелками (`3Arrows`, `4Arrows`, `5Arrows`, …).
    Arrows,
    /// Звёзды и рейтинги (`3Stars`, `5Rating`, …).
    Star,
    /// Остальные наборы: светофоры, флаги, четверти, символы, знаки.
    Other,
}

impl IconShape {
    /// Категория по имени набора; повторяет прежние проверки `str::contains`.
    fn of(icon_set: &str) -> Self {
        if icon_set.contains("Arrow") {
            Self::Arrows
        } else if icon_set.contains("Star") || icon_set.contains("Rating") {
            Self::Star
        } else {
            Self::Other
        }
    }
}

impl<'a> VisualRule<'a> {
    /// Разобрать правило; `None` — правило не визуальное или нечего рисовать.
    ///
    /// У шкалы число цветов должно совпадать с числом порогов, иначе неясно,
    /// какому порогу какой цвет; у всех трёх видов порогов должно быть не
    /// меньше двух — с одним полосу и значок не с чем сравнить.
    fn parse(kind: &'a RuleKind) -> Option<Self> {
        let rule = match kind {
            RuleKind::ColorScale(scale) => {
                if scale.colors.len() != scale.thresholds.len() {
                    return None;
                }
                Self::ColorScale(scale)
            }
            RuleKind::DataBar(bar) => Self::DataBar(bar),
            RuleKind::IconSet(set) => Self::IconSet(set, IconShape::of(&set.icon_set)),
            _ => return None,
        };
        (rule.thresholds().len() >= 2).then_some(rule)
    }

    /// Пороги правила.
    fn thresholds(self) -> &'a [Threshold] {
        match self {
            Self::ColorScale(scale) => &scale.thresholds,
            Self::DataBar(bar) => &bar.thresholds,
            Self::IconSet(set, _) => &set.thresholds,
        }
    }
}

/// Изображение, которое правило рисует само, без `dxf`.
///
/// Цвета разрешены в RGBA здесь же: шкала интерполируется между порогами, а
/// палитра темы доступна только на этом шаге.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Visual {
    /// Фон ячейки из цветовой шкалы.
    ColorScale(Rgba),
    /// Полоса данных: доли ширины ячейки от левого и правого края (0…1).
    DataBar {
        start: f32,
        end: f32,
        color: Rgba,
        show_value: bool,
    },
    /// Значок набора: глиф и цвет.
    Icon {
        glyph: &'static str,
        color: Rgba,
        show_value: bool,
    },
}

/// Числовые значения порогов правила по содержимому его диапазона.
///
/// `NaN` — порог не вычислить: формула (полноценного вычислителя формул в
/// крейте нет) или в диапазоне нет ни одного числа. Правило с таким порогом не
/// рисуется вовсе: потерять оформление безопаснее, чем показать неверную
/// границу.
#[derive(Debug)]
struct Thresholds {
    /// По значению на порог, в порядке порогов.
    values: Vec<f64>,
}

impl Thresholds {
    fn new(sheet: &Sheet, range: Range, thresholds: &[Threshold]) -> Self {
        let want_sorted = thresholds
            .iter()
            .any(|threshold| threshold.kind == ThresholdKind::Percentile);
        let numbers = Numbers::of(sheet, range, want_sorted);
        Self {
            values: thresholds
                .iter()
                .map(|threshold| numbers.resolve(threshold))
                .collect(),
        }
    }
}

/// Числа диапазона: границы и, если нужен процентиль, отсортированный список.
///
/// Обход идёт по непустым строкам диапазона: диапазон правила бывает во весь
/// лист, а ячеек в нём — единицы, и перебирать пустые строки нельзя.
#[derive(Debug)]
struct Numbers {
    count: u64,
    min: f64,
    max: f64,
    sorted: Vec<f64>,
}

impl Numbers {
    fn of(sheet: &Sheet, range: Range, want_sorted: bool) -> Self {
        let mut numbers = Self {
            count: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            sorted: Vec::new(),
        };
        for (_, cells) in sheet.cells.rows_in(range.first.row, range.last.row) {
            // Внутри строки столбцы отсортированы — границы берутся поиском.
            let start = cells.partition_point(|cell| cell.col < range.first.col);
            let end = cells.partition_point(|cell| cell.col <= range.last.col);
            for cell in &cells[start..end] {
                let CellValue::Number(value) = &cell.value else {
                    continue;
                };
                numbers.count += 1;
                numbers.min = numbers.min.min(*value);
                numbers.max = numbers.max.max(*value);
                if want_sorted {
                    numbers.sorted.push(*value);
                }
            }
        }
        if want_sorted {
            numbers.sorted.sort_by(f64::total_cmp);
        }
        numbers
    }

    /// Значение порога; `NaN` — вычислить нечем.
    fn resolve(&self, threshold: &Threshold) -> f64 {
        if self.count == 0 && threshold.kind != ThresholdKind::Number {
            // Ни одного числа: ни границ, ни процентилей, ни процентов от
            // диапазона. Числовой порог от содержимого не зависит.
            return f64::NAN;
        }
        match threshold.kind {
            ThresholdKind::Min => self.min,
            ThresholdKind::Max => self.max,
            ThresholdKind::Number => threshold.value.unwrap_or(f64::NAN),
            ThresholdKind::Percent => threshold.value.map_or(f64::NAN, |percent| {
                self.min + (self.max - self.min) * percent / 100.0
            }),
            ThresholdKind::Percentile => threshold
                .value
                .map_or(f64::NAN, |percent| percentile(&self.sorted, percent)),
            // Значение формулы зависит от листа; вычислителя формул в крейте нет.
            ThresholdKind::Formula => f64::NAN,
        }
    }
}

/// Процентиль по линейной интерполяции — как `PERCENTILE.INC` в Excel.
///
/// Допущение: каким алгоритмом считает процентиль условное форматирование
/// Excel, не проверялось; взят стандартный линейный.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn percentile(sorted: &[f64], percent: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (percent / 100.0).clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let low = rank.floor();
    let index = low as usize;
    let fraction = rank - low;
    match sorted.get(index + 1) {
        Some(next) => sorted[index] + (next - sorted[index]) * fraction,
        None => sorted[index],
    }
}

/// Цвет шкалы для значения: интерполяция между её порогами.
///
/// Значения вне шкалы получают крайние цвета. Порядок порогов задаёт файл, но
/// полагаться на него нельзя: Excel расставляет их по возрастанию.
#[allow(clippy::cast_possible_truncation)]
fn color_scale(theme: &Theme, scale: &ColorScale, values: &[f64], value: f64) -> Option<Visual> {
    let mut points = Vec::with_capacity(values.len());
    for (threshold, color) in values.iter().zip(&scale.colors) {
        // Невычислимый порог или неразрешённый цвет роняет шкалу целиком:
        // пропущенная точка сдвинула бы цвета на всех ячейках.
        if !threshold.is_finite() {
            return None;
        }
        points.push((*threshold, resolve_color(theme, *color)?));
    }
    if points.len() < 2 {
        return None;
    }
    points.sort_by(|left, right| left.0.total_cmp(&right.0));

    let last = points.len() - 1;
    if value <= points[0].0 {
        return Some(Visual::ColorScale(points[0].1));
    }
    if value >= points[last].0 {
        return Some(Visual::ColorScale(points[last].1));
    }
    for pair in points.windows(2) {
        let (low, low_color) = pair[0];
        let (high, high_color) = pair[1];
        if value >= low && value <= high {
            if high <= low {
                return Some(Visual::ColorScale(high_color));
            }
            let fraction = ((value - low) / (high - low)) as f32;
            return Some(Visual::ColorScale(lerp_color(
                low_color, high_color, fraction,
            )));
        }
    }
    None
}

/// Линейная интерполяция цвета по каналам — в sRGB.
///
/// Допущение: в каком пространстве интерполирует Excel, не проверялось; взята
/// покомпонентная линейная — самое простое чтение формата.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn lerp_color(from: Rgba, to: Rgba, t: f32) -> Rgba {
    let channel = |shift: u32| {
        let start = ((from.0 >> shift) & 0xFF) as f32;
        let end = ((to.0 >> shift) & 0xFF) as f32;
        (start + (end - start) * t).round().clamp(0.0, 255.0) as u32
    };
    Rgba(channel(0) | (channel(8) << 8) | (channel(16) << 16) | (channel(24) << 24))
}

/// Полоса данных: доли ширины ячейки, которые она занимает.
fn data_bar(theme: &Theme, bar: &DataBar, values: &[f64], value: f64) -> Option<Visual> {
    let low = *values.first()?;
    let high = *values.last()?;
    if !low.is_finite() || !high.is_finite() {
        return None;
    }
    let color = resolve_color(theme, bar.color)?;
    let (start, end) = bar_span(low, high, value);
    Some(Visual::DataBar {
        start,
        end,
        color,
        show_value: bar.show_value,
    })
}

/// Доли ячейки, которые занимает полоса: `(начало, конец)`.
///
/// Шкала — диапазон порогов `[low, high]`. Если среди значений есть
/// отрицательные, Excel 2010+ отсчитывает полосы от нулевой оси: положительные
/// растут вправо, отрицательные — влево. Если отрицательных нет, ось не
/// показывается и полоса отсчитывается от левого края — самое малое значение
/// получает полосу нулевой длины.
///
/// Не поддержано, потому что этого нет в разобранной модели: цвет
/// отрицательных полос (`negativeFillColor`), положение оси (`axisPosition`) и
/// наименьшая длина полосы (`PercentMin`) — расширения `x14`, в файле они лежат
/// вне `<dataBar>`. Полосы обоих знаков рисуются цветом правила.
#[allow(clippy::cast_possible_truncation)]
fn bar_span(low: f64, high: f64, value: f64) -> (f32, f32) {
    if high <= low {
        // Вырожденная шкала: сравнивать значение не с чем.
        return (0.0, 1.0);
    }
    if low >= 0.0 {
        let end = ((value - low) / (high - low)).clamp(0.0, 1.0) as f32;
        return (0.0, end);
    }
    // Ось — положение нуля в шкале.
    let axis = (-low / (high - low)).clamp(0.0, 1.0);
    if value >= 0.0 {
        let end = if high > 0.0 {
            axis + (1.0 - axis) * (value / high)
        } else {
            axis
        };
        (axis as f32, end.clamp(axis, 1.0) as f32)
    } else {
        // `value / low` положительно: оба отрицательны, отношение — доля
        // |value| от |low|.
        let start = axis - axis * (value / low);
        (start.clamp(0.0, axis) as f32, axis as f32)
    }
}

/// Значок набора по значению.
fn icon_set(set: &IconSet, shape: IconShape, values: &[f64], value: f64) -> Option<Visual> {
    let count = values.len();
    let mut level = 0;
    for (index, (threshold, cfvo)) in values.iter().zip(&set.thresholds).enumerate() {
        if !threshold.is_finite() {
            return None;
        }
        let passed = if cfvo.gte {
            value >= *threshold
        } else {
            value > *threshold
        };
        if passed && index > level {
            level = index;
        }
    }
    // `reverse="1"` переворачивает значки, а не пороги: нижнему диапазону
    // достаётся значок верхнего. Значение ниже первого порога получает нижний
    // значок — как в Excel.
    let shown = if set.reverse {
        count - 1 - level
    } else {
        level
    };
    Some(Visual::Icon {
        glyph: icon_glyph(shape, shown, count),
        color: icon_color(shown, count),
        show_value: set.show_value,
    })
}

/// Глиф значка.
///
/// В `DrawCommand` нет путей и полигонов, поэтому значок — текстовый глиф: одна
/// команда на ячейку против пяти `Rect` у фигуры из прямоугольников (на 10k
/// ячеек это разница в сотни килобайт кадра). Canvas подставляет шрифт по
/// символу, если глифа нет в гарнитуре ячейки, поэтому `▲`/`▼`/`●`/`★` не
/// превратятся в пустые квадраты.
///
/// Растровые значки Excel не воспроизводим: направление несут только наборы
/// стрелок, у остальных наборов форма одна, а уровень виден по цвету.
#[allow(clippy::cast_precision_loss)]
fn icon_glyph(shape: IconShape, shown: usize, count: usize) -> &'static str {
    let position = if count > 1 {
        shown as f32 / (count - 1) as f32
    } else {
        0.0
    };
    match shape {
        IconShape::Arrows => {
            if position < 1.0 / 3.0 {
                "▼"
            } else if position > 2.0 / 3.0 {
                "▲"
            } else {
                "●"
            }
        }
        IconShape::Star => "★",
        // 3TrafficLights1, 5Quarters, 3Flags, 4Boxes, 3Symbols, 3Signs, …
        IconShape::Other => "●",
    }
}

/// Цвет значка: палитра условного форматирования Excel, растянутая на число
/// уровней набора.
///
/// Её три цвета — умолчания Excel (`F8696B` → `FFEB84` → `63BE7B`), те же, что
/// стоят в фикстуре цветовой шкалы. Растровые значки Excel цвета не отдают,
/// поэтому взят общий ряд «плохо → хорошо».
#[allow(clippy::cast_precision_loss)]
fn icon_color(shown: usize, count: usize) -> Rgba {
    const LOW: Rgba = Rgba(0xF8_69_6B_FF);
    const MID: Rgba = Rgba(0xFF_EB_84_FF);
    const HIGH: Rgba = Rgba(0x63_BE_7B_FF);
    if count <= 1 {
        return MID;
    }
    let position = shown as f32 / (count - 1) as f32;
    if position <= 0.5 {
        lerp_color(LOW, MID, position * 2.0)
    } else {
        lerp_color(MID, HIGH, (position - 0.5) * 2.0)
    }
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

    // --- Визуальные правила -------------------------------------------------

    /// Правило, рисующее само, — без `dxf`.
    fn visual_rule(priority: u32, kind: RuleKind) -> ConditionalRule {
        ConditionalRule {
            priority,
            stop_if_true: false,
            dxf_id: None,
            kind,
        }
    }

    fn cfvo(kind: ThresholdKind, value: Option<f64>) -> Threshold {
        Threshold {
            kind,
            value,
            gte: true,
        }
    }

    fn style_at<'a>(
        book: &'a Workbook,
        index: &RuleIndex<'a>,
        row: u32,
        col: u32,
    ) -> EffectiveStyle<'a> {
        index.style_at(book, CellRef::new(row, col))
    }

    fn visual_of(book: &Workbook, index: &RuleIndex<'_>, row: u32, col: u32) -> Option<Visual> {
        style_at(book, index, row, col).visual()
    }

    fn background_of(book: &Workbook, index: &RuleIndex<'_>, row: u32, col: u32) -> Option<Rgba> {
        style_at(book, index, row, col).background()
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-4
    }

    /// Книга с визуальным правилом на `A1:A10` и числами в столбце A.
    fn book_with_visual(kind: RuleKind, cells: &[(u32, u32, f64)]) -> Workbook {
        book_with_cells(vec![visual_rule(1, kind)], vec![], cells)
    }

    fn two_color_scale() -> ColorScale {
        ColorScale {
            thresholds: vec![
                cfvo(ThresholdKind::Min, None),
                cfvo(ThresholdKind::Max, None),
            ],
            colors: vec![Color::Rgb(0xFF00_0000), Color::Rgb(0xFFFF_FFFF)],
        }
    }

    #[test]
    fn color_scale_interpolates_between_thresholds() {
        let book = book_with_visual(
            RuleKind::ColorScale(two_color_scale()),
            &[(0, 0, 0.0), (1, 0, 50.0), (2, 0, 100.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Ноль — первый цвет, сотня — второй, середина — ровно между ними.
        assert_eq!(background_of(&book, &index, 0, 0), Some(Rgba(0x0000_00FF)));
        assert_eq!(background_of(&book, &index, 2, 0), Some(Rgba(0xFFFF_FFFF)));
        assert_eq!(background_of(&book, &index, 1, 0), Some(Rgba(0x8080_80FF)));
    }

    #[test]
    fn color_scale_uses_percentile_threshold() {
        let scale = ColorScale {
            thresholds: vec![
                cfvo(ThresholdKind::Min, None),
                cfvo(ThresholdKind::Percentile, Some(50.0)),
                cfvo(ThresholdKind::Max, None),
            ],
            colors: vec![
                Color::Rgb(0xFF00_0000),
                Color::Rgb(0xFFFF_0000),
                Color::Rgb(0xFFFF_FFFF),
            ],
        };
        // Процентиль 50 от [10, 20, 30, 40] — это 25: значение ровно на
        // среднем пороге получает средний цвет.
        let book = book_with_range(
            vec![visual_rule(1, RuleKind::ColorScale(scale))],
            vec![],
            &[
                (0, 0, 10.0),
                (0, 1, 25.0),
                (1, 0, 20.0),
                (2, 0, 30.0),
                (3, 0, 40.0),
            ],
            "A1:B10",
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // `Rgb(0xFFFF_0000)` — это AARRGGBB, в кадре красный: `FF0000FF`.
        assert_eq!(background_of(&book, &index, 0, 1), Some(Rgba(0xFF00_00FF)));
        assert_eq!(background_of(&book, &index, 0, 0), Some(Rgba(0x0000_00FF)));
        assert_eq!(background_of(&book, &index, 3, 0), Some(Rgba(0xFFFF_FFFF)));
    }

    #[test]
    fn color_scale_counts_only_cells_inside_the_range() {
        let book = book_with_range(
            vec![visual_rule(1, RuleKind::ColorScale(two_color_scale()))],
            vec![],
            &[(0, 0, 10.0), (1, 0, 20.0), (5, 0, 100.0)],
            "A1:A2",
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Сотня лежит вне диапазона и максимумом не становится: 20 — белый.
        assert_eq!(background_of(&book, &index, 1, 0), Some(Rgba(0xFFFF_FFFF)));
        assert_eq!(background_of(&book, &index, 5, 0), None);
    }

    #[test]
    fn color_scale_skips_text_and_empty_cells() {
        let book = book_with_range(
            vec![visual_rule(1, RuleKind::ColorScale(two_color_scale()))],
            vec![],
            &[(0, 0, 10.0), (1, 0, 90.0)],
            "A1:C10",
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Текстовая ячейка шкалой не красится.
        assert_eq!(background_of(&book, &index, 0, 0), Some(Rgba(0x0000_00FF)));
        assert_eq!(background_of(&book, &index, 0, 1), None);
        assert_eq!(background_of(&book, &index, 0, 2), None);
    }

    #[test]
    fn color_scale_drops_the_rule_when_a_threshold_is_uncomputable() {
        let scale = ColorScale {
            thresholds: vec![
                cfvo(ThresholdKind::Formula, None),
                cfvo(ThresholdKind::Max, None),
            ],
            colors: vec![Color::Rgb(0xFF00_0000), Color::Rgb(0xFFFF_FFFF)],
        };
        let book = book_with_visual(RuleKind::ColorScale(scale), &[(0, 0, 10.0), (1, 0, 20.0)]);
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        assert_eq!(background_of(&book, &index, 0, 0), None);
    }

    fn two_threshold_bar(color: Color) -> DataBar {
        DataBar {
            thresholds: vec![
                cfvo(ThresholdKind::Number, Some(10.0)),
                cfvo(ThresholdKind::Number, Some(65.0)),
            ],
            color,
            show_value: true,
        }
    }

    fn bar_span_of(visual: Option<Visual>) -> (f32, f32) {
        match visual {
            Some(Visual::DataBar { start, end, .. }) => (start, end),
            other => panic!("ожидалась полоса данных, получено {other:?}"),
        }
    }

    #[test]
    fn data_bar_is_proportional_for_positive_values() {
        let book = book_with_visual(
            RuleKind::DataBar(two_threshold_bar(Color::Rgb(0xFF00_0000))),
            &[(0, 0, 10.0), (1, 0, 37.5), (2, 0, 65.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Все значения положительные: шкала от минимума к максимуму, и
        // минимум получает полосу нулевой длины.
        assert_eq!(bar_span_of(visual_of(&book, &index, 0, 0)), (0.0, 0.0));
        let (start, end) = bar_span_of(visual_of(&book, &index, 1, 0));
        assert!(close(start, 0.0), "полоса от левого края: {start}");
        assert!(close(end, 0.5), "середина шкалы — половина полосы: {end}");
        assert_eq!(bar_span_of(visual_of(&book, &index, 2, 0)), (0.0, 1.0));
    }

    #[test]
    fn data_bar_of_negative_values_grows_from_the_zero_axis() {
        let bar = DataBar {
            thresholds: vec![
                cfvo(ThresholdKind::Number, Some(-50.0)),
                cfvo(ThresholdKind::Number, Some(70.0)),
            ],
            color: Color::Rgb(0xFF00_0000),
            show_value: true,
        };
        let book = book_with_visual(
            RuleKind::DataBar(bar),
            &[(0, 0, -40.0), (1, 0, 0.0), (2, 0, 70.0), (3, 0, -50.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Ось — на 50/120 ширины: отрицательная полоса уходит влево от неё,
        // положительная — вправо.
        let (start, end) = bar_span_of(visual_of(&book, &index, 0, 0));
        assert!(close(start, 0.0833), "начало отрицательной полосы: {start}");
        assert!(close(end, 0.4167), "конец отрицательной полосы: {end}");

        // Ноль — полоса нулевой длины ровно на оси.
        let (start, end) = bar_span_of(visual_of(&book, &index, 1, 0));
        assert!(
            close(start, 0.4167) && close(end, 0.4167),
            "ноль: {start}…{end}"
        );

        let (start, end) = bar_span_of(visual_of(&book, &index, 2, 0));
        assert!(close(start, 0.4167), "начало положительной полосы: {start}");
        assert!(close(end, 1.0), "полоса до правого края: {end}");

        // Самое отрицательное значение — полоса от левого края до оси.
        let (start, end) = bar_span_of(visual_of(&book, &index, 3, 0));
        assert!(close(start, 0.0), "полоса минимума от левого края: {start}");
        assert!(close(end, 0.4167), "конец полосы минимума: {end}");
    }

    #[test]
    fn data_bar_of_an_all_negative_scale_grows_from_the_right() {
        let bar = DataBar {
            thresholds: vec![
                cfvo(ThresholdKind::Number, Some(-50.0)),
                cfvo(ThresholdKind::Number, Some(-10.0)),
            ],
            color: Color::Rgb(0xFF00_0000),
            show_value: true,
        };
        let book = book_with_visual(RuleKind::DataBar(bar), &[(0, 0, -50.0), (1, 0, -10.0)]);
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        assert_eq!(bar_span_of(visual_of(&book, &index, 0, 0)), (0.0, 1.0));
        let (start, end) = bar_span_of(visual_of(&book, &index, 1, 0));
        assert!(close(start, 0.8), "короткая полоса у правого края: {start}");
        assert!(close(end, 1.0), "короткая полоса до правого края: {end}");
    }

    fn three_arrow_set(reverse: bool, show_value: bool) -> IconSet {
        IconSet {
            icon_set: "3Arrows".into(),
            reverse,
            show_value,
            thresholds: vec![
                cfvo(ThresholdKind::Number, Some(0.0)),
                cfvo(ThresholdKind::Number, Some(50.0)),
                cfvo(ThresholdKind::Number, Some(100.0)),
            ],
        }
    }

    fn icon_of(visual: Option<Visual>) -> (&'static str, Rgba, bool) {
        match visual {
            Some(Visual::Icon {
                glyph,
                color,
                show_value,
            }) => (glyph, color, show_value),
            other => panic!("ожидался значок, получено {other:?}"),
        }
    }

    #[test]
    fn icon_set_picks_the_icon_by_threshold() {
        let book = book_with_visual(
            RuleKind::IconSet(three_arrow_set(false, true)),
            &[(0, 0, 10.0), (1, 0, 60.0), (2, 0, 100.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        assert_eq!(
            icon_of(visual_of(&book, &index, 0, 0)),
            ("▼", Rgba(0xF869_6BFF), true)
        );
        assert_eq!(
            icon_of(visual_of(&book, &index, 1, 0)),
            ("●", Rgba(0xFFEB_84FF), true)
        );
        assert_eq!(
            icon_of(visual_of(&book, &index, 2, 0)),
            ("▲", Rgba(0x63BE_7BFF), true)
        );
    }

    /// Звёзды и рейтинги рисуются одним глифом на любом уровне.
    #[test]
    fn star_sets_use_the_star_glyph() {
        for name in ["3Stars", "5Rating"] {
            let mut set = three_arrow_set(false, true);
            set.icon_set = name.into();
            let book = book_with_visual(RuleKind::IconSet(set), &[(0, 0, 10.0), (1, 0, 100.0)]);
            let sheet = &book.sheets()[0];
            let index = RuleIndex::new(&book, sheet);

            assert_eq!(icon_of(visual_of(&book, &index, 0, 0)).0, "★", "{name}");
            assert_eq!(icon_of(visual_of(&book, &index, 1, 0)).0, "★", "{name}");
        }
    }

    #[test]
    fn icon_set_reverse_mirrors_the_icons() {
        let book = book_with_visual(
            RuleKind::IconSet(three_arrow_set(true, true)),
            &[(0, 0, 10.0), (1, 0, 100.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        // Высшему значению достаётся значок нижнего диапазона и наоборот.
        assert_eq!(
            icon_of(visual_of(&book, &index, 1, 0)),
            ("▼", Rgba(0xF869_6BFF), true)
        );
        assert_eq!(
            icon_of(visual_of(&book, &index, 0, 0)),
            ("▲", Rgba(0x63BE_7BFF), true)
        );
    }

    #[test]
    fn icon_set_keeps_its_levels_for_four_and_five_icons() {
        let mut set = three_arrow_set(false, true);
        set.thresholds = vec![
            cfvo(ThresholdKind::Number, Some(0.0)),
            cfvo(ThresholdKind::Number, Some(25.0)),
            cfvo(ThresholdKind::Number, Some(50.0)),
            cfvo(ThresholdKind::Number, Some(75.0)),
        ];
        let book = book_with_visual(
            RuleKind::IconSet(set.clone()),
            &[(0, 0, 0.0), (1, 0, 30.0), (2, 0, 60.0), (3, 0, 80.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        let glyphs: Vec<&str> = (0..4)
            .map(|row| icon_of(visual_of(&book, &index, row, 0)).0)
            .collect();
        assert_eq!(glyphs, ["▼", "●", "●", "▲"]);

        // Пять уровней: цвет идёт тем же рядом, а не повторяет четвёрку.
        set.thresholds.push(cfvo(ThresholdKind::Number, Some(90.0)));
        let book = book_with_visual(RuleKind::IconSet(set), &[(0, 0, 0.0), (1, 0, 95.0)]);
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);
        assert_eq!(icon_of(visual_of(&book, &index, 0, 0)).1, Rgba(0xF869_6BFF));
        assert_eq!(icon_of(visual_of(&book, &index, 1, 0)).1, Rgba(0x63BE_7BFF));
    }

    #[test]
    fn icon_set_hides_the_value_when_asked() {
        let book = book_with_visual(
            RuleKind::IconSet(three_arrow_set(false, false)),
            &[(0, 0, 10.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);
        let style = style_at(&book, &index, 0, 0);

        assert!(style.hides_value());
        assert!(style.background().is_none());
    }

    #[test]
    fn visual_rules_share_the_priority_order_with_dxf_rules() {
        // `cellIs` с `stopIfTrue` и меньшим номером приоритета гасит значок.
        let mut stop = rule(1, 0, CellIsOperator::GreaterThan, &["0"]);
        stop.stop_if_true = true;
        let book = book_with_cells(
            vec![
                stop,
                visual_rule(2, RuleKind::IconSet(three_arrow_set(false, true))),
            ],
            vec![red()],
            &[(0, 0, 10.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);

        assert!(visual_of(&book, &index, 0, 0).is_none());
        assert!(style_at(&book, &index, 0, 0).fill(book.styles()).is_some());
    }

    #[test]
    fn dxf_fill_overrides_the_color_scale_background() {
        // Правило с заливкой идёт позже шкалы — фон берёт заливка `dxf`.
        let mut fill_rule = rule(2, 0, CellIsOperator::GreaterThan, &["0"]);
        fill_rule.stop_if_true = false;
        let book = book_with_cells(
            vec![
                visual_rule(1, RuleKind::ColorScale(two_color_scale())),
                fill_rule,
            ],
            vec![red()],
            &[(0, 0, 10.0), (1, 0, 20.0)],
        );
        let sheet = &book.sheets()[0];
        let index = RuleIndex::new(&book, sheet);
        let style = style_at(&book, &index, 0, 0);

        assert!(style.background().is_none());
        assert!(style.fill(book.styles()).is_some());
    }
}
