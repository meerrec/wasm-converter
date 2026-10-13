//! Гиперссылки листа → аннотации `/Link`, примечания → `/Text` на страницах PDF.
//!
//! Прямоугольник аннотации берётся из геометрии листа [`SheetLayout`] — той же,
//! что рисует ячейки painter, — и переносится в координаты страницы общим с
//! painter'ом сдвигом среза, поэтому аннотация накрывает ровно напечатанную
//! ячейку, в том числе после разбивки по страницам.
//!
//! Внешняя ссылка становится действием `/URI`, внутренняя — `/GoTo`:
//! в документ попадает один лист книги, поэтому переход разрешается только
//! на печатаемом листе. `HyperlinkTarget::Broken` аннотации не даёт: цель не
//! разрешилась ещё при разборе книги, и делать вид, что адрес известен, нельзя.
//! Примечание — текст в `/Contents` и автор в `/T`; страницу ему выбирает
//! [`comment_placements`], и она одна на примечание.
//!
//! Аннотации не пишутся в content stream: операции [`Op::Annotation`] painter
//! вынимает из списка и отдаёт [`annotation_to_dict`], который собирает
//! словарь `/Annots` (печать — своя, по образцу `annotation_to_dict` из
//! вендоренного форка: та же форма `/Rect`, `/A`, `/Dest` и UTF-16BE с BOM).
//! Закладки outline собирает сессия ([`crate::pdf::session::Session::set_outline`]),
//! painter копит для неё пункты из [`SheetSpan`].

use doc_converter_xlsx::layout::SheetLayout;
use doc_converter_xlsx::{CellRef, Comment, HyperlinkTarget, Range, Sheet};
use lopdf::{Dictionary, Object, ObjectId, StringFormat::Literal};

use crate::layout::{PageGeometry, RectPx};
use crate::pagination::{PageSlice, SheetPage};
use crate::painter::page_rect;
use crate::pdf::ops::{Actions, Annotation, Destination, LinkAnnotation, Op, TextAnnotation};

/// Аннотации страницы: ссылки, чьи ячейки попали в срез, и примечания,
/// которым досталась эта страница.
///
/// `pages` — все страницы листа: по ним цель внутренней ссылки разрешается
/// в номер страницы PDF. `placements` — примечания с их страницами,
/// посчитанные один раз на лист: искать страницу для каждого примечания на
/// каждом кадре — квадрат по числу страниц. `page_index` — номер текущей
/// страницы в `pages`, с нуля.
#[must_use]
pub(crate) fn page_annotations(
    sheet: &Sheet,
    layout: &SheetLayout,
    pages: &[SheetPage<'_>],
    placements: &[CommentPlacement<'_>],
    geometry: &PageGeometry,
    slice: &PageSlice,
    page_index: usize,
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
        ops.push(Op::Annotation {
            annot: Annotation::Link(LinkAnnotation::new(
                geometry.rect_to_pt(rect).to_pdf(geometry.height_pt()),
                actions,
                None,
                None,
                None,
            )),
        });
    }
    for placement in placements {
        // Примечание — одно на книгу: страницу получает первая, где видна
        // ячейка, включая случай повторяемой шапки. `None` — ячейка не
        // напечатана нигде, аннотировать нечего.
        let Some((page, rect)) = placement.page else {
            continue;
        };
        if page != page_index + 1 {
            continue;
        }
        ops.push(Op::Annotation {
            annot: Annotation::Text(TextAnnotation {
                rect: geometry.rect_to_pt(rect).to_pdf(geometry.height_pt()),
                contents: placement.comment.text.clone(),
                title: placement.comment.author.clone(),
            }),
        });
    }
    ops
}

/// Аннотация страницы → словарь PDF: общие ключи плюс нагрузка подтипа.
///
/// Своя печать повторяет `annotation_to_dict` из вендоренного форка: `/Type /Annot`,
/// `/Subtype` по варианту, `/Rect` из углов, у ссылки — `/A` с действием,
/// `/Border`, `/C` и `/H`; у заметки — `/Contents` и `/T` как UTF-16BE с BOM
/// (текст произвольный Unicode, и кириллица в `PDFDocEncoding` не помещается,
/// ISO 32000, 7.9.2.2). `page_ref` разрешает номер страницы `/GoTo` в id
/// объекта — сессия резервирует их заранее ([`crate::pdf::session::Session::page_ref`]).
pub(crate) fn annotation_to_dict(
    annot: &Annotation,
    page_ref: &dyn Fn(usize) -> Option<ObjectId>,
) -> Dictionary {
    match annot {
        Annotation::Link(link) => {
            let mut dict = annotation_base("Link", &link.rect);
            dict.set("A", actions_to_dict(&link.actions, page_ref));
            dict.set(
                "Border",
                Object::Array(
                    link.border
                        .to_array()
                        .into_iter()
                        .map(Object::Real)
                        .collect(),
                ),
            );
            dict.set(
                "C",
                Object::Array(link.color.to_vec().into_iter().map(Object::Real).collect()),
            );
            dict.set("H", link.highlighting.get_id());
            dict
        }
        Annotation::Text(text) => {
            let mut dict = annotation_base("Text", &text.rect);
            dict.set("Contents", utf16be(&text.contents));
            if let Some(title) = &text.title {
                dict.set("T", utf16be(title));
            }
            dict
        }
    }
}

/// Общие ключи любой аннотации: `/Type`, `/Subtype` и `/Rect`.
fn annotation_base(subtype: &str, rect: &crate::pdf::ops::Rect) -> Dictionary {
    let ll = rect.lower_left();
    let ur = rect.upper_right();

    let mut dict = Dictionary::new();
    dict.set("Type", "Annot");
    dict.set("Subtype", subtype);
    dict.set(
        "Rect",
        Object::Array(vec![
            Object::Real(ll.x.0),
            Object::Real(ll.y.0),
            Object::Real(ur.x.0),
            Object::Real(ur.y.0),
        ]),
    );
    dict
}

/// Действие аннотации: `/URI` или `/GoTo` с целью `/D`.
fn actions_to_dict(actions: &Actions, page_ref: &dyn Fn(usize) -> Option<ObjectId>) -> Dictionary {
    let mut dict = Dictionary::new();
    dict.set("S", actions.get_action_type_id());
    match actions {
        Actions::Goto(destination) => {
            dict.set("D", destination_to_obj(destination, page_ref));
        }
        Actions::Uri(uri) => {
            dict.set("URI", Object::String(uri.clone().into_bytes(), Literal));
        }
    }
    dict
}

/// Цель `/GoTo`: `[страница /XYZ left top zoom]`; страница без резервированного
/// id (за пределами `page_count` сессии) — `null`, как печатал форк.
fn destination_to_obj(
    destination: &Destination,
    page_ref: &dyn Fn(usize) -> Option<ObjectId>,
) -> Object {
    match destination {
        Destination::Xyz {
            page,
            left,
            top,
            zoom,
        } => Object::Array(vec![
            page_ref(page.saturating_sub(1)).map_or(Object::Null, Object::Reference),
            Object::Name(b"XYZ".to_vec()),
            left.map_or(Object::Null, Object::Real),
            top.map_or(Object::Null, Object::Real),
            zoom.map_or(Object::Null, Object::Real),
        ]),
    }
}

/// Текстовая строка PDF как UTF-16BE с BOM.
fn utf16be(text: &str) -> Object {
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

/// Примечание и страница, на которую попала его ячейка.
///
/// Запись одна на примечание; `page` — номер страницы PDF (с единицы) и
/// прямоугольник на ней в координатах страницы, пиксели раскладки. `None` —
/// ячейка не напечатана ни на одной странице; запись всё равно остаётся в
/// списке, чтобы потеря не обнаруживалась молча по недостаче.
pub(crate) struct CommentPlacement<'a> {
    /// Примечание из книги: текст, автор и адрес ячейки.
    pub comment: &'a Comment,
    /// Страница и прямоугольник ячейки; `None` — ячейка не попала в печать.
    pub page: Option<(usize, RectPx)>,
}

/// Сопоставить примечания листа страницам: страница, прямоугольник и текст.
///
/// Страницу выбирает тот же механизм, что и у ссылки ([`link_rect_on_page`]):
/// первая страница, на которой напечатаны и строка, и столбец ячейки. Поэтому
/// примечание в повторяемой шапке привязано к первой странице, а не размножено
/// по всем: `/Text`-аннотация — одна на примечание, так её и считает `DoD` E4,
/// тогда как ссылку надо нажимать на каждой странице, где видна ячейка.
///
/// Прямоугольник — в координатах страницы, пиксели раскладки, как у ссылок:
/// перед записью в PDF его прогоняет `PageGeometry::rect_to_pt` страницы,
/// собранной с `with_page_top_px(slice.offset_y)`. Ячейка, закрытая чужим
/// объединением, не печатается, как и её содержимое, — примечание на ней
/// остаётся без страницы.
///
/// Результат считается один раз на лист: [`page_annotations`] зовётся на
/// каждой странице, и поиск здесь на каждый кадр был бы квадратом по числу
/// страниц.
#[must_use]
pub(crate) fn comment_placements<'a>(
    sheet: &'a Sheet,
    layout: &SheetLayout,
    pages: &[SheetPage<'_>],
) -> Vec<CommentPlacement<'a>> {
    sheet
        .comments
        .iter()
        .map(|comment| {
            let cell = Range {
                first: comment.cell,
                last: comment.cell,
            };
            CommentPlacement {
                comment,
                page: pages.iter().enumerate().find_map(|(index, page)| {
                    link_rect_on_page(sheet, layout, &page.slice, cell)
                        .map(|rect| (index + 1, rect))
                }),
            }
        })
        .collect()
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
    use super::{annotation_to_dict, comment_placements};
    use crate::layout::PageGeometry;
    use crate::options::{Margins, PageConfig, PageSize};
    use crate::pagination::{paginate, print_scale, SheetPagination};
    use crate::pdf::ops::{Actions, Annotation, LinkAnnotation, Pt, Rect, TextAnnotation};
    use doc_converter_xlsx::layout::SheetLayout;
    use doc_converter_xlsx::{
        Cell, CellRef, CellValue, Comment, SharedStrings, Sheet, SheetContent, SheetState,
        StyleTable, Theme, Workbook, WorksheetBuilder, WorksheetMeta,
    };
    use lopdf::{Dictionary, Object, ObjectId};

    /// Часть пакета синтетического листа; нужна только текстам ошибок.
    const PART: &str = "xl/worksheets/sheet1.xml";

    /// Зарезервированный id «первой страницы» — цель `/Dest` в тестах словарей.
    fn first_page_id() -> ObjectId {
        (7, 0)
    }

    /// Словарь ссылки на внешний адрес.
    fn uri_link() -> Dictionary {
        annotation_to_dict(
            &Annotation::Link(LinkAnnotation::new(
                Rect {
                    x: Pt(10.0),
                    y: Pt(20.0),
                    width: Pt(30.0),
                    height: Pt(5.0),
                },
                Actions::uri(String::from("https://example.com")),
                None,
                None,
                None,
            )),
            &|_| Some(first_page_id()),
        )
    }

    /// Текст UTF-16BE с BOM из строкового объекта (как в `tests/annot.rs`).
    #[allow(clippy::chunks_exact_to_as_chunks)] // MSRV 1.82, см. `session.rs`
    fn decode_utf16be(object: &Object) -> String {
        let Object::String(bytes, _) = object else {
            panic!("не строка: {object:?}");
        };
        assert_eq!(bytes.get(..2), Some(&[0xFE, 0xFF][..]), "нет BOM");
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units).expect("корректный UTF-16")
    }

    /// Ссылка с `/URI` печатается как `/Link` с действием и рамкой по умолчанию.
    #[test]
    fn uri_link_becomes_link_annotation_dict() {
        let dict = uri_link();
        assert_eq!(dict.get(b"Type").unwrap().as_name().unwrap(), b"Annot");
        assert_eq!(dict.get(b"Subtype").unwrap().as_name().unwrap(), b"Link");
        let rect: Vec<f32> = dict
            .get(b"Rect")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect();
        assert_eq!(rect, vec![10.0, 20.0, 40.0, 25.0]);

        let action = dict.get(b"A").unwrap().as_dict().unwrap();
        assert_eq!(action.get(b"S").unwrap().as_name().unwrap(), b"URI");
        assert_eq!(
            action.get(b"URI").unwrap().as_str().unwrap(),
            b"https://example.com"
        );
        // Оформление по умолчанию печатается так же, как печатал форк.
        let border: Vec<f32> = dict
            .get(b"Border")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect();
        assert_eq!(border, vec![0.0, 0.0, 1.0]);
        assert_eq!(dict.get(b"H").unwrap().as_name().unwrap(), b"I");
    }

    /// Внутренняя ссылка разрешается в id зарезервированной страницы.
    #[test]
    fn goto_action_resolves_page_through_page_ref() {
        let link = LinkAnnotation::new(
            Rect {
                x: Pt(0.0),
                y: Pt(0.0),
                width: Pt(10.0),
                height: Pt(10.0),
            },
            Actions::go_to(crate::pdf::ops::Destination::Xyz {
                page: 3,
                left: None,
                top: None,
                zoom: None,
            }),
            None,
            None,
            None,
        );
        // Номер страницы с единицы, `page_ref` — с нуля.
        let dict = annotation_to_dict(&Annotation::Link(link.clone()), &|index| {
            (index == 2).then_some(first_page_id())
        });
        let action = dict.get(b"A").unwrap().as_dict().unwrap();
        assert_eq!(action.get(b"S").unwrap().as_name().unwrap(), b"GoTo");
        let dest = action.get(b"D").unwrap().as_array().unwrap();
        assert_eq!(dest[0].as_reference().unwrap(), first_page_id());
        assert_eq!(dest[1].as_name().unwrap(), b"XYZ");
        assert!(dest[2..].iter().all(|value| matches!(value, Object::Null)));

        // Вне диапазона зарезервированных страниц — `null`, как печатал форк.
        let dict = annotation_to_dict(&Annotation::Link(link), &|_| None);
        let action = dict.get(b"A").unwrap().as_dict().unwrap();
        let dest = action.get(b"D").unwrap().as_array().unwrap();
        assert!(matches!(dest[0], Object::Null), "{:?}", dest[0]);
    }

    /// Заметка печатается как `/Text` с UTF-16BE-текстом; без автора `/T` нет.
    #[test]
    fn text_annotation_encodes_contents_and_title() {
        let rect = || Rect {
            x: Pt(0.0),
            y: Pt(0.0),
            width: Pt(10.0),
            height: Pt(10.0),
        };
        let annot = Annotation::Text(TextAnnotation {
            rect: rect(),
            contents: String::from("проверь формулу"),
            title: Some(String::from("Иван")),
        });
        let dict = annotation_to_dict(&annot, &|_| Some(first_page_id()));
        assert_eq!(dict.get(b"Subtype").unwrap().as_name().unwrap(), b"Text");
        assert_eq!(
            decode_utf16be(dict.get(b"Contents").unwrap()),
            "проверь формулу"
        );
        assert_eq!(decode_utf16be(dict.get(b"T").unwrap()), "Иван");
        assert!(dict.get(b"A").is_err(), "у заметки нет действия");

        let no_title = Annotation::Text(TextAnnotation {
            rect: rect(),
            contents: String::from("заметка"),
            title: None,
        });
        let dict = annotation_to_dict(&no_title, &|_| Some(first_page_id()));
        assert!(dict.get(b"T").is_err(), "/T взялся неоткуда");
    }

    /// Книга с одним листом: ячейки и примечания — `(строка, столбец, текст)`.
    fn commented_book(cells: &[(u32, u32, &str)], comments: &[(u32, u32, &str)]) -> Workbook {
        let mut builder = WorksheetBuilder::new(PART);
        for (row, col, text) in cells {
            builder
                .push(
                    *row,
                    Cell::new(*col, 0, CellValue::InlineString((*text).into())),
                )
                .expect("ячейка");
        }
        let content = SheetContent {
            cells: builder.finish(),
            comments: comments
                .iter()
                .map(|(row, col, text)| Comment {
                    cell: CellRef::new(*row, *col),
                    author: None,
                    text: (*text).into(),
                })
                .collect(),
            ..SheetContent::default()
        };
        let meta = WorksheetMeta {
            name: "Лист".into(),
            part: PART.into(),
            state: SheetState::Visible,
        };
        Workbook::new(
            vec![Sheet::new(meta, content)],
            SharedStrings::default(),
            StyleTable::default(),
            Theme::default(),
            false,
        )
    }

    /// Настройки страницы высотой в `content_rows` строк листа; ширина — с
    /// запасом. Вызывающий берёт число строк плюс полстроки, чтобы разрыв не
    /// сдвинуло округление `f32`.
    fn short_page(sheet: &Sheet, content_rows: f32) -> PageConfig {
        let layout = SheetLayout::new(sheet);
        let row_h = layout.row_y(1) - layout.row_y(0);
        let h_mm = content_rows * row_h * 0.75 * 25.4 / 72.0;
        PageConfig {
            size: PageSize::Custom { w_mm: 100.0, h_mm },
            margins: Some(Margins {
                top_mm: 0.0,
                right_mm: 0.0,
                bottom_mm: 0.0,
                left_mm: 0.0,
            }),
            ..PageConfig::default()
        }
    }

    /// Разложить лист книги так же, как painter: масштаб уже с учётом
    /// [`print_scale`], геометрия — итоговая.
    fn paginated<'a>(book: &'a Workbook, cfg: &PageConfig) -> (SheetLayout, SheetPagination<'a>) {
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let mut cfg = cfg.clone();
        cfg.scale = print_scale(&cfg, sheet, &layout);
        let page = PageGeometry::new(&cfg);
        let pages = paginate(book, sheet, &layout, &page, &cfg);
        (layout, pages)
    }

    /// Примечание едет за своей ячейкой: уехавшей на вторую страницу — вторая
    /// страница и прямоугольник её строки.
    #[test]
    fn comment_follows_cell_to_second_page() {
        let book = commented_book(
            &[(0, 0, "первая"), (1, 0, "вторая")],
            &[
                (0, 0, "примечание к первой"),
                (1, 0, "примечание ко второй"),
            ],
        );
        let (layout, pagination) = paginated(&book, &short_page(&book.sheets()[0], 1.5));
        assert_eq!(pagination.pages.len(), 2, "каждая строка — своя страница");

        let sheet = &book.sheets()[0];
        let found = comment_placements(sheet, &layout, &pagination.pages);
        assert_eq!(found.len(), 2, "примечания не потерялись");

        let (page, rect) = found[1].page.expect("вторая ячейка напечатана");
        assert_eq!(page, 2, "примечание уехало вместе с ячейкой");
        assert!((rect.x - layout.column_x(0)).abs() < 1e-3, "{rect:?}");
        assert!(
            (rect.y - layout.row_y(1)).abs() < 1e-3,
            "прямоугольник — на строке ячейки: {rect:?}"
        );
        assert!(
            (rect.h - (layout.row_y(2) - layout.row_y(1))).abs() < 1e-3,
            "высота — строка ячейки: {rect:?}"
        );

        let (first, _) = found[0].page.expect("первая ячейка напечатана");
        assert_eq!(first, 1);
    }

    /// Ячейка вне напечатанного диапазона оставляет примечание без страницы,
    /// но не выбрасывает его: потеря должна быть видна вызывающему.
    ///
    /// Так выглядит примечание на пустой ячейке за `used_range` — например,
    /// содержимое стёрли, а примечание осталось. `fit_to_width` такого дать не
    /// может: он ужимает лист, а не обрезает столбцы, поэтому непопадание даёт
    /// только ячейка вне занятого диапазона — печатается лишь он.
    #[test]
    fn comment_outside_used_range_has_no_page() {
        let book = commented_book(
            &[(0, 0, "первая"), (1, 0, "вторая")],
            &[(9, 0, "оставшееся")],
        );
        let (layout, pagination) = paginated(&book, &short_page(&book.sheets()[0], 1.5));

        let sheet = &book.sheets()[0];
        let found = comment_placements(sheet, &layout, &pagination.pages);
        assert_eq!(found.len(), 1, "примечание не потерялось");
        assert_eq!(found[0].comment.text, "оставшееся");
        assert_eq!(found[0].page, None, "печатать его негде");
    }

    /// Примечание в повторяемой шапке привязано к первой странице, а не
    /// размножено по всем страницам, где шапка повторяется.
    ///
    /// Примечание — одно на книгу: `/Text`-аннотацию `DoD` E4 считает против
    /// числа примечаний, и копия на каждой странице разошлась бы с ним. Ссылка
    /// в шапке ведёт себя иначе (аннотация на каждой странице): её нажимают
    /// там, где видна ячейка, а примечание лишь отдаёт текст.
    #[test]
    fn header_comment_is_pinned_to_first_page() {
        let book = commented_book(
            &[(0, 0, "шапка"), (1, 0, "a"), (2, 0, "b"), (3, 0, "c")],
            &[(0, 0, "сноска к шапке")],
        );
        let mut cfg = short_page(&book.sheets()[0], 2.5);
        cfg.repeat_header_rows = 1;
        let (layout, pagination) = paginated(&book, &cfg);
        assert_eq!(
            pagination.pages.len(),
            3,
            "по строке на страницу, шапка сверху"
        );

        let sheet = &book.sheets()[0];
        let found = comment_placements(sheet, &layout, &pagination.pages);
        assert_eq!(found.len(), 1, "одно примечание — одна запись");
        let (page, rect) = found[0].page.expect("шапка напечатана");
        assert_eq!(page, 1, "первая страница, а не каждая с шапкой");
        assert!(
            (rect.y - layout.row_y(0)).abs() < 1e-3,
            "прямоугольник — на строке шапки: {rect:?}"
        );
        assert!(
            (rect.h - (layout.row_y(1) - layout.row_y(0))).abs() < 1e-3,
            "высота — строка шапки: {rect:?}"
        );
    }
}
