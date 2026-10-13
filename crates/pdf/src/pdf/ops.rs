//! Модель операций страницы PDF (ADR-0022).
//!
//! Типы повторяют имена и формы printpdf (вендор 0.8.2): перевод вызывающего
//! кода — `use printpdf::…` → `use crate::pdf::…` — механический. Здесь только
//! то, что нужно операциям, которые эмитит продукт, и будущему `annot.rs`;
//! `serde` не нужен: модель живёт внутри процесса, в файл она попадает через
//! `content.rs` и словари страницы.
// XLSX-путь подключён, но часть операций эмитит только DOCX-путь (на printpdf
// до следующей волны, ADR-0022), поэтому dead_code глушится здесь; снять
// вместе с переводом docx-пути.
#![allow(dead_code)]

use std::fmt;
use std::num::FpCategory;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};

/// Расстояние в пунктах (1/72 дюйма) — единица координат страницы PDF.
#[derive(Debug, Default, Copy, Clone, PartialOrd)]
pub struct Pt(pub f32);

impl From<f32> for Pt {
    fn from(value: f32) -> Pt {
        Pt(value)
    }
}

/// Расстояние в миллиметрах.
#[derive(Debug, Default, Copy, Clone, PartialOrd)]
pub struct Mm(pub f32);

/// Расстояние в пикселях; в пункты переводится только с dpi ([`Px::into_pt`]).
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Px(pub usize);

impl From<Pt> for Mm {
    fn from(value: Pt) -> Mm {
        Mm(value.0 * 0.352_778_f32)
    }
}

impl From<Mm> for Pt {
    fn from(value: Mm) -> Pt {
        Pt(value.0 * 2.834_646_f32)
    }
}

impl Px {
    /// Пиксели в пункты при заданном dpi.
    #[must_use]
    // Размеры картинок далеки от 2^53: потеря точности неразличима в пунктах.
    #[allow(clippy::cast_precision_loss)]
    pub fn into_pt(self, dpi: f32) -> Pt {
        Mm(self.0 as f32 * (25.4 / dpi)).into()
    }
}

/// Сравнение расстояний с округлением до тысячных: переводы единиц дают
/// разницу в последнем разряде, и точное сравнение превращало бы её в
/// неравенство — как и в printpdf.
macro_rules! impl_rounded_eq {
    ($t:ty) => {
        impl PartialEq for $t {
            fn eq(&self, other: &Self) -> bool {
                if (self.0.classify() == FpCategory::Zero
                    || self.0.classify() == FpCategory::Normal)
                    && (other.0.classify() == FpCategory::Zero
                        || other.0.classify() == FpCategory::Normal)
                {
                    (self.0 * 1000.0).round() == (other.0 * 1000.0).round()
                } else {
                    false
                }
            }
        }
    };
}

impl_rounded_eq!(Pt);
impl_rounded_eq!(Mm);

/// Сложение и вычитание одноимённых расстояний.
macro_rules! impl_units_arith {
    ($t:ty) => {
        impl Add for $t {
            type Output = Self;

            fn add(self, other: Self) -> Self {
                Self(self.0 + other.0)
            }
        }
        impl AddAssign for $t {
            fn add_assign(&mut self, other: Self) {
                self.0 += other.0;
            }
        }
        impl Sub for $t {
            type Output = Self;

            fn sub(self, other: Self) -> Self {
                Self(self.0 - other.0)
            }
        }
        impl SubAssign for $t {
            fn sub_assign(&mut self, other: Self) {
                self.0 -= other.0;
            }
        }
    };
}

/// Масштабирование и деление на безразмерный `f32` — только для `Pt` и `Mm`:
/// у пикселя масштаб не безразмерный, он задаётся dpi в [`Px::into_pt`].
macro_rules! impl_units_scale {
    ($t:ty) => {
        impl Mul<f32> for $t {
            type Output = Self;

            fn mul(self, other: f32) -> Self {
                Self(self.0 * other)
            }
        }
        impl MulAssign<f32> for $t {
            fn mul_assign(&mut self, other: f32) {
                self.0 *= other;
            }
        }
        impl Div<$t> for $t {
            type Output = f32;

            fn div(self, other: $t) -> f32 {
                self.0 / other.0
            }
        }
        impl Div<f32> for $t {
            type Output = Self;

            fn div(self, other: f32) -> Self {
                Self(self.0 / other)
            }
        }
        impl DivAssign<f32> for $t {
            fn div_assign(&mut self, other: f32) {
                self.0 /= other;
            }
        }
    };
}

impl_units_arith!(Pt);
impl_units_arith!(Mm);
impl_units_arith!(Px);

impl_units_scale!(Pt);
impl_units_scale!(Mm);

/// Точка на странице; начало координат — левый нижний угол, единицы — пункты.
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct Point {
    /// Горизонтальная координата.
    pub x: Pt,
    /// Вертикальная координата.
    pub y: Pt,
}

impl Point {
    /// Точка из миллиметров.
    ///
    /// **Внимание: точка отсчёта — левый нижний угол, не верхний левый.**
    #[must_use]
    pub fn new(x: Mm, y: Mm) -> Self {
        Self {
            x: x.into(),
            y: y.into(),
        }
    }
}

/// Точка пути: вершина либо опорная точка кривой Безье.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinePoint {
    /// Положение точки.
    pub p: Point,
    /// `true` — точка ведёт кривую Безье (нужны две подряд + конечная).
    pub bezier: bool,
}

/// Ломаная с участками Безье; замыкается по [`Line::is_closed`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    /// Точки ломаной.
    pub points: Vec<LinePoint>,
    /// Замкнуть ли ломаную перед обводкой.
    pub is_closed: bool,
}

/// Многоугольник: один или несколько колец, режим заливки и правило обхода.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Polygon {
    /// Кольца пути; дырки — дополнительные кольца.
    pub rings: Vec<PolygonRing>,
    /// Режим отрисовки: заливка, обводка, клип или их сочетание.
    pub mode: PaintMode,
    /// Правило обхода для заливки и клипа.
    pub winding_order: WindingOrder,
}

/// Кольцо многоугольника — замкнутая цепочка точек.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PolygonRing {
    /// Точки кольца.
    pub points: Vec<LinePoint>,
}

/// Правило обхода пути при заливке и клипе.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindingOrder {
    /// Чётно-нечётное правило.
    EvenOdd,
    /// Ненулевое правило обхода.
    #[default]
    NonZero,
}

impl WindingOrder {
    /// Оператор клипа по правилу обхода.
    #[must_use]
    pub fn get_clip_op(self) -> &'static str {
        match self {
            WindingOrder::NonZero => "W",
            WindingOrder::EvenOdd => "W*",
        }
    }

    /// Оператор заливки по правилу обхода.
    #[must_use]
    pub fn get_fill_op(self) -> &'static str {
        match self {
            WindingOrder::NonZero => "f",
            WindingOrder::EvenOdd => "f*",
        }
    }

    /// Оператор «замкнуть, залить и обвести» по правилу обхода.
    #[must_use]
    pub fn get_fill_stroke_close_op(self) -> &'static str {
        match self {
            WindingOrder::NonZero => "b",
            WindingOrder::EvenOdd => "b*",
        }
    }

    /// Оператор «залить и обвести» по правилу обхода.
    #[must_use]
    pub fn get_fill_stroke_op(self) -> &'static str {
        match self {
            WindingOrder::NonZero => "B",
            WindingOrder::EvenOdd => "B*",
        }
    }
}

/// Режим отрисовки пути.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum PaintMode {
    /// Путь не рисуется, а становится клипом.
    Clip,
    /// Залить путь.
    #[default]
    Fill,
    /// Обвести путь.
    Stroke,
    /// Залить и обвести.
    FillStroke,
}

/// Прямоугольник `(x, y, width, height)` от левого нижнего угла страницы,
/// в пунктах.
#[derive(Clone, PartialEq)]
pub struct Rect {
    /// Левый край.
    pub x: Pt,
    /// Нижний край.
    pub y: Pt,
    /// Ширина.
    pub width: Pt,
    /// Высота.
    pub height: Pt,
}

impl fmt::Debug for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{} @ {} - {}",
            self.width.0, self.height.0, self.x.0, self.y.0
        )
    }
}

impl Rect {
    /// Левый нижний угол.
    #[must_use]
    pub fn lower_left(&self) -> Point {
        Point {
            x: self.x,
            y: self.y,
        }
    }

    /// Правый верхний угол.
    #[must_use]
    pub fn upper_right(&self) -> Point {
        Point {
            x: self.x + self.width,
            y: self.y + self.height,
        }
    }

    /// Прямоугольник от начала координат с заданными размерами.
    #[must_use]
    pub fn from_wh(width: Pt, height: Pt) -> Self {
        Self {
            x: Pt(0.0),
            y: Pt(0.0),
            width,
            height,
        }
    }
}

/// Id ICC-профиля; каркас под совместимость сигнатур printpdf — профили в
/// файл пока не пишутся.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IccProfileId(pub String);

/// Цвет; все компоненты нормированы в `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    /// RGB, нормированный.
    Rgb(Rgb),
    /// CMYK, нормированный.
    Cmyk(Cmyk),
    /// Серый, нормированный.
    Greyscale(Greyscale),
    /// Спот-цвет: в PDF кодируется как CMYK.
    SpotColor(SpotColor),
}

impl Color {
    /// За пределами `0.0..=1.0`? PDF такие значения принимает, но печать
    /// выйдет непредсказуемой.
    #[must_use]
    pub fn is_out_of_range(&self) -> bool {
        match self {
            Color::Rgb(rgb) => rgb.is_out_of_range(),
            Color::Cmyk(cmyk) => cmyk.is_out_of_range(),
            Color::Greyscale(gs) => gs.is_out_of_range(),
            Color::SpotColor(spot) => spot.is_out_of_range(),
        }
    }

    /// Компоненты в порядке операндов оператора цвета.
    #[must_use]
    pub fn to_vec(&self) -> Vec<f32> {
        match self {
            Color::Rgb(rgb) => vec![rgb.r, rgb.g, rgb.b],
            Color::Cmyk(cmyk) => vec![cmyk.c, cmyk.m, cmyk.y, cmyk.k],
            Color::Greyscale(gs) => vec![gs.percent],
            Color::SpotColor(spot) => vec![spot.c, spot.m, spot.y, spot.k],
        }
    }
}

/// RGB-цвет; компоненты `0.0..=1.0`, не `0..=255`.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgb {
    /// Красный.
    pub r: f32,
    /// Зелёный.
    pub g: f32,
    /// Синий.
    pub b: f32,
    /// Опциональный ICC-профиль.
    pub icc_profile: Option<IccProfileId>,
}

impl Rgb {
    /// Новый RGB-цвет; компоненты `0.0..=1.0`, не `0..=255`.
    #[must_use]
    pub fn new(r: f32, g: f32, b: f32, icc_profile: Option<IccProfileId>) -> Self {
        Self {
            r,
            g,
            b,
            icc_profile,
        }
    }

    /// За пределами `0.0..=1.0`?
    #[must_use]
    pub fn is_out_of_range(&self) -> bool {
        self.r < 0.0 || self.r > 1.0 || self.g < 0.0 || self.g > 1.0 || self.b < 0.0 || self.b > 1.0
    }
}

/// CMYK-цвет; компоненты `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cmyk {
    /// Голубой.
    pub c: f32,
    /// Пурпурный.
    pub m: f32,
    /// Жёлтый.
    pub y: f32,
    /// Чёрный.
    pub k: f32,
    /// Опциональный ICC-профиль.
    pub icc_profile: Option<IccProfileId>,
}

impl Cmyk {
    /// Новый CMYK-цвет; компоненты `0.0..=1.0`.
    #[must_use]
    pub fn new(c: f32, m: f32, y: f32, k: f32, icc_profile: Option<IccProfileId>) -> Self {
        Self {
            c,
            m,
            y,
            k,
            icc_profile,
        }
    }

    /// За пределами `0.0..=1.0`?
    #[must_use]
    pub fn is_out_of_range(&self) -> bool {
        self.c < 0.0
            || self.c > 1.0
            || self.m < 0.0
            || self.m > 1.0
            || self.y < 0.0
            || self.y > 1.0
            || self.k < 0.0
            || self.k > 1.0
    }
}

/// Серый цвет; компонент `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Greyscale {
    /// Яркость.
    pub percent: f32,
    /// Опциональный ICC-профиль.
    pub icc_profile: Option<IccProfileId>,
}

impl Greyscale {
    /// Новый серый цвет; компонент `0.0..=1.0`.
    #[must_use]
    pub fn new(percent: f32, icc_profile: Option<IccProfileId>) -> Self {
        Self {
            percent,
            icc_profile,
        }
    }

    /// За пределами `0.0..=1.0`?
    #[must_use]
    pub fn is_out_of_range(&self) -> bool {
        self.percent < 0.0 || self.percent > 1.0
    }
}

/// Спот-цвет: те же компоненты, что CMYK.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct SpotColor {
    /// Голубой.
    pub c: f32,
    /// Пурпурный.
    pub m: f32,
    /// Жёлтый.
    pub y: f32,
    /// Чёрный.
    pub k: f32,
}

impl SpotColor {
    /// Новый спот-цвет; компоненты `0.0..=1.0`.
    #[must_use]
    pub fn new(c: f32, m: f32, y: f32, k: f32) -> Self {
        Self { c, m, y, k }
    }

    /// За пределами `0.0..=1.0`?
    #[must_use]
    pub fn is_out_of_range(&self) -> bool {
        self.c < 0.0
            || self.c > 1.0
            || self.m < 0.0
            || self.m > 1.0
            || self.y < 0.0
            || self.y > 1.0
            || self.k < 0.0
            || self.k > 1.0
    }
}

/// Текущая матрица трансформации (CTM) графического состояния PDF.
///
/// Матрица действует на все последующие операции рисования через `cm` и
/// умножается на текущую (кумулятивна); сменить её целиком может только
/// `Raw`.
#[derive(Debug, Copy, Clone)]
pub enum CurTransMat {
    /// Перенос на `(x, y)` пунктов от левого нижнего угла.
    Translate(Pt, Pt),
    /// Поворот по часовой стрелке в градусах вокруг начала координат.
    Rotate(f32),
    /// Поворот вокруг точки `(x, y)` в дополнение к текущей матрице.
    TranslateRotate(Pt, Pt, f32),
    /// Масштаб по осям (`1.0` — без изменения).
    Scale(f32, f32),
    /// Сырая PDF-матрица `[a b c d e f]`.
    Raw([f32; 6]),
    /// Единичная матрица.
    Identity,
}

impl PartialEq for CurTransMat {
    fn eq(&self, other: &Self) -> bool {
        self.as_array() == other.as_array()
    }
}

impl CurTransMat {
    /// Матрица как `[a b c d e f]` для оператора `cm`/`Tm`.
    #[must_use]
    pub fn as_array(&self) -> [f32; 6] {
        match self {
            CurTransMat::Translate(x, y) => [1.0, 0.0, 0.0, 1.0, x.0, y.0],
            CurTransMat::TranslateRotate(x, y, rot) => {
                let rad = (360.0 - rot).to_radians();
                [rad.cos(), -rad.sin(), rad.sin(), rad.cos(), x.0, y.0]
            }
            CurTransMat::Rotate(rot) => {
                let rad = (360.0 - rot).to_radians();
                [rad.cos(), -rad.sin(), rad.sin(), rad.cos(), 0.0, 0.0]
            }
            CurTransMat::Raw(r) => *r,
            CurTransMat::Scale(x, y) => [*x, 0.0, 0.0, *y, 0.0, 0.0],
            CurTransMat::Identity => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        }
    }

    /// Произведение двух PDF-матриц: `a · b`, результат снова `[a b c d e f]`.
    #[must_use]
    pub fn combine_matrix(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
        // Матрицы 3×3 в гомогенных координатах: последняя строка всегда
        // `0 0 1`, поэтому она не хранится.
        let a = [
            [a[0], a[1], 0.0, 0.0],
            [a[2], a[3], 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [a[4], a[5], 0.0, 1.0],
        ];
        let b = [
            [b[0], b[1], 0.0, 0.0],
            [b[2], b[3], 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [b[4], b[5], 0.0, 1.0],
        ];
        let mut result = [[0.0_f32; 4]; 4];
        for row in 0..4 {
            for col in 0..4 {
                result[row][col] = mul_add(
                    a[row][0],
                    b[0][col],
                    mul_add(
                        a[row][1],
                        b[1][col],
                        mul_add(a[row][2], b[2][col], a[row][3] * b[3][col]),
                    ),
                );
            }
        }
        [
            result[0][0],
            result[0][1],
            result[1][0],
            result[1][1],
            result[3][0],
            result[3][1],
        ]
    }
}

/// `(a * b) + c` через собственную функцию: `{f32}::mul_add` сломан на
/// arm-unknown-linux-gnueabi (rust-lang/rust#46950), а результат нужен
/// одинаковый на всех целях.
fn mul_add(a: f32, b: f32, c: f32) -> f32 {
    if cfg!(all(
        target_arch = "arm",
        target_os = "linux",
        target_env = "gnu"
    )) {
        (a * b) + c
    } else {
        a.mul_add(b, c)
    }
}

/// Текстовая матрица для оператора `Tm`.
///
/// В отличие от CTM, `Tm` **заменяет** текущую текстовую матрицу целиком,
/// поэтому «повернуть на месте» нельзя: поворот требует и координат.
#[derive(Debug, Copy, Clone)]
pub enum TextMatrix {
    /// Перенос текстового курсора: эквивалент `SetTextCursor`.
    Translate(Pt, Pt),
    /// Поворот текста вокруг точки страницы.
    TranslateRotate(Pt, Pt, f32),
    /// Сырая матрица `[a b c d e f]` оператора `Tm`.
    Raw([f32; 6]),
}

impl PartialEq for TextMatrix {
    fn eq(&self, other: &Self) -> bool {
        self.as_array() == other.as_array()
    }
}

impl TextMatrix {
    /// Матрица как `[a b c d e f]` для оператора `Tm`.
    #[must_use]
    pub fn as_array(&self) -> [f32; 6] {
        match self {
            TextMatrix::Translate(x, y) => [1.0, 0.0, 0.0, 1.0, x.0, y.0],
            TextMatrix::Raw(r) => *r,
            TextMatrix::TranslateRotate(x, y, rot) => {
                let rad = (360.0 - rot).to_radians();
                [rad.cos(), -rad.sin(), rad.sin(), rad.cos(), x.0, y.0]
            }
        }
    }
}

/// Шаблон штриха линии: до трёх пар «штрих-пробел»; `None` — пара не задана.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineDashPattern {
    /// Смещение начала шаблона от начала линии.
    pub offset: i64,
    /// Длина первого штриха; `None` — линия сплошная.
    pub dash_1: Option<i64>,
    /// Пробел после первого штриха; `None` — как `dash_1`.
    pub gap_1: Option<i64>,
    /// Длина второго штриха; `None` — как `dash_1`.
    pub dash_2: Option<i64>,
    /// Пробел после второго штриха; `None` — как `gap_1`.
    pub gap_2: Option<i64>,
    /// Длина третьего штриха; `None` — как `dash_1`.
    pub dash_3: Option<i64>,
    /// Пробел после третьего штриха; `None` — как `gap_1`.
    pub gap_3: Option<i64>,
}

impl LineDashPattern {
    /// Заданные пары «штрих-пробел» подряд; хвостовые `None` отброшены.
    #[must_use]
    pub fn as_array(&self) -> Vec<i64> {
        [
            self.dash_1,
            self.gap_1,
            self.dash_2,
            self.gap_2,
            self.dash_3,
            self.gap_3,
        ]
        .iter()
        .copied()
        .take_while(Option::is_some)
        .flatten()
        .collect()
    }

    /// Шаблон из массива длин (до шести; дальше игнорируется) и смещения.
    #[must_use]
    pub fn from_array(dashes: &[i64], offset: i64) -> Self {
        let mut pattern = LineDashPattern {
            offset,
            ..Default::default()
        };
        match dashes.len() {
            0 => {}
            1 => pattern.dash_1 = Some(dashes[0]),
            2 => {
                pattern.dash_1 = Some(dashes[0]);
                pattern.gap_1 = Some(dashes[1]);
            }
            3 => {
                pattern.dash_1 = Some(dashes[0]);
                pattern.gap_1 = Some(dashes[1]);
                pattern.dash_2 = Some(dashes[2]);
            }
            4 => {
                pattern.dash_1 = Some(dashes[0]);
                pattern.gap_1 = Some(dashes[1]);
                pattern.dash_2 = Some(dashes[2]);
                pattern.gap_2 = Some(dashes[3]);
            }
            5 => {
                pattern.dash_1 = Some(dashes[0]);
                pattern.gap_1 = Some(dashes[1]);
                pattern.dash_2 = Some(dashes[2]);
                pattern.gap_2 = Some(dashes[3]);
                pattern.dash_3 = Some(dashes[4]);
            }
            _ => {
                pattern.dash_1 = Some(dashes[0]);
                pattern.gap_1 = Some(dashes[1]);
                pattern.dash_2 = Some(dashes[2]);
                pattern.gap_2 = Some(dashes[3]);
                pattern.dash_3 = Some(dashes[4]);
                pattern.gap_3 = Some(dashes[5]);
            }
        }
        pattern
    }
}

/// Id картинки (Image-XObject) в ресурсах.
#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Hash)]
pub struct XObjectId(pub String);

/// Id шрифта в ресурсах.
#[derive(Debug, PartialEq, Hash, Clone, Eq, PartialOrd, Ord)]
pub struct FontId(pub String);

/// Id расширенного графического состояния (`ExtGState`).
#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Hash)]
pub struct ExtendedGraphicsStateId(pub String);

/// Расширенное графическое состояние (`/ExtGState`).
///
/// printpdf несёт весь набор полей ISO 32000 (8.4.5); продукту нужно только
/// прозрачность водяного знака (`/CA`/`/ca`), остальные поля остаются по
/// умолчанию и в словарь не пишутся.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtendedGraphicsState {
    /// `/CA` — непрозрачность обводки в `[0, 1]`.
    pub current_stroke_alpha: f32,
    /// `/ca` — непрозрачность заливки в `[0, 1]`.
    pub current_fill_alpha: f32,
}

impl Default for ExtendedGraphicsState {
    fn default() -> Self {
        Self {
            current_stroke_alpha: 1.0,
            current_fill_alpha: 1.0,
        }
    }
}

/// Трансформ, применяемый только к картинке перед отрисовкой.
#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct XObjectTransform {
    /// Сдвиг по горизонтали, в пунктах.
    pub translate_x: Option<Pt>,
    /// Сдвиг по вертикали, в пунктах.
    pub translate_y: Option<Pt>,
    /// Поворот по часовой стрелке, в градусах.
    pub rotate: Option<XObjectRotation>,
    /// Масштаб по горизонтали (`1.0` — без изменения).
    pub scale_x: Option<f32>,
    /// Масштаб по вертикали (`1.0` — без изменения).
    pub scale_y: Option<f32>,
    /// DPI картинки; `None` — 300.0.
    pub dpi: Option<f32>,
}

impl XObjectTransform {
    /// Матрицы трансформа в порядке применения: размер картинки при dpi
    /// (если известен размер), масштаб, поворот вокруг центра, перенос.
    ///
    /// `wh` — размер картинки в пикселях; `None` — шаг размера пропускается.
    #[must_use]
    pub fn get_ctms(&self, wh: Option<(Px, Px)>) -> Vec<CurTransMat> {
        let mut transforms = Vec::new();
        let dpi = self.dpi.unwrap_or(300.0);

        if let Some((w, h)) = wh {
            // PDF кладёт картинку в квадрат 1×1; растянуть до её размера в
            // пунктах, где 1 px = 1 pt при заданном dpi.
            transforms.push(CurTransMat::Scale(w.into_pt(dpi).0, h.into_pt(dpi).0));
        }

        if self.scale_x.is_some() || self.scale_y.is_some() {
            let scale_x = self.scale_x.unwrap_or(1.0);
            let scale_y = self.scale_y.unwrap_or(1.0);
            transforms.push(CurTransMat::Scale(scale_x, scale_y));
        }

        if let Some(rotate) = self.rotate {
            transforms.push(CurTransMat::Translate(
                Pt(-rotate.rotation_center_x.into_pt(dpi).0),
                Pt(-rotate.rotation_center_y.into_pt(dpi).0),
            ));
            transforms.push(CurTransMat::Rotate(rotate.angle_ccw_degrees));
            transforms.push(CurTransMat::Translate(
                rotate.rotation_center_x.into_pt(dpi),
                rotate.rotation_center_y.into_pt(dpi),
            ));
        }

        if self.translate_x.is_some() || self.translate_y.is_some() {
            transforms.push(CurTransMat::Translate(
                self.translate_x.unwrap_or(Pt(0.0)),
                self.translate_y.unwrap_or(Pt(0.0)),
            ));
        }

        transforms
    }
}

/// Поворот картинки: угол и центр.
#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct XObjectRotation {
    /// Угол против часовой стрелки, в градусах.
    pub angle_ccw_degrees: f32,
    /// Горизонтальная координата центра поворота, в пикселях.
    pub rotation_center_x: Px,
    /// Вертикальная координата центра поворота, в пикселях.
    pub rotation_center_y: Px,
}

/// Сегмент текста либо сдвиг между сегментами.
#[derive(Debug, Clone, PartialEq)]
pub enum TextItem {
    /// Сегмент текста.
    Text(String),
    /// Сдвиг в тысячных em.
    Offset(f32),
}

/// Аннотация страницы: подтип и его нагрузка.
///
/// Подтип — данные, а не отдельная операция: `/Link` и `/Text` сериализуются
/// одной веткой, и следующий подтип добавится вариантом здесь. Нагрузка
/// варианта обязательна, поэтому «аннотация без действия» невыразима.
#[derive(Debug, PartialEq, Clone)]
pub enum Annotation {
    /// Ссылка: прямоугольник, оформление и действие (`/URI` или `/GoTo`).
    Link(LinkAnnotation),
    /// Текстовая заметка: текст в `/Contents`, автор в `/T`.
    Text(TextAnnotation),
}

/// Ссылка `/Link`.
#[derive(Debug, PartialEq, Clone)]
pub struct LinkAnnotation {
    /// Прямоугольник на странице в пунктах.
    pub rect: Rect,
    /// Действие по клику.
    pub actions: Actions,
    /// Оформление рамки.
    pub border: BorderArray,
    /// Цвет рамки.
    pub color: ColorArray,
    /// Режим подсветки.
    pub highlighting: HighlightingMode,
}

impl LinkAnnotation {
    /// Новая ссылка; `None` — оформление по умолчанию.
    #[must_use]
    pub fn new(
        rect: Rect,
        actions: Actions,
        border: Option<BorderArray>,
        color: Option<ColorArray>,
        highlighting: Option<HighlightingMode>,
    ) -> Self {
        Self {
            rect,
            border: border.unwrap_or_default(),
            color: color.unwrap_or_default(),
            actions,
            highlighting: highlighting.unwrap_or_default(),
        }
    }
}

/// Текстовая заметка (`/Subtype /Text`) — примечание ячейки XLSX.
#[derive(Debug, PartialEq, Clone)]
pub struct TextAnnotation {
    /// Прямоугольник на странице в пунктах, как у [`LinkAnnotation::rect`].
    pub rect: Rect,
    /// Текст заметки — PDF-ключ `/Contents`.
    pub contents: String,
    /// Автор — PDF-ключ `/T`; `None` — автора в книге нет, выдумывать нельзя.
    pub title: Option<String>,
}

/// Рамка аннотации: сплошная либо штриховая с фазой.
#[derive(Debug, PartialEq, Clone)]
pub enum BorderArray {
    /// Сплошная: ширина и два неиспользуемых компонента.
    Solid([f32; 3]),
    /// Штриховая: `[ширина, стиль, счётчик]` и фаза.
    Dashed([f32; 3], DashPhase),
}

impl BorderArray {
    /// Массив `/Border` целиком, включая фазу штриха.
    #[must_use]
    pub fn to_array(&self) -> Vec<f32> {
        match self {
            BorderArray::Solid(s) => s.to_vec(),
            BorderArray::Dashed(s, dash_phase) => {
                let mut values = s.to_vec();
                values.push(dash_phase.phase);
                values
            }
        }
    }
}

impl Default for BorderArray {
    fn default() -> Self {
        BorderArray::Solid([0.0, 0.0, 1.0])
    }
}

/// Шаблон и фаза штриховой рамки.
#[derive(Debug, PartialEq, Clone)]
pub struct DashPhase {
    /// Длины штрихов.
    pub dash_array: Vec<f32>,
    /// Фаза шаблона.
    pub phase: f32,
}

/// Цвет рамки аннотации (`/C`).
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ColorArray {
    /// Без рамки.
    Transparent,
    /// Серый.
    Gray([f32; 1]),
    /// RGB.
    Rgb([f32; 3]),
    /// CMYK.
    Cmyk([f32; 4]),
}

impl Default for ColorArray {
    fn default() -> Self {
        ColorArray::Rgb([0.0, 1.0, 1.0])
    }
}

impl ColorArray {
    /// Компоненты цвета для `/C`; у `Transparent` массив пуст — рамки нет.
    #[must_use]
    pub fn to_vec(self) -> Vec<f32> {
        match self {
            ColorArray::Transparent => Vec::new(),
            ColorArray::Gray(gray) => gray.to_vec(),
            ColorArray::Rgb(rgb) => rgb.to_vec(),
            ColorArray::Cmyk(cmyk) => cmyk.to_vec(),
        }
    }
}

/// Режим подсветки ссылки.
#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub enum HighlightingMode {
    /// Без подсветки.
    None,
    /// Инвертировать содержимое.
    #[default]
    Invert,
    /// Обвести рамкой.
    Outline,
    /// Вдавить.
    Push,
}

impl HighlightingMode {
    /// Буква `/H` из ISO 32000, таблица 166.
    #[must_use]
    pub fn get_id(self) -> &'static str {
        match self {
            HighlightingMode::None => "N",
            HighlightingMode::Invert => "I",
            HighlightingMode::Outline => "O",
            HighlightingMode::Push => "P",
        }
    }
}

/// Действие аннотации: переход по документу или по URI.
#[derive(Debug, PartialEq, Clone)]
pub enum Actions {
    /// Переход к месту в документе (`/GoTo`).
    Goto(Destination),
    /// Внешний адрес (`/URI`).
    Uri(String),
}

impl Actions {
    /// Имя подтипа действия (`/S`).
    #[must_use]
    pub fn get_action_type_id(&self) -> &'static str {
        match self {
            Actions::Goto(_) => "GoTo",
            Actions::Uri(_) => "URI",
        }
    }

    /// Переход внутри документа.
    #[must_use]
    pub fn go_to(destination: Destination) -> Self {
        Self::Goto(destination)
    }

    /// Внешняя ссылка.
    #[must_use]
    pub fn uri(uri: String) -> Self {
        Self::Uri(uri)
    }
}

/// Место в документе для действия `/GoTo`.
#[derive(Debug, PartialEq, Clone)]
pub enum Destination {
    /// Страница `page` (с единицы), верхний левый угол окна на `(left, top)`
    /// и масштаб `zoom`; `None` — оставить текущее значение.
    Xyz {
        /// Номер страницы, с единицы.
        page: usize,
        /// Горизонтальная координата окна.
        left: Option<f32>,
        /// Вертикальная координата окна.
        top: Option<f32>,
        /// Масштаб; `None` или `0` — оставить текущий.
        zoom: Option<f32>,
    },
}

/// Операция content stream страницы.
///
/// Варианты — ровно те, что эмитит продукт; `Annotation` в поток не пишется,
/// но живёт в общем списке операций страницы.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// `q`: сохранить графическое состояние на стек.
    SaveGraphicsState,
    /// `Q`: восстановить последнее сохранённое состояние.
    RestoreGraphicsState,
    /// `gs`: загрузить именованное расширенное графическое состояние.
    LoadGraphicsState { gs: ExtendedGraphicsStateId },
    /// `BT`: начать текстовую секцию.
    StartTextSection,
    /// `ET`: закончить текстовую секцию.
    EndTextSection,
    /// `Tj`/`TJ`: вывести текст текущими шрифтом и размером.
    WriteText { items: Vec<TextItem>, font: FontId },
    /// `Tf`: задать шрифт и его размер.
    SetFontSize { size: Pt, font: FontId },
    /// `Td`: перенести текстовый курсор.
    SetTextCursor { pos: Point },
    /// `rg`/`k`/`g`: цвет заливки текста и фигур.
    SetFillColor { col: Color },
    /// `RG`/`K`/`G`: цвет обводки.
    SetOutlineColor { col: Color },
    /// `w`: толщина обводки в пунктах.
    SetOutlineThickness { pt: Pt },
    /// `d`: шаблон штриха обводки.
    SetLineDashPattern { dash: LineDashPattern },
    /// `Tm`: текстовая матрица (заменяет текущую).
    SetTextMatrix { matrix: TextMatrix },
    /// Путь из отрезков и кривых, завершаемый обводкой (`S`).
    DrawLine { line: Line },
    /// Замкнутый путь; отрисовка по [`Polygon::mode`].
    DrawPolygon { polygon: Polygon },
    /// `q cm Do Q`: картинка (Image-XObject) с трансформом.
    UseXobject {
        /// Id картинки в ресурсах.
        id: XObjectId,
        /// Трансформ картинки.
        transform: XObjectTransform,
    },
    /// Аннотация страницы; в content stream не пишется (см. `annot.rs`).
    Annotation { annot: Annotation },
}

#[cfg(test)]
mod tests {
    // Тесты сверяют матрицы с точными ожидаемыми значениями (эталоны — те же,
    // что в printpdf): все они представимы точно, вычислений с погрешностью в
    // сравнении нет.
    #![allow(clippy::float_cmp)]

    use super::{
        CurTransMat, LineDashPattern, Mm, PaintMode, Point, Pt, Rect, TextMatrix, WindingOrder,
    };

    #[test]
    fn mm_and_pt_convert_like_printpdf() {
        let mm: Mm = Pt(1.0).into();
        assert_eq!(mm, Mm(0.352_778_f32));
        let mm: Mm = Pt(15.0).into();
        assert_eq!(mm, Mm(5.291_67_f32));

        let pt: Pt = Mm(1.0).into();
        assert_eq!(pt, Pt(2.834_646_f32));
        let pt: Pt = Mm(23.0).into();
        assert_eq!(pt, Pt(65.196_9_f32));
    }

    #[test]
    fn pt_arithmetic_scales_and_divides() {
        assert_eq!(Pt(3.0) + Pt(4.0), Pt(7.0));
        assert_eq!(Pt(3.0) - Pt(4.0), Pt(-1.0));
        assert_eq!(Pt(3.0) * 2.0, Pt(6.0));
        assert_eq!(Pt(6.0) / 2.0, Pt(3.0));
        assert_eq!(Pt(6.0) / Pt(2.0), 3.0_f32);
    }

    #[test]
    fn cur_trans_mat_arrays_match_pdf_layout() {
        assert_eq!(
            CurTransMat::Translate(Pt(150.0), Pt(50.0)).as_array(),
            [1.0_f32, 0.0, 0.0, 1.0, 150.0, 50.0]
        );
        assert_eq!(
            CurTransMat::Scale(2.0, 4.0).as_array(),
            [2.0_f32, 0.0, 0.0, 4.0, 0.0, 0.0]
        );
        assert_eq!(
            CurTransMat::Raw([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).as_array(),
            [1.0_f32, 2.0, 3.0, 4.0, 5.0, 6.0]
        );
        assert_eq!(
            CurTransMat::Identity.as_array(),
            [1.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0]
        );
        let rotated = CurTransMat::Rotate(30.0).as_array();
        assert!((rotated[0] - 0.866_025_4).abs() < 1e-6, "{rotated:?}");
        assert!((rotated[1] - 0.500_000_2).abs() < 1e-6, "{rotated:?}");
    }

    #[test]
    fn combine_matrix_matches_manual_product() {
        // Перенос на (10, 20) после единичной — сам перенос.
        let combined = CurTransMat::combine_matrix(
            CurTransMat::Identity.as_array(),
            CurTransMat::Translate(Pt(10.0), Pt(20.0)).as_array(),
        );
        assert_eq!(combined, [1.0_f32, 0.0, 0.0, 1.0, 10.0, 20.0]);
    }

    #[test]
    fn text_matrix_translate_is_identity_with_offset() {
        assert_eq!(
            TextMatrix::Translate(Pt(72.0), Pt(700.0)).as_array(),
            [1.0_f32, 0.0, 0.0, 1.0, 72.0, 700.0]
        );
        assert_eq!(
            TextMatrix::Raw([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).as_array(),
            [1.0_f32, 2.0, 3.0, 4.0, 5.0, 6.0]
        );
    }

    #[test]
    fn rect_corners_sum_sides() {
        let rect = Rect {
            x: Pt(10.0),
            y: Pt(20.0),
            width: Pt(30.0),
            height: Pt(40.0),
        };
        assert_eq!(
            rect.lower_left(),
            Point {
                x: Pt(10.0),
                y: Pt(20.0)
            }
        );
        assert_eq!(
            rect.upper_right(),
            Point {
                x: Pt(40.0),
                y: Pt(60.0)
            }
        );
        assert_eq!(
            Rect::from_wh(Pt(5.0), Pt(6.0)),
            Rect {
                x: Pt(0.0),
                y: Pt(0.0),
                width: Pt(5.0),
                height: Pt(6.0),
            }
        );
    }

    #[test]
    fn dash_array_drops_trailing_none() {
        let pattern = LineDashPattern {
            offset: 1,
            dash_1: Some(3),
            gap_1: Some(2),
            dash_2: Some(1),
            gap_2: None,
            dash_3: None,
            gap_3: None,
        };
        assert_eq!(pattern.as_array(), vec![3, 2, 1]);

        let solid = LineDashPattern::default();
        assert_eq!(solid.as_array(), Vec::<i64>::new());

        let from_array = LineDashPattern::from_array(&[4, 3, 2], 5);
        assert_eq!(from_array.offset, 5);
        assert_eq!(from_array.as_array(), vec![4, 3, 2]);
    }

    #[test]
    fn polygon_defaults_fill_with_nonzero_winding() {
        let polygon = super::Polygon::default();
        assert_eq!(polygon.mode, PaintMode::Fill);
        assert_eq!(polygon.winding_order, WindingOrder::NonZero);
    }

    #[test]
    fn winding_order_operators_match_iso() {
        assert_eq!(WindingOrder::NonZero.get_fill_op(), "f");
        assert_eq!(WindingOrder::EvenOdd.get_fill_op(), "f*");
        assert_eq!(WindingOrder::NonZero.get_fill_stroke_close_op(), "b");
        assert_eq!(WindingOrder::EvenOdd.get_fill_stroke_close_op(), "b*");
        assert_eq!(WindingOrder::NonZero.get_clip_op(), "W");
        assert_eq!(WindingOrder::EvenOdd.get_clip_op(), "W*");
    }
}
