//! Сериализация объектов PDF, xref и сжатие — писатель своего бэкенда
//! (ADR-0022).
//!
//! Перенос стримингового писателя из вендоренного форка (наш код из форка,
//! ADR-0010).
//! Синтаксис значений, разделители и экранирование повторяют `lopdf::writer`
//! байт в байт, иначе пути разошлись бы на одних и тех же данных; свой
//! сериализатор нужен потому, что модуль `writer` у lopdf приватный, а
//! документ пишется постранично в `W: Write` без seek и накопления объектов.

use std::collections::BTreeMap;
use std::io::{self, Write};

use lopdf::{
    Dictionary, Object,
    StringFormat::{Hexadecimal, Literal},
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
pub(crate) struct StreamWriter<W: Write> {
    out: CountingWriter<W>,
    /// id → (смещение, generation). Таблица пишется одной секцией в `finish`.
    entries: BTreeMap<u32, (u64, u16)>,
    /// Сжимать потоки — там же, где это делает `Document::compress()` при
    /// `optimize`: до записи, чтобы `/Filter` и `/Length` попали в файл.
    compress: bool,
}

impl<W: Write> StreamWriter<W> {
    pub(crate) fn new(writer: W, compress: bool) -> Self {
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
    ///
    /// # Errors
    ///
    /// Ошибка записи в приёмник.
    pub(crate) fn begin(&mut self, version: &str, binary_mark: &[u8]) -> io::Result<()> {
        writeln!(self.out, "%PDF-{version}")?;
        self.out.write_all(b"%")?;
        self.out.write_all(binary_mark)?;
        self.out.write_all(b"\n")
    }

    /// # Errors
    ///
    /// Ошибка записи в приёмник.
    pub(crate) fn write_indirect(
        &mut self,
        id: u32,
        generation: u16,
        object: &Object,
    ) -> io::Result<()> {
        let offset = self.out.count;
        self.entries.insert(id, (offset, generation));
        writeln!(self.out, "{id} {generation} obj")?;
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
    ///
    /// # Errors
    ///
    /// Ошибка записи или flush приёмника.
    pub(crate) fn finish(&mut self, max_id: u32, trailer: &Dictionary) -> io::Result<()> {
        let startxref = self.out.count;
        self.out.write_all(b"xref\n")?;
        let size = max_id + 1;
        writeln!(self.out, "0 {size}")?;
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

fn write_dictionary<W: Write>(w: &mut W, dict: &Dictionary) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use lopdf::{Dictionary, Document, Object, Stream, StringFormat};

    use super::StreamWriter;

    /// Собирает файл: объекты, xref одной секцией, trailer.
    fn write_file(objects: &[(u32, u16, Object)], max_id: u32, compress: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = StreamWriter::new(&mut bytes, compress);
            writer.begin("1.7", b"\xE2\xE3\xCF\xD3").unwrap();
            for (id, generation, object) in objects {
                writer.write_indirect(*id, *generation, object).unwrap();
            }
            writer.finish(max_id, &make_trailer(max_id)).unwrap();
        }
        bytes
    }

    fn make_trailer(max_id: u32) -> Dictionary {
        let mut trailer = Dictionary::new();
        trailer.set("Size", i64::from(max_id) + 1);
        trailer.set("Root", Object::Reference((1, 0)));
        trailer
    }

    #[test]
    fn writes_pdf_header_with_binary_mark() {
        let objects: Vec<(u32, u16, Object)> = Vec::new();
        let bytes = write_file(&objects, 0, false);
        assert!(bytes.starts_with(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n"));
    }

    #[test]
    fn round_trips_objects_through_lopdf() {
        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set(
            "Contents",
            Object::Array(vec![
                Object::Integer(-42),
                Object::Real(3.5),
                Object::Boolean(true),
                Object::Null,
                Object::Name(b"Weird Name#".to_vec()),
                Object::String(b"literal (string)".to_vec(), StringFormat::Literal),
                Object::String(vec![0x00, 0xDE, 0xAD], StringFormat::Hexadecimal),
                Object::Reference((3, 0)),
            ]),
        );
        let stream = Object::Stream(Stream::new(Dictionary::new(), b"stream payload".to_vec()));
        let page = {
            let mut dict = Dictionary::new();
            dict.set("Type", "Page");
            dict.set("Parent", Object::Reference((1, 0)));
            dict
        };
        let objects = vec![
            (1, 0, Object::Dictionary(catalog)),
            (2, 0, stream),
            (3, 0, Object::Dictionary(page)),
        ];

        let bytes = write_file(&objects, 3, false);
        let doc = Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.version, "1.7");
        assert_eq!(doc.get_object((1, 0)).unwrap(), &objects[0].2);
        assert_eq!(doc.get_object((3, 0)).unwrap(), &objects[2].2);
        let parsed_stream = doc.get_object((2, 0)).unwrap().as_stream().unwrap();
        assert_eq!(parsed_stream.content, b"stream payload");
        assert_eq!(
            parsed_stream.dict.get(b"Length").unwrap().as_i64().unwrap(),
            14
        );
    }

    #[test]
    fn compression_follows_the_flag() {
        let content = vec![b'a'; 4096];
        let objects = vec![(
            1,
            0,
            Object::Stream(Stream::new(Dictionary::new(), content.clone())),
        )];

        let plain = Document::load_mem(&write_file(&objects, 1, false)).unwrap();
        let stream = plain.get_object((1, 0)).unwrap().as_stream().unwrap();
        assert!(stream.dict.get(b"Filter").is_err());
        assert_eq!(stream.content, content);

        let packed = Document::load_mem(&write_file(&objects, 1, true)).unwrap();
        let stream = packed.get_object((1, 0)).unwrap().as_stream().unwrap();
        assert_eq!(
            stream.dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"FlateDecode"
        );
        assert_eq!(stream.decompressed_content().unwrap(), content);
    }

    #[test]
    fn strings_survive_round_trip() {
        let literal = b"open ( close ) back \\ return \r high \xFF\x80 nested (( ".to_vec();
        let hex = vec![0x00, 0x10, 0x7F, 0x80, 0xFF, 0x28];
        let mut dict = Dictionary::new();
        dict.set(
            "Literal",
            Object::String(literal.clone(), StringFormat::Literal),
        );
        dict.set(
            "Hex",
            Object::String(hex.clone(), StringFormat::Hexadecimal),
        );
        let objects = vec![(1, 0, Object::Dictionary(dict))];

        let doc = Document::load_mem(&write_file(&objects, 1, false)).unwrap();
        let parsed = doc.get_object((1, 0)).unwrap().as_dict().unwrap();
        assert_eq!(
            parsed.get(b"Literal").unwrap(),
            &Object::String(literal, StringFormat::Literal)
        );
        assert_eq!(
            parsed.get(b"Hex").unwrap(),
            &Object::String(hex, StringFormat::Hexadecimal)
        );
    }

    #[test]
    fn skipped_ids_leave_free_xref_records() {
        let objects = vec![(1, 0, Object::Integer(11)), (3, 0, Object::Integer(33))];
        let bytes = write_file(&objects, 3, false);
        let doc = Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.get_object((1, 0)).unwrap().as_i64().unwrap(), 11);
        assert_eq!(doc.get_object((3, 0)).unwrap().as_i64().unwrap(), 33);
        assert!(doc.get_object((2, 0)).is_err());

        let text = String::from_utf8_lossy(&bytes);
        let xref = text
            .split_once("xref\n")
            .and_then(|(_, rest)| rest.split_once("trailer\n"))
            .unwrap()
            .0;
        // Свободные записи: нулевой объект и пропущенный id 2.
        assert_eq!(xref.matches("0000000000 65535 f \n").count(), 2);
    }
}
