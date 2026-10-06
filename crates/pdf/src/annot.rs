//! Гиперссылки листа → аннотации `/Link` на страницах PDF.
//!
//! Прямоугольник ссылки берётся из геометрии листа [`SheetLayout`] — той же,
//! что рисует ячейки painter, — и переносится в координаты страницы общим с
//! painter'ом сдвигом среза, поэтому аннотация накрывает ровно напечатанную
//! ячейку, в том числе после разбивки по страницам.
//!
//! Внешняя ссылка становится действием `/URI`, внутренняя — `/GoTo`:
//! в документ попадает один лист книги, поэтому переход разрешается только
//! на печатаемом листе. `HyperlinkTarget::Broken` аннотации не даёт: цель не
//! разрешилась ещё при разборе книги, и делать вид, что адрес известен, нельзя.

use doc_converter_xlsx::layout::SheetLayout;
use doc_converter_xlsx::{CellRef, HyperlinkTarget, Range, Sheet};
use printpdf::{Actions, Destination, LinkAnnotation, Op};

use crate::layout::{PageGeometry, RectPx};
use crate::pagination::{PageSlice, SheetPage};
use crate::painter::page_rect;

/// Аннотации страницы: по одной на каждую ссылку, чьи ячейки попали в срез.
///
/// `pages` — все страницы листа: по ним цель внутренней ссылки разрешается
/// в номер страницы PDF.
#[must_use]
pub(crate) fn page_annotations(
    sheet: &Sheet,
    layout: &SheetLayout,
    pages: &[SheetPage<'_>],
    geometry: &PageGeometry,
    slice: &PageSlice,
) -> Vec<Op> {
    let mut ops = Vec::new();
    for link in &sheet.hyperlinks {
        let actions = match &link.target {
            HyperlinkTarget::External(uri) => Actions::uri(uri.clone()),
            HyperlinkTarget::Internal(target) => Actions::go_to(Destination::Xyz {
                page: destination_page(target, &sheet.meta.name, pages),
                left: None,
                top: None,
                zoom: None,
            }),
            // Цели нет — вести некуда; в DoD E1 такие ссылки не считаются.
            HyperlinkTarget::Broken(_) => continue,
        };
        let Some(rect) = link_rect_on_page(sheet, layout, slice, link.range) else {
            continue;
        };
        ops.push(Op::LinkAnnotation {
            link: LinkAnnotation::new(
                geometry.rect_to_pt(rect).to_pdf(geometry.height_pt()),
                actions,
                None,
                None,
                None,
            ),
        });
    }
    ops
}

/// Номер страницы PDF (с единицы), на которую ведёт внутренняя ссылка.
///
/// Цель вида `Лист2!A1` адресует лист книги, а в PDF попал один лист: если это
/// не печатаемый лист, а также если цель — определённое имя или часть пакета,
/// страницы-цели в документе нет. Такие ссылки `DoD` считает наравне с
/// внешними, поэтому ведём их на первую страницу: висячий `/D` с `null` вместо
/// страницы ломает `/Annots` (`qpdf --check`), а `/URI` с именем листа уводил
/// бы в никуда.
fn destination_page(target: &str, sheet_name: &str, pages: &[SheetPage<'_>]) -> usize {
    let (name, cell) = split_target(target);
    if name.is_some_and(|name| name != sheet_name) {
        return 1;
    }
    let Ok(at) = CellRef::parse(cell) else {
        return 1;
    };
    pages
        .iter()
        .position(|page| covers(&page.slice.rows, &page.slice.header_rows, at.row))
        .map_or(1, |index| index + 1)
}

/// Разделить цель внутренней ссылки на имя листа и адрес.
///
/// Имя листа в ссылке экранируется кавычками (`'Мой лист'!A1`), внутри кавычек
/// апостроф удваивается. `None` — имени нет, адрес относится к текущему листу
/// (или это определённое имя, которое [`CellRef::parse`] не разберёт).
#[must_use]
fn split_target(target: &str) -> (Option<String>, &str) {
    let Some(pos) = target.rfind('!') else {
        return (None, target);
    };
    let name = &target[..pos];
    let unquoted = name
        .strip_prefix('\'')
        .and_then(|name| name.strip_suffix('\''))
        .unwrap_or(name)
        .replace("''", "'");
    (Some(unquoted), &target[pos + 1..])
}

/// Прямоугольник ссылки на этой странице; `None` — диапазон сюда не попал.
///
/// Диапазон ссылки может лежать сразу на двух страницах (разрыв по столбцам):
/// берутся только строки и столбцы, попавшие в срез, а аннотация накрывает их
/// объединение. Обход идёт по полосам страницы, а не по диапазону: ссылка на
/// целый столбец не должна заставлять перебирать миллион строк.
fn link_rect_on_page(
    sheet: &Sheet,
    layout: &SheetLayout,
    slice: &PageSlice,
    range: Range,
) -> Option<RectPx> {
    let range = range.normalized();
    let mut united: Option<RectPx> = None;
    for row_band in [&slice.rows, &slice.header_rows] {
        let Some(rows) = clip(range.first.row, range.last.row, row_band) else {
            continue;
        };
        for col_band in [&slice.cols, &slice.repeat_cols] {
            let Some(cols) = clip(range.first.col, range.last.col, col_band) else {
                continue;
            };
            for row in rows.clone() {
                for col in cols.clone() {
                    let Some(mut rect) = cell_rect(sheet, layout, CellRef::new(row, col)) else {
                        continue;
                    };
                    // Повторяемые части painter прикалывает к краю страницы,
                    // прибавляя сдвиг среза; `page_rect` его снова вычтет —
                    // ячейка останется в координатах листа, как и при рисовании.
                    if slice.repeat_cols.contains(&col) {
                        rect.x += slice.offset_x;
                    }
                    if slice.header_rows.contains(&row) {
                        rect.y += slice.offset_y;
                    }
                    united = Some(united.map_or(rect, |union| union_rect(union, rect)));
                }
            }
        }
    }
    united.map(|rect| page_rect(rect, slice))
}

/// Пересечение диапазона строк (столбцов) ссылки с полосой страницы.
///
/// Пустая полоса (нет повторов или нет полосы потока) не содержит ничего:
/// её границы `0..0` иначе притворились бы строкой (столбцом) с номером 0.
#[must_use]
fn clip(
    first: u32,
    last: u32,
    band: &std::ops::Range<u32>,
) -> Option<std::ops::RangeInclusive<u32>> {
    if band.is_empty() {
        return None;
    }
    let first = first.max(band.start);
    let last = last.min(band.end - 1);
    (first <= last).then_some(first..=last)
}

/// Прямоугольник ячейки в координатах листа.
///
/// Повторяет `cell_rect` из `pagination.rs`: ячейка объединения растягивается
/// до всего объединения, а её не-первая ячейка не рисуется — painter пропускает
/// такие же.
fn cell_rect(sheet: &Sheet, layout: &SheetLayout, at: CellRef) -> Option<RectPx> {
    let (first, last) = match sheet.merges.covering(at) {
        Some(range) if range.first != at => return None,
        Some(range) => (range.first, range.last),
        None => (at, at),
    };
    let x = layout.column_x(first.col);
    let y = layout.row_y(first.row);
    Some(RectPx::new(
        x,
        y,
        layout.column_x(last.col + 1) - x,
        layout.row_y(last.row + 1) - y,
    ))
}

/// Лежит ли строка (столбец) на странице: в полосе потока или в повторяемой части.
#[must_use]
fn covers(band: &std::ops::Range<u32>, repeated: &std::ops::Range<u32>, index: u32) -> bool {
    band.contains(&index) || repeated.contains(&index)
}

/// Объединение прямоугольников — рамка, накрывающая оба.
#[must_use]
fn union_rect(a: RectPx, b: RectPx) -> RectPx {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    RectPx::new(
        x,
        y,
        (a.x + a.w).max(b.x + b.w) - x,
        (a.y + a.h).max(b.y + b.h) - y,
    )
}
