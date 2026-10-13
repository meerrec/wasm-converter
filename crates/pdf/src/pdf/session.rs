//! Сессия постраничной записи PDF: глобальные объекты — один раз, страница —
//! сразу в приёмник, xref — одной секцией в конце (ADR-0022, слайс 3).
//!
//! Перенос `StreamSession` из `vendor/printpdf/src/streaming.rs` (наш код
//! форка, ADR-0010) на свой писатель: `lopdf::Document` служит только
//! реестром идентификаторов, объекты страниц в нём не задерживаются. Версия и
//! бинарная метка заголовка повторяют printpdf (`Document::with_version("1.3")`
//! и метка lopdf по умолчанию) — продукт не должен сменить формат файла при
//! переходе на свой бэкенд.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream,
    StringFormat::{Hexadecimal, Literal},
};

use crate::pdf::fonts;
use crate::pdf::image;
use crate::pdf::writer::StreamWriter;
use crate::PdfError;

/// Producer документа: продукт представляется сам — printpdf писал пустую
/// строку, по файлу нельзя было понять, чем он собран.
const PRODUCER: &str = "doc-converter";

/// Сессия постраничной записи: `begin` — глобальные объекты, `write_page` на
/// каждую страницу, `finish` — хвост и xref.
pub(crate) struct Session<'w, W: Write> {
    writer: StreamWriter<&'w mut W>,
    /// Реестр идентификаторов и место сборки объектов шрифтов и картинок
    /// ([`fonts::write_font`], [`image::write_image`]): всё, что сюда попало,
    /// немедленно уходит в писатель и удаляется, иначе объекты копились бы до
    /// `finish`.
    doc: Document,
    pages_id: ObjectId,
    /// id страниц зарезервированы в `begin`: на последующие страницы уже
    /// ссылаются аннотации (`/Dest`), а `lopdf::Document` раздаёт id только
    /// возрастающие.
    page_ids: Vec<ObjectId>,
    /// Сколько страниц записано: `page_ids[..written]` — готовые, остальные
    /// станут свободными записями xref.
    written: usize,
    /// Имя в ресурсах («F0», «F1», …) → id словаря Type0.
    font_dict: BTreeMap<String, ObjectId>,
    /// Подготовленные шрифты по имени ресурса — аргумент `content::translate`.
    prepared: BTreeMap<String, fonts::PreparedFont>,
    /// Словарь `/Font` ушёл в писатель: добавлять шрифты позже нельзя.
    fonts_written: bool,
    font_dict_id: ObjectId,
    /// Имя в ресурсах («X0», «X1», …) → id картинки.
    xobject_dict: BTreeMap<String, ObjectId>,
    /// Имя в ресурсах («GS0», «GS1», …) → id `ExtGState`.
    extgstate_dict: BTreeMap<String, ObjectId>,
    /// Словари `/XObject` и `/ExtGState` ушли в писатель: добавлять ресурсы
    /// позже нельзя.
    resource_dicts_written: bool,
    xobject_dict_id: ObjectId,
    extgstate_dict_id: ObjectId,
    title: Option<String>,
    /// Закладки одним уровнем: заголовок → индекс страницы; пишутся в `finish`.
    outline: Vec<(String, usize)>,
}

impl<'w, W: Write> Session<'w, W> {
    /// Начинает файл: заголовок и резервирование id дерева страниц, страниц и
    /// глобальных словарей ресурсов.
    ///
    /// `page_count` — сколько страниц будет записано: id резервируются сразу,
    /// чтобы ссылки на будущие страницы были разрешены до первой страницы;
    /// незаписанные id останутся свободными записями xref.
    ///
    /// # Errors
    ///
    /// Ошибка записи в приёмник.
    pub(crate) fn begin(
        writer: &'w mut W,
        page_count: usize,
        compress: bool,
    ) -> Result<Self, PdfError> {
        let mut doc = Document::with_version("1.3");
        let mut writer = StreamWriter::new(writer, compress);
        writer.begin(&doc.version, &doc.binary_mark)?;

        let pages_id = doc.new_object_id();
        let page_ids = (0..page_count).map(|_| doc.new_object_id()).collect();
        let font_dict_id = doc.new_object_id();
        let xobject_dict_id = doc.new_object_id();
        let extgstate_dict_id = doc.new_object_id();

        // Словари ресурсов пишутся не здесь, а перед первой страницей
        // (`flush_resource_dicts`), как и `/Font`: наполнять их разрешено до
        // первой страницы, поэтому в файл они уходят с содержимым, а не пустыми.

        Ok(Self {
            writer,
            doc,
            pages_id,
            page_ids,
            written: 0,
            font_dict: BTreeMap::new(),
            prepared: BTreeMap::new(),
            fonts_written: false,
            font_dict_id,
            xobject_dict: BTreeMap::new(),
            extgstate_dict: BTreeMap::new(),
            resource_dicts_written: false,
            xobject_dict_id,
            extgstate_dict_id,
            title: None,
            outline: Vec::new(),
        })
    }

    /// Заголовок документа в `/Info` (до `finish`).
    pub(crate) fn set_title(&mut self, title: &str) {
        self.title = Some(title.to_owned());
    }

    /// Подрезает шрифт по `used` и добавляет его глобально; возвращает имя в
    /// ресурсах («F0», «F1», …). Вызывать только до первой страницы.
    ///
    /// # Errors
    ///
    /// [`PdfError::Font`], если байты не разбираются как TrueType, `subsetter`
    /// отказал на таблицах, или шрифт добавляется после первой страницы:
    /// словарь `/Font` уже записан, и поздний шрифт молча выпал бы из файла.
    pub(crate) fn add_font(
        &mut self,
        bytes: &[u8],
        used: &BTreeSet<char>,
    ) -> Result<String, PdfError> {
        if self.fonts_written {
            return Err(PdfError::Font(String::from(
                "font added after the first page was written",
            )));
        }
        let prepared = fonts::prepare(bytes, used)?;
        let type0_id = fonts::write_font(&prepared, &mut self.doc);
        self.flush_objects()?;

        let name = format!("F{}", self.font_dict.len());
        self.font_dict.insert(name.clone(), type0_id);
        self.prepared.insert(name.clone(), prepared);
        Ok(name)
    }

    /// Подготовленные шрифты по имени ресурса («F0», …) — для `translate`.
    #[must_use]
    pub(crate) fn prepared_fonts(&self) -> &BTreeMap<String, fonts::PreparedFont> {
        &self.prepared
    }

    /// Зарегистрировать изображение в глобальном словаре `/XObject`, вернуть
    /// имя («X0», «X1», …). Вызывать только до первой страницы.
    ///
    /// # Errors
    ///
    /// [`PdfError::Image`], если изображение добавляется после первой
    /// страницы: словарь `/XObject` уже записан, и поздняя картинка молча
    /// выпала бы из файла.
    pub(crate) fn add_image(&mut self, img: &image::DecodedImage) -> Result<String, PdfError> {
        if self.written > 0 {
            return Err(PdfError::Image(String::from(
                "image added after the first page was written",
            )));
        }
        let xobject_id = image::write_image(img, &mut self.doc);
        self.flush_objects()?;

        let name = format!("X{}", self.xobject_dict.len());
        self.xobject_dict.insert(name.clone(), xobject_id);
        Ok(name)
    }

    /// Зарегистрировать `ExtGState` (готовый словарь) в глобальном словаре
    /// `/ExtGState` под именем, которое выбирает вызывающий (например,
    /// `WatermarkAlpha` у painter'а). Вызывать только до первой страницы.
    ///
    /// # Errors
    ///
    /// Ошибка записи в приёмник; [`PdfError::Io`] с
    /// [`io::ErrorKind::InvalidInput`], если состояние добавляется после
    /// первой страницы или под именем, которое уже занято.
    pub(crate) fn add_graphics_state(
        &mut self,
        name: &str,
        dict: Dictionary,
    ) -> Result<(), PdfError> {
        if self.written > 0 {
            return Err(PdfError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "graphics state added after the first page was written",
            )));
        }
        if self.extgstate_dict.contains_key(name) {
            return Err(PdfError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("graphics state '{name}' was already added"),
            )));
        }
        let id = self.doc.new_object_id();
        self.writer
            .write_indirect(id.0, id.1, &Object::Dictionary(dict))?;
        self.extgstate_dict.insert(name.to_owned(), id);
        Ok(())
    }

    /// Id страницы по индексу — для `/Dest` аннотаций и закладок.
    ///
    /// Возвращает `None` вне диапазона `page_count` из [`Session::begin`]:
    /// id резервируются только на это количество страниц.
    #[must_use]
    pub(crate) fn page_ref(&self, index: usize) -> Option<ObjectId> {
        self.page_ids.get(index).copied()
    }

    /// Пишет поток содержимого страницы и объект страницы: `MediaBox`,
    /// ресурсы, аннотации (пустой список — без `/Annots`); страница
    /// освобождается сразу после записи.
    ///
    /// # Errors
    ///
    /// Ошибка записи в приёмник; [`PdfError::Io`] с
    /// [`io::ErrorKind::InvalidInput`], если вызвана больше `page_count` раз.
    pub(crate) fn write_page(
        &mut self,
        content: &[u8],
        media_box: [f32; 4],
        annots: &[Dictionary],
    ) -> Result<(), PdfError> {
        let Some(&page_id) = self.page_ids.get(self.written) else {
            return Err(PdfError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write_page called more times than page_count",
            )));
        };
        self.flush_fonts()?;
        self.flush_resource_dicts()?;

        // Содержимое разрешено к сжатию — как у `Document::compress()`:
        // писатель сожмёт его только при `compress`, а `/Filter` и `/Length`
        // обновит сам lopdf.
        let content_id = self.doc.new_object_id();
        let stream = Stream::new(Dictionary::new(), content.to_vec()).with_compression(true);
        self.writer
            .write_indirect(content_id.0, content_id.1, &Object::Stream(stream))?;

        let mut resources = Dictionary::new();
        resources.set("Font", Object::Reference(self.font_dict_id));
        resources.set("XObject", Object::Reference(self.xobject_dict_id));
        resources.set("ExtGState", Object::Reference(self.extgstate_dict_id));

        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", Object::Reference(self.pages_id));
        page.set("MediaBox", media_box_object(media_box));
        page.set("Resources", resources);
        page.set("Contents", Object::Reference(content_id));
        if !annots.is_empty() {
            page.set(
                "Annots",
                Object::Array(annots.iter().cloned().map(Object::Dictionary).collect()),
            );
        }
        self.writer
            .write_indirect(page_id.0, page_id.1, &Object::Dictionary(page))?;
        self.written += 1;
        Ok(())
    }

    /// Закладки `/Outlines` одним уровнем: заголовок → индекс страницы.
    ///
    /// Пустой список — без outline. Пишутся в `finish`: каталог ещё не записан,
    /// и ссылку на `/Outlines` есть куда положить.
    ///
    /// # Errors
    ///
    /// [`PdfError::Io`] с [`io::ErrorKind::InvalidInput`], если какой-то
    /// индекс вне диапазона `page_count` из [`Session::begin`].
    pub(crate) fn set_outline(&mut self, entries: &[(String, usize)]) -> Result<(), PdfError> {
        for (index, (_, page)) in entries.iter().enumerate() {
            if *page >= self.page_ids.len() {
                return Err(PdfError::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "outline entry {index} references page {page}, but page_count is {}",
                        self.page_ids.len()
                    ),
                )));
            }
        }
        self.outline = entries.to_vec();
        Ok(())
    }

    /// Хвост: дерево страниц, закладки, каталог, `/Info`, trailer и xref.
    ///
    /// # Errors
    ///
    /// Ошибка записи или flush приёмника.
    pub(crate) fn finish(mut self) -> Result<(), PdfError> {
        self.flush_fonts()?;
        self.flush_resource_dicts()?;

        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Count", i64::try_from(self.written).unwrap_or(i64::MAX));
        pages.set(
            "Kids",
            Object::Array(
                self.page_ids[..self.written]
                    .iter()
                    .map(|&(id, generation)| Object::Reference((id, generation)))
                    .collect(),
            ),
        );
        self.writer
            .write_indirect(self.pages_id.0, self.pages_id.1, &Object::Dictionary(pages))?;

        // Закладки — до каталога: каталог ссылается на словарь `/Outlines`.
        let outlines_id = if self.outline.is_empty() {
            None
        } else {
            Some(self.write_outline()?)
        };

        let catalog_id = self.doc.new_object_id();
        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", Object::Reference(self.pages_id));
        if let Some(outlines_id) = outlines_id {
            catalog.set("Outlines", Object::Reference(outlines_id));
        }
        self.writer
            .write_indirect(catalog_id.0, catalog_id.1, &Object::Dictionary(catalog))?;

        let mut info = Dictionary::new();
        if let Some(title) = self.title.take() {
            info.set("Title", encode_utf16be(&title));
        }
        info.set(
            "Producer",
            Object::String(PRODUCER.as_bytes().to_vec(), Literal),
        );
        let info_id = self.doc.new_object_id();
        self.writer
            .write_indirect(info_id.0, info_id.1, &Object::Dictionary(info))?;

        let mut trailer = Dictionary::new();
        trailer.set("Size", i64::from(self.doc.max_id) + 1);
        trailer.set("Root", Object::Reference(catalog_id));
        trailer.set("Info", Object::Reference(info_id));
        Ok(self.writer.finish(self.doc.max_id, &trailer)?)
    }

    /// Перекладывает накопленные объекты `Document` в писатель и освобождает
    /// их: реестр существует ради `fonts::write_font`, копить в нём нечего.
    fn flush_objects(&mut self) -> Result<(), PdfError> {
        let ids: Vec<ObjectId> = self.doc.objects.keys().copied().collect();
        for id in ids {
            if let Some(object) = self.doc.objects.remove(&id) {
                self.writer.write_indirect(id.0, id.1, &object)?;
            }
        }
        Ok(())
    }

    /// Пишет глобальный словарь `/Font` — перед первой страницей (`write_page`
    /// зовёт сам) или в `finish` для документа без страниц. После записи
    /// добавлять шрифты нельзя: объект уже в файле.
    fn flush_fonts(&mut self) -> Result<(), PdfError> {
        if self.fonts_written {
            return Ok(());
        }
        let mut dict = Dictionary::new();
        for (name, &id) in &self.font_dict {
            dict.set(name.as_str(), Object::Reference(id));
        }
        self.writer.write_indirect(
            self.font_dict_id.0,
            self.font_dict_id.1,
            &Object::Dictionary(dict),
        )?;
        self.fonts_written = true;
        Ok(())
    }

    /// Пишет глобальные словари `/XObject` и `/ExtGState` — перед первой
    /// страницей (`write_page` зовёт сам) или в `finish` для документа без
    /// страниц. После записи добавлять ресурсы нельзя: объекты уже в файле.
    fn flush_resource_dicts(&mut self) -> Result<(), PdfError> {
        if self.resource_dicts_written {
            return Ok(());
        }
        let mut xobjects = Dictionary::new();
        for (name, &id) in &self.xobject_dict {
            xobjects.set(name.as_str(), Object::Reference(id));
        }
        self.writer.write_indirect(
            self.xobject_dict_id.0,
            self.xobject_dict_id.1,
            &Object::Dictionary(xobjects),
        )?;

        let mut extgstate = Dictionary::new();
        for (name, &id) in &self.extgstate_dict {
            extgstate.set(name.as_str(), Object::Reference(id));
        }
        self.writer.write_indirect(
            self.extgstate_dict_id.0,
            self.extgstate_dict_id.1,
            &Object::Dictionary(extgstate),
        )?;
        self.resource_dicts_written = true;
        Ok(())
    }

    /// Пишет закладки и их словарь `/Outlines`, возвращает id словаря.
    ///
    /// Элементы одного уровня связаны `/Prev`/`/Next`, родитель у всех —
    /// словарь `/Outlines`; дочерних элементов нет, поэтому `/First`/`/Last`/
    /// `/Count` несёт только корень (ISO 32000, 12.3.3).
    fn write_outline(&mut self) -> Result<ObjectId, PdfError> {
        let outlines_id = self.doc.new_object_id();
        let item_ids: Vec<ObjectId> = (0..self.outline.len())
            .map(|_| self.doc.new_object_id())
            .collect();
        for (index, ((title, page), &item_id)) in self.outline.iter().zip(&item_ids).enumerate() {
            // `/Dest` — `[page /XYZ null null null]`: вид задаёт страница,
            // остальные параметры оставляются как есть.
            let dest = Object::Array(vec![
                Object::Reference(self.page_ids[*page]),
                Object::Name(b"XYZ".to_vec()),
                Object::Null,
                Object::Null,
                Object::Null,
            ]);
            let mut item = Dictionary::new();
            item.set("Title", encode_utf16be(title));
            item.set("Parent", Object::Reference(outlines_id));
            item.set("Dest", dest);
            if let Some(&prev) = index.checked_sub(1).and_then(|i| item_ids.get(i)) {
                item.set("Prev", Object::Reference(prev));
            }
            if let Some(&next) = item_ids.get(index + 1) {
                item.set("Next", Object::Reference(next));
            }
            self.writer
                .write_indirect(item_id.0, item_id.1, &Object::Dictionary(item))?;
        }

        let mut outlines = Dictionary::new();
        outlines.set("Type", "Outlines");
        outlines.set(
            "Count",
            i64::try_from(self.outline.len()).unwrap_or(i64::MAX),
        );
        outlines.set("First", Object::Reference(item_ids[0]));
        outlines.set("Last", Object::Reference(item_ids[item_ids.len() - 1]));
        self.writer
            .write_indirect(outlines_id.0, outlines_id.1, &Object::Dictionary(outlines))?;
        Ok(outlines_id)
    }
}

/// `/MediaBox` из `[x0, y0, x1, y1]`: значения пишутся `Real` без округления —
/// раскладка даёт дробные пункты, и сдвиг страницы был бы заметен на печати.
fn media_box_object(media_box: [f32; 4]) -> Object {
    Object::Array(media_box.into_iter().map(Object::Real).collect())
}

/// Строка PDF как UTF-16BE с BOM: `/Title` бывает кириллическим, а
/// `PDFDocEncoding` её не покрывает (ISO 32000, 7.9.2.2).
fn encode_utf16be(text: &str) -> Object {
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, Hexadecimal)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;
    use std::io::Write;
    use std::process::Command;
    use std::time::Instant;

    use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

    use super::{Session, PRODUCER};
    use crate::pdf::content;
    use crate::pdf::fonts::{self, PreparedFont};
    use crate::pdf::image::{DecodedImage, PixelFormat};
    use crate::pdf::ops::{ExtendedGraphicsStateId, Op, XObjectId, XObjectTransform};
    use crate::PdfError;

    /// Размер страницы A4 в пунктах.
    const A4: [f32; 4] = [0.0, 0.0, 595.0, 842.0];

    /// Страницы теста: заголовок жирным и три строки обычным.
    const PAGE_TEXTS: [(&str, [&str; 3]); 3] = [
        (
            "Страница один",
            ["Привет, мир!", "Hello, ASCII 123", "Ёжик и кошка"],
        ),
        (
            "Страница два",
            ["Второй разворот", "Line two: numbers 456", "Крем и торт"],
        ),
        ("Страница три", ["Финал", "Goodbye, ASCII 789", "Ура"]),
    ];

    /// Приёмник, считающий байты (как `CountingSink` в `examples/mem_probe.rs`).
    #[derive(Default)]
    struct CountingSink {
        bytes: usize,
    }

    impl Write for CountingSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Множество символов всех строк — аргумент подрезки.
    fn used_of(texts: &[&str]) -> BTreeSet<char> {
        texts.iter().flat_map(|text| text.chars()).collect()
    }

    /// Текст, закодированный шрифтом, в шестнадцатеричном виде: так строка
    /// попадает в content stream Identity-H (следующий слайс заменит это
    /// `pdf::content`).
    fn hex(prepared: &PreparedFont, text: &str) -> String {
        let mut out = String::new();
        for byte in prepared.encode(text) {
            let _ = write!(out, "{byte:02X}");
        }
        out
    }

    /// Поток содержимого страницы: заголовок жирным, строки обычным и рамка.
    /// Модель операций (`pdf::ops`/`pdf::content`) — следующий слайс, поэтому
    /// байты пишутся вручную и тест от неё не зависит.
    fn page_content(
        heading: &str,
        lines: &[&str],
        regular: &PreparedFont,
        bold: &PreparedFont,
    ) -> Vec<u8> {
        let mut out = format!(
            "BT /F1 24 Tf 1 0 0 1 72 760 Tm <{}> Tj ET\n",
            hex(bold, heading)
        );
        let mut y = 700.0_f32;
        for line in lines {
            let _ = writeln!(
                out,
                "BT /F0 12 Tf 1 0 0 1 72 {y} Tm <{}> Tj ET",
                hex(regular, line)
            );
            y -= 20.0;
        }
        out.push_str("0.5 w 0 0 0 RG 40 40 515 762 re S\n40 100 m 555 100 l S\n");
        out.into_bytes()
    }

    /// Тестовый документ: байты PDF и потоки содержимого страниц, с которыми
    /// сверяются сжатые потоки после разбора.
    struct Sample {
        bytes: Vec<u8>,
        contents: Vec<Vec<u8>>,
    }

    /// Собирает документ теста: три страницы, два начертания, заданное сжатие.
    fn build_sample(compress: bool) -> Sample {
        let regular_texts: Vec<&str> = PAGE_TEXTS
            .iter()
            .flat_map(|(_, lines)| lines.iter().copied())
            .collect();
        let headings: Vec<&str> = PAGE_TEXTS.iter().map(|(heading, _)| *heading).collect();
        let regular_used = used_of(&regular_texts);
        let bold_used = used_of(&headings);

        // Копии шрифтов для сборки контента: сессия подрежет те же байты тем
        // же `used` и получит те же CID — подрезка детерминирована.
        let regular = fonts::prepare(
            doc_converter_render::font::default_font_bytes(),
            &regular_used,
        )
        .unwrap();
        let bold =
            fonts::prepare(doc_converter_render::font::bold_font_bytes(), &bold_used).unwrap();
        let contents: Vec<Vec<u8>> = PAGE_TEXTS
            .iter()
            .map(|(heading, lines)| page_content(heading, lines, &regular, &bold))
            .collect();

        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, PAGE_TEXTS.len(), compress).unwrap();
        session.set_title("Тестовый документ");
        assert_eq!(
            session
                .add_font(
                    doc_converter_render::font::default_font_bytes(),
                    &regular_used
                )
                .unwrap(),
            "F0"
        );
        assert_eq!(
            session
                .add_font(doc_converter_render::font::bold_font_bytes(), &bold_used)
                .unwrap(),
            "F1"
        );
        assert_eq!(
            session.prepared_fonts().len(),
            2,
            "оба начертания доступны для `translate`"
        );
        for content in &contents {
            session.write_page(content, A4, &[]).unwrap();
        }
        session.finish().unwrap();

        Sample { bytes, contents }
    }

    /// id шрифта из ресурсов первой страницы по имени в ресурсах.
    fn page_font(loaded: &Document, name: &[u8]) -> ObjectId {
        let (_, page_id) = loaded
            .get_pages()
            .into_iter()
            .next()
            .expect("есть страница");
        let page = loaded.get_dictionary(page_id).unwrap();
        let font_dict_id = page
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Font")
            .unwrap()
            .as_reference()
            .unwrap();
        loaded
            .get_dictionary(font_dict_id)
            .unwrap()
            .get(name)
            .unwrap()
            .as_reference()
            .unwrap()
    }

    /// Поток `FontFile2` шрифта: Type0 → `DescendantFonts` → `FontDescriptor`.
    fn font_file(loaded: &Document, font_id: ObjectId) -> &Stream {
        let font = loaded.get_dictionary(font_id).unwrap();
        let cid_id = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap();
        let cid = loaded.get_dictionary(cid_id).unwrap();
        let descriptor_id = cid.get(b"FontDescriptor").unwrap().as_reference().unwrap();
        let descriptor = loaded.get_dictionary(descriptor_id).unwrap();
        let file_id = descriptor
            .get(b"FontFile2")
            .unwrap()
            .as_reference()
            .unwrap();
        loaded.get_object(file_id).unwrap().as_stream().unwrap()
    }

    /// UTF-16BE с BOM → строка: проверка заголовка не опирается на декодер
    /// lopdf, только на формат (ISO 32000, 7.9.2.2).
    // `as_chunks` стабилен с Rust 1.88, а MSRV workspace — 1.82, поэтому
    // предложение clippy здесь не применяется.
    #[allow(clippy::chunks_exact_to_as_chunks)]
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

    /// Есть ли утилита в `PATH`: у `pdftotext` нет длинной формы `--version`,
    /// и ненулевой код на незнакомый флаг — не признак отсутствия утилиты.
    fn pdftotext_available() -> bool {
        Command::new("pdftotext").arg("--version").output().is_ok()
    }

    /// Текст без пробельных символов: pdftotext расставляет переводы строк и
    /// пробелы по позициям глифов (как в тесте `fonts.rs`).
    fn strip(value: &str) -> String {
        value.chars().filter(|ch| !ch.is_whitespace()).collect()
    }

    #[test]
    fn compressed_document_parses_and_extracts_text() {
        let sample = build_sample(true);
        assert!(
            sample.bytes.starts_with(b"%PDF-1.3\n%\xBB\xAD\xC0\xDE\n"),
            "версия и метка заголовка — как у printpdf"
        );
        let loaded = Document::load_mem(&sample.bytes).expect("вывод парсится lopdf");

        let pages = loaded.get_pages();
        assert_eq!(pages.len(), 3, "страницы: {:?}", pages.keys());
        for (page_id, content) in pages.values().zip(&sample.contents) {
            let page = loaded.get_dictionary(*page_id).unwrap();
            let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
            let font_dict_id = resources.get(b"Font").unwrap().as_reference().unwrap();
            let fonts = loaded.get_dictionary(font_dict_id).unwrap();
            assert_eq!(fonts.len(), 2, "в ресурсах страницы оба начертания");
            assert!(resources.get(b"ExtGState").is_ok());
            assert!(resources.get(b"XObject").is_ok());

            let media_box: Vec<f32> = page
                .get(b"MediaBox")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_float().unwrap())
                .collect();
            assert_eq!(media_box, A4.to_vec());

            let stream_id = page.get(b"Contents").unwrap().as_reference().unwrap();
            let stream = loaded.get_object(stream_id).unwrap().as_stream().unwrap();
            assert_eq!(
                stream.dict.get(b"Filter").unwrap().as_name().unwrap(),
                b"FlateDecode"
            );
            assert_eq!(&stream.decompressed_content().unwrap(), content);
        }

        // Шрифты: Type0/Identity-H, `FontFile2` сжат, `Length1` — размер
        // распакованной программы.
        let (f0, f1) = (page_font(&loaded, b"F0"), page_font(&loaded, b"F1"));
        assert_ne!(f0, f1, "начертания не слились в одно подмножество");
        for font_id in [f0, f1] {
            let font = loaded.get_dictionary(font_id).unwrap();
            assert_eq!(font.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
            assert_eq!(
                font.get(b"Encoding").unwrap().as_name().unwrap(),
                b"Identity-H"
            );
            let program = font_file(&loaded, font_id).decompressed_content().unwrap();
            assert!(!program.is_empty());
        }
        let file = font_file(&loaded, f0);
        assert_eq!(
            file.dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"FlateDecode"
        );
        assert_eq!(
            file.dict.get(b"Length1").unwrap().as_i64().unwrap(),
            i64::try_from(file.decompressed_content().unwrap().len()).unwrap()
        );

        let info_id = loaded.trailer.get(b"Info").unwrap().as_reference().unwrap();
        let info = loaded.get_dictionary(info_id).unwrap();
        assert_eq!(
            decode_utf16be(info.get(b"Title").unwrap()),
            "Тестовый документ"
        );
        assert_eq!(
            info.get(b"Producer").unwrap().as_str().unwrap(),
            PRODUCER.as_bytes()
        );

        if !pdftotext_available() {
            eprintln!("пропуск pdftotext: утилиты нет в PATH (в CI ставится шагом «pdf tooling»)");
            return;
        }
        let path = std::env::temp_dir().join(format!(
            "doc-converter-pdf-session-{}.pdf",
            std::process::id()
        ));
        std::fs::write(&path, &sample.bytes).unwrap();
        let output = Command::new("pdftotext")
            .arg(&path)
            .arg("-")
            .output()
            .expect("pdftotext запускается");
        std::fs::remove_file(&path).unwrap();

        let expected: String = PAGE_TEXTS
            .iter()
            .flat_map(|(heading, lines)| std::iter::once(*heading).chain(lines.iter().copied()))
            .collect();
        let extracted = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            strip(&extracted),
            strip(&expected),
            "извлечено: {extracted:?}"
        );
    }

    #[test]
    fn uncompressed_streams_have_no_filter() {
        let text = "Hi, мир";
        let used = used_of(&[text]);
        let regular =
            fonts::prepare(doc_converter_render::font::default_font_bytes(), &used).unwrap();
        let content = format!(
            "BT /F0 12 Tf 1 0 0 1 72 700 Tm <{}> Tj ET",
            hex(&regular, text)
        );

        let mut bytes = Vec::new();
        {
            let mut session = Session::begin(&mut bytes, 1, false).unwrap();
            session
                .add_font(doc_converter_render::font::default_font_bytes(), &used)
                .unwrap();
            session.write_page(content.as_bytes(), A4, &[]).unwrap();
            session.finish().unwrap();
        }

        let loaded = Document::load_mem(&bytes).unwrap();
        let (_, page_id) = loaded.get_pages().into_iter().next().unwrap();
        let page = loaded.get_dictionary(page_id).unwrap();
        let stream_id = page.get(b"Contents").unwrap().as_reference().unwrap();
        let stream = loaded.get_object(stream_id).unwrap().as_stream().unwrap();
        assert!(
            stream.dict.get(b"Filter").is_err(),
            "без compress фильтра нет"
        );
        assert_eq!(stream.content, content.as_bytes());

        let f0 = page_font(&loaded, b"F0");
        let file = font_file(&loaded, f0);
        assert!(file.dict.get(b"Filter").is_err());
        // Сжатие выключено: в файле лежит распакованная программа, и её длина
        // равна `Length1`.
        assert_eq!(
            file.dict.get(b"Length1").unwrap().as_i64().unwrap(),
            i64::try_from(file.content.len()).unwrap()
        );
    }

    #[test]
    fn add_font_after_first_page_is_an_error() {
        let used = used_of(&["раз"]);
        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, 1, true).unwrap();
        session
            .add_font(doc_converter_render::font::default_font_bytes(), &used)
            .unwrap();
        session.write_page(b"BT ET", A4, &[]).unwrap();

        let error = session
            .add_font(doc_converter_render::font::bold_font_bytes(), &used)
            .unwrap_err();
        assert!(matches!(error, PdfError::Font(_)), "{error}");
    }

    #[test]
    fn write_page_beyond_page_count_is_an_error() {
        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, 1, true).unwrap();
        session.write_page(b"BT ET", A4, &[]).unwrap();
        let error = session.write_page(b"BT ET", A4, &[]).unwrap_err();
        assert!(matches!(error, PdfError::Io(_)), "{error}");
    }

    /// «Повторный `write_page` после `finish`» не выражается типами:
    /// `finish(self)` забирает сессию по значению, `write_page` требует `&mut`.
    /// Тест фиксирует сигнатуры — ослабить `self` до `&mut self` не даст
    /// компилятор.
    #[test]
    #[allow(clippy::type_complexity)] // предмет теста — сама сигнатура, упрощать нечего
    fn finish_consumes_the_session() {
        // `'static` — конкретизация времени жизни сессии: её параметр у метода
        // ранний, и без конкретизации fn-указатель не получается.
        let finish: fn(Session<'static, Vec<u8>>) -> Result<(), PdfError> = Session::finish;
        let write_page: fn(
            &mut Session<'static, Vec<u8>>,
            &[u8],
            [f32; 4],
            &[Dictionary],
        ) -> Result<(), PdfError> = Session::write_page;
        let _ = (finish, write_page);
    }

    /// Картинка 2x2 для тестов: RGBA — красный, зелёный, синий, белый с
    /// разной альфой.
    fn decoded_rgba() -> DecodedImage {
        DecodedImage {
            width: 2,
            height: 2,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
            format: PixelFormat::Rgb,
            alpha: Some(vec![10, 20, 30, 255]),
        }
    }

    /// Картинка `/X0` страницы: `Resources` → `XObject` → имя.
    fn page_xobject<'a>(loaded: &'a Document, page_id: ObjectId, name: &[u8]) -> &'a Stream {
        let page = loaded.get_dictionary(page_id).unwrap();
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        let xobjects = loaded
            .get_dictionary(resources.get(b"XObject").unwrap().as_reference().unwrap())
            .unwrap();
        let id = xobjects.get(name).unwrap().as_reference().unwrap();
        loaded.get_object(id).unwrap().as_stream().unwrap()
    }

    /// Сквозной тест картинки: RGBA → сессия → разбор lopdf. Поток картинки
    /// несёт RGB без альфы, альфа — отдельный поток `/SMask` (`DeviceGray`).
    #[test]
    fn image_round_trips_through_session_with_smask() {
        let decoded = decoded_rgba();
        let ops = vec![Op::UseXobject {
            id: XObjectId(String::from("X0")),
            transform: XObjectTransform::default(),
        }];
        let content = content::translate(&ops, &BTreeMap::new()).unwrap();

        let mut bytes = Vec::new();
        {
            let mut session = Session::begin(&mut bytes, 1, true).unwrap();
            assert_eq!(session.add_image(&decoded).unwrap(), "X0");
            session.write_page(&content, A4, &[]).unwrap();
            session.finish().unwrap();
        }

        let loaded = Document::load_mem(&bytes).unwrap();
        let (_, page_id) = loaded.get_pages().into_iter().next().unwrap();
        let xobject = page_xobject(&loaded, page_id, b"X0");
        assert_eq!(
            xobject.dict.get(b"Type").unwrap().as_name().unwrap(),
            b"XObject"
        );
        assert_eq!(
            xobject.dict.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Image"
        );
        assert_eq!(xobject.dict.get(b"Width").unwrap().as_i64().unwrap(), 2);
        assert_eq!(xobject.dict.get(b"Height").unwrap().as_i64().unwrap(), 2);
        assert_eq!(
            xobject
                .dict
                .get(b"BitsPerComponent")
                .unwrap()
                .as_i64()
                .unwrap(),
            8
        );
        assert_eq!(
            xobject.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
            b"DeviceRGB"
        );
        assert_eq!(xobject.get_plain_content().unwrap(), decoded.pixels);

        let smask_id = xobject.dict.get(b"SMask").unwrap().as_reference().unwrap();
        let smask = loaded.get_object(smask_id).unwrap().as_stream().unwrap();
        assert_eq!(
            smask.dict.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Image"
        );
        assert_eq!(
            smask.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
            b"DeviceGray"
        );
        assert_eq!(
            smask
                .dict
                .get(b"BitsPerComponent")
                .unwrap()
                .as_i64()
                .unwrap(),
            8
        );
        assert_eq!(smask.get_plain_content().unwrap(), decoded.alpha.unwrap());
    }

    /// Серая картинка без альфы — `DeviceGray` и без `/SMask`.
    #[test]
    fn grayscale_image_has_no_smask() {
        let decoded = DecodedImage {
            width: 1,
            height: 2,
            pixels: vec![0, 255],
            format: PixelFormat::Gray,
            alpha: None,
        };

        let mut bytes = Vec::new();
        {
            let mut session = Session::begin(&mut bytes, 1, true).unwrap();
            assert_eq!(session.add_image(&decoded).unwrap(), "X0");
            session.write_page(b"q /X0 Do Q", A4, &[]).unwrap();
            session.finish().unwrap();
        }

        let loaded = Document::load_mem(&bytes).unwrap();
        let (_, page_id) = loaded.get_pages().into_iter().next().unwrap();
        let xobject = page_xobject(&loaded, page_id, b"X0");
        assert!(
            xobject.dict.get(b"SMask").is_err(),
            "у серой картинки без альфы SMask нет"
        );
        assert_eq!(
            xobject.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
            b"DeviceGray"
        );
        assert_eq!(xobject.get_plain_content().unwrap(), vec![0, 255]);
    }

    /// `ExtGState`: `translate` → `q /GS0 gs Q`, сессия кладёт словарь в
    /// ресурсы страницы.
    #[test]
    fn graphics_state_round_trips_through_session() {
        let mut gs = Dictionary::new();
        gs.set("CA", 0.5);
        let ops = vec![
            Op::SaveGraphicsState,
            Op::LoadGraphicsState {
                gs: ExtendedGraphicsStateId(String::from("GS0")),
            },
            Op::RestoreGraphicsState,
        ];
        let content = content::translate(&ops, &BTreeMap::new()).unwrap();
        assert_eq!(content, b"q\n/GS0 gs\nQ");

        let mut bytes = Vec::new();
        {
            let mut session = Session::begin(&mut bytes, 1, true).unwrap();
            session.add_graphics_state("GS0", gs).unwrap();
            session.write_page(&content, A4, &[]).unwrap();
            session.finish().unwrap();
        }

        let loaded = Document::load_mem(&bytes).unwrap();
        let (_, page_id) = loaded.get_pages().into_iter().next().unwrap();
        let page = loaded.get_dictionary(page_id).unwrap();
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        let gstates = loaded
            .get_dictionary(resources.get(b"ExtGState").unwrap().as_reference().unwrap())
            .unwrap();
        let gs_id = gstates.get(b"GS0").unwrap().as_reference().unwrap();
        let gs = loaded.get_dictionary(gs_id).unwrap();
        let ca = gs.get(b"CA").unwrap().as_float().unwrap();
        assert!((ca - 0.5).abs() < f32::EPSILON, "CA = {ca}");
    }

    /// Закладки одним уровнем: `/Dest` указывают на id своих страниц,
    /// `/Outlines` связан с каталогом, элементы — `/Prev`/`/Next`.
    #[test]
    fn outline_links_titles_to_pages() {
        let entries = vec![
            (String::from("Раздел один"), 0),
            (String::from("Раздел два"), 1),
        ];
        let mut bytes = Vec::new();
        {
            let mut session = Session::begin(&mut bytes, 3, true).unwrap();
            session.write_page(b"BT ET", A4, &[]).unwrap();
            session.write_page(b"BT ET", A4, &[]).unwrap();
            session.set_outline(&entries).unwrap();
            session.finish().unwrap();
        }

        let loaded = Document::load_mem(&bytes).unwrap();
        let root = loaded.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let catalog = loaded.get_dictionary(root).unwrap();
        let outlines_id = catalog.get(b"Outlines").unwrap().as_reference().unwrap();
        let outlines = loaded.get_dictionary(outlines_id).unwrap();
        assert_eq!(
            outlines.get(b"Type").unwrap().as_name().unwrap(),
            b"Outlines"
        );
        assert_eq!(outlines.get(b"Count").unwrap().as_i64().unwrap(), 2);
        let first_id = outlines.get(b"First").unwrap().as_reference().unwrap();
        let last_id = outlines.get(b"Last").unwrap().as_reference().unwrap();

        let pages = loaded.get_pages();
        let items = [
            loaded.get_dictionary(first_id).unwrap(),
            loaded.get_dictionary(last_id).unwrap(),
        ];
        assert_eq!(
            items[0].get(b"Next").unwrap().as_reference().unwrap(),
            last_id
        );
        assert!(items[0].get(b"Prev").is_err(), "у первого нет предыдущего");
        assert!(
            items[1].get(b"Next").is_err(),
            "у последнего нет следующего"
        );
        assert_eq!(
            items[1].get(b"Prev").unwrap().as_reference().unwrap(),
            first_id
        );

        for (index, item) in items.iter().enumerate() {
            assert_eq!(
                item.get(b"Parent").unwrap().as_reference().unwrap(),
                outlines_id
            );
            assert_eq!(
                decode_utf16be(item.get(b"Title").unwrap()),
                entries[index].0
            );
            let dest = item.get(b"Dest").unwrap().as_array().unwrap();
            let page_number = u32::try_from(index + 1).unwrap();
            assert_eq!(dest[0].as_reference().unwrap(), pages[&page_number]);
            assert_eq!(dest[1].as_name().unwrap(), b"XYZ");
            for value in &dest[2..] {
                assert!(matches!(value, Object::Null), "{value:?}");
            }
        }
    }

    /// Без закладок каталог не несёт `/Outlines`.
    #[test]
    fn empty_outline_leaves_catalog_without_outlines() {
        let mut bytes = Vec::new();
        {
            let mut session = Session::begin(&mut bytes, 1, true).unwrap();
            session.set_outline(&[]).unwrap();
            session.write_page(b"BT ET", A4, &[]).unwrap();
            session.finish().unwrap();
        }

        let loaded = Document::load_mem(&bytes).unwrap();
        let root = loaded.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let catalog = loaded.get_dictionary(root).unwrap();
        assert!(catalog.get(b"Outlines").is_err());
    }

    /// Индекс закладки за пределами `page_count` — ошибка, а не битый `/Dest`.
    #[test]
    fn outline_beyond_page_count_is_an_error() {
        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, 1, true).unwrap();
        let error = session
            .set_outline(&[(String::from("Раздел"), 1)])
            .unwrap_err();
        assert!(matches!(error, PdfError::Io(_)), "{error}");
    }

    /// `page_ref` отдаёт зарезервированные id страниц и `None` вне диапазона.
    #[test]
    fn page_ref_returns_reserved_page_ids() {
        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, 2, true).unwrap();
        let first = session.page_ref(0).unwrap();
        let second = session.page_ref(1).unwrap();
        assert_ne!(first, second);
        assert!(session.page_ref(2).is_none(), "id не резервировались");
        session.write_page(b"BT ET", A4, &[]).unwrap();
        session.write_page(b"BT ET", A4, &[]).unwrap();
        session.finish().unwrap();

        let loaded = Document::load_mem(&bytes).unwrap();
        let pages = loaded.get_pages();
        assert_eq!(pages[&1], first);
        assert_eq!(pages[&2], second);
    }

    /// Картинка после первой страницы — ошибка: словарь `/XObject` уже записан.
    #[test]
    fn add_image_after_first_page_is_an_error() {
        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, 2, true).unwrap();
        session.write_page(b"BT ET", A4, &[]).unwrap();
        let error = session.add_image(&decoded_rgba()).unwrap_err();
        assert!(matches!(error, PdfError::Image(_)), "{error}");
    }

    /// `ExtGState` после первой страницы — ошибка, как и картинка.
    #[test]
    fn add_graphics_state_after_first_page_is_an_error() {
        let mut gs = Dictionary::new();
        gs.set("CA", 0.5);
        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, 2, true).unwrap();
        session.write_page(b"BT ET", A4, &[]).unwrap();
        let error = session.add_graphics_state("GS0", gs).unwrap_err();
        assert!(matches!(error, PdfError::Io(_)), "{error}");
    }

    /// Повторное имя `ExtGState` — ошибка: ресурс молча перезаписался бы.
    #[test]
    fn duplicate_graphics_state_name_is_an_error() {
        let mut gs = Dictionary::new();
        gs.set("CA", 0.5);
        let mut bytes = Vec::new();
        let mut session = Session::begin(&mut bytes, 1, true).unwrap();
        session.add_graphics_state("GS0", gs.clone()).unwrap();
        let error = session.add_graphics_state("GS0", gs).unwrap_err();
        assert!(matches!(error, PdfError::Io(_)), "{error}");
    }

    /// Тексты страницы спайка: заголовок и три строки.
    fn spike_texts(index: usize) -> (String, Vec<String>) {
        (
            format!("Страница {index}"),
            vec![
                format!("Измерение скорости записи, страница {index}"),
                String::from("Проверка размера и времени экспорта"),
                format!("Номер страницы: {index}"),
            ],
        )
    }

    /// Собирает документ спайка в приёмник: шрифты подрезаются по всем текстам
    /// заранее, каждая страница уходит в приёмник и освобождается.
    fn build_spike<W: Write>(out: &mut W, pages: usize) -> Result<(), PdfError> {
        let texts: Vec<(String, Vec<String>)> = (0..pages).map(spike_texts).collect();
        let regular_texts: Vec<&str> = texts
            .iter()
            .flat_map(|(_, lines)| lines.iter().map(String::as_str))
            .collect();
        let headings: Vec<&str> = texts.iter().map(|(heading, _)| heading.as_str()).collect();
        let regular_used = used_of(&regular_texts);
        let bold_used = used_of(&headings);
        let regular = fonts::prepare(
            doc_converter_render::font::default_font_bytes(),
            &regular_used,
        )
        .unwrap();
        let bold =
            fonts::prepare(doc_converter_render::font::bold_font_bytes(), &bold_used).unwrap();

        let mut session = Session::begin(out, pages, true)?;
        session.add_font(
            doc_converter_render::font::default_font_bytes(),
            &regular_used,
        )?;
        session.add_font(doc_converter_render::font::bold_font_bytes(), &bold_used)?;
        for (heading, lines) in &texts {
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            session.write_page(&page_content(heading, &lines, &regular, &bold), A4, &[])?;
        }
        session.finish()
    }

    /// Спайк-замер слайса: 500 синтетических страниц в счётчик байт — размер и
    /// скорость постраничной записи. Запуск вручную в release:
    ///
    ///     cargo test -p doc-converter-pdf --release --lib spike -- --ignored --nocapture
    ///
    /// `SPIKE_OUT=<путь>` дополнительно пишет PDF в файл (ручная проверка
    /// `pdftotext`/`qpdf`); без переменной файл не создаётся.
    #[test]
    #[ignore = "ручной замер: см. доккомментарий"]
    #[allow(clippy::cast_precision_loss)] // число страниц далеко от 2^53
    fn spike_500_synthetic_pages() {
        const PAGES: usize = 500;
        let started = Instant::now();
        let (size, target) = if let Some(path) = std::env::var_os("SPIKE_OUT") {
            let mut bytes = Vec::new();
            build_spike(&mut bytes, PAGES).unwrap();
            std::fs::write(&path, &bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            (bytes.len(), format!("записан {}", path.display()))
        } else {
            let mut sink = CountingSink::default();
            build_spike(&mut sink, PAGES).unwrap();
            (sink.bytes, String::from("счётчик байт"))
        };
        let elapsed = started.elapsed();
        println!(
            "спайк: {PAGES} страниц, PDF {size} Б ({:.1} КиБ), {target}, \
             экспорт {:.0} мс, {:.2} мс/страница",
            size as f64 / 1024.0,
            elapsed.as_secs_f64() * 1000.0,
            elapsed.as_secs_f64() * 1000.0 / PAGES as f64,
        );
    }
}
