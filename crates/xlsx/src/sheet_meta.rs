//! Всё, что лист говорит о себе помимо ячеек: вид окна, закреплённые области,
//! объединённые ячейки, гиперссылки.
//!
//! Это метаданные уровня листа, а не книги (`workbook.xml`): они лежат в той же
//! части `sheetN.xml`, что и ячейки, и разбираются тем же проходом.

use doc_converter_core::rels::RelMap;

use crate::cellref::{CellRef, Range};
use crate::error::{Result, XlsxError};
use crate::xml::{find, is_true, Attr};

/// Режим закрепления областей (`state` у `<pane>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneState {
    /// Границы областей заданы в двадцатых долях пункта: пользователь двигал
    /// разделители мышью, а не выбирал «закрепить».
    Split,
    /// Области закреплены по числу строк и столбцов.
    #[default]
    Frozen,
    /// То же, что [`Self::Frozen`], но разделители можно двигать.
    FrozenSplit,
}

impl PaneState {
    /// Режим по значению `state`; незнакомое значение считается закреплением.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "split" => Self::Split,
            "frozenSplit" => Self::FrozenSplit,
            _ => Self::Frozen,
        }
    }
}

/// Активная область окна (`activePane`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneKind {
    /// Верхняя левая — она же активна, когда закреплений нет.
    #[default]
    TopLeft,
    /// Верхняя правая (закреплены столбцы).
    TopRight,
    /// Нижняя левая (закреплены строки).
    BottomLeft,
    /// Нижняя правая (закреплены и строки, и столбцы).
    BottomRight,
}

impl PaneKind {
    /// Область по значению `activePane`; незнакомое значение — верхняя левая.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "topRight" => Self::TopRight,
            "bottomLeft" => Self::BottomLeft,
            "bottomRight" => Self::BottomRight,
            _ => Self::TopLeft,
        }
    }
}

/// Закреплённые области (`<pane>`).
///
/// Разделение окна мышью (`state="split"`) в модели не разворачивается: там
/// границы заданы в двадцатых долях пункта и зависят от размера окна, а не от
/// структуры листа. Такой `<pane>` сохраняется как есть, но закреплений не
/// даёт — [`Self::cols`] и [`Self::rows`] у него нулевые.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pane {
    /// Число закреплённых столбцов слева; 0 — не закреплены.
    pub cols: u32,
    /// Число закреплённых строк сверху; 0 — не закреплены.
    pub rows: u32,
    /// Режим закрепления.
    pub state: PaneState,
    /// Активная область.
    pub active: PaneKind,
    /// Первая ячейка прокручиваемой области (`topLeftCell`). Excel пишет её
    /// при сохранении, но на геометрию закрепления она не влияет.
    pub top_left: Option<CellRef>,
}

impl Pane {
    /// Есть ли что закреплять.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.cols == 0 && self.rows == 0
    }
}

/// Вид листа (`<sheetView>`).
///
/// У книги бывает несколько видов — по одному на окно Excel, — но лист при этом
/// один. Берём первый: остальные описывают то же содержимое.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SheetView {
    /// Показывать сетку (`showGridLines`).
    pub show_grid_lines: bool,
    /// Показывать заголовки строк и столбцов (`showRowColHeaders`).
    pub show_headers: bool,
    /// Масштаб в процентах (`zoomScale`).
    pub zoom: u32,
    /// Лист отображается справа налево (`rightToLeft`).
    pub right_to_left: bool,
    /// Закреплённые области.
    pub pane: Option<Pane>,
}

impl Default for SheetView {
    fn default() -> Self {
        Self {
            show_grid_lines: true,
            show_headers: true,
            zoom: 100,
            right_to_left: false,
            pane: None,
        }
    }
}

/// Объединённые ячейки листа (`<mergeCells>`).
///
/// Диапазоны хранятся как записаны: Excel пишет их нормализованными, но
/// `A1:B2` и `B2:A1` — один и тот же прямоугольник, и полагаться на порядок
/// границ в чужом файле не стоит. Приведение — [`Self::ranges`] с
/// [`Range::normalized`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Merges {
    ranges: Vec<Range>,
}

impl Merges {
    /// Диапазоны в порядке из файла, без нормализации.
    #[must_use]
    pub fn raw(&self) -> &[Range] {
        &self.ranges
    }

    /// Нормализованные диапазоны (`first <= last` покомпонентно).
    pub fn ranges(&self) -> impl Iterator<Item = Range> + '_ {
        self.ranges.iter().copied().map(Range::normalized)
    }

    /// Число объединений.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Объединений нет.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Объединение, накрывающее ячейку.
    #[must_use]
    pub fn covering(&self, cell: CellRef) -> Option<Range> {
        self.ranges().find(|range| range.contains(cell))
    }

    /// Объединение, для которого эта ячейка — левая верхняя.
    ///
    /// Только она хранит значение: остальные ячейки диапазона пусты, и рисовать
    /// их отдельно не нужно.
    #[must_use]
    pub fn anchored_at(&self, cell: CellRef) -> Option<Range> {
        self.ranges().find(|range| range.first == cell)
    }

    /// Добавить диапазон.
    pub fn push(&mut self, range: Range) {
        self.ranges.push(range);
    }
}

/// Куда ведёт гиперссылка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyperlinkTarget {
    /// Внешний адрес: URL, почта, путь к файлу. Берётся из `_rels` листа.
    External(String),
    /// Переход внутри книги: `Лист2!A1`, определённое имя, часть пакета.
    Internal(String),
    /// Ссылка объявлена, но цель не разрешилась; внутри — неразрешённый `r:id`.
    Broken(String),
}

impl HyperlinkTarget {
    /// Адрес, по которому пойдёт переход; `None` — цель не разрешилась.
    #[must_use]
    pub fn address(&self) -> Option<&str> {
        match self {
            Self::External(address) | Self::Internal(address) => Some(address),
            Self::Broken(_) => None,
        }
    }
}

/// Гиперссылка листа (`<hyperlink>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyperlink {
    /// Ячейка или диапазон, на котором лежит ссылка.
    pub range: Range,
    /// Куда ведёт.
    pub target: HyperlinkTarget,
    /// Подпись, которую показывает Excel (`display`).
    pub display: Option<String>,
    /// Всплывающая подсказка (`tooltip`).
    pub tooltip: Option<String>,
}

/// Прочитать атрибуты `<sheetView>` в уже собранный вид.
///
/// Вид собирается по частям: атрибуты приходят на открывающем теге, а `<pane>`
/// вложен внутрь и обрабатывается отдельным событием.
pub(crate) fn read_view(view: &mut SheetView, attrs: &[Attr<'_>]) {
    view.show_grid_lines = !find(attrs, "showGridLines").is_some_and(is_false);
    view.show_headers = !find(attrs, "showRowColHeaders").is_some_and(is_false);
    view.right_to_left = find(attrs, "rightToLeft").is_some_and(is_true);
    if let Some(zoom) = find(attrs, "zoomScale").and_then(|raw| raw.trim().parse::<u32>().ok()) {
        // Excel ограничивает масштаб 10…400 %; всё прочее — порча файла.
        view.zoom = zoom.clamp(10, 400);
    }
}

/// Разобрать `<pane>`.
pub(crate) fn read_pane(attrs: &[Attr<'_>], part: &str) -> Result<Pane> {
    let state = find(attrs, "state").map_or(PaneState::Frozen, PaneState::parse);
    let split = |name: &str| {
        find(attrs, name)
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
    };

    // При `split` те же атрибуты значат двадцатые доли пункта — в столбцы и
    // строки они не переводятся без размера окна.
    let (cols, rows) = if state == PaneState::Split {
        (0, 0)
    } else {
        (to_count(split("xSplit")), to_count(split("ySplit")))
    };

    let top_left = match find(attrs, "topLeftCell") {
        Some(raw) => Some(CellRef::parse(raw.trim()).map_err(|e| {
            XlsxError::malformed(part, format!("pane topLeftCell `{raw}` is not a cell: {e}"))
        })?),
        None => None,
    };

    Ok(Pane {
        cols,
        rows,
        state,
        active: find(attrs, "activePane").map_or(PaneKind::TopLeft, PaneKind::parse),
        top_left,
    })
}

/// Разобрать `<mergeCell ref="A1:B2"/>`.
///
/// # Errors
///
/// [`XlsxError::Malformed`] — нет атрибута `ref` или он не разбирается.
pub(crate) fn read_merge(attrs: &[Attr<'_>], part: &str) -> Result<Range> {
    match find(attrs, "ref") {
        Some(raw) => Range::parse_ref(raw.trim()).map_err(|e| {
            XlsxError::malformed(part, format!("mergeCell ref `{raw}` is not a range: {e}"))
        }),
        None => Err(XlsxError::malformed(part, "<mergeCell> without ref")),
    }
}

/// Разобрать `<hyperlink>`.
///
/// Внешняя цель лежит не в самом элементе, а в `_rels` листа, куда ведёт
/// `r:id`. Если карты связей нет или идентификатор в ней не нашёлся, ссылка
/// остаётся [`HyperlinkTarget::Broken`]: рисовать её как обычный текст можно,
/// а вести по ней — нельзя, и делать вид, что цель известна, нечестно.
///
/// # Errors
///
/// [`XlsxError::Malformed`] — нет атрибута `ref` или он не разбирается.
pub(crate) fn read_hyperlink(
    attrs: &[Attr<'_>],
    rels: Option<&RelMap>,
    part: &str,
) -> Result<Hyperlink> {
    let Some(raw) = find(attrs, "ref") else {
        return Err(XlsxError::malformed(part, "<hyperlink> without ref"));
    };
    let range = Range::parse_ref(raw.trim()).map_err(|e| {
        XlsxError::malformed(part, format!("hyperlink ref `{raw}` is not a range: {e}"))
    })?;

    let location = find(attrs, "location").map(trim_fragment);
    let target = match find(attrs, "id").and_then(|id| rels.and_then(|map| map.get(id))) {
        // Переход внутрь книги Excel пишет и как `location`, и как внешнюю
        // связь с фрагментом (`Target="#Лист2!A1"`). Второе — не адрес в
        // интернете: отдать такое браузеру значит увести пользователя в никуда.
        Some(rel) if rel.target.starts_with('#') => {
            HyperlinkTarget::Internal(trim_fragment(&rel.target))
        }
        Some(rel) if rel.target_mode.as_deref() == Some("External") => {
            HyperlinkTarget::External(rel.target.clone())
        }
        // Связь без `External` ведёт на часть пакета — для листа это внутренний
        // переход, а не адрес в интернете.
        Some(rel) => HyperlinkTarget::Internal(rel.target.clone()),
        None => match (&location, find(attrs, "id")) {
            (Some(location), _) => HyperlinkTarget::Internal(location.clone()),
            (None, Some(id)) => HyperlinkTarget::Broken(id.to_owned()),
            (None, None) => return Err(XlsxError::malformed(part, "<hyperlink> without target")),
        },
    };

    Ok(Hyperlink {
        range,
        target,
        display: find(attrs, "display").map(str::to_owned),
        tooltip: find(attrs, "tooltip").map(str::to_owned),
    })
}

/// Убрать ведущий `#`: так Excel помечает переход внутри книги.
fn trim_fragment(target: &str) -> String {
    target.strip_prefix('#').unwrap_or(target).to_owned()
}

/// Флаг `xsd:boolean`, записанный как ложь.
fn is_false(value: &str) -> bool {
    matches!(value, "0" | "false")
}

/// Разделитель областей в число строк или столбцов.
///
/// В файле это `xsd:double`, но при закреплении Excel пишет целое; дробное
/// значение означает уже не закрепление, а разделение окна.
fn to_count(split: Option<f64>) -> u32 {
    let Some(value) = split else {
        return 0;
    };
    // Значения заведомо малы (максимум — размер листа), поэтому дробная часть
    // здесь не важна: важно не «уехать» в ноль у значения вида 0.5.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = value.round() as u32;
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cellref::CellRef;
    use crate::xml::attributes;
    use quick_xml::events::BytesStart;

    const PART: &str = "xl/worksheets/sheet1.xml";

    /// Атрибуты элемента, записанного строкой; элемент утекает — тест живёт
    /// одну проверку, а `attributes` одалживает значения у него.
    fn attrs(xml: &'static str) -> Vec<Attr<'static>> {
        let start: &'static BytesStart<'static> =
            Box::leak(Box::new(BytesStart::from_content(xml, 0)));
        attributes(start, PART)
            .unwrap_or_else(|e| panic!("attributes: {e}"))
            .into_vec()
    }

    fn pane(xml: &'static str) -> Pane {
        read_pane(&attrs(xml), PART).unwrap()
    }

    fn merge(xml: &'static str) -> Range {
        read_merge(&attrs(xml), PART).unwrap()
    }

    fn hyperlink(xml: &'static str, rels: Option<&RelMap>) -> Hyperlink {
        read_hyperlink(&attrs(xml), rels, PART).unwrap()
    }

    fn rels() -> RelMap {
        RelMap::parse(
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                  <Relationship Id="rId1" Type="http://x/hyperlink"
                                Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/>
                  <Relationship Id="rId2" Type="http://x/hyperlink" Target="../other.xml"/>
                </Relationships>"#,
        )
        .unwrap()
    }

    #[test]
    fn frozen_pane_counts_rows_and_columns() {
        let pane = pane(
            r#"xSplit="2" ySplit="1" topLeftCell="C2" activePane="bottomRight" state="frozen""#,
        );

        assert_eq!(pane.cols, 2);
        assert_eq!(pane.rows, 1);
        assert_eq!(pane.state, PaneState::Frozen);
        assert_eq!(pane.active, PaneKind::BottomRight);
        assert_eq!(pane.top_left, Some(CellRef::new(1, 2)));
        assert!(!pane.is_empty());
    }

    #[test]
    fn split_pane_is_kept_but_gives_no_freeze() {
        // При `split` те же числа — двадцатые доли пункта, а не счётчики.
        let pane = pane(r#"xSplit="2000" ySplit="1000" state="split""#);

        assert_eq!(pane.state, PaneState::Split);
        assert_eq!(pane.cols, 0);
        assert_eq!(pane.rows, 0);
        assert!(pane.is_empty());
    }

    #[test]
    fn pane_defaults_to_freezing_the_first_row() {
        // Так Excel пишет «закрепить верхнюю строку»: без `state`, с одним ySplit.
        let pane = pane(r#"ySplit="1" topLeftCell="A2" activePane="bottomLeft""#);

        assert_eq!(pane.rows, 1);
        assert_eq!(pane.cols, 0);
        assert_eq!(pane.state, PaneState::Frozen);
        assert_eq!(pane.active, PaneKind::BottomLeft);
    }

    #[test]
    fn broken_pane_cell_is_malformed() {
        let err = read_pane(&attrs(r#"topLeftCell="не ячейка""#), PART).unwrap_err();
        assert!(matches!(err, XlsxError::Malformed { .. }));
    }

    #[test]
    fn view_attributes_are_read_from_the_element() {
        let mut view = SheetView::default();
        read_view(
            &mut view,
            &attrs(
                r#"showGridLines="0" showRowColHeaders="0" zoomScale="85" rightToLeft="1" tabSelected="1""#,
            ),
        );

        assert!(!view.show_grid_lines);
        assert!(!view.show_headers);
        assert_eq!(view.zoom, 85);
        assert!(view.right_to_left);

        // Пустой `<sheetView/>` оставляет значения по умолчанию.
        let mut plain = SheetView::default();
        read_view(&mut plain, &attrs(""));
        assert_eq!(plain, SheetView::default());

        // Масштаб за пределами 10…400 % обрезается, а не ломает вид.
        let mut clamped = SheetView::default();
        read_view(&mut clamped, &attrs(r#"zoomScale="9000""#));
        assert_eq!(clamped.zoom, 400);
    }

    #[test]
    fn merges_are_looked_up_normalized() {
        let mut merges = Merges::default();
        merges.push(merge(r#"ref="B2:C3""#));
        // Обратный порядок границ — тот же прямоугольник.
        merges.push(merge(r#"ref="D5:A4""#));

        assert_eq!(merges.len(), 2);
        assert_eq!(merges.raw()[1].first, CellRef::new(4, 3));

        let covering = merges.covering(CellRef::new(2, 2)).unwrap();
        assert_eq!(covering.first, CellRef::new(1, 1));
        assert_eq!(merges.covering(CellRef::new(0, 0)), None);
        // Ячейка внутри диапазона с обратными границами тоже находится.
        assert!(merges.covering(CellRef::new(4, 1)).is_some());

        // Значение хранит только левая верхняя ячейка.
        assert!(merges.anchored_at(CellRef::new(1, 1)).is_some());
        assert!(merges.anchored_at(CellRef::new(2, 1)).is_none());
        assert!(Merges::default().is_empty());
    }

    #[test]
    fn single_cell_merge_is_legal() {
        let single = merge(r#"ref="A1""#);
        assert_eq!(single.first, single.last);
    }

    #[test]
    fn merge_without_ref_is_malformed() {
        let err = read_merge(&attrs(r#"count="1""#), PART).unwrap_err();
        assert!(err.to_string().contains("without ref"));
    }

    #[test]
    fn external_hyperlink_resolves_through_rels() {
        let link = hyperlink(r#"ref="A1" r:id="rId1""#, Some(&rels()));

        assert_eq!(
            link.target,
            HyperlinkTarget::External("https://example.com/?a=1&b=2".into())
        );
        assert_eq!(link.target.address(), Some("https://example.com/?a=1&b=2"));
        assert_eq!(link.range.first, CellRef::new(0, 0));

        // Связь без `External` — не адрес в интернете, а часть пакета.
        let internal = hyperlink(r#"ref="A1" r:id="rId2""#, Some(&rels()));
        assert_eq!(
            internal.target,
            HyperlinkTarget::Internal("../other.xml".into())
        );
    }

    #[test]
    fn fragment_relationship_is_an_internal_jump() {
        // Так Excel пишет переход на другой лист: внешняя связь с `#` в начале.
        // `#` внутри значения закрывает `r#"…"#`, поэтому решёток здесь две.
        let rels = RelMap::parse(
            r##"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                  <Relationship Id="rId1" Type="http://x/hyperlink"
                                Target="#Лист2!A1" TargetMode="External"/>
                </Relationships>"##
                .as_bytes(),
        )
        .unwrap();
        let link = hyperlink(r#"ref="A1" r:id="rId1""#, Some(&rels));

        assert_eq!(link.target, HyperlinkTarget::Internal("Лист2!A1".into()));
        assert_eq!(link.target.address(), Some("Лист2!A1"));
    }

    #[test]
    fn internal_and_broken_targets() {
        let jump = hyperlink(
            r#"ref="B2:C3" location="Лист2!A1" display="туда" tooltip="подсказка""#,
            None,
        );
        assert_eq!(jump.target, HyperlinkTarget::Internal("Лист2!A1".into()));
        assert_eq!(jump.display.as_deref(), Some("туда"));
        assert_eq!(jump.tooltip.as_deref(), Some("подсказка"));
        assert_eq!(jump.range.last, CellRef::new(2, 2));

        // `r:id` есть, а карты связей нет — цель неизвестна, но она сохранена.
        let broken = hyperlink(r#"ref="A1" r:id="rId9""#, None);
        assert_eq!(broken.target, HyperlinkTarget::Broken("rId9".into()));
        assert_eq!(broken.target.address(), None);

        // Идентификатор, которого нет в карте, — тоже неизвестная цель.
        let missing = hyperlink(r#"ref="A1" r:id="rId9""#, Some(&rels()));
        assert_eq!(missing.target, HyperlinkTarget::Broken("rId9".into()));
    }

    #[test]
    fn hyperlink_without_any_target_is_malformed() {
        let err = read_hyperlink(&attrs(r#"ref="A1""#), None, PART).unwrap_err();
        assert!(err.to_string().contains("without target"));
    }
}
