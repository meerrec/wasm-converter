//! Начертания шрифтов PDF и их идентификаторы в ресурсах документа.
//!
//! Байты берутся из `doc-converter-render` — тот же Carlito, которым canvas
//! измеряет текст (`render::font`). Один шрифт на экран и на PDF — залог
//! совпадения переносов (ADR-0007); своей копии TTF в pdf-крейте нет.
//!
//! Встраиваются только те начертания, что встретились у ячеек страницы:
//! каждое подмножество весит около 90 КБ, и возить неиспользованные незачем.
//! Метрики при этом общие — regular (`DEFAULT_FONT_ID`), как у canvas-пути:
//! начертание выбирает встраиваемый шрифт записи текста, а не точки переноса.
//!
//! Подрезку по использованным символам делает [`crate::pdf::fonts`] (ADR-0022);
//! здесь живут только набор начертаний и их идентификаторы в ресурсах —
//! [`EmbeddedFonts`] строит те же имена («F0», «F1», …), что раздаёт сессия,
//! поэтому зондовый проход painter'а работает ещё до [`crate::pdf::session::Session::begin`].

use doc_converter_render::font::{
    bold_font_bytes, bold_italic_font_bytes, default_font_bytes, italic_font_bytes,
};

use crate::pdf::ops::FontId;

/// Начертание шрифта — то, что записано в `styles.xml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Face {
    /// Начертание по флагам стиля ячейки.
    #[must_use]
    pub const fn of(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => Self::Regular,
            (true, false) => Self::Bold,
            (false, true) => Self::Italic,
            (true, true) => Self::BoldItalic,
        }
    }

    /// Место начертания в [`FaceSet`].
    const fn index(self) -> usize {
        match self {
            Self::Regular => 0,
            Self::Bold => 1,
            Self::Italic => 2,
            Self::BoldItalic => 3,
        }
    }

    /// Байты Carlito этого начертания — аргумент подрезки сессии.
    #[must_use]
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::Regular => default_font_bytes(),
            Self::Bold => bold_font_bytes(),
            Self::Italic => italic_font_bytes(),
            Self::BoldItalic => bold_italic_font_bytes(),
        }
    }
}

/// Набор начертаний, встретившихся на страницах листа.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FaceSet([bool; 4]);

impl FaceSet {
    /// Добавить начертание в набор.
    pub fn insert(&mut self, face: Face) {
        self.0[face.index()] = true;
    }

    /// Есть ли начертание в наборе.
    #[must_use]
    pub fn contains(self, face: Face) -> bool {
        self.0[face.index()]
    }

    /// Добавить все начертания другого набора.
    ///
    /// Наборы собираются по листам, а шрифты встраиваются один раз на
    /// документ: книге нужен общий набор, а не по листу.
    pub fn merge(&mut self, other: Self) {
        for (slot, present) in self.0.iter_mut().zip(other.0) {
            *slot |= present;
        }
    }
}

/// Шрифты страниц: regular всегда, выделенные начертания — как встретились.
///
/// Идентификаторы — наши [`FontId`] в ресурсах документа («F0», «F1», … в
/// порядке начертаний). Имена строит [`EmbeddedFonts::from_faces`] до начала
/// сессии: [`crate::pdf::session::Session`] раздаёт имена тем же порядком, и
/// зондовый проход, идущий раньше [`crate::pdf::session::Session::begin`],
/// может ссылаться на шрифты, которых ещё нет.
#[derive(Debug)]
pub struct EmbeddedFonts {
    regular: FontId,
    bold: Option<FontId>,
    italic: Option<FontId>,
    bold_italic: Option<FontId>,
}

impl EmbeddedFonts {
    /// Идентификаторы начертаний набора — до регистрации в сессии.
    ///
    /// Имена «F0», «F1», … раздаются в фиксированном порядке начертаний
    /// Regular → Bold → Italic → `BoldItalic`; сессия раздаёт имена тем же
    /// счётчиком, поэтому после `add_font` они совпадут по построению.
    #[must_use]
    pub fn from_faces(faces: FaceSet) -> Self {
        let mut next = 0;
        let mut take = || {
            let font = FontId(format!("F{next}"));
            next += 1;
            font
        };
        let regular = take();
        let bold = faces.contains(Face::Bold).then(&mut take);
        let italic = faces.contains(Face::Italic).then(&mut take);
        let bold_italic = faces.contains(Face::BoldItalic).then(&mut take);
        Self {
            regular,
            bold,
            italic,
            bold_italic,
        }
    }

    /// Шрифт начертания; невстроенное начертание заменяется regular.
    ///
    /// Замена — страховка на случай, если спрашивают начертание, которого в
    /// наборе не было: пустой записи текста PDF не примет.
    #[must_use]
    pub fn id(&self, face: Face) -> &FontId {
        let embedded = match face {
            Face::Regular => None,
            Face::Bold => self.bold.as_ref(),
            Face::Italic => self.italic.as_ref(),
            Face::BoldItalic => self.bold_italic.as_ref(),
        };
        embedded.unwrap_or(&self.regular)
    }

    /// Начертание по идентификатору; неизвестный — regular.
    ///
    /// Идентификаторы отдаёт сам [`EmbeddedFonts::id`], поэтому посторонних
    /// значений в операциях не бывает: обратное сопоставление нужно зондовому
    /// проходу, который копит символы по начертаниям.
    #[must_use]
    pub fn face(&self, id: &FontId) -> Face {
        if self.bold.as_ref() == Some(id) {
            Face::Bold
        } else if self.italic.as_ref() == Some(id) {
            Face::Italic
        } else if self.bold_italic.as_ref() == Some(id) {
            Face::BoldItalic
        } else {
            Face::Regular
        }
    }
}
