//! `TextLayer` — позиционированный текст как отдельный выход раскладки.
//!
//! Источник геометрии — [`PageLayout`]: `bbox` и `line_rect` собираются из тех
//! же координат, что уходят в `DisplayList`, и ничего не перемеряют. `DisplayList`
//! при этом не меняется: `TextLayer` живёт параллельно кадру и не требует
//! `DL_VERSION`.
//!
//! Порядок `reading_order` — логический (для AT и копирования). Визуальный
//! порядок RTL-строки обеспечивает браузер: DOM не переупорядочивается, а
//! `bidi_level`/`direction` несут уровни UAX #9.
//!
//! # Пример
//!
//! ```ignore
//! use doc_converter_docx::layout::layout_document;
//! use doc_converter_docx::text_layer::text_layer;
//!
//! let layout = layout_document(&document, &options)?;
//! let layer = text_layer(&layout);
//! assert_eq!(layer.page_map.len(), layer.runs.iter().map(|r| r.page).max().map_or(0, |p| p as usize + 1));
//! ```

use doc_converter_core::wire::{
    BBoxWire, BlockKindWire, PageRunsWire, TableContextWire, TextAlignWire, TextDirectionWire,
    TextLayerWire, TextRunWire, TextStyleWire,
};
use unicode_bidi::BidiInfo;

use crate::layout::bidi;
use crate::layout::engine::half_points_to_px;
use crate::layout::paragraph::{LineAlign, LineFragment};
use crate::layout::{LayoutItem, Page, PageLayout, Rect, TableCellLayout};

/// Собрать `TextLayer` всего документа.
#[must_use]
pub fn text_layer(layout: &PageLayout) -> TextLayerWire {
    let mut builder = Builder::default();
    let mut page_map = Vec::with_capacity(layout.pages.len());
    for (index, page) in layout.pages.iter().enumerate() {
        page_map.push(builder.collect_page(index, page));
    }
    TextLayerWire {
        runs: builder.runs,
        page_map,
    }
}

/// Собрать `TextLayer` одной страницы (0-based).
///
/// `reading_order` остаётся сквозным: перед целевой страницей run'ы
/// подсчитываются без аллокаций, поэтому нумерация совпадает с
/// [`text_layer`].
#[must_use]
pub fn text_layer_page(layout: &PageLayout, page: u32) -> Option<TextLayerWire> {
    let index = usize::try_from(page).ok()?;
    if index >= layout.pages.len() {
        return None;
    }
    let start: u32 = layout.pages[..index].iter().map(page_run_count).sum();
    let mut builder = Builder {
        order: start,
        ..Builder::default()
    };
    let page_runs = builder.collect_page(index, &layout.pages[index]);
    Some(TextLayerWire {
        runs: builder.runs,
        page_map: vec![page_runs],
    })
}

/// Сколько run'ов даст страница — для сквозного `reading_order`.
#[must_use]
pub fn page_run_count(page: &Page) -> u32 {
    page.items.iter().map(item_run_count).sum()
}

fn item_run_count(item: &LayoutItem) -> u32 {
    match item {
        LayoutItem::Paragraph {
            text, fragments, ..
        } => u32::try_from(if fragments.is_empty() && !text.is_empty() {
            1
        } else {
            fragments.len()
        })
        .unwrap_or(u32::MAX),
        LayoutItem::Table { cells, .. } => cells
            .iter()
            .filter(|cell| !cell.v_merge_continue)
            .flat_map(|cell| cell.content.iter().map(item_run_count))
            .sum(),
        _ => 0,
    }
}

#[derive(Default)]
struct Builder {
    runs: Vec<TextRunWire>,
    order: u32,
}

impl Builder {
    /// Собрать страницу и вернуть её срез в общем списке runs.
    fn collect_page(&mut self, page_index: usize, page: &Page) -> PageRunsWire {
        let start = u32::try_from(self.runs.len()).unwrap_or(u32::MAX);
        let mut line_no = 0u32;
        for item in &page.items {
            self.collect_item(page_index, item, None, &mut line_no);
        }
        let count = u32::try_from(self.runs.len())
            .unwrap_or(u32::MAX)
            .saturating_sub(start);
        PageRunsWire {
            page: u32::try_from(page_index).unwrap_or(u32::MAX),
            start,
            count,
        }
    }

    // Один проход по элементам раскладки: индексы страниц и строк считаются
    // в локальном контексте обхода; деление ради счётчика строк размыло бы их.
    #[allow(clippy::too_many_lines)]
    fn collect_item(
        &mut self,
        page_index: usize,
        item: &LayoutItem,
        table: Option<TableContextWire>,
        line_no: &mut u32,
    ) {
        match item {
            LayoutItem::Paragraph {
                node_id,
                rect,
                text,
                line_height,
                color,
                font_family,
                size,
                bold,
                italic,
                underline,
                fragments,
                align,
                rtl,
                heading_level,
            } => {
                if text.is_empty() {
                    return;
                }
                let line_index = *line_no;
                *line_no = line_no.saturating_add(1);
                let fallback = LineFragment {
                    range: 0..text.len(),
                    x: 0.0,
                    width: rect.width,
                    font_family: font_family.clone(),
                    size: *size,
                    bold: *bold,
                    italic: *italic,
                    underline: *underline,
                    color: *color,
                    highlight: None,
                    // Строка без фрагментов — стиль первого прогона: модельного
                    // адреса у неё нет.
                    paragraph_index: None,
                    run_index: None,
                };
                let fragments = if fragments.is_empty() {
                    std::slice::from_ref(&fallback)
                } else {
                    fragments.as_slice()
                };
                let bidi = bidi::levels(text, *rtl);
                let direction = if *rtl {
                    TextDirectionWire::Rtl
                } else {
                    TextDirectionWire::Ltr
                };
                for (fragment_index, fragment) in fragments.iter().enumerate() {
                    let Some(fragment_text) = text.get(fragment.range.clone()) else {
                        continue;
                    };
                    if fragment_text.is_empty() {
                        continue;
                    }
                    let level = fragment_level(&bidi, text, fragment.range.clone());
                    let bbox = Rect::new(rect.x + fragment.x, rect.y, fragment.width, *line_height);
                    self.runs.push(TextRunWire {
                        id: format!(
                            "p{}:pg{}:l{}:f{}",
                            node_id.value(),
                            page_index,
                            line_index,
                            fragment_index
                        ),
                        bbox: bbox_wire(bbox),
                        line_rect: bbox_wire(*rect),
                        text: fragment_text.to_owned(),
                        style: TextStyleWire {
                            font_family: fragment.font_family.clone(),
                            size_px: half_points_to_px(fragment.size.value()),
                            bold: fragment.bold,
                            italic: fragment.italic,
                            underline: fragment.underline,
                            color: fragment.color,
                        },
                        page: u32::try_from(page_index).unwrap_or(u32::MAX),
                        line_index,
                        fragment_index: u32::try_from(fragment_index).unwrap_or(u32::MAX),
                        paragraph_index: fragment.paragraph_index,
                        run_index: fragment.run_index,
                        reading_order: self.order,
                        direction,
                        bidi_level: level,
                        text_align: align_wire(*align),
                        node_id: wire_id(node_id.value()),
                        block_kind: if table.is_some() {
                            BlockKindWire::TableCell
                        } else if heading_level.is_some() {
                            BlockKindWire::Heading
                        } else {
                            BlockKindWire::Paragraph
                        },
                        heading_level: table.is_none().then_some(*heading_level).flatten(),
                        table,
                    });
                    self.order = self.order.saturating_add(1);
                }
            }
            LayoutItem::Table { cells, .. } => {
                self.collect_cells(page_index, cells, line_no);
            }
            _ => {}
        }
    }

    fn collect_cells(&mut self, page_index: usize, cells: &[TableCellLayout], line_no: &mut u32) {
        for cell in cells {
            // Продолжение вертикального объединения своего текста не несёт —
            // иначе ARIA получила бы дубль ячейки.
            if cell.v_merge_continue {
                continue;
            }
            let context = TableContextWire {
                table_id: wire_id(cell.table_id.value()),
                cell_id: wire_id(cell.cell_id.value()),
                row: cell.row,
                col: cell.col,
                col_span: cell.grid_span.max(1),
                row_span: cell.row_span.max(1),
                is_header: cell.is_header,
                is_repeated_header: cell.header_repeat,
                is_vmerge_continuation: cell.v_merge_continue,
            };
            for item in &cell.content {
                self.collect_item(page_index, item, Some(context), line_no);
            }
        }
    }
}

fn bbox_wire(rect: Rect) -> BBoxWire {
    BBoxWire {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

/// `NodeId` в wire: `u32` вместо `u64`, чтобы TS не получал `bigint`.
///
/// Документ с более чем 4 млрд узлов не помещается в память wasm; насыщение
/// оставляет идентификатор стабильным для всех реальных документов.
fn wire_id(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn align_wire(align: LineAlign) -> TextAlignWire {
    match align {
        LineAlign::Left => TextAlignWire::Left,
        LineAlign::Center => TextAlignWire::Center,
        LineAlign::Right => TextAlignWire::Right,
    }
}

/// Уровень фрагмента по его первому сильному символу.
///
/// Ведущий нейтрал (пробел между арабским и латиницей) наследует уровень
/// предыдущего куска, а сам фрагмент должен сообщать уровень своего текста:
/// иначе латинский run в RTL-абзаце выглядел бы RTL-ным.
fn fragment_level(bidi: &BidiInfo<'_>, text: &str, range: std::ops::Range<usize>) -> u8 {
    let Some(fragment) = text.get(range.clone()) else {
        return 0;
    };
    for (offset, ch) in fragment.char_indices() {
        if matches!(
            bidi::class(ch),
            unicode_bidi::BidiClass::L
                | unicode_bidi::BidiClass::R
                | unicode_bidi::BidiClass::AL
                | unicode_bidi::BidiClass::AN
        ) {
            return bidi
                .levels
                .get(range.start + offset)
                .map_or(0, unicode_bidi::Level::number);
        }
    }
    bidi.levels
        .get(range.start)
        .or_else(|| bidi.levels.first())
        .map_or(0, unicode_bidi::Level::number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{layout_document, LayoutOptions};
    use crate::model::raw::HalfPoint;
    use doc_converter_render::font::FontRegistry;

    fn sample_layout() -> PageLayout {
        let bytes = include_bytes!("../../../test-fixtures/docx/simple/one_paragraph.docx");
        let document = crate::open(bytes.to_vec()).expect("fixture parses");
        let mut fonts = FontRegistry::new(8);
        layout_document(&document, &LayoutOptions::default(), &mut fonts).expect("fixture lays out")
    }

    fn layout_of(bytes: &[u8]) -> PageLayout {
        let document = crate::open(bytes.to_vec()).expect("fixture parses");
        let mut fonts = FontRegistry::new(8);
        layout_document(&document, &LayoutOptions::default(), &mut fonts).expect("fixture lays out")
    }

    #[test]
    fn mixed_styles_produce_fragments() {
        let layout = layout_of(include_bytes!(
            "../../../test-fixtures/docx/styles/character_style_run.docx"
        ));
        let layer = text_layer(&layout);
        assert!(layer.runs.len() > 1, "runs: {}", layer.runs.len());
        let styled = layer
            .runs
            .iter()
            .any(|run| run.style.bold || run.style.italic);
        assert!(styled, "ни один run не несёт bold/italic");
    }

    #[test]
    fn heading_block_kind_and_level() {
        let layout = layout_of(include_bytes!(
            "../../../test-fixtures/docx/formatting/heading_1.docx"
        ));
        let layer = text_layer(&layout);
        let heading = layer
            .runs
            .iter()
            .find(|run| run.block_kind == BlockKindWire::Heading)
            .expect("heading run");
        assert_eq!(heading.heading_level, Some(1));
    }

    #[test]
    fn table_runs_carry_context() {
        let layout = layout_of(include_bytes!(
            "../../../test-fixtures/docx/tables/header_repeat.docx"
        ));
        let layer = text_layer(&layout);
        let cell = layer
            .runs
            .iter()
            .find(|run| run.table.is_some())
            .expect("table run");
        let table = cell.table.expect("table context");
        assert_eq!(table.row, 0);
        assert_eq!(table.col, 0);
        assert!(table.col_span >= 1);
        assert!(table.row_span >= 1);
        assert!(table.is_header, "первая строка фикстуры — заголовок");
    }

    #[test]
    fn arabic_and_hebrew_are_rtl() {
        for (name, bytes) in [
            (
                "arabic_basic",
                include_bytes!("../../../test-fixtures/docx/rtl/arabic_basic.docx").as_slice(),
            ),
            (
                "hebrew_basic",
                include_bytes!("../../../test-fixtures/docx/rtl/hebrew_basic.docx").as_slice(),
            ),
        ] {
            let layout = layout_of(bytes);
            let layer = text_layer(&layout);
            let rtl = layer
                .runs
                .iter()
                .find(|run| run.direction == TextDirectionWire::Rtl)
                .unwrap_or_else(|| panic!("{name}: нет RTL-run"));
            assert_eq!(rtl.bidi_level % 2, 1, "{name}: RTL-уровень чётный");
        }
    }

    #[test]
    fn mixed_direction_has_both_levels() {
        let layout = layout_of(include_bytes!(
            "../../../test-fixtures/docx/rtl/mixed_direction.docx"
        ));
        let layer = text_layer(&layout);
        assert!(layer.runs.iter().any(|run| run.bidi_level % 2 == 1));
        assert!(layer.runs.iter().any(|run| run.bidi_level % 2 == 0));
    }

    #[test]
    fn bidi_visual_table_mirrors_columns_but_keeps_logical_col() {
        let layout = layout_of(include_bytes!(
            "../../../test-fixtures/docx/rtl/rtl_table.docx"
        ));
        let layer = text_layer(&layout);
        let first = layer
            .runs
            .iter()
            .find(|run| {
                run.table
                    .is_some_and(|table| table.row == 0 && table.col == 0)
            })
            .expect("col 0 run");
        let second = layer
            .runs
            .iter()
            .find(|run| {
                run.table
                    .is_some_and(|table| table.row == 0 && table.col == 1)
            })
            .expect("col 1 run");
        assert!(
            first.bbox.x > second.bbox.x,
            "при w:bidiVisual логическая колонка 0 стоит правее: {} vs {}",
            first.bbox.x,
            second.bbox.x
        );
    }

    #[test]
    fn reading_order_is_monotonic() {
        let layout = layout_of(include_bytes!(
            "../../../test-fixtures/docx/rtl/mixed_direction.docx"
        ));
        let layer = text_layer(&layout);
        let mut previous = None;
        for run in &layer.runs {
            if let Some(previous) = previous {
                assert!(run.reading_order > previous);
            }
            previous = Some(run.reading_order);
        }
    }

    #[test]
    fn one_paragraph_gives_runs() {
        let layout = sample_layout();
        let layer = text_layer(&layout);
        assert!(!layer.runs.is_empty());
        assert_eq!(layer.page_map.len(), layout.pages.len());
        let first = &layer.runs[0];
        assert_eq!(first.page, 0);
        assert!(!first.text.is_empty());
        assert!(!first.id.is_empty());
    }

    /// Раскладка документа из байтов — общий хвост теста правок.
    fn lay_out(document: &crate::model::Document) -> PageLayout {
        let mut fonts = FontRegistry::new(8);
        layout_document(document, &LayoutOptions::default(), &mut fonts)
            .expect("фикстура раскладывается")
    }

    #[test]
    fn run_edit_reaches_layout_and_layer() {
        let bytes = include_bytes!("../../../test-fixtures/docx/simple/one_paragraph.docx");
        let document = crate::open(bytes.to_vec()).expect("фикстура разбирается");

        // Без правок: слой несёт текст документа и модельный адрес первого run'а.
        let before = text_layer(&lay_out(&document));
        let original = before.runs[0].text.clone();
        assert!(!original.is_empty(), "первый run фикстуры не пуст");
        assert_eq!(before.runs[0].paragraph_index, Some(0));
        assert_eq!(before.runs[0].run_index, Some(0));

        // Правка попадает и в строку кадра (текст `LayoutItem`), и в слой.
        document
            .set_run_text(0, 0, "правка интерфейса".to_owned())
            .expect("адрес первого run'а валиден");
        let layout = lay_out(&document);
        let after = text_layer(&layout);
        let edited = after
            .runs
            .iter()
            .find(|run| run.text.contains("правка интерфейса"))
            .expect("правленый run в слое");
        assert_eq!(edited.paragraph_index, Some(0));
        assert_eq!(edited.run_index, Some(0));
        let item_texts: Vec<&str> = layout
            .pages
            .iter()
            .flat_map(|page| page.items.iter())
            .filter_map(|item| match item {
                LayoutItem::Paragraph { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            item_texts
                .iter()
                .any(|text| text.contains("правка интерфейса")),
            "кадр несёт правленый текст"
        );
        assert!(!after.runs.iter().any(|run| run.text == original));

        // Снимок правок детерминирован: одна запись по возрастающему адресу.
        assert_eq!(
            document.run_edits(),
            vec![(0, 0, "правка интерфейса".to_owned())]
        );

        // Адреса вне документа и вне абзаца — ошибки, а не паника.
        assert!(matches!(
            document.set_run_text(7, 0, "x".to_owned()),
            Err(crate::RunEditError::ParagraphOutOfRange(7))
        ));
        assert!(matches!(
            document.set_run_text(0, 9, "x".to_owned()),
            Err(crate::RunEditError::RunOutOfRange {
                paragraph: 0,
                run: 9
            })
        ));
    }

    #[test]
    fn page_runs_match_full_layer() {
        let layout = sample_layout();
        let full = text_layer(&layout);
        for (index, page_runs) in full.page_map.iter().enumerate() {
            let page = text_layer_page(&layout, page_runs.page).expect("page exists");
            let expected =
                &full.runs[page_runs.start as usize..(page_runs.start + page_runs.count) as usize];
            assert_eq!(page.runs.len(), expected.len(), "page {index}");
            for (got, want) in page.runs.iter().zip(expected) {
                assert_eq!(got.text, want.text);
                assert!(
                    close(got.bbox.x, want.bbox.x),
                    "{} vs {}",
                    got.bbox.x,
                    want.bbox.x
                );
                assert_eq!(got.reading_order, want.reading_order);
            }
        }
    }

    #[test]
    fn page_out_of_range_is_none() {
        let layout = sample_layout();
        assert!(text_layer_page(&layout, 99).is_none());
    }

    #[test]
    fn bbox_comes_from_layout() {
        let rect = Rect::new(10.0, 20.0, 100.0, 14.0);
        let fragment = LineFragment {
            range: 0..5,
            x: 3.0,
            width: 40.0,
            font_family: Some("Carlito".to_owned()),
            size: HalfPoint::new(24),
            bold: false,
            italic: false,
            underline: false,
            color: None,
            highlight: None,
            paragraph_index: None,
            run_index: None,
        };
        let bbox = Rect::new(rect.x + fragment.x, rect.y, fragment.width, rect.height);
        assert!(close(bbox.x, 13.0));
        assert!(close(bbox.width, 40.0));
        assert!(close(half_points_to_px(fragment.size.value()), 16.0));
    }

    /// Сравнение float в тестах: значения приходят из одной арифметики, но
    /// clippy запрещает `assert_eq!` для `f32`.
    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() <= f32::EPSILON
    }
}
