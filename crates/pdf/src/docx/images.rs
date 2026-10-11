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
//! до того, как страницы начнут собираться (так же устроен лист XLSX,
//! `painter::image_ops`). Отсюда [`Images::collect`] — проход по медиачастям до
//! записи — и [`Images::ops`], чистая сборка операций при отрисовке: страница
//! собирается дважды (зонд глифов и запись), и второй раз декодировать нечего.
//!
//! Байтов может не быть: `image_id` в раскладке — `Option`, и `None` означает
//! битую часть, внешнюю цель или часть выше предела. Такой рисунок не рисуется
//! (встроенный — вовсе, плавающий — остаётся плейсхолдером, как и раньше);
//! не рисуется и тот, чьи байты не принял декодер, — форматы кроме PNG и JPEG
//! экспорт не поддерживает ([`crate::image::DecodeError`]). Ошибка одной
//! картинки не роняет экспорт и не сдвигает остальные: как и у XLSX
//! (`image::place_all`), пропуск стоит только этой картинки.

use std::collections::BTreeMap;

use doc_converter_docx::layout::Rect;
use doc_converter_docx::Document;
use printpdf::{Op, PdfResources};

use crate::image::{self, Registered};
use crate::layout::{PageGeometry, RectPx};

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
    /// внутри [`image::register`].
    pub(crate) fn collect(doc: &Document, resources: &mut PdfResources) -> Self {
        let parts = doc
            .media()
            .iter()
            .map(|part| {
                let registered = image::register(resources, &part.mime, &part.bytes).ok();
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
        let pdf = page
            .rect_to_pt(RectPx::new(
                rect.x,
                rect.y + offset_y,
                rect.width,
                rect.height,
            ))
            .to_pdf(page.height_pt());
        registered.ops(&pdf)
    }
}
