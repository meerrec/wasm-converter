//! Рисунки DOCX: media-часть документа → `XObject` на его страницах.
//!
//! Раскладка рисунками не занимается: она отдаёт только прямоугольник и
//! `image_id` — номер записи в [`Document::media`]. Здесь эти номера
//! разбираются один раз в таблицу «`image_id` → `XObject`», и её спрашивают
//! оба диспетчера, рисующие элементы: [`super::draw_items`] для встроенных
//! рисунков и [`super::floats`] для плавающих.
//!
//! Регистрация отделена от отрисовки не для красоты: `XObject`'ы — ресурсы
//! документа, а `StreamSession` переводит `Op::UseXobject` по ним в момент
//! записи страницы. Значит, ресурсы обязаны быть заполнены до `begin`, то есть
//! до того, как страницы начнут собираться. Отсюда [`Images::collect`] —
//! проход по медиачастям до записи — и [`Images::ops`], чистая сборка операций
//! при отрисовке: страница собирается дважды (зонд глифов и запись), и второй
//! раз декодировать нечего.
//!
//! Байтов может не быть: `image_id` в раскладке — `Option`, и `None` означает
//! битую часть, внешнюю цель или часть выше предела. Такой рисунок не рисуется
//! (встроенный — вовсе, плавающий — остаётся плейсхолдером, как и раньше);
//! не рисуется и тот, чьи байты не принял декодер, — форматы кроме PNG и JPEG
//! экспорт не поддерживает. Ошибка одной картинки не роняет экспорт и не
//! сдвигает остальные: пропуск стоит только этой картинки.
//!
//! DOCX-путь пока на printpdf (ADR-0022, слайс 5), поэтому модуль несёт
//! собственную регистрацию в `printpdf::PdfResources` — общий модуль
//! [`crate::image`] обслуживает XLSX-путь на своём бэкенде и сюда не
//! годится: имена и операции у путей разные.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use doc_converter_docx::layout::Rect;
use doc_converter_docx::Document;
use printpdf::{
    LinePoint, Op, PaintMode, PdfResources, Point, Polygon, PolygonRing, Pt, Px, RawImage,
    Rect as PdfRect, WindingOrder, XObject, XObjectId, XObjectTransform,
};

use crate::layout::{PageGeometry, RectPx};

/// DPI, при котором первый масштаб из `XObjectTransform::get_ctms` —
/// тождественный (1 px = 1 pt). Дальше размер картинки на странице задают
/// только `scale_x`/`scale_y`, то есть растягивание на прямоугольник якоря.
const DPI: f32 = 72.0;

/// Изображения документа: `image_id` раскладки → зарегистрированный `XObject`.
///
/// Номер, которого в документе нет (раскладка и медиатаблица разошлись),
/// равнозначен пропуску: рисовать нечего.
pub(crate) struct Images {
    /// `None` — часть нашлась, но не декодировалась; такой рисунок пропускается.
    parts: BTreeMap<u32, Option<Registered>>,
}

impl Images {
    /// Регистрирует все медиачасти документа в ресурсах PDF.
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
    /// внутри [`register`].
    pub(crate) fn collect(doc: &Document, resources: &mut PdfResources) -> Self {
        let parts = doc
            .media()
            .iter()
            .map(|part| {
                let registered = register(resources, &part.mime, &part.bytes).ok();
                (part.id, registered)
            })
            .collect();
        Self { parts }
    }

    /// Операции вывода рисунка в прямоугольник раскладки: `q`, отсечение по
    /// прямоугольнику, `Do`, `Q`. Пусто — рисовать нечем.
    ///
    /// `rect` — прямоугольник в пикселях 96 dpi от левого верхнего угла
    /// страницы (как у остальных элементов раскладки), `offset_y` — сдвиг
    /// содержимого ячейки под `w:vAlign`.
    pub(crate) fn ops(
        &self,
        image_id: Option<u32>,
        rect: Rect,
        offset_y: f32,
        page: &PageGeometry,
    ) -> Vec<Op> {
        // Вырожденный прямоугольник (нулевой `extent` битого `wp:drawing`)
        // пропускается: `Registered::ops` делить на его стороны не должен.
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Vec::new();
        }
        let Some(registered) = image_id
            .and_then(|id| self.parts.get(&id))
            .and_then(Option::as_ref)
        else {
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
        registered.ops(&pdf)
    }
}

/// Изображение, уже лежащее в ресурсах документа: имя `XObject` и размеры
/// исходника в пикселях.
///
/// Разделение [`register`] и [`Registered::ops`] нужно, когда прямоугольники
/// размещения известны не сразу: `XObject`'ы — ресурсы документа, и запись
/// страниц (`StreamSession`) читает их в момент вывода, поэтому
/// зарегистрировать все картинки приходится до начала записи, а операции
/// собрать потом.
#[derive(Debug, Clone)]
struct Registered {
    /// Имя `XObject` в ресурсах документа.
    id: XObjectId,
    /// Ширина исходника в пикселях.
    width_px: usize,
    /// Высота исходника в пикселях.
    height_px: usize,
}

impl Registered {
    /// Операции вывода изображения в прямоугольник страницы (точки, начало
    /// координат — левый нижний угол).
    ///
    /// `rect` обязан быть невырожденным: стороны проверяет вызывающий
    /// ([`Images::ops`]), иначе нулевая сторона дала бы `NaN` в матрице.
    #[must_use]
    fn ops(&self, rect: &PdfRect) -> Vec<Op> {
        placement_ops(&self.id, self.width_px, self.height_px, rect)
    }
}

/// Кладёт изображение в ресурсы или находит уже собранный `XObject`.
///
/// Возвращает [`Registered`]: имя `XObject` и размеры исходника в пикселях.
/// Имя выводится из содержимого, поэтому одинаковые байты попадают в одну
/// запись ресурсов: [`RawImage`] переезжает по владению только при первом
/// размещении, повторы декодирования не требуют. Ключ — именно байты, а не id
/// media-части: строка id приходит от модели и ничем не подтверждена —
/// отпечаток байтов честнее и вдобавок склеивает одинаковые копии, разложенные
/// по разным частям пакета.
fn register(resources: &mut PdfResources, mime: &str, bytes: &[u8]) -> Result<Registered, String> {
    let id = xobject_id(bytes);
    if let Some((Px(width), Px(height))) = resources
        .xobjects
        .map
        .get(&id)
        .and_then(XObject::get_width_height)
    {
        return Ok(Registered {
            id,
            width_px: width,
            height_px: height,
        });
    }
    let image = decode(mime, bytes)?;
    let (width_px, height_px) = (image.width, image.height);
    resources
        .xobjects
        .map
        .insert(id.clone(), XObject::Image(image));
    Ok(Registered {
        id,
        width_px,
        height_px,
    })
}

/// Декодирует байты media-части в [`RawImage`].
///
/// Формат определяется по содержимому (`RawImage::decode_from_bytes` зовёт
/// `guess_format`) — mime нужен только чтобы отсеять заведомо неподдержанные
/// `image/*` до разбора байтов. Пустой mime и mime не-картинки не отсеивают:
/// часть могла приехать без `Content_Types`, и последнее слово всё равно за
/// содержимым.
fn decode(mime: &str, bytes: &[u8]) -> Result<RawImage, String> {
    let normalized = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if normalized.starts_with("image/") && !is_supported(&normalized) {
        return Err(format!("unsupported image format '{mime}'"));
    }
    RawImage::decode_from_bytes(bytes, &mut Vec::new())
        .map_err(|err| format!("cannot decode image: {err}"))
}

/// PNG и JPEG в написаниях, которые встречаются в книгах Office.
fn is_supported(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/x-png" | "image/jpeg" | "image/jpg" | "image/pjpeg"
    )
}

/// Имя `XObject` по содержимому media-части — ключ дедупликации.
///
/// `DefaultHasher` (SipHash-1-3) — 64 бита: на сотнях картинок книги коллизия
/// пренебрежимо маловероятна, а имя живёт только внутри одного сеанса сборки
/// PDF, поэтому межзапусковая стабильность хеша не нужна. Строка имени — из
/// одних hex-цифр: она становится именем ресурса в `/XObject`, и экранирование
/// спецсимволов пути media-части тут ни к чему.
fn xobject_id(bytes: &[u8]) -> XObjectId {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    XObjectId(format!("img{:016x}", hasher.finish()))
}

/// Операции отрисовки зарегистрированного `XObject`: `q`, отсечение, `Do`, `Q`.
///
/// Картинка растягивается на `rect` независимыми масштабами по осям — так же,
/// как её кладёт canvas-путь; пропорции исходника при этом не сохраняются.
/// Отсечение по прямоугольнику якоря остаётся: у Image-`XObject` нет ни
/// `/BBox`, ни `/Rect`, зато клип гарантирует, что пиксели не выйдут за якорь
/// ни при каких обстоятельствах.
fn placement_ops(id: &XObjectId, width_px: usize, height_px: usize, rect: &PdfRect) -> Vec<Op> {
    let transform = XObjectTransform {
        dpi: Some(DPI),
        // Растягивание точно на якорь: так же картинку кладёт canvas-путь.
        scale_x: Some(rect.width.0 / px_to_pt(width_px)),
        scale_y: Some(rect.height.0 / px_to_pt(height_px)),
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

/// `RectPt` (начало — левый верхний угол страницы) в printpdf-`Rect`
/// (начало — левый нижний угол).
///
/// Двойник `RectPt::to_pdf` из `layout.rs`: тот отдаёт `Rect` своего бэкенда,
/// а DOCX-путь на printpdf собирает свой.
fn to_pdf(rect: crate::layout::RectPt, page_height_pt: f32) -> PdfRect {
    PdfRect {
        x: Pt(rect.x),
        y: Pt(page_height_pt - rect.y - rect.h),
        width: Pt(rect.w),
        height: Pt(rect.h),
    }
}

/// Пиксели изображения в точки при [`DPI`]: 1 px = 1 pt.
///
/// Погрешность `f32` (после 2^24 px по стороне) заведомо ниже точности
/// печати; картинок больше 16 млн пикселей по стороне не бывает.
#[allow(clippy::cast_precision_loss)]
fn px_to_pt(px: usize) -> f32 {
    px as f32
}
