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
    pub fn new(bytes: &'a [u8], part: impl Into<String>) -> Self {
        let mut inner = Reader::from_reader(bytes);
        inner.config_mut().trim_text(true);
        Self { inner, buf: Vec::new(), part: part.into() }
    }

    /// Читает следующий значимый event, пропуская `Decl`/`Comment`/`PI`.
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
                Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => continue,
                Event::Eof => return Ok(None),
                other => return Ok(Some(other.into_owned())),
            }
        }
    }
}
