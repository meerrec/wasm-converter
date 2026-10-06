//! Потоковая запись PDF — правка форка (ADR-0010, спайк B1).
//!
//! Механизм: сессия записи. Глобальные объекты (каталог, шрифты, XObject'ы,
//! ExtGState, ICC/XMP, слои) пишутся один раз в начале; каждая страница
//! переводится в объекты PDF, немедленно сериализуется в `Write` и
//! освобождается — готовый документ не накапливает объекты страниц. Таблица
//! xref строится инкрементально по мере записи, `/Pages`, outline, каталог,
//! info и trailer дописываются в конце; seek не нужен, поэтому путь годится и
//! для файла, и для чанкового потока.
//!
//! Почему свой сериализатор: lopdf 0.35 держит модуль `writer` приватным,
//! публичных `write_indirect_object`/`write_object` нет, а складывать объекты
//! страниц в `Document`, чтобы потом отдать их `save_to`, — значит снова
//! копить их в памяти. Синтаксис значений повторяет `lopdf::writer`
//! (экранирование, разделители, xref), иначе пути разошлись бы на одних и тех
//! же данных. Побайтового совпадения всего файла со старым путём при этом нет:
//! объекты пишутся в другом порядке, расхождение начинается с 15-го байта
//! (после заголовка). Побайтово равны только потоки содержимого страниц — на
//! это и опирается тест совместимости.
//!
//! Шрифты: сессия готовит subset-шрифты до первой страницы, поэтому
//! вызывающий передаёт страницы для сбора глифов (`font_probe_pages`); без них
//! подрезанные шрифты окажутся пустыми. Ленивая подача страниц и пре-проход
//! глифов — задача B3 (см. ADR-0010, «Стриминг»).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use lopdf::{
    Dictionary as LoDictionary, Object, ObjectId, Stream as LoStream,
    StringFormat::{Hexadecimal, Literal},
};

use crate::{
    ops::Op,
    serialize::{
        build_globals, docinfo_to_dict, encode_text_to_utf16be, get_used_internal_fonts,
        annotation_to_dict, translate_operations, PdfGlobals, PreparedFont,
    },
    BuiltinFont, FontId, LayerInternalId, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg,
};

/// Пишущая обёртка с подсчётом байт: xref нужны точные смещения объектов.
struct CountingWriter<W: Write> {
    inner: W,
    count: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.count += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Потоковый сериализатор объектов PDF: пишет объекты и попутно копит xref.
struct StreamWriter<W: Write> {
    out: CountingWriter<W>,
    /// id → (смещение, generation). Таблица пишется одной секцией в `finish`.
    entries: BTreeMap<u32, (u64, u16)>,
    /// Сжимать потоки — там же, где это делает `Document::compress()` при
    /// `optimize`: до записи, чтобы `/Filter` и `/Length` попали в файл.
    compress: bool,
}

impl<W: Write> StreamWriter<W> {
    fn new(writer: W, compress: bool) -> Self {
        Self {
            out: CountingWriter {
                inner: writer,
                count: 0,
            },
            entries: BTreeMap::new(),
            compress,
        }
    }

    /// Заголовок файла: версия и бинарная метка — как у `Document::save_to`.
    fn begin(&mut self, version: &str, binary_mark: &[u8]) -> io::Result<()> {
        write!(self.out, "%PDF-{version}\n")?;
        self.out.write_all(b"%")?;
        self.out.write_all(binary_mark)?;
        self.out.write_all(b"\n")
    }

    fn write_indirect(&mut self, id: u32, generation: u16, object: &Object) -> io::Result<()> {
        let offset = self.out.count;
        self.entries.insert(id, (offset, generation));
        write!(self.out, "{id} {generation} obj\n")?;
        if needs_separator(object) {
            self.out.write_all(b" ")?;
        }
        match object {
            Object::Stream(stream) if self.compress && stream.allows_compression => {
                let mut stream = stream.clone();
                // Как `Document::compress()`: неудача сжатия не фатальна.
                let _ = stream.compress();
                write_value(&mut self.out, &Object::Stream(stream))?;
            }
            _ => write_value(&mut self.out, object)?,
        }
        if needs_end_separator(object) {
            self.out.write_all(b" ")?;
        }
        self.out.write_all(b"\nendobj\n")
    }

    /// Закрывает файл: xref одной секцией `0..=max_id`, trailer, startxref.
    fn finish(&mut self, max_id: u32, trailer: &LoDictionary) -> io::Result<()> {
        let startxref = self.out.count;
        self.out.write_all(b"xref\n")?;
        let size = max_id + 1;
        write!(self.out, "0 {size}\n")?;
        // Нулевой объект — всегда свободный, generation 65535 по ISO 32000.
        self.out.write_all(b"0000000000 65535 f \n")?;
        for id in 1..size {
            match self.entries.get(&id) {
                Some(&(offset, generation)) => {
                    writeln!(self.out, "{offset:>010} {generation:>05} n ")?;
                }
                None => self.out.write_all(b"0000000000 65535 f \n")?,
            }
        }
        self.out.write_all(b"trailer\n")?;
        write_dictionary(&mut self.out, trailer)?;
        write!(self.out, "\nstartxref\n{startxref}\n%%EOF")?;
        self.out.flush()
    }
}

fn needs_separator(object: &Object) -> bool {
    matches!(
        object,
        Object::Null
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Real(_)
            | Object::Reference(_)
    )
}

fn needs_end_separator(object: &Object) -> bool {
    matches!(
        object,
        Object::Null
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Real(_)
            | Object::Name(_)
            | Object::Reference(_)
            | Object::Stream(_)
    )
}

fn write_value<W: Write>(w: &mut W, object: &Object) -> io::Result<()> {
    match object {
        Object::Null => w.write_all(b"null"),
        Object::Boolean(value) => w.write_all(if *value { b"true" } else { b"false" }),
        Object::Integer(value) => write!(w, "{value}"),
        Object::Real(value) => write!(w, "{value}"),
        Object::String(bytes, Hexadecimal) => {
            w.write_all(b"<")?;
            for byte in bytes {
                write!(w, "{byte:02X}")?;
            }
            w.write_all(b">")
        }
        Object::String(bytes, Literal) => write_literal_string(w, bytes),
        Object::Name(name) => write_name(w, name),
        Object::Array(items) => {
            w.write_all(b"[")?;
            for (index, item) in items.iter().enumerate() {
                if index > 0 && needs_separator(item) {
                    w.write_all(b" ")?;
                }
                write_value(w, item)?;
            }
            w.write_all(b"]")
        }
        Object::Dictionary(dict) => write_dictionary(w, dict),
        Object::Stream(stream) => {
            write_dictionary(w, &stream.dict)?;
            w.write_all(b"stream\n")?;
            w.write_all(&stream.content)?;
            w.write_all(b"\nendstream")
        }
        Object::Reference((id, generation)) => write!(w, "{id} {generation} R"),
    }
}

/// Экранирование повторяет `lopdf::writer` байт в байт: сбалансированные
/// скобки не экранируются, одиночные — да; `\r` пишется как `\r`, иначе
/// перевод строки внутри литерала превратился бы в `\n` при разборе.
fn write_literal_string<W: Write>(w: &mut W, text: &[u8]) -> io::Result<()> {
    let mut escape = Vec::new();
    let mut unbalanced_open = Vec::new();
    for (index, &byte) in text.iter().enumerate() {
        match byte {
            b'(' => unbalanced_open.push(index),
            b')' => {
                if unbalanced_open.is_empty() {
                    escape.push(index);
                } else {
                    unbalanced_open.pop();
                }
            }
            b'\\' | b'\r' => escape.push(index),
            _ => {}
        }
    }
    escape.append(&mut unbalanced_open);

    w.write_all(b"(")?;
    if escape.is_empty() {
        w.write_all(text)?;
    } else {
        for (index, &byte) in text.iter().enumerate() {
            if escape.contains(&index) {
                w.write_all(b"\\")?;
                w.write_all(&[if byte == b'\r' { b'r' } else { byte }])?;
            } else {
                w.write_all(&[byte])?;
            }
        }
    }
    w.write_all(b")")
}

fn write_dictionary<W: Write>(w: &mut W, dict: &LoDictionary) -> io::Result<()> {
    w.write_all(b"<<")?;
    for (key, value) in dict {
        write_name(w, key)?;
        if needs_separator(value) {
            w.write_all(b" ")?;
        }
        write_value(w, value)?;
    }
    w.write_all(b">>")
}

fn write_name<W: Write>(w: &mut W, name: &[u8]) -> io::Result<()> {
    w.write_all(b"/")?;
    for &byte in name {
        // Пробелы, разделители и всё вне 33..=126 кодируются как `#XX`
        // (ISO 32000-1, 7.3.5).
        if b" \t\n\r\x0C()<>[]{}/%#".contains(&byte) || !(33..=126).contains(&byte) {
            write!(w, "#{byte:02X}")?;
        } else {
            w.write_all(&[byte])?;
        }
    }
    Ok(())
}

/// Сессия потоковой записи PDF.
///
/// Порядок работы: [`StreamSession::begin`] — глобальные объекты,
/// [`StreamSession::write_page`] на каждую страницу, [`StreamSession::finish`] —
/// хвост и xref.
pub struct StreamSession<'p, 'w, W: Write> {
    sw: StreamWriter<&'w mut W>,
    pdf: &'p PdfDocument,
    secure: bool,
    next_id: u32,
    pages_id: ObjectId,
    catalog: LoDictionary,
    layer_ids: Option<BTreeMap<LayerInternalId, ObjectId>>,
    global_font_dict_id: ObjectId,
    global_xobject_dict_id: ObjectId,
    global_extgstate_dict_id: ObjectId,
    prepared_fonts: BTreeMap<FontId, PreparedFont>,
    page_ids_reserved: Vec<ObjectId>,
    page_ids: Vec<ObjectId>,
    next_page: usize,
}

impl<'p, 'w, W: Write> StreamSession<'p, 'w, W> {
    /// Начинает сессию: собирает глобальные объекты и пишет их в поток.
    ///
    /// `font_probe_pages` — страницы, по которым `prepare_fonts` собирает
    /// глифы subset-шрифтов: подрезка происходит до первой страницы, поэтому
    /// для документов с subset-шрифтами срез обязан содержать все страницы.
    /// `builtin_fonts` — стандартные шрифты документа: у ленивого пути их набор
    /// нельзя вывести из `pdf.pages`. `page_count` — сколько страниц будет
    /// записано: id резервируются сразу, иначе аннотации-ссылки на последующие
    /// страницы (`/Dest`) было бы некуда вести.
    ///
    /// # Errors
    /// Ошибки записи в `writer`.
    pub fn begin(
        pdf: &'p PdfDocument,
        font_probe_pages: &[PdfPage],
        builtin_fonts: BTreeSet<BuiltinFont>,
        page_count: usize,
        opts: &PdfSaveOptions,
        writer: &'w mut W,
        warnings: &mut Vec<PdfWarnMsg>,
    ) -> io::Result<Self> {
        let PdfGlobals {
            doc,
            pages_id,
            catalog,
            layer_ids,
            prepared_fonts,
            global_font_dict_id,
            global_xobject_dict_id,
            global_extgstate_dict_id,
            page_ids_reserved,
        } = build_globals(
            pdf,
            font_probe_pages,
            builtin_fonts,
            page_count,
            opts,
            warnings,
        );

        let next_id = doc.max_id + 1;
        let mut sw = StreamWriter::new(writer, opts.optimize);
        sw.begin(&doc.version, &doc.binary_mark)?;
        for (&(id, generation), object) in &doc.objects {
            sw.write_indirect(id, generation, object)?;
        }

        Ok(Self {
            sw,
            pdf,
            secure: opts.secure,
            next_id,
            pages_id,
            catalog,
            layer_ids,
            global_font_dict_id,
            global_xobject_dict_id,
            global_extgstate_dict_id,
            prepared_fonts,
            page_ids_reserved,
            page_ids: Vec::with_capacity(page_count),
            next_page: 0,
        })
    }

    /// Выдаёт id из того же диапазона, что и `Document::add_object`, — id
    /// страниц и объектов обязаны совпадать со старым путём.
    fn alloc_id(&mut self) -> ObjectId {
        let id = (self.next_id, 0);
        self.next_id += 1;
        id
    }

    /// Сериализует страницу и отпускает её содержимое.
    ///
    /// # Errors
    /// Ошибки записи в `writer`; [`io::ErrorKind::InvalidInput`], если вызвана
    /// больше `page_count` раз.
    pub fn write_page(&mut self, page: &PdfPage, warnings: &mut Vec<PdfWarnMsg>) -> io::Result<()> {
        let Some(page_id) = self.page_ids_reserved.get(self.next_page).copied() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write_page called more times than page_count",
            ));
        };
        self.next_page += 1;

        let mut page_resources = LoDictionary::new();

        // Слои страницы ссылаются на глобальные OCG-объекты из каталога —
        // блок повторяет `serialize_pdf`, чтобы страницы с BeginLayer
        // собирались одинаково обоими путями.
        if let Some(ref layer_ids) = self.layer_ids {
            let page_layers = page
                .ops
                .iter()
                .filter_map(|op| {
                    if let Op::BeginLayer { layer_id } = op {
                        layer_ids
                            .get(layer_id)
                            .map(|ocg_obj_id| (layer_id.0.clone(), *ocg_obj_id))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if !page_layers.is_empty() {
                page_resources.set(
                    "Properties",
                    LoDictionary::from_iter(
                        page_layers
                            .iter()
                            .map(|(name, ocg_obj_id)| (name.as_str(), Object::Reference(*ocg_obj_id))),
                    ),
                );
            }
        }

        let annots = page
            .ops
            .iter()
            .filter_map(|l| match l {
                Op::Annotation { annot } => Some(annot.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();

        page_resources.set("Font", Object::Reference(self.global_font_dict_id));
        page_resources.set("XObject", Object::Reference(self.global_xobject_dict_id));
        page_resources.set("ExtGState", Object::Reference(self.global_extgstate_dict_id));

        let layer_stream = translate_operations(
            &page.ops,
            &self.prepared_fonts,
            &self.pdf.resources.xobjects.map,
            self.secure,
            warnings,
        );
        // Форк ADR-0010: `Document::compress()` сжимает только потоки с
        // `allows_compression = true`, поэтому содержимое страницы разрешено
        // явно; сам сжимает `write_indirect`, если `optimize`.
        let merged_layer_stream =
            LoStream::new(LoDictionary::new(), layer_stream).with_compression(true);

        let resources_id = self.alloc_id();
        let content_id = self.alloc_id();
        let mut page_entries: Vec<(&str, Object)> = vec![
            ("Type", "Page".into()),
            ("MediaBox", page.get_media_box()),
            ("TrimBox", page.get_trim_box()),
            ("CropBox", page.get_crop_box()),
            ("Parent", Object::Reference(self.pages_id)),
            ("Resources", Object::Reference(resources_id)),
            ("Contents", Object::Reference(content_id)),
        ];
        // Ровно как `serialize_pdf`: `/Annots` — ключ страницы, пустой массив
        // не пишется.
        if !annots.is_empty() {
            page_entries.push((
                "Annots",
                Object::Array(
                    annots
                        .iter()
                        .map(|a| {
                            Object::Dictionary(annotation_to_dict(a, &self.page_ids_reserved))
                        })
                        .collect(),
                ),
            ));
        }

        self.sw
            .write_indirect(resources_id.0, resources_id.1, &Object::Dictionary(page_resources))?;
        self.sw.write_indirect(
            content_id.0,
            content_id.1,
            &Object::Stream(merged_layer_stream),
        )?;
        self.sw.write_indirect(
            page_id.0,
            page_id.1,
            &Object::Dictionary(LoDictionary::from_iter(page_entries)),
        )?;
        self.page_ids.push(page_id);
        Ok(())
    }

    /// Дописывает outline, `/Pages`, каталог, info и xref с trailer'ом.
    ///
    /// # Errors
    /// Ошибки записи в `writer`.
    pub fn finish(mut self) -> io::Result<()> {
        if !self.pdf.bookmarks.map.is_empty() {
            let mut bookmarks_sorted = self.pdf.bookmarks.map.iter().collect::<Vec<_>>();
            bookmarks_sorted.sort_by(|(_, v), (_, v2)| (v.page, &v.name).cmp(&(v2.page, &v2.name)));
            let bookmarks_sorted = bookmarks_sorted
                .into_iter()
                .filter_map(|(id, bookmark)| {
                    let page_id = self
                        .page_ids
                        .get(bookmark.page.saturating_sub(1))
                        .copied()?;
                    Some((id, &bookmark.name, page_id))
                })
                .collect::<Vec<_>>();

            // Если все закладки ссылаются на несуществующие страницы, старый
            // путь падал на unwrap — потоковый просто не пишет outline.
            if !bookmarks_sorted.is_empty() {
                let bookmarks_id = self.alloc_id();
                let bookmark_ids = bookmarks_sorted
                    .iter()
                    .map(|(id, name, page_id)| {
                        let self_id = self.alloc_id();
                        (*id, *name, *page_id, self_id)
                    })
                    .collect::<Vec<_>>();

                if let (Some(first), Some(last)) = (
                    bookmark_ids.first().map(|s| s.3),
                    bookmark_ids.last().map(|s| s.3),
                ) {
                    for (index, (_id, name, page_id, self_id)) in bookmark_ids.iter().enumerate() {
                        let prev = index
                            .checked_sub(1)
                            .and_then(|i| bookmark_ids.get(i))
                            .map(|s| s.3);
                        let next = bookmark_ids.get(index + 1).map(|s| s.3);
                        let dest = Object::Array(vec![
                            Object::Reference(*page_id),
                            "XYZ".into(),
                            Object::Null,
                            Object::Null,
                            Object::Null,
                        ]);
                        let mut dict = LoDictionary::from_iter(vec![
                            ("Parent", Object::Reference(bookmarks_id)),
                            ("Title", encode_text_to_utf16be(name)),
                            ("Dest", dest),
                        ]);
                        if let Some(prev) = prev {
                            dict.set("Prev", Object::Reference(prev));
                        }
                        if let Some(next) = next {
                            dict.set("Next", Object::Reference(next));
                        }
                        self.sw.write_indirect(
                            self_id.0,
                            self_id.1,
                            &Object::Dictionary(dict),
                        )?;
                    }

                    let bookmarks_list = LoDictionary::from_iter(vec![
                        ("Type", "Outlines".into()),
                        ("Count", Object::Integer(self.pdf.bookmarks.map.len() as i64)),
                        ("First", Object::Reference(first)),
                        ("Last", Object::Reference(last)),
                    ]);
                    self.sw.write_indirect(
                        bookmarks_id.0,
                        bookmarks_id.1,
                        &Object::Dictionary(bookmarks_list),
                    )?;
                    self.catalog
                        .set("Outlines", Object::Reference(bookmarks_id));
                    self.catalog
                        .set("PageMode", Object::String(b"UseOutlines".to_vec(), Literal));
                }
            }
        }

        let pages_dict = LoDictionary::from_iter(vec![
            ("Type", "Pages".into()),
            ("Count", Object::Integer(self.page_ids.len() as i64)),
            (
                "Kids",
                Object::Array(
                    self.page_ids
                        .iter()
                        .map(|id| Object::Reference(*id))
                        .collect::<Vec<_>>(),
                ),
            ),
        ]);
        let pages_id = self.pages_id;
        self.sw
            .write_indirect(pages_id.0, pages_id.1, &Object::Dictionary(pages_dict))?;

        let catalog_id = self.alloc_id();
        self.sw.write_indirect(
            catalog_id.0,
            catalog_id.1,
            &Object::Dictionary(std::mem::take(&mut self.catalog)),
        )?;
        let document_info_id = self.alloc_id();
        self.sw.write_indirect(
            document_info_id.0,
            document_info_id.1,
            &Object::Dictionary(docinfo_to_dict(&self.pdf.metadata.info)),
        )?;

        let instance_id = crate::utils::random_character_string_32();
        let document_id = crate::utils::random_character_string_32();
        let mut trailer = LoDictionary::new();
        trailer.set("Size", Object::Integer(i64::from(self.next_id)));
        trailer.set("Root", Object::Reference(catalog_id));
        trailer.set("Info", Object::Reference(document_info_id));
        trailer.set(
            "ID",
            Object::Array(vec![
                Object::String(document_id.as_bytes().to_vec(), Literal),
                Object::String(instance_id.as_bytes().to_vec(), Literal),
            ]),
        );

        let max_id = self.next_id - 1;
        self.sw.finish(max_id, &trailer)
    }
}

/// Однократная потоковая сериализация готового документа: страницы забираются
/// из `pdf.pages` (в документе их не остаётся) и освобождаются по одной.
///
/// # Errors
/// Ошибки записи в `writer`.
pub fn serialize_pdf_streaming<W: Write>(
    pdf: &mut PdfDocument,
    opts: &PdfSaveOptions,
    writer: &mut W,
    warnings: &mut Vec<PdfWarnMsg>,
) -> io::Result<()> {
    let pages = std::mem::take(&mut pdf.pages);
    let builtin_fonts = get_used_internal_fonts(&pages);
    let mut session =
        StreamSession::begin(pdf, &pages, builtin_fonts, pages.len(), opts, writer, warnings)?;
    for page in &pages {
        session.write_page(page, warnings)?;
    }
    drop(pages);
    session.finish()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io;

    use lopdf::Document;

    use super::*;
    use crate::ops::Op;
    use crate::{
        Actions, Annotation, BuiltinFont, Destination, LinkAnnotation, Mm, PageAnnotId,
        PageAnnotation, PdfDocument, PdfPage, PdfSaveOptions, Point, Pt, Rect, TextItem,
    };

    fn render(object: &Object) -> String {
        let mut bytes = Vec::new();
        write_value(&mut bytes, object).expect("запись в Vec не падает");
        String::from_utf8(bytes).expect("сериализатор пишет ASCII")
    }

    fn literal(text: &[u8]) -> String {
        render(&Object::String(text.to_vec(), Literal))
    }

    #[test]
    fn literal_strings_escape_only_what_lopdf_escapes() {
        assert_eq!(literal(b"plain"), "(plain)");
        // Сбалансированные скобки не экранируются.
        assert_eq!(literal(b"a(b)c"), "(a(b)c)");
        // Одиночные — экранируются.
        assert_eq!(literal(b"a(b"), "(a\\(b)");
        assert_eq!(literal(b"a)b"), "(a\\)b)");
        // Закрывающие без открывающих перед ними — обе экранируются,
        // одиночная открывающая в конце — тоже (так же считает lopdf).
        assert_eq!(literal(b"))("), "(\\)\\)\\()");
        assert_eq!(literal(b"(a(b)c)d"), "((a(b)c)d)");
        assert_eq!(literal(b"a\\b"), "(a\\\\b)");
        assert_eq!(literal(b"a\rb"), "(a\\rb)");
        assert_eq!(literal(b"a\nb"), "(a\nb)");
    }

    #[test]
    fn names_escape_delimiters_and_non_ascii() {
        assert_eq!(render(&Object::Name(b"simple".to_vec())), "/simple");
        assert_eq!(render(&Object::Name(b"a b".to_vec())), "/a#20b");
        assert_eq!(render(&Object::Name(b"a#b".to_vec())), "/a#23b");
        assert_eq!(render(&Object::Name(b"a/b".to_vec())), "/a#2Fb");
        assert_eq!(render(&Object::Name(b"a(b)".to_vec())), "/a#28b#29");
        assert_eq!(render(&Object::Name(vec![0xE9])), "/#E9");
        assert_eq!(render(&Object::Name(b"\t".to_vec())), "/#09");
    }

    #[test]
    fn scalars_use_pdf_spelling() {
        assert_eq!(render(&Object::Null), "null");
        assert_eq!(render(&Object::Boolean(true)), "true");
        assert_eq!(render(&Object::Boolean(false)), "false");
        assert_eq!(render(&Object::Integer(-42)), "-42");
        assert_eq!(render(&Object::Real(0.5)), "0.5");
        assert_eq!(render(&Object::Reference((7, 0))), "7 0 R");
    }

    #[test]
    fn arrays_and_dicts_insert_separators_like_lopdf() {
        let array = Object::Array(vec![
            Object::Integer(1),
            Object::Name(b"n".to_vec()),
            Object::Boolean(true),
            Object::String(b"s".to_vec(), Literal),
        ]);
        assert_eq!(render(&array), "[1/n true(s)]");

        let mut dict = LoDictionary::new();
        dict.set("A", Object::Integer(1));
        dict.set("B", Object::Name(b"n".to_vec()));
        assert_eq!(render(&Object::Dictionary(dict)), "<</A 1/B/n>>");
    }

    fn text_ops(text: &str) -> Vec<Op> {
        let font = BuiltinFont::Helvetica;
        vec![
            Op::StartTextSection,
            Op::SetTextCursor {
                pos: Point {
                    x: Pt(40.0),
                    y: Pt(780.0),
                },
            },
            Op::SetFontSizeBuiltinFont {
                size: Pt(12.0),
                font,
            },
            Op::WriteTextBuiltinFont {
                items: vec![TextItem::Text(text.to_string())],
                font,
            },
            Op::EndTextSection,
        ]
    }

    fn text_page(text: &str) -> PdfPage {
        PdfPage::new(Mm(210.0), Mm(297.0), text_ops(text))
    }

    fn link_op(actions: Actions) -> Op {
        Op::Annotation {
            annot: Annotation::Link(LinkAnnotation::new(
                Rect {
                    x: Pt(20.0),
                    y: Pt(700.0),
                    width: Pt(120.0),
                    height: Pt(18.0),
                },
                actions,
                None,
                None,
                None,
            )),
        }
    }

    /// Документ с трёх страниц: текст с неудобными символами, две закладки и
    /// две ссылки-аннотации (GoTo на вторую страницу и URI).
    fn tricky_document() -> PdfDocument {
        let mut doc = PdfDocument::new("Стриминг (тест) \\ с CR\r");
        doc.metadata.info.author = "автор)".to_string();
        doc.metadata.info.subject = "sub\\ject (тема)".to_string();
        doc.metadata.info.keywords = vec!["ключ(1)".to_string(), "ключ)2".to_string()];

        let body = "Строка с (скобками), обратным \\ слэшем и парой лишних символов. ".repeat(40);
        let mut first_page = text_ops(&body);
        first_page.push(link_op(Actions::Goto(Destination::Xyz {
            page: 2,
            left: None,
            top: None,
            zoom: None,
        })));
        first_page.push(link_op(Actions::Uri(
            "https://example.com/(x)".to_string(),
        )));

        doc.with_pages(vec![
            PdfPage::new(Mm(210.0), Mm(297.0), first_page),
            text_page("Вторая страница"),
            text_page("Третья страница"),
        ]);
        doc.bookmarks.map.insert(
            PageAnnotId("b1".to_string()),
            PageAnnotation {
                name: "Глава (1) \\ конец".to_string(),
                page: 1,
            },
        );
        doc.bookmarks.map.insert(
            PageAnnotId("b2".to_string()),
            PageAnnotation {
                name: "Глава 2".to_string(),
                page: 3,
            },
        );
        doc
    }

    fn parse(bytes: &[u8]) -> Document {
        Document::load_mem(bytes).expect("PDF обязан разбираться lopdf")
    }

    fn content_stream(doc: &Document, page: ObjectId) -> &lopdf::Stream {
        let contents = doc
            .get_object(page)
            .expect("страница есть")
            .as_dict()
            .expect("страница — словарь")
            .get(b"Contents")
            .expect("у страницы нет /Contents");
        let id = contents.as_reference().expect("/Contents — ссылка");
        doc.get_object(id)
            .expect("объект содержимого есть")
            .as_stream()
            .expect("/Contents — поток")
    }

    fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .rposition(|window| window == needle)
    }

    /// Читает xref вручную: секция одна, записи фиксированной ширины 20 байт.
    fn parse_xref(bytes: &[u8]) -> (usize, u32, Vec<(u64, u16, bool)>) {
        let table = find_last(bytes, b"\nxref\n").expect("нет таблицы xref") + 1;
        let mut pos = table + b"xref\n".len();
        let line_end = bytes[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .expect("нет подсекции")
            + pos;
        let header = std::str::from_utf8(&bytes[pos..line_end]).expect("заголовок не ASCII");
        let mut fields = header.split_ascii_whitespace();
        assert_eq!(
            fields.next(),
            Some("0"),
            "подсекция начинается с нулевого объекта"
        );
        let size: u32 = fields
            .next()
            .expect("нет размера")
            .parse()
            .expect("размер не число");
        pos = line_end + 1;

        let mut entries = Vec::with_capacity(size as usize);
        for _ in 0..size {
            let line = &bytes[pos..pos + 20];
            let text = std::str::from_utf8(line).expect("xref не ASCII");
            let mut fields = text.split_ascii_whitespace();
            let offset: u64 = fields
                .next()
                .expect("нет смещения")
                .parse()
                .expect("смещение не число");
            let generation: u16 = fields
                .next()
                .expect("нет generation")
                .parse()
                .expect("generation не число");
            let used = fields.next() == Some("n");
            entries.push((offset, generation, used));
            pos += 20;
        }
        assert_eq!(&bytes[pos..pos + b"trailer\n".len()], b"trailer\n");
        (table, size, entries)
    }

    #[test]
    fn zero_pages_round_trip() {
        let mut doc = PdfDocument::new("Пустой");
        let mut bytes = Vec::new();
        let mut warnings: Vec<PdfWarnMsg> = Vec::new();
        serialize_pdf_streaming(&mut doc, &PdfSaveOptions::default(), &mut bytes, &mut warnings)
            .expect("нулевая страница не должна падать");

        let parsed = parse(&bytes);
        assert!(parsed.get_pages().is_empty());
        assert!(doc.pages.is_empty(), "страницы забраны из документа");

        let catalog = parsed.catalog().expect("каталог есть");
        let pages_id = catalog
            .get(b"Pages")
            .and_then(Object::as_reference)
            .expect("нет /Pages");
        let pages = parsed
            .get_object(pages_id)
            .and_then(Object::as_dict)
            .expect("/Pages — словарь");
        assert_eq!(
            pages.get(b"Count").and_then(Object::as_i64).expect("нет /Count"),
            0
        );
        assert!(pages
            .get(b"Kids")
            .and_then(Object::as_array)
            .expect("нет /Kids")
            .is_empty());
    }

    #[test]
    fn one_page_round_trips() {
        let mut doc = PdfDocument::new("Одна");
        doc.with_pages(vec![text_page("Одна страница")]);
        let mut bytes = Vec::new();
        let mut warnings: Vec<PdfWarnMsg> = Vec::new();
        serialize_pdf_streaming(&mut doc, &PdfSaveOptions::default(), &mut bytes, &mut warnings)
            .expect("одна страница не должна падать");

        let parsed = parse(&bytes);
        let pages = parsed.get_pages();
        assert_eq!(pages.len(), 1);
        let stream = content_stream(&parsed, pages[&1]);
        assert!(stream.dict.get(b"Length").is_ok(), "у потока нет /Length");
    }

    #[test]
    fn streaming_matches_baseline_objects_and_content() {
        let mut doc = tricky_document();
        let opts = PdfSaveOptions::default();
        let mut warnings: Vec<PdfWarnMsg> = Vec::new();

        let baseline = doc.save(&opts, &mut warnings);
        let mut streamed = Vec::new();
        serialize_pdf_streaming(&mut doc, &opts, &mut streamed, &mut warnings)
            .expect("стриминг не должен падать");
        assert!(doc.pages.is_empty(), "страницы обязаны освобождаться");

        let base = parse(&baseline);
        let new = parse(&streamed);

        assert_eq!(base.get_pages(), new.get_pages(), "id и порядок страниц");
        assert_eq!(
            base.objects, new.objects,
            "все объекты документа, включая сжатые потоки"
        );

        for (number, page_id) in base.get_pages() {
            let old = content_stream(&base, page_id);
            let fresh = content_stream(&new, new.get_pages()[&number]);
            assert_eq!(
                old.content, fresh.content,
                "поток содержимого страницы {number}"
            );
            assert_eq!(old.dict, fresh.dict, "словарь потока страницы {number}");
        }

        let mut base_trailer = base.trailer.clone();
        let mut new_trailer = new.trailer.clone();
        base_trailer.remove(b"ID");
        new_trailer.remove(b"ID");
        assert_eq!(base_trailer, new_trailer, "trailer без случайного /ID");
    }

    #[test]
    fn outline_and_annotations_survive_streaming() {
        let mut doc = tricky_document();
        let mut bytes = Vec::new();
        let mut warnings: Vec<PdfWarnMsg> = Vec::new();
        serialize_pdf_streaming(&mut doc, &PdfSaveOptions::default(), &mut bytes, &mut warnings)
            .expect("стриминг не должен падать");
        let parsed = parse(&bytes);

        let catalog = parsed.catalog().expect("каталог есть");
        let outlines_id = catalog
            .get(b"Outlines")
            .and_then(Object::as_reference)
            .expect("каталог потерял /Outlines");
        let outlines = parsed
            .get_object(outlines_id)
            .and_then(Object::as_dict)
            .expect("/Outlines — словарь");
        assert_eq!(
            outlines
                .get(b"Count")
                .and_then(Object::as_i64)
                .expect("нет /Count"),
            2
        );

        let first_id = outlines
            .get(b"First")
            .and_then(Object::as_reference)
            .expect("нет /First");
        let last_id = outlines
            .get(b"Last")
            .and_then(Object::as_reference)
            .expect("нет /Last");
        let first = parsed
            .get_object(first_id)
            .and_then(Object::as_dict)
            .expect("закладка — словарь");
        let last = parsed
            .get_object(last_id)
            .and_then(Object::as_dict)
            .expect("закладка — словарь");
        assert_eq!(
            first.get(b"Next").and_then(Object::as_reference).expect("нет /Next"),
            last_id
        );
        assert_eq!(
            last.get(b"Prev").and_then(Object::as_reference).expect("нет /Prev"),
            first_id
        );

        // Заголовок закладки — UTF-16BE с BOM, как кодирует общий код.
        let mut expected = vec![0xFE, 0xFF];
        for unit in "Глава (1) \\ конец".encode_utf16() {
            expected.extend(unit.to_be_bytes());
        }
        let title = first
            .get(b"Title")
            .and_then(Object::as_str)
            .expect("нет /Title");
        assert_eq!(title, expected.as_slice());

        let page_ids = parsed.get_pages();
        for bookmark in [first, last] {
            let dest = bookmark
                .get(b"Dest")
                .and_then(Object::as_array)
                .expect("нет /Dest");
            let target = dest
                .first()
                .and_then(|o| o.as_reference().ok())
                .expect("/Dest не на страницу");
            assert!(
                page_ids.values().any(|id| *id == target),
                "закладка ведёт в никуда"
            );
        }

        // Аннотации: /Annots лежит в словаре страницы, не в ресурсах.
        let page1 = parsed
            .get_object(page_ids[&1])
            .and_then(Object::as_dict)
            .expect("страница — словарь");
        let annots = page1
            .get(b"Annots")
            .and_then(Object::as_array)
            .expect("страница потеряла /Annots");
        assert_eq!(annots.len(), 2);
        let resources = page1
            .get(b"Resources")
            .and_then(Object::as_reference)
            .expect("нет /Resources");
        let resources = parsed
            .get_object(resources)
            .and_then(Object::as_dict)
            .expect("ресурсы — словарь");
        assert!(
            resources.get(b"Annots").is_err(),
            "аннотации вернулись в ресурсы"
        );

        let goto = annots[0].as_dict().expect("аннотация — словарь");
        assert_eq!(
            goto.get(b"Subtype").and_then(Object::as_name).expect("нет /Subtype"),
            &b"Link"[..]
        );
        let action = goto.get(b"A").and_then(Object::as_dict).expect("нет /A");
        assert_eq!(
            action.get(b"S").and_then(Object::as_name).expect("нет /S"),
            &b"GoTo"[..]
        );
        let dest = action.get(b"D").and_then(Object::as_array).expect("нет /D");
        assert_eq!(
            dest.first().and_then(|o| o.as_reference().ok()),
            page_ids.get(&2).copied()
        );

        let uri = annots[1].as_dict().expect("аннотация — словарь");
        let action = uri.get(b"A").and_then(Object::as_dict).expect("нет /A");
        assert_eq!(
            action.get(b"S").and_then(Object::as_name).expect("нет /S"),
            &b"URI"[..]
        );
    }

    #[test]
    fn xref_offsets_point_at_objects_and_trailer_is_well_formed() {
        let mut doc = tricky_document();
        let mut bytes = Vec::new();
        let mut warnings: Vec<PdfWarnMsg> = Vec::new();
        serialize_pdf_streaming(&mut doc, &PdfSaveOptions::default(), &mut bytes, &mut warnings)
            .expect("стриминг не должен падать");
        let parsed = parse(&bytes);

        let (table, size, entries) = parse_xref(&bytes);
        assert!(bytes.ends_with(b"%%EOF"), "файл не закрыт %%EOF");

        let startxref_at = find_last(&bytes, b"startxref").expect("нет startxref");
        let tail = std::str::from_utf8(&bytes[startxref_at..]).expect("хвост не ASCII");
        let startxref: usize = tail[b"startxref".len()..]
            .trim_start()
            .lines()
            .next()
            .expect("нет значения startxref")
            .trim()
            .parse()
            .expect("startxref не число");
        assert_eq!(startxref, table, "startxref не на таблицу xref");

        assert!(!entries[0].2, "нулевой объект обязан быть свободным");
        assert_eq!(entries[0].1, 65535);

        let mut used = 0usize;
        for id in 1..size {
            let (offset, generation, is_used) = entries[id as usize];
            if !is_used {
                continue;
            }
            used += 1;
            let header = format!("{id} {generation} obj");
            assert!(
                bytes[offset as usize..].starts_with(header.as_bytes()),
                "xref для объекта {id} указывает мимо заголовка"
            );
        }
        assert_eq!(
            used,
            parsed.objects.len(),
            "не все объекты попали в xref"
        );
        assert_eq!(
            parsed
                .trailer
                .get(b"Size")
                .and_then(Object::as_i64)
                .expect("в трейлере нет /Size"),
            i64::from(size)
        );
    }

    #[test]
    fn optimize_toggles_content_compression() {
        let mut doc = tricky_document();
        let mut warnings: Vec<PdfWarnMsg> = Vec::new();

        let mut compressed = Vec::new();
        serialize_pdf_streaming(&mut doc, &PdfSaveOptions::default(), &mut compressed, &mut warnings)
            .expect("стриминг не должен падать");
        let parsed = parse(&compressed);
        let page_id = *parsed.get_pages().get(&1).expect("нет первой страницы");
        let stream = content_stream(&parsed, page_id);
        assert_eq!(
            stream
                .dict
                .get(b"Filter")
                .and_then(Object::as_name)
                .expect("поток не сжат"),
            &b"FlateDecode"[..]
        );

        let mut doc = tricky_document();
        let opts = PdfSaveOptions {
            optimize: false,
            ..PdfSaveOptions::default()
        };
        let mut plain = Vec::new();
        serialize_pdf_streaming(&mut doc, &opts, &mut plain, &mut warnings)
            .expect("стриминг не должен падать");
        let parsed = parse(&plain);
        let page_id = *parsed.get_pages().get(&1).expect("нет первой страницы");
        let stream = content_stream(&parsed, page_id);
        assert!(
            stream.dict.get(b"Filter").is_err(),
            "optimize=false обязан оставить поток несжатым"
        );
        assert!(
            stream.content.windows(2).any(|w| w == b"BT"),
            "несжатый поток должен читаться как content stream"
        );
    }

    #[test]
    fn write_page_beyond_page_count_is_an_error() {
        let doc = PdfDocument::new("Пустой");
        let opts = PdfSaveOptions::default();
        let mut warnings: Vec<PdfWarnMsg> = Vec::new();
        let mut out = Vec::new();
        let mut session = StreamSession::begin(
            &doc,
            &[],
            BTreeSet::new(),
            0,
            &opts,
            &mut out,
            &mut warnings,
        )
        .expect("сессия на нуле страниц");

        let error = session
            .write_page(&text_page("лишняя"), &mut warnings)
            .expect_err("лишнюю страницу обязано отклонить");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        session.finish().expect("пустой файл дописывается");
        assert!(parse(&out).get_pages().is_empty());
    }
}
