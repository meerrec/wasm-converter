//! Прототип потоковой записи PDF — спайк B1 спринта 7.
//!
//! Механизм: сессия записи. Глобальные объекты (шрифты, XObject'ы, ExtGState)
//! пишутся один раз в начале; каждая страница переводится в объекты PDF и
//! немедленно сериализуется в `Write`, после чего освобождается — документ
//! не накапливает объекты страниц. xref собирается инкрементально, трейлер
//! и /Pages пишутся в конце.
//!
//! lopdf 0.35 не публикует `Writer::write_indirect_object`, поэтому объекты
//! сериализует собственный код (см. `write_value`), а таблицу xref —
//! `StreamWriter::finish`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use lopdf::{
    Dictionary as LoDictionary,
    Object::{Array, Boolean, Dictionary, Integer, Name, Null, Real, Reference, Stream, String as LoString},
    ObjectId,
    Stream as LoStream,
    StringFormat::{Hexadecimal, Literal},
};

use crate::{
    ops::Op,
    serialize::{
        add_font_to_pdf, builtin_font_to_dict, docinfo_to_dict, get_used_internal_fonts,
        link_annotation_to_dict, prepare_fonts, translate_operations, PreparedFont,
    },
    BuiltinFont, FontId, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg,
};

/// Пишущая обёртка, считающая байты (для xref нужны смещения).
struct CountingWriter<W: Write> {
    inner: W,
    count: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Потоковый писатель объектов PDF: таблица xref строится на лету.
struct StreamWriter<W: Write> {
    out: CountingWriter<W>,
    entries: BTreeMap<u32, (u64, u16)>,
}

impl<W: Write> StreamWriter<W> {
    fn new(writer: W) -> Self {
        Self {
            out: CountingWriter { inner: writer, count: 0 },
            entries: BTreeMap::new(),
        }
    }

    fn begin(&mut self) -> io::Result<()> {
        self.out.write_all(b"%PDF-1.3\n")?;
        // Бинарная метка lopdf по умолчанию.
        self.out.write_all(b"%\xBB\xAD\xC0\xDE\n")
    }

    fn write_indirect(&mut self, id: u32, generation: u16, object: &lopdf::Object) -> io::Result<()> {
        let offset = self.out.count;
        self.entries.insert(id, (offset, generation));
        write!(self.out, "{id} {generation} obj\n")?;
        if needs_separator(object) {
            self.out.write_all(b" ")?;
        }
        write_value(&mut self.out, object)?;
        if needs_end_separator(object) {
            self.out.write_all(b" ")?;
        }
        self.out.write_all(b"\nendobj\n")
    }

    /// Закрывает файл: xref одной секцией 0..=max_id, trailer, startxref.
    fn finish(&mut self, max_id: u32, trailer: &LoDictionary) -> io::Result<()> {
        let startxref = self.out.count;
        self.out.write_all(b"xref\n")?;
        let size = max_id + 1;
        write!(self.out, "0 {size}\n")?;
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

fn needs_separator(object: &lopdf::Object) -> bool {
    matches!(object, Null | Boolean(_) | Integer(_) | Real(_) | Reference(_))
}

fn needs_end_separator(object: &lopdf::Object) -> bool {
    matches!(
        object,
        Null | Boolean(_) | Integer(_) | Real(_) | Name(_) | Reference(_) | Stream(_)
    )
}

fn write_value<W: Write>(w: &mut W, object: &lopdf::Object) -> io::Result<()> {
    match object {
        Null => w.write_all(b"null"),
        Boolean(v) => w.write_all(if *v { b"true" } else { b"false" }),
        Integer(v) => write!(w, "{v}"),
        Real(v) => write!(w, "{v}"),
        LoString(bytes, Hexadecimal) => {
            w.write_all(b"<")?;
            for b in bytes {
                write!(w, "{b:02X}")?;
            }
            w.write_all(b">")
        }
        LoString(bytes, Literal) => {
            w.write_all(b"(")?;
            for &b in bytes {
                match b {
                    b'(' | b')' | b'\\' => {
                        w.write_all(b"\\")?;
                        w.write_all(&[b])?;
                    }
                    b'\r' => w.write_all(b"\\r")?,
                    _ => w.write_all(&[b])?,
                }
            }
            w.write_all(b")")
        }
        Name(name) => write_name(w, name),
        Array(items) => {
            w.write_all(b"[")?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 && needs_separator(item) {
                    w.write_all(b" ")?;
                }
                write_value(w, item)?;
            }
            w.write_all(b"]")
        }
        Dictionary(dict) => write_dictionary(w, dict),
        Stream(stream) => {
            write_dictionary(w, &stream.dict)?;
            w.write_all(b"stream\n")?;
            w.write_all(&stream.content)?;
            w.write_all(b"\nendstream")
        }
        Reference((id, generation)) => write!(w, "{id} {generation} R"),
    }
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
        if b" \t\n\r\x0C()<>[]{}/%#".contains(&byte) || !(33..=126).contains(&byte) {
            write!(w, "#{byte:02X}")?;
        } else {
            w.write_all(&[byte])?;
        }
    }
    Ok(())
}

/// Сессия потоковой записи.
///
/// `page_count` нужен заранее: id всех страниц резервируются в начале, иначе
/// ссылки-аннотации на последующие страницы (`/Dest`) было бы некуда вести.
pub struct StreamSession<'p, 'w, W: Write> {
    sw: StreamWriter<&'w mut W>,
    pdf: &'p PdfDocument,
    secure: bool,
    next_id: u32,
    pages_id: ObjectId,
    catalog: LoDictionary,
    global_font_dict_id: ObjectId,
    global_xobject_dict_id: ObjectId,
    global_extgstate_dict_id: ObjectId,
    prepared_fonts: BTreeMap<FontId, PreparedFont>,
    page_ids_reserved: Vec<ObjectId>,
    page_ids: Vec<ObjectId>,
    next_page: usize,
}

impl<'p, 'w, W: Write> StreamSession<'p, 'w, W> {
    /// Резервирует глобальные объекты и пишет их в поток.
    ///
    /// `builtin_fonts` — какие стандартные шрифты использует документ: их
    /// нельзя вывести из страниц, потому что страниц в памяти ещё нет
    /// (их точный аналог — `get_used_internal_fonts` по готовым страницам).
    pub fn begin(
        pdf: &'p PdfDocument,
        builtin_fonts: BTreeSet<BuiltinFont>,
        page_count: usize,
        opts: &PdfSaveOptions,
        writer: &'w mut W,
        warnings: &mut Vec<PdfWarnMsg>,
    ) -> io::Result<Self> {
        let mut doc = lopdf::Document::with_version("1.3");
        doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;

        let mut global_xobject_dict = LoDictionary::new();
        for (k, v) in pdf.resources.xobjects.map.iter() {
            global_xobject_dict.set(
                k.0.clone(),
                crate::xobject::add_xobject_to_document(
                    v,
                    &mut doc,
                    opts.image_optimization.as_ref(),
                ),
            );
        }

        let pages_id = doc.new_object_id();
        let mut catalog = LoDictionary::from_iter(vec![
            ("Type", "Catalog".into()),
            ("PageLayout", "OneColumn".into()),
            ("PageMode", "UseNone".into()),
            ("Pages", Reference(pages_id)),
        ]);

        // ICC/XMP/слои — как serialize.rs:92-160; дефолтные опции их не включают.

        let mut global_font_dict = LoDictionary::new();
        let prepared_fonts = prepare_fonts(&pdf.resources, &pdf.pages, warnings);
        for (font_id, prepared) in prepared_fonts.iter() {
            let font_dict = add_font_to_pdf(&mut doc, font_id, prepared);
            let font_dict_id = doc.add_object(font_dict);
            global_font_dict.set(font_id.0.clone(), Reference(font_dict_id));
        }
        for internal_font in builtin_fonts {
            let font_dict = builtin_font_to_dict(&internal_font);
            let font_dict_id = doc.add_object(font_dict);
            global_font_dict.set(internal_font.get_pdf_id(), Reference(font_dict_id));
        }
        let global_font_dict_id = doc.add_object(global_font_dict);
        let global_xobject_dict_id = doc.add_object(global_xobject_dict);

        let mut global_extgstate_dict = LoDictionary::new();
        for (k, v) in pdf.resources.extgstates.map.iter() {
            global_extgstate_dict.set(k.0.clone(), crate::graphics::extgstate_to_dict(v));
        }
        let global_extgstate_dict_id = doc.add_object(global_extgstate_dict);

        let page_ids_reserved = (0..page_count).map(|_| doc.new_object_id()).collect::<Vec<_>>();
        let next_id = doc.max_id + 1;

        let mut sw = StreamWriter::new(writer);
        sw.begin()?;
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
            global_font_dict_id,
            global_xobject_dict_id,
            global_extgstate_dict_id,
            prepared_fonts,
            page_ids_reserved,
            page_ids: Vec::with_capacity(page_count),
            next_page: 0,
        })
    }

    fn alloc_id(&mut self) -> ObjectId {
        let id = (self.next_id, 0);
        self.next_id += 1;
        id
    }

    /// Сериализует страницу и отпускает её содержимое.
    pub fn write_page(
        &mut self,
        page: &PdfPage,
        warnings: &mut Vec<PdfWarnMsg>,
    ) -> io::Result<()> {
        let page_id = *self
            .page_ids_reserved
            .get(self.next_page)
            .ok_or_else(|| io::Error::other("page count exceeded"))?;
        self.next_page += 1;

        let mut page_resources = LoDictionary::new();
        let links = page
            .ops
            .iter()
            .filter_map(|l| match l {
                Op::LinkAnnotation { link } => Some(link.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        page_resources.set(
            "Annots",
            Array(
                links
                    .iter()
                    .map(|l| Dictionary(link_annotation_to_dict(l, &self.page_ids_reserved)))
                    .collect(),
            ),
        );
        page_resources.set("Font", Reference(self.global_font_dict_id));
        page_resources.set("XObject", Reference(self.global_xobject_dict_id));
        page_resources.set("ExtGState", Reference(self.global_extgstate_dict_id));

        let layer_stream = translate_operations(
            &page.ops,
            &self.prepared_fonts,
            &self.pdf.resources.xobjects.map,
            self.secure,
            warnings,
        );
        let merged_layer_stream =
            LoStream::new(LoDictionary::new(), layer_stream).with_compression(false);

        let resources_id = self.alloc_id();
        let content_id = self.alloc_id();
        let page_obj = LoDictionary::from_iter(vec![
            ("Type", "Page".into()),
            ("MediaBox", page.get_media_box()),
            ("TrimBox", page.get_trim_box()),
            ("CropBox", page.get_crop_box()),
            ("Parent", Reference(self.pages_id)),
            ("Resources", Reference(resources_id)),
            ("Contents", Reference(content_id)),
        ]);

        self.sw
            .write_indirect(resources_id.0, resources_id.1, &Dictionary(page_resources))?;
        self.sw
            .write_indirect(content_id.0, content_id.1, &Stream(merged_layer_stream))?;
        self.sw
            .write_indirect(page_id.0, page_id.1, &Dictionary(page_obj))?;
        self.page_ids.push(page_id);
        Ok(())
    }

    /// Дописывает outline, /Pages, каталог, info и xref.
    pub fn finish(mut self) -> io::Result<()> {
        if !self.pdf.bookmarks.map.is_empty() {
            let bookmarks_id = self.alloc_id();
            let mut bookmarks_sorted = self.pdf.bookmarks.map.iter().collect::<Vec<_>>();
            bookmarks_sorted
                .sort_by(|(_, v), (_, v2)| (v.page, &v.name).cmp(&(v2.page, &v2.name)));
            let bookmarks_sorted = bookmarks_sorted
                .into_iter()
                .filter_map(|(k, v)| {
                    let page_obj_id = self.page_ids.get(v.page.saturating_sub(1)).copied()?;
                    Some((k, &v.name, page_obj_id))
                })
                .collect::<Vec<_>>();

            let bookmark_ids = bookmarks_sorted
                .iter()
                .map(|(id, name, page_id)| {
                    let newid = self.alloc_id();
                    (*id, *name, *page_id, newid)
                })
                .collect::<Vec<_>>();

            let first = bookmark_ids.first().map(|s| s.3).unwrap();
            let last = bookmark_ids.last().map(|s| s.3).unwrap();
            for (i, (_id, name, pageid, self_id)) in bookmark_ids.iter().enumerate() {
                let prev = if i == 0 {
                    None
                } else {
                    bookmark_ids.get(i - 1).map(|s| s.3)
                };
                let next = bookmark_ids.get(i + 1).map(|s| s.3);
                let dest = Array(vec![Reference(*pageid), "XYZ".into(), Null, Null, Null]);
                let mut dict = LoDictionary::from_iter(vec![
                    ("Parent", Reference(bookmarks_id)),
                    ("Title", crate::serialize::encode_text_to_utf16be(name)),
                    ("Dest", dest),
                ]);
                if let Some(prev) = prev {
                    dict.set("Prev", Reference(prev));
                }
                if let Some(next) = next {
                    dict.set("Next", Reference(next));
                }
                self.sw
                    .write_indirect(self_id.0, self_id.1, &Dictionary(dict))?;
            }

            let bookmarks_list = LoDictionary::from_iter(vec![
                ("Type", "Outlines".into()),
                ("Count", Integer(self.pdf.bookmarks.map.len() as i64)),
                ("First", Reference(first)),
                ("Last", Reference(last)),
            ]);
            self.sw.write_indirect(
                bookmarks_id.0,
                bookmarks_id.1,
                &Dictionary(bookmarks_list),
            )?;
            self.catalog.set("Outlines", Reference(bookmarks_id));
            self.catalog.set("PageMode", LoString("UseOutlines".into(), Literal));
        }

        let pages_dict = LoDictionary::from_iter(vec![
            ("Type", "Pages".into()),
            ("Count", Integer(self.page_ids.len() as i64)),
            (
                "Kids",
                Array(
                    self.page_ids
                        .iter()
                        .map(|q| Reference(*q))
                        .collect::<Vec<_>>(),
                ),
            ),
        ]);
        let pages_id = self.pages_id;
        self.sw
            .write_indirect(pages_id.0, pages_id.1, &Dictionary(pages_dict))?;

        let catalog_id = self.alloc_id();
        self.sw
            .write_indirect(catalog_id.0, catalog_id.1, &Dictionary(std::mem::take(&mut self.catalog)))?;
        let document_info_id = self.alloc_id();
        self.sw.write_indirect(
            document_info_id.0,
            document_info_id.1,
            &Dictionary(docinfo_to_dict(&self.pdf.metadata.info)),
        )?;

        let instance_id = crate::utils::random_character_string_32();
        let document_id = crate::utils::random_character_string_32();
        let mut trailer = LoDictionary::new();
        trailer.set("Size", Integer(i64::from(self.next_id)));
        trailer.set("Root", Reference(catalog_id));
        trailer.set("Info", Reference(document_info_id));
        trailer.set(
            "ID",
            Array(vec![
                LoString(document_id.as_bytes().to_vec(), Literal),
                LoString(instance_id.as_bytes().to_vec(), Literal),
            ]),
        );
        let max_id = self.next_id - 1;
        self.sw.finish(max_id, &trailer)
    }
}

/// Однократная потоковая сериализация готового документа: страницы забираются
/// из `pdf.pages` в порядке следования и освобождаются по одной.
pub fn serialize_pdf_streaming<W: Write>(
    pdf: &mut PdfDocument,
    opts: &PdfSaveOptions,
    writer: &mut W,
    warnings: &mut Vec<PdfWarnMsg>,
) -> io::Result<()> {
    let builtin_fonts = get_used_internal_fonts(&pdf.pages);
    let page_count = pdf.pages.len();
    let pages = std::mem::take(&mut pdf.pages);
    let mut session: StreamSession<'_, '_, W> =
        StreamSession::begin(pdf, builtin_fonts, page_count, opts, writer, warnings)?;
    for page in &pages {
        session.write_page(page, warnings)?;
    }
    drop(pages);
    session.finish()
}
