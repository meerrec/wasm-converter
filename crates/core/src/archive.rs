//! Read-only OOXML (ZIP) package.
use std::io::{Cursor, Read};

use crate::error::{Error, Result};

pub struct Archive {
    inner: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Archive {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let inner = zip::ZipArchive::new(Cursor::new(bytes))?;
        Ok(Self { inner })
    }

    pub fn len(&self) -> usize { self.inner.len() }
    pub fn is_empty(&self) -> bool { self.inner.is_empty() }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.file_names()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.inner.file_names().any(|n| n == name)
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let mut file = self
            .inner
            .by_name(name)
            .map_err(|_| Error::MissingPart(name.to_string()))?;
        let mut buf = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }

    pub fn read_string(&mut self, name: &str) -> Result<String> {
        let bytes = self.read(name)?;
        String::from_utf8(bytes).map_err(|e| Error::Malformed(e.to_string()))
    }

    /// Validate presence of `[Content_Types].xml` and `_rels/.rels`.
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
                w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
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
