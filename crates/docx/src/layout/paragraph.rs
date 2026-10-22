//! Раскладка одного абзаца: строки, интервалы и флаги переноса.
//!
//! [`layout_paragraph`] отдаёт строки с готовыми координатами, а о разрывах
//! страниц только сообщает (`w:keepNext`, `w:keepLines`, `w:pageBreakBefore`,
//! `w:widowControl`): решения принимает движок, у которого есть страница целиком.
//!
//! Метрики и начертание приходят из каскада ([`super::cascade`]): кегль, цвет,
//! гарнитура, жирный/курсив/подчёркивание, интервалы и отступы резолвятся из
//! `docDefaults`, стиля абзаца и прямого форматирования. Гарнитура резолвится в
//! шрифт реестра по имени семейства (`w:rFonts`), а строка несёт имя того лица,
//! которым измерена: painter ставит canvas'у ровно его, поэтому точки переноса
//! совпадают с картинкой.
//!
//! Абзац раскладывается как один поток текста: прогоны с разным кеглем,
//! начертанием и гарнитурой складываются в одну строку, пока её суммарная ширина
//! помещается в полосу набора ([`LineBreaker::break_spanned`]). Стиль знака
//! строка берёт у первого прогона (`Segment::at`): пока у строки нет спанов —
//! отдельного слайса, — «обычный **жирный** обычный» рисуется начертанием
//! первого слова, но переносится уже целиком.

use doc_converter_render::font::FontId;
use unicode_bidi::BidiClass;

use crate::model::raw::{HalfPoint, Underline};
use crate::model::{
    Anchor, Document, Inline, InlineOrAnchor, Justification, Paragraph, Run, RunContent, StyleTable,
};

use super::cascade::{resolve_paragraph_styled, resolve_run_styled, ResolvedParagraph, StyleCache};
use super::engine::{LayoutOptions, LayoutState};
use super::line_break::{LineBreaker, LineRule, MetricSpan};

/// Строка абзаца после раскладки.
#[derive(Debug, Clone)]
pub struct Line {
    /// Текст строки.
    pub text: String,
    /// Позиция X от левого края страницы.
    pub x: f32,
    /// Ширина строки.
    pub width: f32,
    /// Высота строки.
    pub height: f32,
    /// Надбавка `w:line` (множитель) над базовым интерлиньяжем.
    ///
    /// `Exact`/`AtLeast` сюда не попадают — их высота не «надбавка»: на разрыве
    /// страницы в таблице отбрасывается только множитель, как и хвостовой
    /// `w:after` (см. `MeasuredBlock::line_rule_excess` в `layout/tables.rs`).
    pub line_rule_excess: f32,
    /// Цвет текста; `None` — цвет по умолчанию.
    pub color: Option<u32>,
    /// Имя гарнитуры, которым строка измерена и будет нарисована.
    ///
    /// Это `w:rFonts` прогона, если реестр знает такое семейство, и имя
    /// запасного лица ([`doc_converter_render::font::DEFAULT_FONT_FAMILY`]),
    /// если не знает: в кадр уходит ровно то имя, которое разрешилось в
    /// измеренный шрифт. `None` — документ гарнитуры не называл, и обе стороны
    /// берут умолчание реестра. Стиль знака берётся у первого прогона строки
    /// (см. `Segment::at`).
    pub font_family: Option<String>,
    /// Кегль (`w:sz`); в пиксели переводит [`half_points_to_px`].
    ///
    /// [`half_points_to_px`]: super::engine::half_points_to_px
    pub size: HalfPoint,
    /// Полужирный (`w:b`).
    pub bold: bool,
    /// Курсив (`w:i`).
    pub italic: bool,
    /// Подчёркивание (`w:u`), сведённое к факту: вид линии несёт только `DisplayList`.
    pub underline: bool,
    /// Фрагменты строки — по одному на отрезок с одним стилем знака.
    ///
    /// `Line` по-прежнему несёт стиль первого прогона: так её рисует canvas и
    /// так её читает PDF. Фрагменты — второй, аддитивный слой для `TextLayer`:
    /// они дают стиль и ширину каждой части, не меняя картинку.
    pub fragments: Vec<LineFragment>,
    /// Физическое выравнивание строки в её полосе набора.
    pub align: LineAlign,
    /// База строки по UAX #9: `true` — справа налево.
    pub rtl: bool,
}

/// Физическое выравнивание строки — без `w:jc`-семантики `Both`/`Distribute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineAlign {
    Left,
    Center,
    Right,
}

/// Отрезок строки с одним стилем знака.
///
/// `range` — байтовый диапазон в [`Line::text`] (не в тексте абзаца), поэтому
/// текст фрагмента — `line.text[range]` без второго хранилища строк.
#[derive(Debug, Clone)]
pub struct LineFragment {
    /// Байтовый диапазон в тексте строки.
    pub range: std::ops::Range<usize>,
    /// Смещение X от начала строки.
    pub x: f32,
    /// Ширина фрагмента.
    pub width: f32,
    /// Гарнитура, которой фрагмент измерен и будет нарисован.
    pub font_family: Option<String>,
    /// Кегль (`w:sz`) в полупунктах.
    pub size: HalfPoint,
    /// Полужирный (`w:b`).
    pub bold: bool,
    /// Курсив (`w:i`).
    pub italic: bool,
    /// Подчёркивание (`w:u`).
    pub underline: bool,
    /// Цвет текста `RRGGBB`; `None` — по умолчанию.
    pub color: Option<u32>,
}

/// Результат раскладки абзаца.
#[derive(Debug, Clone)]
pub struct ParagraphLayout {
    /// Строки сверху вниз.
    pub lines: Vec<Line>,
    /// Интервал перед абзацем.
    pub space_before: f32,
    /// Интервал после абзаца.
    pub space_after: f32,
    /// Абзац начинает новую страницу (`w:pageBreakBefore`).
    pub page_break_before: bool,
    /// Не отрывать абзац от следующего (`w:keepNext`).
    pub keep_next: bool,
    /// Не разрывать абзац между страницами (`w:keepLines`).
    pub keep_lines: bool,
    /// Не оставлять одну строку в начале или конце страницы (`w:widowControl`).
    pub widow_control: bool,
    /// Плавающие объекты абзаца.
    pub anchors: Vec<Anchor>,
    /// Уровень заголовка из `w:outlineLvl` (0–8 соответствует h1–h9);
    /// `None` — обычный абзац.
    pub heading_level: Option<u8>,
}

/// Отрезок абзаца с одним стилем знака: текст, метрики и начертание.
///
/// Абзац Word — один поток текста, а не набор независимых отрезков: отрезки
/// склеиваются в общий текст ([`ParagraphText`]), перенос идёт по суммарной
/// ширине ([`LineBreaker::break_spanned`]), и лишь метрики у частей строки
/// разные. Отрезок отвечает за две вещи: своими шрифтом и кеглем он меряет свою
/// часть строки, а его стиль знака строка берёт, если он её открыл.
///
/// Цвет, начертание и подчёркивание на ширину не влияют намеренно: одна система
/// ширин на экран и PDF держится на regular (ADR-0007), и нарезка по стилю знака
/// развалила бы «обычный **жирный** обычный» на три строки.
struct Segment {
    /// Текст отрезка.
    text: String,
    /// Шрифт отрезка: разрешённый `w:rFonts` или запасное лицо реестра.
    font_id: FontId,
    /// Имя, которым отрезок уходит в кадр: совпадает с измеренным лицом.
    draw_family: Option<String>,
    /// Кегль (`w:sz`), полупункты.
    size: HalfPoint,
    /// Полужирный (`w:b`).
    bold: bool,
    /// Курсив (`w:i`).
    italic: bool,
    /// Подчёркивание (`w:u`).
    underline: bool,
    /// Цвет текста (`w:color`).
    color: Option<u32>,
}

impl Segment {
    /// Отрезок, к которому относится байт `pos`.
    ///
    /// Строка начинается с него, и его стиль знака она несёт: спанов внутри
    /// строки пока нет, поэтому у строки один стиль на всю ширину.
    fn at<'a>(segments: &'a [Segment], starts: &[usize], pos: usize) -> &'a Segment {
        let index = starts
            .partition_point(|start| *start <= pos)
            .saturating_sub(1);
        &segments[index.min(segments.len() - 1)]
    }
}

/// Абзац как один текст: склеенные отрезки и спаны метрик по нему.
struct ParagraphText {
    /// Общий текст всех отрезков — то, по чему идёт перенос.
    text: String,
    /// Начало каждого отрезка в `text`: строка находит по нему свой стиль знака.
    starts: Vec<usize>,
    /// Спаны метрик по `text`: соседние отрезки с одним шрифтом и кеглем слиты,
    /// чтобы измерение слова не перебирало сотню прогонов абзаца.
    spans: Vec<MetricSpan>,
}

impl ParagraphText {
    /// Склеить отрезки в один текст и собрать спаны метрик.
    fn new(segments: &[Segment]) -> Self {
        let mut text = String::new();
        let mut starts = Vec::with_capacity(segments.len());
        let mut spans: Vec<MetricSpan> = Vec::new();

        for segment in segments {
            let start = text.len();
            text.push_str(&segment.text);
            starts.push(start);
            match spans.last_mut() {
                Some(last) if last.font_id == segment.font_id && last.size == segment.size => {
                    last.range.end = text.len();
                }
                _ => spans.push(MetricSpan {
                    range: start..text.len(),
                    font_id: segment.font_id,
                    size: segment.size,
                }),
            }
        }
        Self {
            text,
            starts,
            spans,
        }
    }

    /// Нарезать диапазон строки на фрагменты по границам отрезков.
    ///
    /// X считается той же меркой, что и ширина строки: префикс до фрагмента
    /// меряется спанами `MetricSpan`, поэтому второй системы ширин не
    /// появляется. Диапазоны фрагментов относительны тексту строки.
    fn fragments(
        &self,
        segments: &[Segment],
        line_range: std::ops::Range<usize>,
        breaker: &mut LineBreaker,
    ) -> Vec<LineFragment> {
        let mut fragments = Vec::new();
        let mut cursor = line_range.start;
        let mut x = 0.0;
        for (index, segment) in segments.iter().enumerate() {
            let start = self.starts[index].max(line_range.start);
            let end = self
                .starts
                .get(index + 1)
                .copied()
                .unwrap_or(self.text.len())
                .min(line_range.end);
            if start >= end {
                continue;
            }
            // Промежуток между фрагментами — обычно нулевой; если отрезок
            // начался до строки или между ними остались байты, они тоже
            // занимают ширину и не должны теряться.
            if start > cursor {
                x += breaker.measure_spanned(&self.text, &self.spans, cursor..start);
            }
            let width = breaker.measure_spanned(&self.text, &self.spans, start..end);
            fragments.push(LineFragment {
                range: (start - line_range.start)..(end - line_range.start),
                x,
                width,
                font_family: segment.draw_family.clone(),
                size: segment.size,
                bold: segment.bold,
                italic: segment.italic,
                underline: segment.underline,
                color: segment.color,
            });
            x += width;
            cursor = end;
        }
        fragments
    }
}

/// Подчёркивание знака, сведённое к факту.
///
/// У `ResolvedRPr.underline` три состояния, а не два: `None` — наследование не
/// тронуто, `Some(Underline::None)` — подчёркивание снято явно (`w:val="none"`).
/// Раскладке важен только факт, поэтому оба «нет» дают `false`.
fn is_underlined(underline: Option<&Underline>) -> bool {
    match underline {
        None | Some(Underline::None) => false,
        Some(_) => true,
    }
}

/// Собрать отрезки абзаца: текст прогонов с резолвленными метриками и стилем.
///
/// Гарнитура прогона разрешается в шрифт реестра по имени семейства — тем же
/// именем строка потом рисуется. Незарегистрированное семейство даёт запасное
/// лицо, и строка несёт его имя, а не имя из документа: иначе painter поставил
/// бы canvas'у шрифт, которым раскладка не мерила.
///
/// Гиперссылки и поля обходятся тем же `collect_runs`, что и в таблицах: текст
/// ссылки/оглавления — часть абзаца, и без него не только `TextLayer`, но и
/// canvas теряли содержимое.
fn segments(
    styles: &StyleTable,
    paragraph: &Paragraph,
    resolved: &ResolvedParagraph,
    cache: &mut StyleCache,
    line_breaker: &LineBreaker,
    include_fields: bool,
) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut runs = Vec::new();
    collect_runs(&paragraph.runs, &mut runs, include_fields);
    for run in runs {
        push_segment(&mut segments, styles, resolved, run, cache, line_breaker);
    }
    segments
}

/// Прогоны inline-элементов в порядке текста, с заходом в ссылки.
///
/// `include_fields` — обход результата полей. Тело документа поля не разворачивает
/// (это контракт [`super::fields`]: неизвестное поле остаётся пустым), а ячейка
/// таблицы исторически несёт кэш поля — например, оглавление целиком.
fn collect_runs<'a>(inlines: &'a [Inline], runs: &mut Vec<&'a Run>, include_fields: bool) {
    for inline in inlines {
        match inline {
            Inline::Run(run) => runs.push(run),
            Inline::Hyperlink(link) => collect_runs(&link.runs, runs, include_fields),
            Inline::Field(field) if include_fields => {
                collect_runs(&field.result, runs, include_fields);
            }
            _ => {}
        }
    }
}

/// Отрезок одного прогона: текст с разрешённым по каскаду стилем знака.
fn push_segment(
    segments: &mut Vec<Segment>,
    styles: &StyleTable,
    resolved: &ResolvedParagraph,
    run: &Run,
    cache: &mut StyleCache,
    line_breaker: &LineBreaker,
) {
    let text = run_text(run);
    if text.is_empty() {
        return;
    }
    let rpr = resolve_run_styled(styles, resolved, run, cache);
    for (chunk, rtl) in split_script(&text, rpr.rtl) {
        // RTL-кусок меряется гарнитурой и кеглем сложной раскладки (`w:cs`/
        // `w:szCs`), LTR — обычными `w:rFonts`/`w:sz`. Координаты строки
        // складываются из этих ширин, поэтому смешивать их нельзя.
        let (family, size) = if rtl {
            (
                rpr.cs_font_family.as_deref().or(rpr.font_family.as_deref()),
                rpr.size_cs.unwrap_or(rpr.size),
            )
        } else {
            (rpr.font_family.as_deref(), rpr.size)
        };
        segments.push(Segment {
            text: chunk.to_owned(),
            font_id: line_breaker.resolve_family(family),
            draw_family: line_breaker.drawable_family(family).map(str::to_owned),
            size,
            bold: rpr.bold,
            italic: rpr.italic,
            underline: is_underlined(rpr.underline.as_ref()),
            color: rpr.color,
        });
    }
}

/// Разбить текст run'а на куски сильного направления.
///
/// Нейтральные символы (пробелы, знаки) примыкают к предыдущему сильному куску:
/// иначе пробел между арабскими словами уезжал бы в латинскую мерку. В начале
/// строки направление нейтралов задаёт `w:rtl` прогона.
fn split_script(text: &str, default_rtl: bool) -> Vec<(&str, bool)> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut current: Option<bool> = None;
    for (index, ch) in text.char_indices() {
        let rtl = match super::bidi::class(ch) {
            BidiClass::R | BidiClass::AL | BidiClass::AN => true,
            BidiClass::L => false,
            _ => current.unwrap_or(default_rtl),
        };
        match current {
            Some(previous) if previous != rtl => {
                chunks.push((&text[start..index], previous));
                start = index;
                current = Some(rtl);
            }
            None => current = Some(rtl),
            Some(_) => {}
        }
    }
    if start < text.len() {
        chunks.push((&text[start..], current.unwrap_or(default_rtl)));
    }
    chunks
}

/// Разложить абзац в строки.
///
/// Отступы `w:ind` сужают полосу набора абзаца, а `w:firstLine`/`w:hanging`
/// сдвигает только её первую строку. `state` задаёт левую границу и ширину
/// полосы набора, но не позицию Y: строки ставит движок, который ведёт курсор
/// страницы.
#[must_use]
pub fn layout_paragraph(
    document: &Document,
    paragraph: &Paragraph,
    state: &LayoutState,
    options: &LayoutOptions,
    cache: &mut StyleCache,
    line_breaker: &mut LineBreaker,
) -> ParagraphLayout {
    layout_paragraph_styled(
        &document.styles,
        paragraph,
        state,
        options,
        cache,
        line_breaker,
        false,
    )
}

/// Разложить абзац по явно данным стилям — путь абзаца в ячейке таблицы.
///
/// Тело документа приходит сюда через [`layout_paragraph`]; таблица знает только
/// `&StyleTable` (ею же разрешается `w:tblStyle`), и абзац ячейки резолвится тем
/// же каскадом, что и абзац тела. Раскладка у обоих путей одна: кегль, цвет,
/// начертание, `w:jc`, `w:ind` и межстрочный интервал считаются здесь, а
/// вертикаль строк ведёт владелец абзаца.
#[must_use]
#[allow(clippy::too_many_arguments)]
// Разбивка «текст → строки → фрагменты» живёт одним проходом: вынести
// фрагменты в отдельную функцию значило бы заново передать туда каскад,
// сегменты и мерку.
#[allow(clippy::too_many_lines)]
pub(crate) fn layout_paragraph_styled(
    styles: &StyleTable,
    paragraph: &Paragraph,
    state: &LayoutState,
    options: &LayoutOptions,
    cache: &mut StyleCache,
    line_breaker: &mut LineBreaker,
    include_fields: bool,
) -> ParagraphLayout {
    let resolved = resolve_paragraph_styled(styles, paragraph, cache);
    let ind = resolved.ppr.ind;
    let jc = resolved.ppr.jc.as_ref();

    // Отступы слева и справа сужают полосу набора; первая строка получает ещё и
    // свой сдвиг — положительный отступ её сужает, выступ расширяет.
    let band = (options.effective_width(state.column_width) - ind.left_px - ind.right_px).max(0.0);
    let first_line_band = (band - ind.first_line_px).max(0.0);
    let origin = state.x + ind.left_px;

    let mut lines = Vec::new();
    let segments = segments(
        styles,
        paragraph,
        &resolved,
        cache,
        line_breaker,
        include_fields,
    );
    let flow = ParagraphText::new(&segments);

    // Первую строку абзаца отрезаем её полосой отдельно: перенос идёт по всему
    // тексту, а у первой строки ширина другая (`w:firstLine`/`w:hanging`).
    let mut broken = Vec::new();
    let mut laid_out = 0usize;
    if ind.first_line_px != 0.0 && !flow.text.is_empty() {
        if let Some(first) = line_breaker
            .break_spanned(&flow.text, &flow.spans, first_line_band, 0, Some(1))
            .into_iter()
            .next()
        {
            laid_out = first.range.end;
            broken.push((first, origin + ind.first_line_px, first_line_band));
        }
    }
    broken.extend(
        line_breaker
            .break_spanned(&flow.text, &flow.spans, band, laid_out, None)
            .into_iter()
            .map(|line| (line, origin, band)),
    );

    // База абзаца — из `w:bidi` или первого сильного символа. От неё зависят
    // выравнивание и направление строки в TextLayer.
    let rtl = super::bidi::base_rtl(&flow.text, resolved.ppr.bidi);

    for (line, x, line_band) in broken {
        let style = Segment::at(&segments, &flow.starts, line.range.start);
        // Высота строки — по самому крупному её прогону, а если спанов на
        // диапазон не нашлось (быть такого не должно) — по кеглю стиля.
        let size = if line.max_size.value() > 0 {
            line.max_size
        } else {
            style.size
        };
        // Шрифт — того же прогона, что задал кегль; без спанов — отрезка,
        // открывшего строку. Высоту считает метрика именно этого шрифта.
        let font_id = line.max_size_font.unwrap_or(style.font_id);
        let align = line_align(jc, rtl);
        let fragments = flow.fragments(&segments, line.range.clone(), line_breaker);
        lines.push(Line {
            text: flow.text[line.range].to_owned(),
            x: x + align_shift(jc, line_band, line.width, rtl),
            width: line.width,
            height: line_breaker.line_height_for(font_id, size, resolved.ppr.spacing.line),
            line_rule_excess: line_rule_excess(
                line_breaker,
                font_id,
                size,
                resolved.ppr.spacing.line,
            ),
            color: style.color,
            font_family: style.draw_family.clone(),
            size: style.size,
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
            fragments,
            align,
            rtl,
        });
    }

    // Абзац без текста всё равно занимает строку — как метка абзаца в Word.
    // Высота берётся из каскада: запасной вариант движка знает только 12 pt и
    // разошёлся бы с телом документа, набранным другим кеглем.
    if lines.is_empty() {
        let font_id = line_breaker.resolve_family(resolved.rpr.font_family.as_deref());
        lines.push(Line {
            text: String::new(),
            x: origin + ind.first_line_px + align_shift(jc, first_line_band, 0.0, rtl),
            width: 0.0,
            height: line_breaker.line_height_for(
                font_id,
                resolved.rpr.size,
                resolved.ppr.spacing.line,
            ),
            line_rule_excess: line_rule_excess(
                line_breaker,
                font_id,
                resolved.rpr.size,
                resolved.ppr.spacing.line,
            ),
            color: None,
            // Пустая строка — метка абзаца: её стиль знака берётся из абзаца,
            // откуда пришла и высота, а не из умолчания движка.
            font_family: line_breaker
                .drawable_family(resolved.rpr.font_family.as_deref())
                .map(str::to_owned),
            size: resolved.rpr.size,
            bold: resolved.rpr.bold,
            italic: resolved.rpr.italic,
            underline: is_underlined(resolved.rpr.underline.as_ref()),
            fragments: Vec::new(),
            align: line_align(jc, rtl),
            rtl,
        });
    }

    ParagraphLayout {
        lines,
        space_before: resolved.ppr.spacing.before_px,
        space_after: resolved.ppr.spacing.after_px,
        page_break_before: resolved.ppr.page_break_before,
        keep_next: resolved.ppr.keep_next,
        keep_lines: resolved.ppr.keep_lines,
        widow_control: resolved.ppr.widow_control,
        anchors: collect_anchors(paragraph),
        // `w:outlineLvl` 0-based; ARIA/HTML ждут 1-based уровень заголовка.
        heading_level: resolved
            .ppr
            .outline_lvl
            .map(|level| level.saturating_add(1).min(9)),
    }
}

/// Собрать плавающие рисунки абзаца в порядке появления.
///
/// Встроенные рисунки (`wp:inline`) пропускаются: их место — в строке текста, а
/// не в списке размещаемых объектов. `w:drawing` встречается на двух уровнях —
/// прямо в абзаце (`Inline::Drawing`) и внутри run'а (`RunContent::Drawing`), —
/// поэтому обход рекурсивный.
fn collect_anchors(paragraph: &Paragraph) -> Vec<Anchor> {
    let mut anchors = Vec::new();
    collect_inline_anchors(&paragraph.runs, &mut anchors);
    anchors
}

/// Обойти `Inline` и собрать анкоры; вложенные списки (гиперссылка, поле)
/// обходятся тем же порядком, что и текст в них.
fn collect_inline_anchors(inlines: &[Inline], anchors: &mut Vec<Anchor>) {
    for inline in inlines {
        match inline {
            Inline::Drawing(drawing) => push_anchor(drawing, anchors),
            Inline::Run(run) => {
                for content in &run.content {
                    if let RunContent::Drawing(drawing) = content {
                        push_anchor(drawing, anchors);
                    }
                }
            }
            Inline::Hyperlink(hyperlink) => collect_inline_anchors(&hyperlink.runs, anchors),
            Inline::Field(field) => collect_inline_anchors(&field.result, anchors),
            _ => {}
        }
    }
}

/// Добавить анкор рисунка; у встроенной картинки его нет.
fn push_anchor(drawing: &InlineOrAnchor, anchors: &mut Vec<Anchor>) {
    if let Some(anchor) = &drawing.anchor {
        anchors.push(anchor.clone());
    }
}

/// Текст run'а: содержимое `w:t`; `w:tab`, `w:br` и рисунки раскладка пока не несёт.
fn run_text(run: &Run) -> String {
    run.content
        .iter()
        .filter_map(|content| match content {
            RunContent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Сдвиг строки внутри её полосы по `w:jc`.
///
/// `Both` и `Distribute` идут как начало строки: растянуть строку можно, лишь
/// зная, какие пробелы в ней растяжимы, — это отдельный слайс.
fn align_shift(jc: Option<&Justification>, band: f32, width: f32, rtl: bool) -> f32 {
    match line_align(jc, rtl) {
        LineAlign::Center => (band - width) / 2.0,
        LineAlign::Right => band - width,
        LineAlign::Left => 0.0,
    }
}

/// Физическое выравнивание строки для `TextLayer` и правого якоря canvas.
///
/// `Start`/`End` зависят от направления: в RTL `Start` — справа, `End` — слева.
/// Без явного `w:jc` RTL-абзац выравнивается вправо, как в Word.
fn line_align(jc: Option<&Justification>, rtl: bool) -> LineAlign {
    let start = if rtl {
        LineAlign::Right
    } else {
        LineAlign::Left
    };
    let end = if rtl {
        LineAlign::Left
    } else {
        LineAlign::Right
    };
    match jc {
        Some(Justification::Center) => LineAlign::Center,
        Some(Justification::Right) => LineAlign::Right,
        Some(Justification::Left) => LineAlign::Left,
        Some(Justification::Start) => start,
        Some(Justification::End) => end,
        Some(Justification::Both | Justification::Distribute | Justification::Other(_)) | None => {
            start
        }
    }
}

/// Надбавка `w:line` над базовым интерлиньяжем: только множитель `Auto` больше
/// единицы. `Exact`/`AtLeast` — не надбавка, а явная высота: на разрыве
/// страницы в таблице она не отбрасывается (см. `MeasuredBlock::line_rule_excess`
/// в `layout/tables.rs`).
fn line_rule_excess(
    breaker: &mut LineBreaker,
    font_id: FontId,
    size: HalfPoint,
    rule: LineRule,
) -> f32 {
    match rule {
        LineRule::Auto(multiplier) if multiplier > 1.0 => {
            breaker.line_height(font_id, size) * (multiplier - 1.0)
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::layout::engine::{half_points_to_px, twips_to_px};
    use crate::layout::float::emu_to_px;
    use crate::model::raw::{
        Color, HalfPoint, Ind, LineSpacing, LineSpacingRule, ParagraphSpacing, RFonts, RawPPr,
        RawRPr, Toggle, Underline,
    };
    use crate::model::{
        BlockItem, Body, DocDefaults, Extent, Hyperlink, InlineImage, InlineOrAnchor, Metadata,
        NumberingTable, PositionH, PositionV, RelFromH, RelFromV, Relationships, Settings,
        StyleTable, WrapKind,
    };
    use crate::Twips;
    use doc_converter_core::NodeId;
    use doc_converter_render::font::FontRegistry;

    /// Сравнить пиксели с допуском: `f32` в `assert_eq!` не пройдёт `clippy::float_cmp`.
    #[track_caller]
    fn assert_px(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 0.01,
            "{what}: ожидалось {expected} px, получено {actual} px"
        );
    }

    /// Документ с пустым телом и заданными `docDefaults`.
    fn document_with_defaults(r_pr: RawRPr, p_pr: RawPPr) -> Document {
        Document {
            id: NodeId::ROOT,
            body: Body {
                id: NodeId::new(1),
                items: Vec::new(),
                sections: Vec::new(),
            },
            styles: StyleTable {
                doc_defaults: DocDefaults { r_pr, p_pr },
                ..StyleTable::default()
            },
            numbering: NumberingTable::default(),
            settings: Settings::default(),
            metadata: Metadata::default(),
            rels: Relationships::default(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            comments: Vec::new(),
            headers: std::collections::BTreeMap::new(),
            footers: std::collections::BTreeMap::new(),
            warnings: Vec::new(),
            media: Vec::new(),
        }
    }

    /// Документ без стилей: каскад отдаёт умолчание `docDefaults` — 12 pt без `w:line`.
    fn document() -> Document {
        document_with_defaults(RawRPr::default(), RawPPr::default())
    }

    /// Полоса набора шириной 600 px от левого края страницы.
    fn state() -> LayoutState {
        LayoutState {
            x: 0.0,
            y: 0.0,
            column_index: 0,
            column_width: 600.0,
            column_x: 0.0,
            content_top: 0.0,
            content_bottom: 1000.0,
        }
    }

    /// Абзац из одного run'а с текстом и заданным `w:pPr`.
    fn paragraph(text: &str, ppr: RawPPr) -> Paragraph {
        Paragraph {
            runs: vec![Inline::Run(Run {
                id: NodeId::new(2),
                content: vec![RunContent::Text(text.to_owned())],
                ..Run::default()
            })],
            ppr: Box::new(ppr),
            ..Paragraph::default()
        }
    }

    /// Разложить абзац в заданной полосе набора.
    fn layout(document: &Document, paragraph: &Paragraph, state: &LayoutState) -> ParagraphLayout {
        layout_with(document, paragraph, state, &mut FontRegistry::new(64))
    }

    /// Разложить абзац реестром с зарегистрированными семействами.
    fn layout_with(
        document: &Document,
        paragraph: &Paragraph,
        state: &LayoutState,
        fonts: &mut FontRegistry,
    ) -> ParagraphLayout {
        let mut line_breaker = LineBreaker::new(fonts, FontId::default());
        layout_paragraph(
            document,
            paragraph,
            state,
            &LayoutOptions::default(),
            &mut StyleCache::new(),
            &mut line_breaker,
        )
    }

    /// Полоса набора заданной ширины: ею проверяется перенос.
    fn state_with_width(column_width: f32) -> LayoutState {
        LayoutState {
            column_width,
            ..state()
        }
    }

    /// Абзац из прогонов: у каждого свой текст и свои `w:rPr`.
    fn paragraph_of_runs(runs: &[(&str, RawRPr)]) -> Paragraph {
        Paragraph {
            runs: runs
                .iter()
                .enumerate()
                .map(|(index, (text, r_pr))| {
                    Inline::Run(Run {
                        id: NodeId::new(index as u64 + 2),
                        content: vec![RunContent::Text((*text).to_owned())],
                        rpr: Box::new(r_pr.clone()),
                        ..Run::default()
                    })
                })
                .collect(),
            ..Paragraph::default()
        }
    }

    /// `w:rFonts` с одной гарнитурой.
    fn family(name: &str) -> RawRPr {
        RawRPr {
            r_fonts: Some(RFonts {
                ascii: Some(name.to_owned()),
                ..RFonts::default()
            }),
            ..RawRPr::default()
        }
    }

    /// Длинный текст: на 600 px он переносится, иначе проверить первую строку не на чем.
    const LONG_TEXT: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do \
                             eiusmod tempor incididunt ut labore et dolore magna aliqua ut enim";

    #[test]
    fn flags_and_spacing_come_from_the_cascade() {
        let ppr = RawPPr {
            keep_next: Some(Toggle::On),
            keep_lines: Some(Toggle::Off),
            page_break_before: Some(Toggle::On),
            widow_control: Some(Toggle::Inherit),
            spacing: Some(ParagraphSpacing {
                before: Some(Twips::new(240)),
                after: Some(Twips::new(120)),
                ..ParagraphSpacing::default()
            }),
            ..RawPPr::default()
        };

        let layout = layout(&document(), &paragraph("hello", ppr), &state());

        assert!(layout.keep_next, "keepNext включён");
        assert!(!layout.keep_lines, "keepLines выключен явно");
        assert!(layout.page_break_before, "pageBreakBefore включён");
        assert!(
            layout.widow_control,
            "widowControl включён в Word по умолчанию, а `inherit` его не снимает"
        );
        assert_px(layout.space_before, 16.0, "before 240 twips");
        assert_px(layout.space_after, 8.0, "after 120 twips");
        assert!(
            layout.anchors.is_empty(),
            "в текстовом абзаце плавающих рисунков нет"
        );
        assert_eq!(layout.lines.len(), 1, "короткий текст — одна строка");
        assert_eq!(layout.lines[0].text, "hello");
        assert_px(layout.lines[0].x, 0.0, "без отступов строка у левого края");
        assert_px(layout.lines[0].height, 19.53125, "12 pt без `w:line`");
        assert_eq!(layout.lines[0].color, None);
    }

    #[test]
    fn size_comes_from_the_document_defaults() {
        let document = document_with_defaults(
            RawRPr {
                sz: Some(HalfPoint::new(22)),
                ..RawRPr::default()
            },
            RawPPr::default(),
        );

        let layout = layout(&document, &paragraph("hello", RawPPr::default()), &state());

        // 11 pt = 14.667 px, метрика Carlito 2500/2048 em — 17.903646 px.
        assert_px(layout.lines[0].height, 17.903_646, "кегль docDefaults");
    }

    #[test]
    fn line_rule_sets_the_height() {
        // 12 pt даёт базу 19.53125 px: множитель 1.5 — 29.296875, абсолютные
        // правила — twips.
        let cases = [
            (LineSpacing::new(360), LineSpacingRule::Auto, 29.296_875),
            (LineSpacing::new(240), LineSpacingRule::Exact, 16.0),
            (LineSpacing::new(720), LineSpacingRule::AtLeast, 48.0),
        ];

        for (line, rule, expected) in cases {
            let ppr = RawPPr {
                spacing: Some(ParagraphSpacing {
                    line: Some(line),
                    line_rule: Some(rule),
                    ..ParagraphSpacing::default()
                }),
                ..RawPPr::default()
            };
            let layout = layout(&document(), &paragraph("hello", ppr), &state());
            assert_px(
                layout.lines[0].height,
                expected,
                "высота строки по `w:line`",
            );
        }
    }

    #[test]
    fn indents_narrow_the_band_and_shift_the_first_line() {
        let indented = RawPPr {
            ind: Some(Ind {
                left: Some(Twips::new(720)),
                right: Some(Twips::new(240)),
                ..Ind::default()
            }),
            ..RawPPr::default()
        };
        let layout = layout(&document(), &paragraph(LONG_TEXT, indented), &state());

        assert!(layout.lines.len() > 1, "длинный текст переносится");
        for line in &layout.lines {
            assert_px(line.x, 48.0, "отступ слева 720 twips");
            assert!(
                line.width <= 600.0 - 48.0 - 16.0 + 0.01,
                "строка шире полосы между отступами: {}",
                line.width
            );
        }
    }

    #[test]
    fn first_line_indent_moves_only_the_first_line() {
        let ppr = RawPPr {
            ind: Some(Ind {
                first_line: Some(Twips::new(720)),
                ..Ind::default()
            }),
            ..RawPPr::default()
        };
        let layout = layout(&document(), &paragraph(LONG_TEXT, ppr), &state());

        assert!(layout.lines.len() > 1, "длинный текст переносится");
        assert_px(layout.lines[0].x, 48.0, "первая строка со отступом");
        assert_px(layout.lines[1].x, 0.0, "последующие строки — без отступа");
    }

    #[test]
    fn hanging_indent_pulls_the_first_line_left() {
        let ppr = RawPPr {
            ind: Some(Ind {
                left: Some(Twips::new(1440)),
                hanging: Some(Twips::new(360)),
                ..Ind::default()
            }),
            ..RawPPr::default()
        };
        let layout = layout(&document(), &paragraph(LONG_TEXT, ppr), &state());

        assert!(layout.lines.len() > 1, "длинный текст переносится");
        assert_px(layout.lines[0].x, 96.0 - 24.0, "выступ первой строки");
        assert_px(
            layout.lines[1].x,
            96.0,
            "последующие строки — по отступу слева",
        );
    }

    #[test]
    fn alignment_shifts_the_line_inside_its_band() {
        let document = document();
        let jc = |value: Justification| RawPPr {
            jc: Some(value),
            ..RawPPr::default()
        };

        let left = layout(
            &document,
            &paragraph("short", jc(Justification::Left)),
            &state(),
        );
        let center = layout(
            &document,
            &paragraph("short", jc(Justification::Center)),
            &state(),
        );
        let right = layout(
            &document,
            &paragraph("short", jc(Justification::Right)),
            &state(),
        );
        let both = layout(
            &document,
            &paragraph("short", jc(Justification::Both)),
            &state(),
        );
        let width = left.lines[0].width;

        assert_px(left.lines[0].x, 0.0, "по левому краю");
        assert_px(center.lines[0].x, (600.0 - width) / 2.0, "по центру");
        assert_px(right.lines[0].x, 600.0 - width, "по правому краю");
        assert_px(both.lines[0].x, 0.0, "по ширине пока как по левому краю");
    }

    #[test]
    fn a_paragraph_without_text_takes_one_empty_line() {
        let defaults = document_with_defaults(
            RawRPr {
                sz: Some(HalfPoint::new(22)),
                ..RawRPr::default()
            },
            RawPPr::default(),
        );

        let layout = layout(&defaults, &paragraph("", RawPPr::default()), &state());

        assert_eq!(
            layout.lines.len(),
            1,
            "абзац без текста — одна пустая строка"
        );
        assert_eq!(layout.lines[0].text, "");
        assert_px(layout.lines[0].x, 0.0, "строка стоит у левого края полосы");
        assert_px(
            layout.lines[0].height,
            17.903_646,
            "высота пустой строки — из каскада, а не из умолчания движка",
        );
    }

    #[test]
    fn run_colour_reaches_the_line() {
        let paragraph = Paragraph {
            runs: vec![Inline::Run(Run {
                id: NodeId::new(2),
                content: vec![RunContent::Text("colored".to_owned())],
                rpr: Box::new(RawRPr {
                    color: Some(Color::Rgb(0x00FF_0000)),
                    ..RawRPr::default()
                }),
                ..Run::default()
            })],
            ..Paragraph::default()
        };

        let layout = layout(&document(), &paragraph, &state());

        assert_eq!(layout.lines[0].color, Some(0xFF_0000), "цвет run'а");
    }

    /// Абзац из одного run'а с заданными `w:rPr`: проверяются свойства знака,
    /// а не абзаца.
    fn styled_paragraph(text: &str, r_pr: RawRPr) -> Paragraph {
        Paragraph {
            runs: vec![Inline::Run(Run {
                id: NodeId::new(2),
                content: vec![RunContent::Text(text.to_owned())],
                rpr: Box::new(r_pr),
                ..Run::default()
            })],
            ..Paragraph::default()
        }
    }

    /// Стиль знака run'а доезжает до строки: гарнитура, курсив и подчёркивание.
    ///
    /// Кегль здесь — умолчание каскада (12 pt); кегль из `docDefaults` проверяет
    /// соседний тест, а `bold` — тест заголовка.
    #[test]
    fn run_style_reaches_the_line() {
        let paragraph = styled_paragraph(
            "styled",
            RawRPr {
                r_fonts: Some(RFonts {
                    ascii: Some("Georgia".to_owned()),
                    ..RFonts::default()
                }),
                i: Some(Toggle::On),
                u: Some(Underline::Single),
                ..RawRPr::default()
            },
        );

        let fallback = layout(&document(), &paragraph, &state());
        assert_eq!(
            fallback.lines[0].font_family.as_deref(),
            Some(doc_converter_render::font::DEFAULT_FONT_FAMILY),
            "гарнитуры нет в реестре — строка несёт имя запасного лица"
        );

        let mut fonts = FontRegistry::new(64);
        fonts
            .register_family(
                "Georgia",
                doc_converter_render::font::default_font_bytes().to_vec(),
            )
            .expect("Georgia регистрируется");
        let styled = layout_with(&document(), &paragraph, &state(), &mut fonts);
        let line = &styled.lines[0];

        assert_eq!(
            line.font_family.as_deref(),
            Some("Georgia"),
            "w:rFonts w:ascii"
        );
        assert!(line.italic, "w:i");
        assert!(line.underline, "w:u single");
        assert!(!line.bold, "w:b не задан");
        assert_eq!(line.size, HalfPoint::new(24), "12 pt по умолчанию");
        assert_px(
            half_points_to_px(line.size.value()),
            16.0,
            "кегль в пикселях",
        );
    }

    /// Гарнитура решает, где строке переноситься: одно и то же предложение в
    /// разных семействах ломается по-разному, и меряет его то самое лицо,
    /// которым строка названа.
    ///
    /// Байты для семейств берутся готовые — regular и bold Carlito: проверяется
    /// не конкретный шрифт, а то, что имя `w:rFonts` доводит до измерения и до кадра.
    #[test]
    fn family_decides_where_the_line_breaks() {
        let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor";
        let size = HalfPoint::new(24);
        let mut fonts = FontRegistry::new(1024);
        let narrow = fonts
            .register_family(
                "Arial",
                doc_converter_render::font::default_font_bytes().to_vec(),
            )
            .expect("Arial регистрируется");
        let wide = fonts
            .register_family(
                "Times New Roman",
                doc_converter_render::font::bold_font_bytes().to_vec(),
            )
            .expect("Times New Roman регистрируется");
        assert_ne!(narrow, wide, "семейства — разные лица реестра");

        // Полосу берём между ширинами одного и того же куска у двух лиц: узкому
        // он влезает, широкому — нет, и первые строки обязаны разойтись.
        // Половина предложения этого не даёт: разница в считаные проценты тонет
        // в запасе до границы соседнего слова, и перенос совпадает.
        let clause = "Lorem ipsum dolor sit amet,";
        let (narrow_clause, wide_clause) = {
            let mut breaker = LineBreaker::new(&mut fonts, FontId::default());
            (
                breaker.measure_text(clause, narrow, size),
                breaker.measure_text(clause, wide, size),
            )
        };
        assert!(
            (narrow_clause - wide_clause).abs() > 1.0,
            "тест вырождается, если лица меряют одинаково: {narrow_clause} и {wide_clause}"
        );

        let band = f32::midpoint(narrow_clause, wide_clause);
        let arial = layout_with(
            &document(),
            &paragraph_of_runs(&[(text, family("Arial"))]),
            &state_with_width(band),
            &mut fonts,
        );
        let times = layout_with(
            &document(),
            &paragraph_of_runs(&[(text, family("Times New Roman"))]),
            &state_with_width(band),
            &mut fonts,
        );

        assert_eq!(
            arial.lines[0].font_family.as_deref(),
            Some("Arial"),
            "строка названа семейством документа"
        );
        assert_eq!(
            times.lines[0].font_family.as_deref(),
            Some("Times New Roman"),
            "и второе — своим"
        );
        let arial_lines: Vec<&str> = arial.lines.iter().map(|line| line.text.as_str()).collect();
        let times_lines: Vec<&str> = times.lines.iter().map(|line| line.text.as_str()).collect();
        assert_ne!(
            arial_lines, times_lines,
            "разные лица — разные точки переноса"
        );
        assert!(
            times_lines.len() > 1,
            "предложение шире полосы — строка обязана переноситься"
        );
        // И мерена каждая строка своим лицом: ширина равна измерению тем
        // шрифтом, которым строка названа.
        let mut breaker = LineBreaker::new(&mut fonts, FontId::default());
        for (layout, font_id) in [(&arial, narrow), (&times, wide)] {
            for line in &layout.lines {
                assert_px(
                    line.width,
                    breaker.measure_text(&line.text, font_id, size),
                    "ширина строки мерена её лицом",
                );
            }
        }
    }

    /// Ширина строки — сумма ширин её частей: ровно столько места она займёт на
    /// картинке, и по этой ширине её выравнивают.
    #[test]
    fn line_width_is_the_sum_of_its_parts() {
        let mut fonts = FontRegistry::new(1024);
        fonts
            .register_family(
                "Times New Roman",
                doc_converter_render::font::bold_font_bytes().to_vec(),
            )
            .expect("Times New Roman регистрируется");

        let mut small = RawRPr {
            sz: Some(HalfPoint::new(22)),
            ..RawRPr::default()
        };
        small.r_fonts = None;
        let mut big = family("Times New Roman");
        big.sz = Some(HalfPoint::new(44));

        let paragraph = paragraph_of_runs(&[("Мелкий ", small), ("Крупный конец", big)]);
        let layout = layout_with(&document(), &paragraph, &state(), &mut fonts);
        assert_eq!(layout.lines.len(), 1, "оба прогона помещаются в полосу");

        let times = fonts.resolve_family(Some("Times New Roman"));
        let mut breaker = LineBreaker::new(&mut fonts, FontId::default());
        let expected = breaker.measure_text("Мелкий ", FontId::default(), HalfPoint::new(22))
            + breaker.measure_text("Крупный конец", times, HalfPoint::new(44));
        assert_px(layout.lines[0].width, expected, "ширина строки");
    }

    /// Кегль прогона (`w:sz`) влияет на перенос: крупное слово занимает больше
    /// места и выдавливает следующее на другую строку.
    #[test]
    fn run_size_decides_where_the_line_breaks() {
        // Прогон без `w:sz` получает кегль умолчания документа — 12 pt
        // (`DEFAULT_SIZE_HALF_POINTS`); им же надо мерить и полосу, иначе
        // «aaa bbb» в неё не влезет уже в раскладке.
        let default_size = HalfPoint::new(24);
        let band = {
            let mut fonts = FontRegistry::new(64);
            let mut breaker = LineBreaker::new(&mut fonts, FontId::default());
            breaker.measure_text("aaa bbb", FontId::default(), default_size)
        };

        let mut plain = FontRegistry::new(64);
        let default_layout = layout_with(
            &document(),
            &paragraph_of_runs(&[("aaa bbb ccc", RawRPr::default())]),
            &state_with_width(band),
            &mut plain,
        );
        assert_eq!(
            default_layout.lines.len(),
            2,
            "одним кеглем «aaa bbb» влезает целиком"
        );

        let sized = RawRPr {
            sz: Some(HalfPoint::new(44)),
            ..RawRPr::default()
        };
        let mut fonts = FontRegistry::new(64);
        let sized_layout = layout_with(
            &document(),
            &paragraph_of_runs(&[
                ("aaa ", RawRPr::default()),
                ("bbb", sized),
                (" ccc", RawRPr::default()),
            ]),
            &state_with_width(band),
            &mut fonts,
        );
        let texts: Vec<&str> = sized_layout
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(
            texts,
            ["aaa", "bbb", "ccc"],
            "крупный прогон не влезает в остаток строки"
        );
        assert_px(
            sized_layout.lines[1].height,
            half_points_to_px(44) * 2500.0 / 2048.0,
            "высота строки — по крупному прогону",
        );
    }

    /// Подчёркивание сводится к факту: `None` и явное `w:val="none"` — «нет».
    #[test]
    fn underline_value_is_reduced_to_a_flag() {
        let cases = [
            (None, false, "не задано"),
            (Some(Underline::None), false, "снято явно"),
            (Some(Underline::Single), true, "одинарное"),
            (Some(Underline::Wave), true, "волнистое"),
        ];

        for (u, expected, what) in cases {
            let paragraph = styled_paragraph(
                "underlined",
                RawRPr {
                    u: u.clone(),
                    ..RawRPr::default()
                },
            );
            let layout = layout(&document(), &paragraph, &state());

            assert_eq!(layout.lines[0].underline, expected, "w:u {what}");
        }
    }

    /// `formatting/heading_1.docx`: абзац целиком набран жирным стилем, все его
    /// run'ы жирные — единственный отрезок несёт начертание заголовка.
    #[test]
    fn fixture_heading_style_reaches_the_line() {
        let heading = layout_fixture("formatting/heading_1.docx", 0);
        let line = &heading.lines[0];

        assert!(line.bold, "жирный из стиля Heading1");
        assert!(!line.italic, "курсива в стиле нет");
        assert!(!line.underline, "подчёркивания в стиле нет");
        assert_eq!(line.size, HalfPoint::new(32), "16 pt из стиля");
        assert_px(
            half_points_to_px(line.size.value()),
            16.0 * 96.0 / 72.0,
            "кегль в пикселях",
        );
        assert_eq!(
            line.font_family.as_deref(),
            Some("Calibri"),
            "гарнитура docDefaults"
        );
    }

    /// `basic/multiple_runs.docx`: семь прогонов с разным начертанием, кеглем и
    /// цветом — один поток текста, и весь он помещается в полосу: строка одна,
    /// как в Word. Стиль знака строка берёт у первого прогона — долг: пока у
    /// строки нет спанов, начертание и цвет внутри неё не появятся.
    #[test]
    fn fixture_runs_of_one_paragraph_share_a_line() {
        let layout = layout_fixture("basic/multiple_runs.docx", 0);

        assert_eq!(
            layout.lines.len(),
            1,
            "семь прогонов абзаца — одна строка, а не четыре"
        );
        let line = &layout.lines[0];
        assert_eq!(
            line.text, "Plain bold italic underlined colored sized emphasized",
            "текст строки — все прогоны подряд"
        );
        assert!(!line.bold, "первый прогон строки обычный");
        assert!(
            !line.italic,
            "курсив второго прогона строка не подхватывает"
        );
        assert!(
            !line.underline,
            "подчёркивание третьего прогона строка не подхватывает"
        );
    }

    /// Путь к фикстуре от корня репозитория.
    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx")
            .join(name)
    }

    /// Разобрать фикстуру; её отсутствие роняет тест, а не пропускает его.
    fn fixture(name: &str) -> Document {
        let path = fixture_path(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|_| panic!("нет фикстуры {}", path.display()));
        crate::open(bytes).expect("фикстура разбирается")
    }

    /// Полоса набора из `w:sectPr` фикстуры: левое поле и ширина между полями.
    fn fixture_state(document: &Document) -> LayoutState {
        let section = document
            .body
            .sections
            .first()
            .expect("в фикстуре есть секция");
        let left = twips_to_px(section.margins.left);
        LayoutState {
            x: left,
            y: 0.0,
            column_index: 0,
            column_width: twips_to_px(section.page_size.width)
                - left
                - twips_to_px(section.margins.right),
            column_x: left,
            content_top: 0.0,
            content_bottom: 1000.0,
        }
    }

    /// Разложить абзац № `index` фикстуры её же полосой набора.
    fn layout_fixture(name: &str, index: usize) -> ParagraphLayout {
        let document = fixture(name);
        let paragraph = document
            .body
            .items
            .iter()
            .filter_map(|item| match item {
                BlockItem::Paragraph(paragraph) => Some(paragraph),
                _ => None,
            })
            .nth(index)
            .expect("абзац с таким номером есть в фикстуре");
        let state = fixture_state(&document);
        layout(&document, paragraph, &state)
    }

    /// `basic/indentation.docx`: отступы 720 twips = 48 px, 1440 = 96 px, выступ
    /// 360 twips = 24 px; левое поле 1134 twips = 75.6 px.
    #[test]
    fn fixture_indentation_places_the_lines() {
        let left = layout_fixture("basic/indentation.docx", 0);
        assert_px(left.lines[0].x, 123.6, "отступ слева 720 twips");

        let right = layout_fixture("basic/indentation.docx", 1);
        assert_px(
            right.lines[0].x,
            75.6,
            "отступ справа начало строки не сдвигает",
        );

        let first = layout_fixture("basic/indentation.docx", 2);
        assert_px(first.lines[0].x, 123.6, "отступ первой строки 720 twips");

        let hanging = layout_fixture("basic/indentation.docx", 3);
        assert_px(hanging.lines[0].x, 147.6, "отступ 1440 с выступом 360");
    }

    /// `basic/paragraph_spacing.docx`: `docDefaults` дают 11 pt (17.9 px) и
    /// `w:line="259"` (множитель 259/240); абсолютные правила — в twips.
    #[test]
    fn fixture_spacing_sets_heights_and_gaps() {
        let plain = layout_fixture("basic/paragraph_spacing.docx", 0);
        assert_px(plain.space_before, 16.0, "before 240 twips");
        assert_px(plain.space_after, 8.0, "after 120 twips");
        assert_px(
            plain.lines[0].height,
            half_points_to_px(22) * 2500.0 / 2048.0 * 259.0 / 240.0,
            "множитель docDefaults",
        );

        let auto = layout_fixture("basic/paragraph_spacing.docx", 1);
        assert_px(
            auto.lines[0].height,
            half_points_to_px(22) * 2500.0 / 2048.0 * 1.5,
            "line 360, lineRule auto",
        );

        let exact = layout_fixture("basic/paragraph_spacing.docx", 2);
        assert_px(exact.lines[0].height, 16.0, "line 240, lineRule exact");
        assert_px(exact.space_before, 0.0, "before 0");

        let at_least = layout_fixture("basic/paragraph_spacing.docx", 3);
        assert_px(at_least.lines[0].height, 48.0, "line 720, lineRule atLeast");
        assert_px(at_least.space_before, 24.0, "before 360 twips");
        assert_px(at_least.space_after, 24.0, "after 360 twips");
    }

    #[test]
    fn fixture_alignment_shifts_the_line() {
        let band = fixture_state(&fixture("basic/paragraph_alignment.docx")).column_width;
        let left = layout_fixture("basic/paragraph_alignment.docx", 0);
        let center = layout_fixture("basic/paragraph_alignment.docx", 1);
        let right = layout_fixture("basic/paragraph_alignment.docx", 2);
        let both = layout_fixture("basic/paragraph_alignment.docx", 3);

        assert_px(left.lines[0].x, 75.6, "по левому краю");
        assert_px(
            center.lines[0].x,
            75.6 + (band - center.lines[0].width) / 2.0,
            "по центру",
        );
        assert_px(
            right.lines[0].x,
            75.6 + band - right.lines[0].width,
            "по правому краю",
        );
        assert_px(both.lines[0].x, 75.6, "по ширине пока как по левому краю");
    }

    /// `formatting/heading_1.docx`: стиль `Heading1` даёт 16 pt (`w:sz w:val="32"`),
    /// а множитель интерлиньяжа 259/240 приходит из `docDefaults`.
    #[test]
    fn fixture_heading_uses_the_style_size() {
        let heading = layout_fixture("formatting/heading_1.docx", 0);
        let expected = 16.0 * 96.0 / 72.0 * 2500.0 / 2048.0 * 259.0 / 240.0;

        assert_px(heading.lines[0].height, expected, "кегль стиля Heading1");
        assert!(
            heading.lines[0].height > 19.53125,
            "заголовок выше строки обычных 12 pt"
        );
        assert_eq!(
            heading.lines[0].color,
            Some(0x2F_5496),
            "цвет стиля Heading1"
        );
        assert!(heading.keep_next, "keepNext из стиля");
        assert!(heading.keep_lines, "keepLines из стиля");
        assert_px(heading.space_before, 16.0, "before 240 twips из стиля");
    }

    /// `basic/multiple_runs.docx`: у прогонов разный кегль и цвет, а строка одна —
    /// её метрики берутся у прогона, который её открыл (цвет и кегль docDefaults),
    /// а высота — по самому крупному прогону (`w:sz 32`), как считает строку Word.
    #[test]
    fn fixture_run_metrics_reach_the_line() {
        let layout = layout_fixture("basic/multiple_runs.docx", 0);
        let line = &layout.lines[0];

        assert_eq!(
            line.color, None,
            "цвет строки — у первого прогона, `w:color` нет"
        );
        assert_eq!(
            line.size,
            HalfPoint::new(22),
            "кегль строки — у первого прогона, 11 pt из docDefaults"
        );
        assert_px(
            line.height,
            16.0 * 96.0 / 72.0 * 2500.0 / 2048.0 * 259.0 / 240.0,
            "высота — по прогону `w:sz 32`, а не по первому",
        );
    }

    /// Плавающий рисунок с заданным номером узла: раскладке анкоров важен только он.
    fn anchor(id: u64) -> Anchor {
        let extent = Extent {
            cx: 914_400,
            cy: 457_200,
        };
        Anchor {
            id: NodeId::new(id),
            extent: extent.clone(),
            horizontal: PositionH {
                relative_from: RelFromH::Column,
                align: None,
                offset: Some(0),
                percent: None,
            },
            vertical: PositionV {
                relative_from: RelFromV::Paragraph,
                align: None,
                offset: Some(0),
                percent: None,
            },
            wrap: WrapKind::Square,
            behind_text: false,
            image: InlineImage {
                id: NodeId::new(id),
                rel_id: format!("rId{id}"),
                part: None,
                name: None,
                description: None,
                extent,
            },
        }
    }

    /// `InlineOrAnchor` с плавающим рисунком.
    fn anchored(id: u64) -> InlineOrAnchor {
        InlineOrAnchor {
            id: NodeId::new(id),
            inline: None,
            anchor: Some(anchor(id)),
        }
    }

    /// `w:drawing` с плавающим рисунком на верхнем уровне абзаца или в гиперссылке.
    fn anchored_drawing(id: u64) -> Inline {
        Inline::Drawing(anchored(id))
    }

    /// `w:drawing` со встроенной картинкой: анкора у неё нет.
    fn inline_drawing(id: u64) -> RunContent {
        let extent = Extent {
            cx: 914_400,
            cy: 685_800,
        };
        RunContent::Drawing(InlineOrAnchor {
            id: NodeId::new(id),
            inline: Some(InlineImage {
                id: NodeId::new(id),
                rel_id: format!("rId{id}"),
                part: None,
                name: None,
                description: None,
                extent,
            }),
            anchor: None,
        })
    }

    /// Run из одного рисунка-анкора.
    fn anchored_run(id: u64) -> Inline {
        Inline::Run(Run {
            id: NodeId::new(id),
            content: vec![RunContent::Drawing(anchored(id))],
            ..Run::default()
        })
    }

    /// `w:drawing` лежит и в run'е, и в гиперссылке: анкоры собираются обходом обоих
    /// уровней и в порядке появления, а встроенная картинка в список не попадает.
    #[test]
    fn anchors_follow_inline_order_across_nesting() {
        let paragraph = Paragraph {
            runs: vec![
                anchored_run(2),
                Inline::Hyperlink(Hyperlink {
                    id: NodeId::new(3),
                    runs: vec![anchored_drawing(4), anchored_run(5)],
                    ..Hyperlink::default()
                }),
                anchored_drawing(6),
                Inline::Run(Run {
                    id: NodeId::new(7),
                    content: vec![inline_drawing(8)],
                    ..Run::default()
                }),
            ],
            ..Paragraph::default()
        };

        let layout = layout(&document(), &paragraph, &state());
        let ids: Vec<u64> = layout.anchors.iter().map(|item| item.id.value()).collect();

        assert_eq!(ids, [2, 4, 5, 6], "плавающие рисунки по порядку обхода");
    }

    /// Сайдекары `images/*.json` дают размеры в EMU; 914400 EMU = 96 px.
    #[test]
    fn fixture_anchored_drawings_keep_their_sidecar_metrics() {
        let cases = [
            (
                "images/anchor_wrap_square.docx",
                120.0,
                90.0,
                WrapKind::Square,
                false,
            ),
            (
                "images/anchor_behind_text.docx",
                144.0,
                96.0,
                WrapKind::None,
                true,
            ),
            (
                "images/anchor_top_and_bottom.docx",
                288.0,
                72.0,
                WrapKind::TopAndBottom,
                false,
            ),
            (
                "images/anchor_wrap_tight.docx",
                72.0,
                72.0,
                WrapKind::Tight,
                false,
            ),
        ];

        for (name, width, height, wrap, behind) in cases {
            let layout = layout_fixture(name, 0);
            assert_eq!(layout.anchors.len(), 1, "{name}: один плавающий рисунок");
            let anchor = &layout.anchors[0];
            assert_px(emu_to_px(anchor.extent.cx), width, name);
            assert_px(emu_to_px(anchor.extent.cy), height, name);
            assert_eq!(anchor.wrap, wrap, "{name}: обтекание");
            assert_eq!(anchor.behind_text, behind, "{name}: под текстом");
            assert!(
                !anchor.image.rel_id.is_empty(),
                "{name}: ссылка на картинку"
            );
        }
    }

    /// Встроенные рисунки остаются в потоке текста и анкорами не становятся.
    #[test]
    fn fixture_inline_drawings_are_not_anchors() {
        for name in ["images/inline.docx", "images/multiple_sizes.docx"] {
            let document = fixture(name);
            let state = fixture_state(&document);
            let anchors: Vec<Anchor> = document
                .body
                .items
                .iter()
                .filter_map(|item| match item {
                    BlockItem::Paragraph(paragraph) => Some(paragraph),
                    _ => None,
                })
                .flat_map(|paragraph| layout(&document, paragraph, &state).anchors)
                .collect();

            assert!(anchors.is_empty(), "{name}: встроенные рисунки не плавают");
        }
    }

    /// Строки абзаца собираются из прогонов, а не режутся по одному на прогон:
    /// семь прогонов `basic/multiple_runs.docx` дают одну строку, а сотня
    /// прогонов `basic/many_runs_paragraph.docx` — семь строк, а не сто.
    #[test]
    fn fixture_runs_share_lines_within_the_page() {
        for (name, lines) in [
            ("basic/multiple_runs.docx", 1usize),
            ("basic/many_runs_paragraph.docx", 7),
        ] {
            let document = fixture(name);
            let layout = crate::layout::layout_document(
                &document,
                &LayoutOptions::default(),
                &mut FontRegistry::new(64),
            )
            .expect("раскладка не должна падать");
            let texts: Vec<&str> = layout
                .pages
                .iter()
                .flat_map(|page| page.items.iter())
                .filter_map(|item| match item {
                    crate::layout::LayoutItem::Paragraph { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(texts.len(), lines, "{name}: строк в раскладке документа");
        }
    }
}
