//! WASM-обвязка DOCX: открыть документ, померить страницы, собрать кадр.
//!
//! Раскладка считается один раз при открытии ([`docx_open`]): она дорогая —
//! каскад стилей и перенос строк по метрикам шрифта, — а документ после
//! открытия не меняется, и кадр любой страницы собирается из готового
//! [`PageLayout`]. Как и у XLSX, документ живёт в `thread_local!` воркера:
//! воркер однопоточный и обслуживает одно окно, а команды кадра уезжают прямо
//! в слот ring'а, минуя `postMessage`.
//!
//! Форма статистики сборки и разбор окна — общие с XLSX ([`crate::xlsx_api`]):
//! контракт кадра у обоих форматов один, и дублировать его незачем.

use std::cell::RefCell;

use doc_converter_docx::layout::layout_document;
use doc_converter_docx::{Document, LayoutOptions, PageLayout};
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::font::FontRegistry;
use doc_converter_render::sab::SabRing;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::xlsx_api::{to_js, viewport_of, BuildStats};

/// Размер LRU-кэша метрик шрифта.
///
/// Раскладка меряет текст одним шрифтом по умолчанию, но на нескольких кеглях
/// и со сотнями различных символов на страницу: 4096 записей — это уже
/// «больше, чем символов в документе», и кэш перестаёт промахиваться на
/// повторяющихся буквах, не вырастая вместе с документом.
const FONT_CACHE: usize = 4096;

thread_local! {
    /// Открытый документ: `None`, пока ничего не открыто.
    static DOC: RefCell<Option<OpenDoc>> = const { RefCell::new(None) };
    /// Буфер кадра: переиспользуется, чтобы не аллоцировать на каждый кадр.
    static FRAME: RefCell<DisplayList> = RefCell::new(DisplayList::new());
}

/// Открытый документ: разобранная модель, реестр шрифтов и готовая раскладка.
struct OpenDoc {
    /// Модель документа. Кадру она не нужна — раскладка уже посчитана, — но
    /// следующим слайсам (картинки, поиск по тексту) без неё не обойтись, и
    /// держать её живой дешевле, чем разбирать пакет заново.
    #[allow(dead_code)]
    document: Document,
    /// Реестр, которым мерялась раскладка; остаётся, чтобы повторная раскладка
    /// (другой кегль, другой шрифт) не регистрировала шрифты заново.
    #[allow(dead_code)]
    fonts: FontRegistry,
    /// Раскладка страниц: из неё собирается каждый кадр.
    layout: PageLayout,
}

/// Описание страницы для интерфейса: размеры в пикселях раскладки — по ним
/// интерфейс строит прокрутку, как по размерам листа XLSX.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    width: f32,
    height: f32,
}

/// Открыть документ из байтов: разобрать, разложить по страницам, запомнить.
///
/// Раскладка (`LayoutOptions::default()`) считается здесь и один раз: ширина
/// полосы набора задана страницей документа, а не окном, поэтому окно и зум
/// на неё не влияют и приходят только с кадром. Предыдущий документ
/// закрывается.
///
/// # Errors
/// Если байты не OOXML-пакет, `word/document.xml` не разбирается или раскладка
/// упирается в [`doc_converter_docx::LayoutError`].
#[wasm_bindgen]
pub fn docx_open(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    let document = doc_converter_docx::open(bytes).map_err(to_js)?;
    let mut fonts = FontRegistry::new(FONT_CACHE);
    let layout =
        layout_document(&document, &LayoutOptions::default(), &mut fonts).map_err(to_js)?;
    let pages: Vec<PageInfo> = layout
        .pages
        .iter()
        .map(|page| PageInfo {
            width: page.width,
            height: page.height,
        })
        .collect();

    DOC.with(|doc| {
        *doc.borrow_mut() = Some(OpenDoc {
            document,
            fonts,
            layout,
        })
    });
    serde_wasm_bindgen::to_value(&pages).map_err(to_js)
}

/// Закрыть документ и освободить память.
#[wasm_bindgen]
pub fn docx_close() {
    DOC.with(|doc| *doc.borrow_mut() = None);
    FRAME.with(|frame| frame.borrow_mut().clear());
}

/// Открыт ли документ. Нужно интерфейсу, чтобы не рисовать пустоту.
#[wasm_bindgen]
pub fn docx_is_open() -> bool {
    DOC.with(|doc| doc.borrow().is_some())
}

/// Собрать кадр страницы в буфер и вернуть его байты.
///
/// Нужен там, где `SharedArrayBuffer` недоступен: при отсутствии
/// COOP/COEP-заголовков кадр уезжает в воркер копией.
///
/// Номера страниц приходят из интерфейса, поэтому страница за пределами
/// документа даёт пустой кадр, а не ошибку: так решено в
/// [`doc_converter_docx::render::build`].
///
/// # Errors
/// Если документ не открыт или окно описано неверно.
#[wasm_bindgen]
pub fn docx_build_display_list(page: usize, viewport: JsValue) -> Result<Vec<u8>, JsValue> {
    let viewport = viewport_of(viewport)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let open = doc.as_ref().ok_or_else(|| to_js("no document is open"))?;
        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            doc_converter_docx::render::build(&open.layout, page, viewport, &mut frame);
            Ok(frame.to_bytes())
        })
    })
}

/// Собрать кадр страницы сразу в слот ring'а.
///
/// Возвращает `written: false`, если свободного слота не было: кадр теряется,
/// но следующий за ним рисуется — так и задумано, догонять прокрутку
/// устаревшими кадрами незачем.
///
/// # Errors
/// Если документ не открыт, окно описано неверно или SAB не тот.
#[wasm_bindgen]
pub fn docx_build_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
    page: usize,
    viewport: JsValue,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let started = js_sys::Date::now();

    let stats = DOC.with(|doc| -> Result<BuildStats, JsValue> {
        let doc = doc.borrow();
        let open = doc.as_ref().ok_or_else(|| to_js("no document is open"))?;

        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            doc_converter_docx::render::build(&open.layout, page, viewport, &mut frame);
            let cmds = u32::try_from(frame.len()).unwrap_or(u32::MAX);

            let mut ring = SabRing::from_sab(sab, slot_capacity as usize)?;
            let written = ring.try_write(&frame.to_bytes())?;
            Ok(BuildStats::new(
                written,
                cmds,
                js_sys::Date::now() - started,
            ))
        })
    })?;

    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Открыть документ из байтов, отдав их без копирования.
///
/// # Errors
/// То же, что у [`docx_open`].
#[wasm_bindgen]
pub fn docx_open_bytes(bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
    docx_open(bytes.to_vec())
}
