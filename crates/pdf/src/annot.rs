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
//!
//! Закладки outline собирает [`add_outline`]: пункт верхнего уровня на лист.
//! У XLSX нет заголовков в смысле DOCX — ни стилей заголовков, ни разделов, —
//! поэтому лист и есть минимальный осмысленный уровень дерева.

use doc_converter_xlsx::layout::SheetLayout;
use doc_converter_xlsx::{CellRef, HyperlinkTarget, Range, Sheet};
use printpdf::{Actions, Destination, LinkAnnotation, Op, PdfDocument};

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

/// Лист в раскладке экспорта: имя и число занятых страниц.
///
/// Закладке нужен не срез страницы ([`SheetPage`]), а место листа в документе:
/// имя — для заголовка пункта, число страниц — чтобы посчитать начало
/// следующего листа.
pub(crate) struct SheetSpan<'a> {
    /// Имя листа — заголовок закладки.
    pub name: &'a str,
    /// Сколько страниц занял лист; минимум одна — даже пустой лист даёт страницу.
    pub pages: usize,
}

/// Добавить закладки outline: по одной на лист, на страницу начала листа.
///
/// Дерево одноуровневое. У XLSX нет заголовков в смысле DOCX — ни стилей
/// заголовков, ни разделов; именованные диапазоны в модель книги не попадают
/// (`workbook.rs` читает из `definedNames` только печатаемые заголовки), а
/// пункт на каждую строку листа превратил бы панель закладок в копию листа.
/// Лист — единственный крупный уровень, который у книги есть; printpdf 0.8.2
/// к тому же пишет все пункты плоским списком.
///
/// Страницы сквозные: лист занимает `pages` страниц, поэтому следующий
/// начинается после него. В PDF попадает ссылка на страницу-объект, а не
/// записанный где-то номер: сменится пагинация — следующий экспорт пересчитает
/// и закладку вместе с ней.
///
/// `enabled` — флаг `PdfOptions::bookmarks`: при `false` в документ не
/// добавляется ничего, и `/Outlines` не появляется — printpdf пишет его,
/// только когда карта закладок непуста.
///
/// Вызывать до записи страниц: `StreamSession::begin` принимает документ по
/// неизменяемой ссылке и резервирует id страниц заранее.
pub(crate) fn add_outline(doc: &mut PdfDocument, sheets: &[SheetSpan<'_>], enabled: bool) {
    if !enabled {
        return;
    }
    let mut page = 1;
    for sheet in sheets {
        doc.add_bookmark(sheet.name, page);
        page += sheet.pages;
    }
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

#[cfg(test)]
mod tests {
    use super::{add_outline, SheetSpan};
    use lopdf::{Dictionary, Document, Object, ObjectId};
    use printpdf::{Mm, PdfDocument, PdfPage, PdfSaveOptions};
    use std::collections::BTreeMap;

    /// Документ из `count` страниц-заготовок: закладке нужно, куда вести.
    fn document(count: usize) -> PdfDocument {
        let mut doc = PdfDocument::new("Тест");
        doc.with_pages(
            (0..count)
                .map(|_| PdfPage::new(Mm(72.0), Mm(72.0), Vec::new()))
                .collect(),
        );
        doc
    }

    /// Закладки в документе как `(заголовок, страница)` по возрастанию страниц.
    fn bookmarks(doc: &PdfDocument) -> Vec<(&str, usize)> {
        let mut found: Vec<_> = doc
            .bookmarks
            .map
            .values()
            .map(|bookmark| (bookmark.name.as_str(), bookmark.page))
            .collect();
        found.sort_by_key(|(_, page)| *page);
        found
    }

    /// Сохранить и разобрать: проверки смотрят на готовый PDF, а не на карту
    /// в памяти.
    fn saved(doc: &PdfDocument) -> Document {
        let mut warnings = Vec::new();
        let bytes = doc.save(&PdfSaveOptions::default(), &mut warnings);
        Document::load_mem(&bytes).expect("PDF разбирается lopdf")
    }

    /// Словарь `/Outlines` из каталога; `None` — дерева закладок нет.
    fn outlines(doc: &Document) -> Option<Dictionary> {
        let catalog = doc.catalog().expect("каталог документа");
        let outlines = catalog.get(b"Outlines").ok()?;
        let (_, outlines) = doc.dereference(outlines).expect("разыменование /Outlines");
        Some(outlines.as_dict().expect("/Outlines — словарь").clone())
    }

    /// Пункты outline в порядке обхода `/First` → `/Next`: заголовок и номер
    /// страницы, на которую ведёт `/Dest`.
    fn pdf_outline(doc: &Document) -> Vec<(String, u32)> {
        let pages = doc.get_pages();
        let Some(root) = outlines(doc) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut next = reference(&root, b"First");
        while let Some(id) = next {
            let item = doc.get_dictionary(id).expect("пункт outline — словарь");
            found.push((title(item), dest_page(&pages, item)));
            next = reference(item, b"Next");
        }
        found
    }

    /// Ссылка из словаря, если ключ есть (`/First`, `/Next`).
    fn reference(dict: &Dictionary, key: &[u8]) -> Option<ObjectId> {
        dict.get(key).ok()?.as_reference().ok()
    }

    /// Заголовок пункта: printpdf кодирует его UTF-16BE с BOM, кириллица цела.
    // `as_chunks` стабилен с Rust 1.88, а MSRV workspace — 1.82, поэтому
    // предложение clippy здесь не применяется.
    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn title(item: &Dictionary) -> String {
        let bytes = item
            .get(b"Title")
            .expect("/Title")
            .as_str()
            .expect("строка")
            .to_vec();
        let (bom, body) = bytes.split_at_checked(2).expect("BOM в заголовке");
        assert_eq!(bom, [0xFE, 0xFF].as_slice(), "заголовок не UTF-16BE");
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units).expect("заголовок — UTF-16")
    }

    /// Номер страницы, на которую ведёт `/Dest`: первым элементом массива
    /// стоит ссылка на страницу-объект — номера в PDF не хранятся.
    fn dest_page(pages: &BTreeMap<u32, ObjectId>, item: &Dictionary) -> u32 {
        let dest = item
            .get(b"Dest")
            .expect("/Dest")
            .as_array()
            .expect("/Dest — массив");
        let object = dest
            .first()
            .expect("первый элемент /Dest")
            .as_reference()
            .expect("/Dest ссылается на страницу");
        pages
            .iter()
            .find(|(_, id)| **id == object)
            .map(|(number, _)| *number)
            .expect("страница цели есть в документе")
    }

    /// Дерево содержит по закладке на лист, и каждая ведёт на страницу начала
    /// листа: листы занимают разное число страниц, номера сквозные.
    #[test]
    fn one_bookmark_per_sheet_on_sheet_start() {
        let mut doc = document(6);
        add_outline(
            &mut doc,
            &[
                SheetSpan {
                    name: "Лист1",
                    pages: 1,
                },
                SheetSpan {
                    name: "Отчёт",
                    pages: 3,
                },
                SheetSpan {
                    name: "Лист3",
                    pages: 2,
                },
            ],
            true,
        );
        assert_eq!(bookmarks(&doc), [("Лист1", 1), ("Отчёт", 2), ("Лист3", 5)]);
    }

    /// Книга из одного листа — одна закладка на первой странице, сколько бы
    /// страниц лист ни занял.
    #[test]
    fn single_sheet_book_has_one_bookmark_at_first_page() {
        let mut doc = document(4);
        add_outline(
            &mut doc,
            &[SheetSpan {
                name: "Лист1",
                pages: 4,
            }],
            true,
        );
        assert_eq!(bookmarks(&doc), [("Лист1", 1)]);
    }

    /// `bookmarks: false` — дерева нет ни в документе, ни в готовом PDF:
    /// printpdf пишет `/Outlines`, только когда карта закладок непуста.
    #[test]
    fn disabled_bookmarks_leave_no_outline() {
        let mut doc = document(2);
        add_outline(
            &mut doc,
            &[SheetSpan {
                name: "Лист1",
                pages: 2,
            }],
            false,
        );
        assert!(doc.bookmarks.map.is_empty());
        assert!(
            outlines(&saved(&doc)).is_none(),
            "при false /Outlines не пишется"
        );
    }

    /// Пустой список листов — не ошибка: закладок просто нет.
    #[test]
    fn empty_sheet_list_adds_no_bookmarks() {
        let mut doc = document(1);
        add_outline(&mut doc, &[], true);
        assert!(doc.bookmarks.map.is_empty());
        assert!(outlines(&saved(&doc)).is_none());
    }

    /// `DoD`: lopdf видит `/Outlines`, число пунктов совпадает с числом листов,
    /// заголовки — имена листов, `/Dest` ведёт на страницу-объект.
    #[test]
    fn outline_survives_save_and_points_at_pages() {
        let mut doc = document(3);
        add_outline(
            &mut doc,
            &[
                SheetSpan {
                    name: "Лист1",
                    pages: 2,
                },
                SheetSpan {
                    name: "Лист2",
                    pages: 1,
                },
            ],
            true,
        );
        let pdf = saved(&doc);
        let root = outlines(&pdf).expect("/Outlines в каталоге");
        let count = match root.get(b"Count").expect("/Count") {
            Object::Integer(count) => *count,
            other => panic!("ожидалось число, получено {other:?}"),
        };
        assert_eq!(count, 2, "пунктов не столько, сколько листов");
        assert_eq!(
            pdf_outline(&pdf),
            [("Лист1".to_owned(), 1), ("Лист2".to_owned(), 3)]
        );
    }
}
