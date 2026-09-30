//! Read-only OOXML (ZIP) package.
use std::io::{Cursor, Read};

use crate::error::{Error, Result};

pub struct Archive {
    inner: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Archive {
    /// Открывает OOXML-пакет из байтов ZIP.
    ///
    /// # Errors
    /// Если байты не являются корректным ZIP-архивом.
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let inner = zip::ZipArchive::new(Cursor::new(bytes))?;
        Ok(Self { inner })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.file_names()
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.inner.file_names().any(|n| n == name)
    }

    /// Читает part целиком.
    ///
    /// # Errors
    /// Если part отсутствует или не читается из архива.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let mut file = self
            .inner
            .by_name(name)
            .map_err(|_| Error::MissingPart(name.to_string()))?;
        // На 32-битных целях (wasm32) размер не всегда влезает в usize:
        // тогда не резервируем заранее — `read_to_end` дорастит буфер сам.
        let size = usize::try_from(file.size()).unwrap_or_default();
        let mut buf = Vec::with_capacity(size);
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Читает part как UTF-8 строку.
    ///
    /// # Errors
    /// Если part отсутствует или содержит невалидный UTF-8.
    pub fn read_string(&mut self, name: &str) -> Result<String> {
        let bytes = self.read(name)?;
        String::from_utf8(bytes).map_err(|e| Error::Malformed(e.to_string()))
    }

    /// Проверяет наличие `[Content_Types].xml` и `_rels/.rels`.
    ///
    /// # Errors
    /// Если обязательный part отсутствует в пакете.
    pub fn validate_ooxml(&self) -> Result<()> {
        if !self.contains(crate::CONTENT_TYPES) {
            return Err(Error::MissingPart(crate::CONTENT_TYPES.into()));
        }
        if !self.contains(crate::ROOT_RELS) {
            return Err(Error::MissingPart(crate::ROOT_RELS.into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, data) in entries {
                w.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn reads_entry() {
        let bytes = make_zip(&[("[Content_Types].xml", b"<Types/>")]);
        let mut a = Archive::new(bytes).unwrap();
        assert_eq!(a.read_string("[Content_Types].xml").unwrap(), "<Types/>");
    }

    #[test]
    fn missing_part_is_error() {
        let bytes = make_zip(&[("a.xml", b"")]);
        let mut a = Archive::new(bytes).unwrap();
        assert!(matches!(a.read("b.xml"), Err(Error::MissingPart(_))));
    }
}
