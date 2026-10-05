//! Streaming XML helpers over `quick-xml`.
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::{Error, Result};

pub struct XmlReader<'a> {
    inner: Reader<&'a [u8]>,
    buf: Vec<u8>,
    part: String,
}

impl<'a> XmlReader<'a> {
    /// Читатель, обрезающий пробелы по краям текстовых узлов.
    ///
    /// Годится для частей, где текст незначим (`_rels/*.rels`), но не для
    /// `sharedStrings.xml` или `<w:t>` — там пробелы это данные, и нужен
    /// [`XmlReader::preserving`].
    #[must_use]
    pub fn new(bytes: &'a [u8], part: impl Into<String>) -> Self {
        Self::with_trim(bytes, part, true)
    }

    /// Читатель, отдающий текст ровно так, как он записан.
    ///
    /// `quick-xml` не смотрит на `xml:space`, поэтому «preserve» обеспечивается
    /// здесь: `trim_text` выключен, и `<t xml:space="preserve"> x </t>` доходит
    /// без потерь.
    #[must_use]
    pub fn preserving(bytes: &'a [u8], part: impl Into<String>) -> Self {
        Self::with_trim(bytes, part, false)
    }

    fn with_trim(bytes: &'a [u8], part: impl Into<String>, trim: bool) -> Self {
        let mut inner = Reader::from_reader(bytes);
        inner.config_mut().trim_text(trim);
        Self {
            inner,
            buf: Vec::new(),
            part: part.into(),
        }
    }

    /// Читает следующий значимый event, пропуская `Decl`/`Comment`/`PI`.
    ///
    /// # Errors
    /// Если XML не разбирается.
    pub fn next_significant(&mut self) -> Result<Option<Event<'static>>> {
        loop {
            self.buf.clear();
            let ev = self
                .inner
                .read_event_into(&mut self.buf)
                .map_err(|e| Error::Xml {
                    part: self.part.clone(),
                    position: self.inner.buffer_position(),
                    message: e.to_string(),
                })?;
            match ev {
                Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
                Event::Eof => return Ok(None),
                other => return Ok(Some(other.into_owned())),
            }
        }
    }
}
