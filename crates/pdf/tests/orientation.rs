//! Ориентация страницы листа XLSX: из `<pageSetup>` книги или из явной
//! настройки `PdfOptions`.
//!
//! Книги собираются в памяти публичным API `doc-converter-xlsx`: среди фикстур
//! `test-fixtures/xlsx` листов с объявленной альбомной ориентацией нет (все
//! генерируются с `orientation="portrait"`), а зависеть от реальных книг
//! пользователя тест не должен — они живут вне репозитория.

use doc_converter_pdf::{PageOrientation, PdfExporter, PdfOptions};
use doc_converter_xlsx::{
    Orientation, PrintSettings, SharedStrings, Sheet, SheetContent, SheetState, StyleTable, Theme,
    Workbook, WorksheetMeta,
};

/// Точки A4 без поворота: 210x297 мм при 72 pt/дюйм.
const A4_W_PT: f32 = 595.3;
const A4_H_PT: f32 = 841.9;

/// Книга из одного пустого листа с заданной ориентацией печати.
fn workbook(orientation: Orientation) -> Workbook {
    let meta = WorksheetMeta {
        name: "Лист1".to_owned(),
        part: "xl/worksheets/sheet1.xml".to_owned(),
        state: SheetState::Visible,
    };
    let content = SheetContent {
        print: PrintSettings {
            orientation,
            ..PrintSettings::default()
        },
        ..SheetContent::default()
    };
    Workbook::new(
        vec![Sheet::new(meta, content)],
        SharedStrings::default(),
        StyleTable::default(),
        Theme::default(),
        false,
    )
}

/// Размер первой страницы PDF в точках: ширина и высота `/MediaBox`.
fn page_size(bytes: &[u8]) -> (f32, f32) {
    let pdf = lopdf::Document::load_mem(bytes).expect("PDF разбирается lopdf");
    let page_id = *pdf.get_pages().values().next().expect("страница есть");
    let page = pdf.get_dictionary(page_id).expect("словарь страницы");
    let media = page
        .get(b"MediaBox")
        .expect("/MediaBox")
        .as_array()
        .expect("/MediaBox — массив");
    let value = |i: usize| media[i].as_float().expect("число");
    (value(2) - value(0), value(3) - value(1))
}

/// Экспортировать лист настройками и вернуть размер его первой страницы.
fn export(book: &Workbook, options: PdfOptions) -> (f32, f32) {
    let bytes = PdfExporter::new(options)
        .export_xlsx_sheet(book, 0)
        .expect("лист экспортируется");
    page_size(&bytes)
}

#[test]
fn landscape_sheet_prints_landscape_by_default() {
    let (w, h) = export(&workbook(Orientation::Landscape), PdfOptions::default());
    assert!(w > h, "альбомный лист развернулся: {w}x{h}");
    assert!(
        (w - A4_H_PT).abs() < 1.0 && (h - A4_W_PT).abs() < 1.0,
        "A4 альбомная: {w}x{h}"
    );
}

#[test]
fn explicit_portrait_wins_over_landscape_sheet() {
    let mut options = PdfOptions::default();
    options.page.orientation = PageOrientation::Portrait;
    let (w, h) = export(&workbook(Orientation::Landscape), options);
    assert!(h > w, "явный портрет сильнее листа: {w}x{h}");
    assert!(
        (w - A4_W_PT).abs() < 1.0 && (h - A4_H_PT).abs() < 1.0,
        "A4 книжная: {w}x{h}"
    );
}

#[test]
fn explicit_landscape_wins_over_sheet_default() {
    let mut options = PdfOptions::default();
    options.page.orientation = PageOrientation::Landscape;
    let (w, h) = export(&workbook(Orientation::Default), options);
    assert!(w > h, "явный альбом сильнее листа: {w}x{h}");
}

#[test]
fn sheet_without_orientation_stays_portrait() {
    let (w, h) = export(&workbook(Orientation::Default), PdfOptions::default());
    assert!(h > w, "лист без настройки — книжный: {w}x{h}");
    assert!(
        (w - A4_W_PT).abs() < 1.0 && (h - A4_H_PT).abs() < 1.0,
        "A4 книжная: {w}x{h}"
    );
}
