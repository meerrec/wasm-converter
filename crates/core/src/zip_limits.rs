//! Лимиты разбора ZIP-пакетов OOXML (ADR-0015).
//!
//! Пакет приходит из недоверенного источника, поэтому каждая часть
//! проверяется по имени, размеру и степени сжатия до распаковки.
use crate::error::{Error, Result};

/// Лимиты разбора OOXML-пакета (ADR-0015 §1).
///
/// Значения по умолчанию рассчитаны на реальные документы OPC и с запасом
/// покрывают их; [`ZipLimits::unlimited`] снимает проверки для тестов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipLimits {
    /// Максимум распакованных байт одной части, по умолчанию `64 МиБ`.
    pub per_part_uncompressed: u64,
    /// Максимум распакованных байт всего архива, по умолчанию `256 МиБ`.
    pub per_archive_uncompressed: u64,
    /// Предельное отношение распакованного к сжатому, по умолчанию 200.
    pub max_ratio: u32,
    /// Максимум частей в архиве, по умолчанию 4096.
    pub max_parts: u32,
    /// Максимум длины имени части в байтах, по умолчанию 255.
    pub max_name_len: u32,
    /// Максимум сегментов в пути части, по умолчанию 16.
    pub max_depth: u32,
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            per_part_uncompressed: 64 * 1024 * 1024,
            per_archive_uncompressed: 256 * 1024 * 1024,
            max_ratio: 200,
            max_parts: 4096,
            max_name_len: 255,
            max_depth: 16,
        }
    }
}

impl ZipLimits {
    /// Без лимитов — для тестов и внутренних сценариев (ADR-0015 §5).
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            per_part_uncompressed: u64::MAX,
            per_archive_uncompressed: u64::MAX,
            max_ratio: u32::MAX,
            max_parts: u32::MAX,
            max_name_len: u32::MAX,
            max_depth: u32::MAX,
        }
    }

    /// Проверка имени части: path traversal, длина, глубина (ADR-0015 §2).
    ///
    /// Завершающий `/` отличает каталог от файла: он не сегмент, поэтому
    /// пустой последний сегмент у каталога допустим.
    ///
    /// # Errors
    /// [`Error::ZipInvalidPath`] — пустое имя, ведущий `/`, `\`, `\0`, сегмент
    /// `..` или пустой сегмент; [`Error::ZipNameTooLong`] — имя длиннее
    /// `max_name_len` байт; [`Error::ZipPathTooDeep`] — сегментов больше
    /// `max_depth`.
    pub fn check_name(&self, name: &str) -> Result<()> {
        if name.is_empty() || name.starts_with('/') || name.contains('\\') || name.contains('\0') {
            return Err(Error::ZipInvalidPath {
                name: name.to_string(),
            });
        }

        if name.len() > usize::try_from(self.max_name_len).unwrap_or(usize::MAX) {
            return Err(Error::ZipNameTooLong {
                name: name.to_string(),
            });
        }

        let trimmed = name.strip_suffix('/').unwrap_or(name);
        let mut depth = 0usize;
        for segment in trimmed.split('/') {
            if segment.is_empty() || segment == ".." {
                return Err(Error::ZipInvalidPath {
                    name: name.to_string(),
                });
            }
            depth += 1;
        }

        if depth > usize::try_from(self.max_depth).unwrap_or(usize::MAX) {
            return Err(Error::ZipPathTooDeep {
                name: name.to_string(),
            });
        }

        Ok(())
    }

    /// Проверка числа частей в архиве.
    ///
    /// # Errors
    /// [`Error::ZipTooManyParts`] — частей больше `max_parts`.
    pub fn check_parts(&self, count: usize) -> Result<()> {
        if count > usize::try_from(self.max_parts).unwrap_or(usize::MAX) {
            return Err(Error::ZipTooManyParts {
                count: u32::try_from(count).unwrap_or(u32::MAX),
            });
        }
        Ok(())
    }

    /// Проверка размеров части по заголовкам центрального каталога.
    ///
    /// Пустые части и каталоги (нулевой распакованный размер) пропускаются.
    ///
    /// # Errors
    /// [`Error::ZipPartTooLarge`] — часть больше `per_part_uncompressed`;
    /// [`Error::ZipRatioExceeded`] — отношение выше `max_ratio`.
    pub fn check_part(&self, name: &str, uncompressed: u64, compressed: u64) -> Result<()> {
        if uncompressed == 0 && compressed == 0 {
            return Ok(());
        }

        if uncompressed > self.per_part_uncompressed {
            return Err(Error::ZipPartTooLarge {
                name: name.to_string(),
                size: uncompressed,
            });
        }

        // Умножение, а не деление: `compressed == 0` иначе пришлось бы
        // выделять в отдельный случай, а `saturating_mul` не даёт переполниться
        // при `max_ratio` из `unlimited()`.
        if uncompressed > compressed.saturating_mul(u64::from(self.max_ratio)) {
            return Err(Error::ZipRatioExceeded {
                name: name.to_string(),
                ratio: uncompressed / compressed.max(1),
            });
        }

        Ok(())
    }

    /// Проверка суммарного распакованного размера архива.
    ///
    /// # Errors
    /// [`Error::ZipArchiveTooLarge`] — сумма больше `per_archive_uncompressed`.
    pub fn check_total(&self, total: u64) -> Result<()> {
        if total > self.per_archive_uncompressed {
            return Err(Error::ZipArchiveTooLarge { size: total });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_adr() {
        let l = ZipLimits::default();
        assert_eq!(l.per_part_uncompressed, 64 * 1024 * 1024);
        assert_eq!(l.per_archive_uncompressed, 256 * 1024 * 1024);
        assert_eq!(l.max_ratio, 200);
        assert_eq!(l.max_parts, 4096);
        assert_eq!(l.max_name_len, 255);
        assert_eq!(l.max_depth, 16);
    }

    #[test]
    fn unlimited_removes_every_check() {
        let l = ZipLimits::unlimited();
        // Имена проверяются всегда: path traversal — не размер, а адресация.
        assert!(l.check_name("a/../b.xml").is_err());
        assert!(l.check_parts(usize::try_from(u32::MAX).unwrap()).is_ok());
        assert!(l.check_part("big", 1 << 32, 1 << 20).is_ok());
        assert!(l.check_total(u64::MAX).is_ok());
    }

    #[test]
    fn check_name_accepts_regular_parts() {
        let l = ZipLimits::default();
        assert!(l.check_name("word/document.xml").is_ok());
        assert!(l.check_name("[Content_Types].xml").is_ok());
        assert!(l.check_name("_rels/.rels").is_ok());
        // Каталог: завершающий `/` не сегмент.
        assert!(l.check_name("word/media/").is_ok());
    }

    #[test]
    fn check_name_rejects_traversal() {
        let l = ZipLimits::default();
        for name in [
            "",
            "/abs.xml",
            "../evil.xml",
            "a/../b.xml",
            "a\\b.xml",
            "a\0b.xml",
            "a//b",
        ] {
            assert!(
                matches!(l.check_name(name), Err(Error::ZipInvalidPath { .. })),
                "имя {name:?} должно быть отвергнуто"
            );
        }
    }

    #[test]
    fn check_name_boundaries() {
        let l = ZipLimits::default();

        let at_limit = "a".repeat(255);
        assert!(l.check_name(&at_limit).is_ok());
        let over_limit = "a".repeat(256);
        assert!(matches!(
            l.check_name(&over_limit),
            Err(Error::ZipNameTooLong { .. })
        ));

        let deep = vec!["a"; 16].join("/");
        assert!(l.check_name(&deep).is_ok());
        let too_deep = vec!["a"; 17].join("/");
        assert!(matches!(
            l.check_name(&too_deep),
            Err(Error::ZipPathTooDeep { .. })
        ));
    }

    #[test]
    fn check_parts_boundaries() {
        let l = ZipLimits::default();
        assert!(l.check_parts(0).is_ok());
        assert!(l.check_parts(4096).is_ok());
        assert!(matches!(
            l.check_parts(4097),
            Err(Error::ZipTooManyParts { count: 4097 })
        ));
    }

    #[test]
    fn check_part_boundaries() {
        let l = ZipLimits::default();

        // Ровно на лимите — ок, на байт больше — ошибка. Сжатый размер берём
        // такой, чтобы не сработал ratio: 64 МиБ при 200:1 — это 335 КиБ.
        let compressed = l.per_part_uncompressed / 200 + 1;
        assert!(l
            .check_part("p", l.per_part_uncompressed, compressed)
            .is_ok());
        assert!(matches!(
            l.check_part("p", l.per_part_uncompressed + 1, compressed),
            Err(Error::ZipPartTooLarge { .. })
        ));

        // Ratio: ровно 200:1 — ок, 200.1:1 — ошибка.
        assert!(l.check_part("p", 2000, 10).is_ok());
        assert!(matches!(
            l.check_part("p", 2001, 10),
            Err(Error::ZipRatioExceeded { ratio: 200, .. })
        ));

        // Пустая часть и каталог не нарушают ничего: compressed == 0 без
        // распакованных данных — это норма.
        assert!(l.check_part("dir/", 0, 0).is_ok());
    }

    #[test]
    fn check_part_ratio_with_zero_compressed() {
        let l = ZipLimits::default();
        assert!(matches!(
            l.check_part("bomb", 1024, 0),
            Err(Error::ZipRatioExceeded { ratio: 1024, .. })
        ));
    }

    #[test]
    fn check_total_boundaries() {
        let l = ZipLimits::default();
        assert!(l.check_total(l.per_archive_uncompressed).is_ok());
        assert!(matches!(
            l.check_total(l.per_archive_uncompressed + 1),
            Err(Error::ZipArchiveTooLarge { .. })
        ));
    }
}
