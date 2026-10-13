//! Рисунки DOCX: media-часть документа → `XObject` на его страницах.
//!
//! Раскладка рисунками не занимается: она отдаёт только прямоугольник и
//! `image_id` — номер записи в [`Document::media`]. Здесь эти номера
//! разбираются один раз в таблицу «`image_id` → декодированная картинка», и её
//! спрашивают оба диспетчера, рисующие элементы: [`super::draw_items`] для
//! встроенных рисунков и [`super::floats`] для плавающих.
//!
//! Регистрация отделена от отрисовки не для красоты: `XObject`'ы — ресурсы
//! документа, а сессия ([`crate::pdf::session::Session::add_image`]) принимает
//! их только до первой страницы. Значит, декодировать все части обязаны
//! до начала записи ([`Images::collect`]), а операции страница собирает потом
//! ([`Images::ops`]), по именам, которые назначила сессия ([`Images::assign`]):
//! страница рисуется дважды (зонд глифов и запись), и второй раз декодировать
//! нечего.
//!
//! Байтов может не быть: `image_id` в раскладке — `Option`, и `None` означает
//! битую часть, внешнюю цель или часть выше предела. Такой рисунок не рисуется
//! (встроенный — вовсе, плавающий — остаётся плейсхолдером, как и раньше);
//! не рисуется и тот, чьи байты не принял декодер, — форматы кроме PNG и JPEG
//! экспорт не поддерживает. Ошибка одной картинки не роняет экспорт и не
//! сдвигает остальные: пропуск стоит только этой картинки.
//!
//! Декодирование — общий модуль своего бэкенда ([`crate::pdf::image`], ADR-0022);
//! здесь живёт только реестр «байты → имя `XObject`» для документов DOCX —
//! имена раздаёт сессия, а не сам модуль, как у листа XLSX (`crate::image`).

use std::collections::BTreeMap;

use doc_converter_docx::layout::Rect;
use doc_converter_docx::Document;

use crate::layout::{PageGeometry, RectPx};
use crate::pdf::image::DecodedImage;
use crate::pdf::ops::{
    LinePoint, Op, PaintMode, Point, Polygon, PolygonRing, Pt, Rect as PdfRect, WindingOrder,
    XObjectId, XObjectTransform,
};

/// DPI, при котором размер картинки в пикселях совпадает с её размером в
/// точках (1 px = 1 pt). Транслятор не знает размеров `XObject`'а и шага
/// «масштаб по размеру картинки» не делает: в матрице остаётся только
/// растягивание на прямоугольник якоря, которое от пиксельных размеров не
/// зависит.
const DPI: f32 = 72.0;

/// Изображения документа: `image_id` раскладки → декодированная картинка.
///
/// Номер, которого в документе нет (раскладка и медиатаблица разошлись),
/// равнозначен пропуску: рисовать нечего.
pub(crate) struct Images {
    /// `None` — часть нашлась, но не декодировалась; такой рисунок пропускается.
    parts: BTreeMap<u32, Option<usize>>,
    /// Уникальные декодированные картинки в порядке первой встречи — порядок
    /// регистрации в сессии.
    decoded: Vec<DecodedImage>,
    /// Имя `XObject` по индексу `decoded`; назначает экспорт после
    /// [`crate::pdf::session::Session::add_image`] ([`Images::assign`]).
    names: Vec<Option<XObjectId>>,
}

impl Images {
    /// Декодирует все медиачасти документа в картинки для сессии.
    ///
    /// Регистрируются все прочитанные части, а не только видимые на страницах:
    /// в [`Document::media`] попадают лишь те, на которые ссылается рисунок
    /// (`parse::read_media`), — то есть каждая из них на какой-нибудь странице
    /// да стоит. Отбирать их по раскладке было бы точнее, но обход страниц
    /// пришлось бы повторять за диспетчером, и промах стоил бы пропавшего
    /// рисунка; лишний же `XObject` в ресурсах безвреден, а часть, потерянная
    /// раскладкой (сегодня — встроенный рисунок в ячейке таблицы, см.
    /// `layout::tables::MeasuredBlock`), иначе и вовсе не оставила бы следа.
    /// Одинаковые байты двух частей дают один `XObject`: склейка по содержимому
    /// внутри [`decode`].
    pub(crate) fn collect(doc: &Document) -> Self {
        let mut decoded = Vec::new();
        let mut names = Vec::new();
        let mut index_by_bytes: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        let parts = doc
            .media()
            .iter()
            .map(|part| {
                let registered = match index_by_bytes.get(part.bytes.as_slice()) {
                    Some(&index) => Some(index),
                    None => decode(&part.mime, &part.bytes).map(|image| {
                        let index = decoded.len();
                        index_by_bytes.insert(part.bytes.clone(), index);
                        decoded.push(image);
                        names.push(None);
                        index
                    }),
                };
                (part.id, registered)
            })
            .collect();
        Self {
            parts,
            decoded,
            names,
        }
    }

    /// Декодированные картинки в порядке регистрации — аргументы
    /// [`crate::pdf::session::Session::add_image`].
    #[must_use]
    pub(crate) fn decoded(&self) -> &[DecodedImage] {
        &self.decoded
    }

    /// Назначить имя `XObject` картинке с индексом в порядке регистрации.
    ///
    /// Имя отдаёт сессия («X0», «X1», …); вызов за пределами числа картинок
    /// игнорируется — рассинхрон порядка значил бы баг экспорта, а не битый PDF.
    pub(crate) fn assign(&mut self, index: usize, name: XObjectId) {
        if let Some(slot) = self.names.get_mut(index) {
            *slot = Some(name);
        }
    }

    /// Операции вывода рисунка в прямоугольник раскладки: `q`, отсечение по
    /// прямоугольнику, `Do`, `Q`. Пусто — рисовать нечем.
    ///
    /// `rect` — прямоугольник в пикселях 96 dpi от левого верхнего угла
    /// страницы (как у остальных элементов раскладки), `offset_y` — сдвиг
    /// содержимого ячейки под `w:vAlign`. Пусто возвращается и до
    /// [`Images::assign`]: зондовый проход глифов идёт раньше сессии, и имён
    /// ещё нет — его операции всё равно выбрасываются.
    pub(crate) fn ops(
        &self,
        image_id: Option<u32>,
        rect: Rect,
        offset_y: f32,
        page: &PageGeometry,
    ) -> Vec<Op> {
        // Вырожденный прямоугольник (нулевой `extent` битого `wp:drawing`)
        // пропускается: `placement_ops` делить на его стороны не должен.
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Vec::new();
        }
        let Some(&Some(index)) = image_id.and_then(|id| self.parts.get(&id)) else {
            return Vec::new();
        };
        let Some(name) = self.names.get(index).and_then(Option::as_ref) else {
            return Vec::new();
        };
        let pdf = to_pdf(
            page.rect_to_pt(RectPx::new(
                rect.x,
                rect.y + offset_y,
                rect.width,
                rect.height,
            )),
            page.height_pt(),
        );
        placement_ops(name, &pdf)
    }
}

/// Декодирует байты media-части своим бэкендом ([`crate::pdf::image::decode`]).
///
/// Формат определяется по содержимому, mime нужен только чтобы отсеять
/// заведомо неподдержанные `image/*` до разбора байтов. Пустой mime и mime
/// не-картинки не отсеивают: часть могла приехать без `Content_Types`, и
/// последнее слово всё равно за содержимым.
fn decode(mime: &str, bytes: &[u8]) -> Option<DecodedImage> {
    crate::pdf::image::decode(mime, bytes).ok()
}

/// Операции отрисовки зарегистрированного `XObject`: `q`, отсечение, `Do`, `Q`.
///
/// Картинка растягивается на `rect` независимыми масштабами по осям — так же,
/// как её кладёт canvas-путь; пропорции исходника при этом не сохраняются.
/// PDF кладёт `Image`-`XObject` в квадрат 1×1, поэтому масштаб — сами стороны
/// прямоугольника (произведение «размер исходника × растягивание» от
/// пиксельных размеров не зависит). Отсечение по прямоугольнику якоря
/// остаётся: у Image-`XObject` нет ни `/BBox`, ни `/Rect`, зато клип
/// гарантирует, что пиксели не выйдут за якорь ни при каких обстоятельствах.
fn placement_ops(id: &XObjectId, rect: &PdfRect) -> Vec<Op> {
    let transform = XObjectTransform {
        dpi: Some(DPI),
        scale_x: Some(rect.width.0),
        scale_y: Some(rect.height.0),
        translate_x: Some(rect.x),
        translate_y: Some(rect.y),
        rotate: None,
    };
    vec![
        Op::SaveGraphicsState,
        Op::DrawPolygon {
            polygon: clip_rect(rect),
        },
        Op::UseXobject {
            id: id.clone(),
            transform,
        },
        Op::RestoreGraphicsState,
    ]
}

/// Путь отсечения по прямоугольнику в координатах страницы (левый нижний угол).
fn clip_rect(rect: &PdfRect) -> Polygon {
    let right = Pt(rect.x.0 + rect.width.0);
    let top = Pt(rect.y.0 + rect.height.0);
    let points = [
        (rect.x, rect.y),
        (right, rect.y),
        (right, top),
        (rect.x, top),
    ]
    .into_iter()
    .map(|(x, y)| LinePoint {
        p: Point { x, y },
        bezier: false,
    })
    .collect();
    Polygon {
        rings: vec![PolygonRing { points }],
        mode: PaintMode::Clip,
        winding_order: WindingOrder::NonZero,
    }
}

/// `RectPt` (начало — левый верхний угол страницы) в `Rect` своего бэкенда
/// (начало — левый нижний угол).
fn to_pdf(rect: crate::layout::RectPt, page_height_pt: f32) -> PdfRect {
    PdfRect {
        x: Pt(rect.x),
        y: Pt(page_height_pt - rect.y - rect.h),
        width: Pt(rect.w),
        height: Pt(rect.h),
    }
}
