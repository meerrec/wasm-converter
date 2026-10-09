//! Read-only OOXML (ZIP) package.
use std::collections::HashSet;
use std::io::{Cursor, Read};

use crate::error::{Error, Result};
use crate::warning::{ParseWarning, WarningKind};
use crate::zip_limits::ZipLimits;

pub struct Archive {
    inner: zip::ZipArchive<Cursor<Vec<u8>>>,
    limits: ZipLimits,
    /// Части-symlink: игнорируются (ADR-0015 §3), наружу не видны.
    symlinks: HashSet<String>,
    warnings: Vec<ParseWarning>,
    /// Сколько байт уже распаковано: лимит архива проверяется по факту, а не по
    /// заголовкам — суммарный размер из центрального каталога может врать.
    read_uncompressed: u64,
}

impl Archive {
    /// Открывает OOXML-пакет из байтов ZIP с лимитами по умолчанию (ADR-0015 §1).
    ///
    /// # Errors
    /// [`Error::ZipInvalidPath`], [`Error::ZipNameTooLong`], [`Error::ZipPathTooDeep`],
    /// [`Error::ZipTooManyParts`], [`Error::ZipPartTooLarge`],
    /// [`Error::ZipRatioExceeded`] или [`Error::ZipArchiveTooLarge`] — пакет
    /// нарушает лимиты ADR-0015; [`Error::Zip`] — байты не являются корректным
    /// ZIP-архивом.
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        Self::open_with_limits(bytes, ZipLimits::default())
    }

    /// Открывает OOXML-пакет из байтов ZIP с заданными лимитами.
    ///
    /// Имена, число частей и размеры проверяются по заголовкам центрального
    /// каталога; сами данные не распаковываются.
    ///
    /// # Errors
    /// [`Error::ZipInvalidPath`], [`Error::ZipNameTooLong`], [`Error::ZipPathTooDeep`],
    /// [`Error::ZipTooManyParts`], [`Error::ZipPartTooLarge`],
    /// [`Error::ZipRatioExceeded`] или [`Error::ZipArchiveTooLarge`] — пакет
    /// нарушает лимиты; [`Error::Zip`] — байты не являются корректным
    /// ZIP-архивом.
    pub fn open_with_limits(bytes: Vec<u8>, limits: ZipLimits) -> Result<Self> {
        let mut inner = zip::ZipArchive::new(Cursor::new(bytes))?;
        limits.check_parts(inner.len())?;

        let mut symlinks = HashSet::new();
        let mut warnings = Vec::new();
        let mut total: u64 = 0;

        for index in 0..inner.len() {
            let file = inner.by_index(index)?;
            let name = file.name().to_string();
            limits.check_name(&name)?;

            // Symlink в OOXML-пакете не бывает легитимным: его содержимое —
            // путь, а не данные (ADR-0015 §3). Размер такой части из лимитов не
            // считаем, распаковать её всё равно нельзя.
            if file.is_symlink() {
                warnings.push(ParseWarning::at(
                    WarningKind::SymlinkIgnored,
                    "symlink part is ignored",
                    name.clone(),
                ));
                symlinks.insert(name);
                continue;
            }

            limits.check_part(&name, file.size(), file.compressed_size())?;
            total = total.saturating_add(file.size());
            limits.check_total(total)?;
        }

        Ok(Self {
            inner,
            limits,
            symlinks,
            warnings,
            read_uncompressed: 0,
        })
    }

    #[must_use]
    pub fn limits(&self) -> &ZipLimits {
        &self.limits
    }

    /// Предупреждения, собранные при открытии (symlink): узел ADR-0016.
    #[must_use]
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }

    /// Забирает накопленные предупреждения, оставляя архив пустым.
    pub fn take_warnings(&mut self) -> Vec<ParseWarning> {
        std::mem::take(&mut self.warnings)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        // Игнорируемые части не видны снаружи, поэтому и в счёте их нет.
        self.inner.len() - self.symlinks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner
            .file_names()
            .filter(|name| !self.symlinks.contains(*name))
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        !self.symlinks.contains(name) && self.inner.file_names().any(|n| n == name)
    }

    /// Читает part целиком, соблюдая лимиты при распаковке.
    ///
    /// # Errors
    /// [`Error::MissingPart`] — part отсутствует в пакете или это symlink;
    /// [`Error::ZipPartTooLarge`] / [`Error::ZipArchiveTooLarge`] — распакованные
    /// данные превысили лимиты (в том числе если размер в заголовке занижен);
    /// [`Error::Zip`] / [`Error::Io`] — данные не распаковываются.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        if self.symlinks.contains(name) {
            return Err(Error::MissingPart(name.to_string()));
        }

        let limit = self.limits.per_part_uncompressed;
        let file = self
            .inner
            .by_name(name)
            .map_err(|_| Error::MissingPart(name.to_string()))?;

        // На 32-битных целях (wasm32) размер не всегда влезает в usize:
        // тогда не резервируем заранее — `read_to_end` дорастит буфер сам.
        let capacity = usize::try_from(file.size().min(limit)).unwrap_or_default();
        let mut buf = Vec::with_capacity(capacity);
        // Размеру из центрального каталога верить нельзя, поэтому поток
        // обрывается на байте сверх лимита: вранье в заголовке ловится здесь.
        let read = file.take(limit.saturating_add(1)).read_to_end(&mut buf)?;
        let read = u64::try_from(read).unwrap_or(u64::MAX);

        if read > limit {
            return Err(Error::ZipPartTooLarge {
                name: name.to_string(),
                size: read,
            });
        }

        self.read_uncompressed = self.read_uncompressed.saturating_add(read);
        self.limits.check_total(self.read_uncompressed)?;
        Ok(buf)
    }

    /// Читает part как UTF-8 строку.
    ///
    /// # Errors
    /// Если part отсутствует, содержит невалидный UTF-8 или превышает лимиты.
    pub fn read_string(&mut self, name: &str) -> Result<String> {
        let bytes = self.read(name)?;
        String::from_utf8(bytes).map_err(|e| Error::Malformed(e.to_string()))
    }

    /// Имя из ADR-0015 §5; синоним [`Archive::read`] — точка входа для docx.
    ///
    /// # Errors
    /// Те же, что у [`Archive::read`].
    pub fn part(&mut self, name: &str) -> Result<Vec<u8>> {
        self.read(name)
    }

    /// Читает `_rels/<part>.rels`, если он есть в пакете.
    ///
    /// `None` — части rels нет; это не ошибка, а повод для предупреждения у
    /// вызывающего парсера.
    ///
    /// # Errors
    /// [`Error::MissingPart`], [`Error::Zip`], [`Error::Io`] — как у [`Archive::read`];
    /// [`Error::Malformed`] / [`Error::Xml`] — rels не разбирается.
    pub fn rels_for(&mut self, source_part: &str) -> Result<Option<crate::rels::RelMap>> {
        let name = crate::rels::rels_part(source_part);
        if !self.contains(&name) {
            return Ok(None);
        }
        let bytes = self.read(&name)?;
        Ok(Some(crate::rels::RelMap::parse(&bytes)?))
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

    /// [`Archive`] не реализует `Debug`, поэтому ошибку достаём через `err()`.
    fn open_err(result: Result<Archive>) -> Error {
        result.err().expect("ожидали ошибку")
    }

    fn make_symlink_zip(name: &str, target: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            w.add_symlink(name, target, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.finish().unwrap();
        }
        buf
    }

    /// Псевдослучайные байты: deflate их почти не сжимает, поэтому размер
    /// части задаётся напрямую, а ratio остаётся в норме.
    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 8);
        let mut x = seed | 1;
        while out.len() < len {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.truncate(len);
        out
    }

    /// Переписывает uncompressed size в центральном каталоге: так выглядит
    /// архив, который врёт о размере части в заголовке.
    fn patch_central_uncompressed(bytes: &mut [u8], size: u32) {
        let eocd = bytes
            .windows(4)
            .rposition(|w| w.starts_with(b"PK\x05\x06"))
            .expect("EOCD не найден");
        let cd_offset = usize::try_from(u32::from_le_bytes(
            bytes[eocd + 16..eocd + 20].try_into().unwrap(),
        ))
        .unwrap();
        assert!(bytes[cd_offset..].starts_with(b"PK\x01\x02"));
        bytes[cd_offset + 24..cd_offset + 28].copy_from_slice(&size.to_le_bytes());
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

    #[test]
    fn zip_bomb_ratio_is_rejected() {
        // 20 МиБ нулей deflate сжимает в сотни раз — классическая бомба.
        let bytes = make_zip(&[("word/document.xml", &vec![0u8; 20 * 1024 * 1024])]);

        let err = open_err(Archive::new(bytes.clone()));
        assert!(
            matches!(err, Error::ZipRatioExceeded { ref name, .. } if name == "word/document.xml"),
            "ожидали ZipRatioExceeded, получили {err:?}"
        );

        let mut a = Archive::open_with_limits(bytes, ZipLimits::unlimited()).unwrap();
        assert_eq!(a.read("word/document.xml").unwrap().len(), 20 * 1024 * 1024);
    }

    #[test]
    fn oversized_part_is_rejected() {
        let data = noise(4096, 0xdead_beef);
        let bytes = make_zip(&[("word/document.xml", &data)]);
        let limits = ZipLimits {
            per_part_uncompressed: 1024,
            ..ZipLimits::default()
        };

        let err = open_err(Archive::open_with_limits(bytes, limits));
        assert!(
            matches!(err, Error::ZipPartTooLarge { size: 4096, .. }),
            "ожидали ZipPartTooLarge, получили {err:?}"
        );
    }

    #[test]
    fn zip_slip_names_are_rejected() {
        for name in ["../evil.xml", "/abs.xml", "a\\b.xml"] {
            let bytes = make_zip(&[(name, b"<x/>")]);
            let err = open_err(Archive::new(bytes));
            assert!(
                matches!(err, Error::ZipInvalidPath { .. }),
                "имя {name:?}: ожидали ZipInvalidPath, получили {err:?}"
            );
        }
    }

    #[test]
    fn long_and_deep_names_are_rejected() {
        let long = format!("{}.xml", "a".repeat(300));
        let err = open_err(Archive::new(make_zip(&[(&long, b"<x/>")])));
        assert!(
            matches!(err, Error::ZipNameTooLong { .. }),
            "ожидали ZipNameTooLong, получили {err:?}"
        );

        let deep = vec!["a"; 17].join("/");
        let err = open_err(Archive::new(make_zip(&[(&deep, b"<x/>")])));
        assert!(
            matches!(err, Error::ZipPathTooDeep { .. }),
            "ожидали ZipPathTooDeep, получили {err:?}"
        );
    }

    #[test]
    fn too_many_parts_is_rejected() {
        let bytes = make_zip(&[("a.xml", b"a"), ("b.xml", b"b"), ("c.xml", b"c")]);
        let limits = ZipLimits {
            max_parts: 2,
            ..ZipLimits::default()
        };

        let err = open_err(Archive::open_with_limits(bytes, limits));
        assert!(
            matches!(err, Error::ZipTooManyParts { count: 3 }),
            "ожидали ZipTooManyParts, получили {err:?}"
        );
    }

    #[test]
    fn symlink_parts_are_ignored() {
        let bytes = make_symlink_zip("evil.xml", "../../etc/passwd");
        let mut a = Archive::new(bytes).unwrap();

        assert_eq!(a.len(), 0);
        assert!(a.is_empty());
        assert!(!a.contains("evil.xml"));
        assert_eq!(a.names().count(), 0);
        assert!(matches!(a.read("evil.xml"), Err(Error::MissingPart(_))));
        assert!(matches!(a.part("evil.xml"), Err(Error::MissingPart(_))));

        let warnings = a.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::SymlinkIgnored);
        assert_eq!(
            warnings[0].location.as_ref().unwrap().part.as_str(),
            "evil.xml"
        );

        assert_eq!(a.take_warnings().len(), 1);
        assert!(a.warnings().is_empty());
    }

    #[test]
    fn lying_header_size_is_caught_on_read() {
        // Заголовок утверждает 10 байт, а deflate-поток разворачивается в 4096:
        // проверки при открытии такой архив проходит.
        let data = noise(4096, 0x1234_5678);
        let mut bytes = make_zip(&[("word/document.xml", &data)]);
        patch_central_uncompressed(&mut bytes, 10);

        let limits = ZipLimits {
            per_part_uncompressed: 1024,
            ..ZipLimits::default()
        };
        let mut a = Archive::open_with_limits(bytes, limits).unwrap();

        let err = a.read("word/document.xml").unwrap_err();
        // Размер в ошибке — прочитанное, то есть лимит + 1: настоящий размер
        // потока неизвестен, обрывать его чтение дольше незачем.
        assert!(
            matches!(err, Error::ZipPartTooLarge { size: 1025, .. }),
            "ожидали ZipPartTooLarge, получили {err:?}"
        );
    }

    #[test]
    fn archive_limit_counts_actual_reads() {
        let data = noise(1000, 42);
        let bytes = make_zip(&[("a.xml", &data)]);
        let limits = ZipLimits {
            per_archive_uncompressed: 1500,
            ..ZipLimits::default()
        };
        let mut a = Archive::open_with_limits(bytes, limits).unwrap();

        assert_eq!(a.read("a.xml").unwrap().len(), 1000);
        let err = a.read("a.xml").unwrap_err();
        assert!(
            matches!(err, Error::ZipArchiveTooLarge { size: 2000 }),
            "ожидали ZipArchiveTooLarge, получили {err:?}"
        );
    }

    #[test]
    fn truncated_archive_is_zip_error() {
        let mut bytes = make_zip(&[("a.xml", b"<x/>")]);
        bytes.truncate(bytes.len() - 100);

        let err = open_err(Archive::new(bytes));
        assert!(
            matches!(err, Error::Zip(_)),
            "ожидали Error::Zip, получили {err:?}"
        );
    }

    #[test]
    fn rels_for_reads_sibling_rels() {
        let rels = br#"<Relationships><Relationship Id="rId1" Type="t" Target="styles.xml"/></Relationships>"#;
        let bytes = make_zip(&[
            ("word/document.xml", b"<w:document/>"),
            ("word/_rels/document.xml.rels", rels),
        ]);
        let mut a = Archive::new(bytes).unwrap();

        let map = a.rels_for("word/document.xml").unwrap().unwrap();
        assert_eq!(map.get("rId1").unwrap().target, "styles.xml");
        assert!(a.rels_for("word/missing.xml").unwrap().is_none());
    }

    #[test]
    fn validate_ooxml_needs_content_types() {
        let bytes = make_zip(&[("a.xml", b"")]);
        let a = Archive::new(bytes).unwrap();
        assert!(matches!(
            a.validate_ooxml(),
            Err(Error::MissingPart(part)) if part == crate::CONTENT_TYPES
        ));
    }
}
