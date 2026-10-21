//! Операции страницы → байты content stream (ADR-0022).
//!
//! Транслятор повторяет `translate_operations` из вендоренного форка байт в
//! байт:
//! те же операторы, тот же порядок операндов и то же форматирование (числа —
//! через `{}`, как у `lopdf::writer`; сериализацию выполняет сам lopdf через
//! `Content::encode`). От совпадения зависит отрисовка при переходе с форка на
//! свой бэкенд. Отличие от форка одно и осознанное: неизвестный шрифт в
//! `WriteText`/`SetFontSize` — ошибка, а не молчаливый пропуск текста.

use std::collections::BTreeMap;

use lopdf::content::{Content, Operation};
use lopdf::Object::{self, Array, Integer, Name, Real};
use lopdf::StringFormat::Hexadecimal;

use super::fonts;
use super::ops::{
    Color, CurTransMat, FontId, Line, LinePoint, Op, PaintMode, Point, Polygon, TextItem,
    XObjectId, XObjectTransform,
};
use crate::PdfError;

/// Перевести операции страницы в байты content stream.
///
/// `fonts` — имя → подготовленный шрифт (для кодирования текста). Аннотации
/// в поток не пишутся: их сериализует словарь страницы (следующие волны,
/// `annot.rs`).
///
/// # Errors
///
/// [`PdfError::Font`], если `WriteText` или `SetFontSize` ссылаются на шрифт,
/// которого нет в `fonts`: молча потерянный текст хуже ошибки экспорта.
pub fn translate(
    ops: &[Op],
    fonts: &BTreeMap<FontId, &fonts::PreparedFont>,
) -> Result<Vec<u8>, PdfError> {
    let mut operations: Vec<Operation> = Vec::new();
    for op in ops {
        match op {
            Op::SaveGraphicsState => push(&mut operations, "q", vec![]),
            Op::RestoreGraphicsState => push(&mut operations, "Q", vec![]),
            Op::LoadGraphicsState { gs } => {
                push(&mut operations, "gs", vec![Name(gs.0.as_bytes().to_vec())]);
            }
            Op::StartTextSection => push(&mut operations, "BT", vec![]),
            Op::EndTextSection => push(&mut operations, "ET", vec![]),
            Op::WriteText { items, font } => {
                let prepared = prepared_font(font, fonts)?;
                encode_text_items(items, prepared, &mut operations);
            }
            Op::SetFontSize { size, font } => {
                prepared_font(font, fonts)?;
                push(
                    &mut operations,
                    "Tf",
                    vec![Name(font.0.as_bytes().to_vec()), Real(size.0)],
                );
            }
            Op::SetTextCursor { pos } => {
                push(&mut operations, "Td", vec![Real(pos.x.0), Real(pos.y.0)]);
            }
            Op::SetFillColor { col } => {
                push(
                    &mut operations,
                    fill_color_operator(col),
                    color_operands(col),
                );
            }
            Op::SetOutlineColor { col } => {
                push(
                    &mut operations,
                    stroke_color_operator(col),
                    color_operands(col),
                );
            }
            Op::SetOutlineThickness { pt } => push(&mut operations, "w", vec![Real(pt.0)]),
            Op::SetLineDashPattern { dash } => {
                let pattern = Array(dash.as_array().into_iter().map(Integer).collect());
                push(&mut operations, "d", vec![pattern, Integer(dash.offset)]);
            }
            Op::SetTextMatrix { matrix } => {
                push(&mut operations, "Tm", matrix_operands(&matrix.as_array()));
            }
            Op::DrawLine { line } => operations.append(&mut line_ops(line)),
            Op::DrawPolygon { polygon } => operations.append(&mut polygon_ops(polygon)),
            Op::UseXobject { id, transform } => operations.append(&mut xobject_ops(id, transform)),
            Op::Annotation { .. } => {}
        }
    }
    // `encode` пишет в `Vec` и ошибок записи не даёт; как и
    // `translate_operations` форка, отказ здесь неотличим от пустого потока.
    Ok(Content { operations }.encode().unwrap_or_default())
}

/// Добавить операцию: операнды записывает `Content::encode` с пробелом после
/// каждого — тот же формат, что у lopdf.
fn push(operations: &mut Vec<Operation>, operator: &str, operands: Vec<Object>) {
    operations.push(Operation::new(operator, operands));
}

/// Шрифт по имени в ресурсах.
fn prepared_font<'a>(
    font: &FontId,
    fonts: &'a BTreeMap<FontId, &fonts::PreparedFont>,
) -> Result<&'a fonts::PreparedFont, PdfError> {
    fonts
        .get(font)
        .copied()
        .ok_or_else(|| PdfError::Font(format!("font '{}' is not registered", font.0)))
}

/// Текстовые элементы в `Tj`/`TJ`: одна строка без кернинга — короткий `Tj`,
/// несколько элементов или сдвиги — массив `TJ`.
fn encode_text_items(
    items: &[TextItem],
    font: &fonts::PreparedFont,
    operations: &mut Vec<Operation>,
) {
    if items.is_empty() {
        return;
    }
    let mut tj_array: Vec<Object> = Vec::new();
    for item in items {
        match item {
            // Identity-H: коды символов — 2-байтовые CID подрезанного шрифта,
            // поэтому строка только шестнадцатеричная.
            TextItem::Text(text) => {
                tj_array.push(Object::String(font.encode(text), Hexadecimal));
            }
            TextItem::Offset(offset) => tj_array.push(Real(*offset)),
        }
    }
    if tj_array.len() == 1 && !items.iter().any(|item| matches!(item, TextItem::Offset(_))) {
        operations.push(Operation::new("Tj", vec![tj_array.remove(0)]));
    } else {
        operations.push(Operation::new("TJ", vec![Array(tj_array)]));
    }
}

/// Оператор цвета заливки по пространству.
fn fill_color_operator(col: &Color) -> &'static str {
    match col {
        Color::Rgb(_) => "rg",
        Color::Cmyk(_) | Color::SpotColor(_) => "k",
        Color::Greyscale(_) => "g",
    }
}

/// Оператор цвета обводки по пространству.
fn stroke_color_operator(col: &Color) -> &'static str {
    match col {
        Color::Rgb(_) => "RG",
        Color::Cmyk(_) | Color::SpotColor(_) => "K",
        Color::Greyscale(_) => "G",
    }
}

/// Компоненты цвета в порядке операндов оператора.
fn color_operands(col: &Color) -> Vec<Object> {
    col.to_vec().into_iter().map(Real).collect()
}

/// Операнды матрицы `[a b c d e f]`.
fn matrix_operands(matrix: &[f32; 6]) -> Vec<Object> {
    matrix.iter().copied().map(Real).collect()
}

/// Операнды точки: `x y`.
fn point_operands(point: Point) -> Vec<Object> {
    vec![Real(point.x.0), Real(point.y.0)]
}

/// `m` в первую точку и последующие `l`/`c`; замыкание и отрисовку добавляет
/// вызывающий.
fn path_ops(points: &[LinePoint]) -> Vec<Operation> {
    let mut operations = Vec::new();
    if points.is_empty() {
        return operations;
    }
    operations.push(Operation::new("m", point_operands(points[0].p)));
    let mut i = 1;
    while i < points.len() {
        let current = &points[i];
        // Кривая Безье: две опорные точки и конечная. Одна опорная точка —
        // участок вырождается в отрезок (как в вендоренном форке).
        if current.bezier && i + 2 < points.len() && points[i + 1].bezier {
            let end = &points[i + 2];
            let mut operands = point_operands(current.p);
            operands.extend(point_operands(points[i + 1].p));
            operands.extend(point_operands(end.p));
            operations.push(Operation::new("c", operands));
            i += 3;
        } else {
            operations.push(Operation::new("l", point_operands(current.p)));
            i += 1;
        }
    }
    operations
}

/// Ломаная: путь, `h` при замкнутости и обводка `S`.
fn line_ops(line: &Line) -> Vec<Operation> {
    let mut operations = path_ops(&line.points);
    if line.is_closed && !line.points.is_empty() {
        operations.push(Operation::new("h", vec![]));
    }
    if !line.points.is_empty() {
        operations.push(Operation::new("S", vec![]));
    }
    operations
}

/// Многоугольник: все кольца, явный `h` и отрисовка по режиму.
fn polygon_ops(polygon: &Polygon) -> Vec<Operation> {
    let mut operations = Vec::new();
    if polygon.rings.is_empty() {
        return operations;
    }
    for ring in &polygon.rings {
        operations.append(&mut path_ops(&ring.points));
    }
    // Путь замыкается явным `h` до отрисовки — как в форке, чтобы `S`/`f`/`b`
    // вели себя одинаково независимо от режима.
    operations.push(Operation::new("h", vec![]));
    match polygon.mode {
        PaintMode::Clip => {
            operations.push(Operation::new(polygon.winding_order.get_clip_op(), vec![]));
            operations.push(Operation::new("n", vec![]));
        }
        PaintMode::Fill => {
            operations.push(Operation::new(polygon.winding_order.get_fill_op(), vec![]));
        }
        PaintMode::Stroke => {
            operations.push(Operation::new("S", vec![]));
        }
        PaintMode::FillStroke => {
            operations.push(Operation::new(
                polygon.winding_order.get_fill_stroke_close_op(),
                vec![],
            ));
        }
    }
    operations
}

/// Картинка: `q`, совмещённые матрицы трансформа, `Do`, `Q`.
fn xobject_ops(id: &XObjectId, transform: &XObjectTransform) -> Vec<Operation> {
    // Реестра картинок у транслятора нет (сигнатура заморожена), поэтому
    // `get_ctms(None)`: шаг «масштаб по размеру картинки» пропускается.
    // Он вернётся вместе с волной перевода `image.rs`; до неё трансформ
    // несёт только масштаб/поворот/перенос, как и печатает форк без XObject.
    let mut combined = CurTransMat::Identity;
    for matrix in transform.get_ctms(None) {
        combined = CurTransMat::Raw(CurTransMat::combine_matrix(
            combined.as_array(),
            matrix.as_array(),
        ));
    }
    vec![
        Operation::new("q", vec![]),
        Operation::new("cm", matrix_operands(&combined.as_array())),
        Operation::new("Do", vec![Name(id.0.as_bytes().to_vec())]),
        Operation::new("Q", vec![]),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;
    use std::process::Command;

    use lopdf::Document;

    use super::translate;
    use crate::pdf::fonts;
    use crate::pdf::ops::{
        Actions, Annotation, Cmyk, Color, ExtendedGraphicsStateId, FontId, Greyscale, Line,
        LineDashPattern, LinePoint, LinkAnnotation, Op, PaintMode, Point, Polygon, PolygonRing, Pt,
        Rect, Rgb, TextItem, TextMatrix, WindingOrder, XObjectId, XObjectTransform,
    };
    use crate::pdf::session::Session;
    use crate::PdfError;

    /// Размер страницы A4 в пунктах.
    const A4: [f32; 4] = [0.0, 0.0, 595.0, 842.0];

    /// Множество символов — аргумент подрезки.
    fn used_of(text: &str) -> BTreeSet<char> {
        text.chars().collect()
    }

    /// Подготовленный Carlito по использованным символам.
    fn prepared(text: &str) -> fonts::PreparedFont {
        fonts::prepare(
            doc_converter_render::font::default_font_bytes(),
            &used_of(text),
        )
        .unwrap()
    }

    /// Байты в шестнадцатеричную строку — так глифы попадают в content stream.
    fn hex_of(bytes: &[u8]) -> String {
        let mut out = String::new();
        for byte in bytes {
            let _ = write!(out, "{byte:02X}");
        }
        out
    }

    /// Реестр шрифтов теста: владеет шрифтами и отдаёт на них ссылки —
    /// иначе временный массив умер бы раньше, чем карта ссылок на него.
    struct FontRegistry {
        ids: Vec<FontId>,
        owned: Vec<fonts::PreparedFont>,
    }

    impl FontRegistry {
        fn new(entries: Vec<(FontId, fonts::PreparedFont)>) -> Self {
            let (ids, owned) = entries.into_iter().unzip();
            Self { ids, owned }
        }

        /// Реестр шрифтов в форме, которую ждёт `translate`.
        fn by_id(&self) -> BTreeMap<FontId, &fonts::PreparedFont> {
            self.ids.iter().cloned().zip(self.owned.iter()).collect()
        }
    }

    /// Пустой реестр — для операций, не трогающих шрифты.
    fn empty_fonts() -> BTreeMap<FontId, &'static fonts::PreparedFont> {
        BTreeMap::new()
    }

    /// Перевод в строку: тесты сверяют точные последовательности операторов.
    fn translated(ops: &[Op], fonts: &BTreeMap<FontId, &fonts::PreparedFont>) -> String {
        String::from_utf8(translate(ops, fonts).unwrap()).unwrap()
    }

    #[test]
    fn text_ops_emit_bt_tf_td_tj_et_with_hex_glyphs() {
        const TEXT: &str = "Hello, мир!";
        let font = prepared(TEXT);
        let hex = hex_of(&font.encode(TEXT));
        let registry = FontRegistry::new(vec![(FontId(String::from("F0")), font)]);
        let fonts = registry.by_id();
        let ops = vec![
            Op::StartTextSection,
            Op::SetFontSize {
                size: Pt(24.0),
                font: FontId(String::from("F0")),
            },
            Op::SetTextCursor {
                pos: Point {
                    x: Pt(72.0),
                    y: Pt(700.0),
                },
            },
            Op::WriteText {
                items: vec![TextItem::Text(String::from(TEXT))],
                font: FontId(String::from("F0")),
            },
            Op::EndTextSection,
        ];
        assert_eq!(
            translated(&ops, &fonts),
            format!("BT\n/F0 24 Tf\n72 700 Td\n<{hex}> Tj\nET")
        );
    }

    #[test]
    fn multiple_text_items_use_tj_array() {
        let font = prepared("AB");
        let hex_a = hex_of(&font.encode("A"));
        let hex_b = hex_of(&font.encode("B"));
        let registry = FontRegistry::new(vec![(FontId(String::from("F0")), font)]);
        let fonts = registry.by_id();
        let ops = vec![Op::WriteText {
            items: vec![
                TextItem::Text(String::from("A")),
                TextItem::Text(String::from("B")),
            ],
            font: FontId(String::from("F0")),
        }];
        assert_eq!(translated(&ops, &fonts), format!("[<{hex_a}><{hex_b}>] TJ"));
    }

    #[test]
    fn offset_items_use_tj_array_with_real() {
        let font = prepared("A");
        let hex_a = hex_of(&font.encode("A"));
        let registry = FontRegistry::new(vec![(FontId(String::from("F0")), font)]);
        let fonts = registry.by_id();
        let ops = vec![Op::WriteText {
            items: vec![TextItem::Text(String::from("A")), TextItem::Offset(-1.5)],
            font: FontId(String::from("F0")),
        }];
        assert_eq!(translated(&ops, &fonts), format!("[<{hex_a}> -1.5] TJ"));
    }

    #[test]
    fn empty_write_text_emits_nothing() {
        let font = prepared("");
        let registry = FontRegistry::new(vec![(FontId(String::from("F0")), font)]);
        let fonts = registry.by_id();
        let ops = vec![Op::WriteText {
            items: vec![],
            font: FontId(String::from("F0")),
        }];
        assert_eq!(translated(&ops, &fonts), "");
    }

    #[test]
    fn line_ops_emit_m_l_c_h_s() {
        let line = Line {
            points: vec![
                LinePoint {
                    p: Point {
                        x: Pt(0.0),
                        y: Pt(0.0),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: Point {
                        x: Pt(10.0),
                        y: Pt(10.0),
                    },
                    bezier: true,
                },
                LinePoint {
                    p: Point {
                        x: Pt(20.0),
                        y: Pt(0.0),
                    },
                    bezier: true,
                },
                LinePoint {
                    p: Point {
                        x: Pt(30.0),
                        y: Pt(10.0),
                    },
                    bezier: false,
                },
            ],
            is_closed: true,
        };
        let ops = vec![Op::DrawLine { line }];
        assert_eq!(
            translated(&ops, &empty_fonts()),
            "0 0 m\n10 10 20 0 30 10 c\nh\nS"
        );
    }

    #[test]
    fn single_bezier_handle_falls_back_to_line() {
        // Одной опорной точки мало для кривой: участок становится отрезком,
        // как в вендоренном форке.
        let line = Line {
            points: vec![
                LinePoint {
                    p: Point {
                        x: Pt(0.0),
                        y: Pt(0.0),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: Point {
                        x: Pt(5.0),
                        y: Pt(5.0),
                    },
                    bezier: true,
                },
                LinePoint {
                    p: Point {
                        x: Pt(10.0),
                        y: Pt(0.0),
                    },
                    bezier: false,
                },
            ],
            is_closed: false,
        };
        let ops = vec![Op::DrawLine { line }];
        assert_eq!(translated(&ops, &empty_fonts()), "0 0 m\n5 5 l\n10 0 l\nS");
    }

    #[test]
    fn polygon_fill_stroke_and_clip_follow_paint_mode() {
        let ring = PolygonRing {
            points: vec![
                LinePoint {
                    p: Point {
                        x: Pt(0.0),
                        y: Pt(0.0),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: Point {
                        x: Pt(10.0),
                        y: Pt(0.0),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: Point {
                        x: Pt(10.0),
                        y: Pt(10.0),
                    },
                    bezier: false,
                },
            ],
        };
        let base = "0 0 m\n10 0 l\n10 10 l\nh\n";

        let fill = Polygon {
            rings: vec![ring.clone()],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        };
        assert_eq!(
            translated(&[Op::DrawPolygon { polygon: fill }], &empty_fonts()),
            format!("{base}f")
        );

        let fill_stroke = Polygon {
            rings: vec![ring.clone()],
            mode: PaintMode::FillStroke,
            winding_order: WindingOrder::EvenOdd,
        };
        assert_eq!(
            translated(
                &[Op::DrawPolygon {
                    polygon: fill_stroke
                }],
                &empty_fonts()
            ),
            format!("{base}b*")
        );

        let stroke = Polygon {
            rings: vec![ring.clone()],
            mode: PaintMode::Stroke,
            winding_order: WindingOrder::NonZero,
        };
        assert_eq!(
            translated(&[Op::DrawPolygon { polygon: stroke }], &empty_fonts()),
            format!("{base}S")
        );

        let clip = Polygon {
            rings: vec![ring],
            mode: PaintMode::Clip,
            winding_order: WindingOrder::NonZero,
        };
        assert_eq!(
            translated(&[Op::DrawPolygon { polygon: clip }], &empty_fonts()),
            format!("{base}W\nn")
        );
    }

    #[test]
    fn color_ops_use_rg_g_k_and_rg_g_k() {
        let ops = vec![
            Op::SetFillColor {
                col: Color::Rgb(Rgb::new(0.1, 0.2, 0.3, None)),
            },
            Op::SetFillColor {
                col: Color::Greyscale(Greyscale::new(0.5, None)),
            },
            Op::SetFillColor {
                col: Color::Cmyk(Cmyk::new(0.1, 0.2, 0.3, 0.4, None)),
            },
            Op::SetOutlineColor {
                col: Color::Rgb(Rgb::new(0.1, 0.2, 0.3, None)),
            },
            Op::SetOutlineColor {
                col: Color::Greyscale(Greyscale::new(0.5, None)),
            },
            Op::SetOutlineColor {
                col: Color::Cmyk(Cmyk::new(0.1, 0.2, 0.3, 0.4, None)),
            },
        ];
        assert_eq!(
            translated(&ops, &empty_fonts()),
            "0.1 0.2 0.3 rg\n0.5 g\n0.1 0.2 0.3 0.4 k\n\
             0.1 0.2 0.3 RG\n0.5 G\n0.1 0.2 0.3 0.4 K"
        );
    }

    #[test]
    fn thickness_and_dash_ops() {
        let dash = LineDashPattern {
            offset: 1,
            dash_1: Some(3),
            gap_1: Some(2),
            dash_2: None,
            gap_2: None,
            dash_3: None,
            gap_3: None,
        };
        let ops = vec![
            Op::SetOutlineThickness { pt: Pt(1.5) },
            Op::SetLineDashPattern { dash },
        ];
        assert_eq!(translated(&ops, &empty_fonts()), "1.5 w\n[3 2] 1 d");
    }

    #[test]
    fn gstate_matrix_and_xobject_ops() {
        let ops = vec![
            Op::SaveGraphicsState,
            Op::LoadGraphicsState {
                gs: ExtendedGraphicsStateId(String::from("GS1")),
            },
            Op::SetTextMatrix {
                matrix: TextMatrix::Raw([1.0, 0.0, 0.0, 1.0, 100.0, 200.0]),
            },
            Op::UseXobject {
                id: XObjectId(String::from("Img0")),
                transform: XObjectTransform {
                    translate_x: Some(Pt(10.0)),
                    translate_y: Some(Pt(20.0)),
                    rotate: None,
                    scale_x: None,
                    scale_y: None,
                    dpi: None,
                },
            },
            Op::RestoreGraphicsState,
        ];
        assert_eq!(
            translated(&ops, &empty_fonts()),
            "q\n/GS1 gs\n1 0 0 1 100 200 Tm\nq\n1 0 0 1 10 20 cm\n/Img0 Do\nQ\nQ"
        );
    }

    #[test]
    fn annotation_emits_nothing() {
        let ops = vec![Op::Annotation {
            annot: Annotation::Link(LinkAnnotation::new(
                Rect {
                    x: Pt(10.0),
                    y: Pt(10.0),
                    width: Pt(50.0),
                    height: Pt(20.0),
                },
                Actions::uri(String::from("https://example.com")),
                None,
                None,
                None,
            )),
        }];
        assert_eq!(translated(&ops, &empty_fonts()), "");
    }

    #[test]
    fn unknown_font_is_an_error() {
        let missing = FontId(String::from("F9"));
        let write = vec![Op::WriteText {
            items: vec![TextItem::Text(String::from("x"))],
            font: missing.clone(),
        }];
        let error = translate(&write, &empty_fonts()).unwrap_err();
        assert!(matches!(error, PdfError::Font(_)), "{error}");

        let size = vec![Op::SetFontSize {
            size: Pt(12.0),
            font: missing,
        }];
        let error = translate(&size, &empty_fonts()).unwrap_err();
        assert!(matches!(error, PdfError::Font(_)), "{error}");
    }

    #[test]
    fn empty_ops_translate_to_empty_stream() {
        assert_eq!(translate(&[], &empty_fonts()).unwrap(), b"");
    }

    /// Есть ли утилита в `PATH`: у `pdftotext` нет длинной формы `--version`,
    /// и ненулевой код на незнакомый флаг — не признак отсутствия утилиты.
    fn pdftotext_available() -> bool {
        Command::new("pdftotext").arg("--version").output().is_ok()
    }

    /// Текст без пробельных символов: pdftotext расставляет переводы строк и
    /// пробелы по позициям глифов (как в тестах `fonts.rs` и `session.rs`).
    fn strip(value: &str) -> String {
        value.chars().filter(|ch| !ch.is_whitespace()).collect()
    }

    /// Сквозной тест: операции с кириллицей → `translate` → `Session` (одна
    /// страница) → разбор lopdf и извлечение текста pdftotext.
    #[test]
    fn end_to_end_cyrillic_round_trip() {
        const TEXT: &str = "Привет, мир! 123";
        let used = used_of(TEXT);
        let font = prepared(TEXT);
        let registry = FontRegistry::new(vec![(FontId(String::from("F0")), font)]);
        let fonts = registry.by_id();
        let ops = vec![
            Op::StartTextSection,
            Op::SetFontSize {
                size: Pt(12.0),
                font: FontId(String::from("F0")),
            },
            Op::SetTextCursor {
                pos: Point {
                    x: Pt(72.0),
                    y: Pt(700.0),
                },
            },
            Op::WriteText {
                items: vec![TextItem::Text(String::from(TEXT))],
                font: FontId(String::from("F0")),
            },
            Op::EndTextSection,
        ];
        let content = translate(&ops, &fonts).unwrap();

        let mut bytes = Vec::new();
        {
            let mut session = Session::begin(&mut bytes, 1, true).unwrap();
            assert_eq!(
                session
                    .add_font(doc_converter_render::font::default_font_bytes(), &used)
                    .unwrap(),
                "F0"
            );
            session.write_page(&content, A4, &[]).unwrap();
            session.finish().unwrap();
        }

        let loaded = Document::load_mem(&bytes).unwrap();
        let (_, page_id) = loaded.get_pages().into_iter().next().unwrap();
        let page = loaded.get_dictionary(page_id).unwrap();
        let stream_id = page.get(b"Contents").unwrap().as_reference().unwrap();
        let stream = loaded.get_object(stream_id).unwrap().as_stream().unwrap();
        // Поток из одного предложения короткий, и сжатие (эвристика «только
        // при экономии» в `Stream::compress`) его не трогает — читаем как есть.
        assert_eq!(stream.get_plain_content().unwrap(), content);

        if !pdftotext_available() {
            eprintln!("пропуск pdftotext: утилиты нет в PATH (в CI ставится шагом «pdf tooling»)");
            return;
        }
        let path = std::env::temp_dir().join(format!(
            "doc-converter-pdf-content-{}.pdf",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).unwrap();
        let output = Command::new("pdftotext")
            .arg(&path)
            .arg("-")
            .output()
            .expect("pdftotext запускается");
        std::fs::remove_file(&path).unwrap();

        let extracted = String::from_utf8_lossy(&output.stdout);
        assert_eq!(strip(&extracted), strip(TEXT), "извлечено: {extracted:?}");
    }
}
