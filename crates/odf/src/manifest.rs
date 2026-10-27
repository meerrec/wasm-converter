//! Каталог ODF-пакета: `mimetype` и `META-INF/manifest.xml`.
//!
//! `mimetype` — первая запись ZIP без сжатия; по ней пакет и опознаётся.
//! `META-INF/manifest.xml` перечисляет части и их media-type; каталог нужен,
//! чтобы отличить ODF от случайного ZIP и чтобы следующие срезы (картинки,
//! вложенные объекты) знали, что лежит в пакете.
//!
//! Порядок записей виден через [`Archive::names`], а способ сжатия — нет:
//! `core` не отдаёт метод сжатия части, и проверка «`mimetype` хранится, а не
//! сжат» здесь не выполняется. Содержимое `mimetype` от этого не меняется.

use doc_converter_core::xml::XmlReader;
use doc_converter_core::Archive;
use quick_xml::events::Event;

use crate::error::{OdfError, Result};
use crate::xml::{find, map_core};

/// Часть с media-type пакета; по спецификации — первая запись архива.
pub const MIMETYPE_PART: &str = "mimetype";

/// Каталог частей пакета.
pub const MANIFEST_PART: &str = "META-INF/manifest.xml";

/// `mimetype` текстового документа ODF.
pub const ODT_MIME: &str = "application/vnd.oasis.opendocument.text";

/// `mimetype` электронной таблицы ODF.
pub const ODS_MIME: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// `mimetype` презентации ODF.
pub const ODP_MIME: &str = "application/vnd.oasis.opendocument.presentation";

/// Вид ODF-пакета, объявленный в `mimetype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    /// Текстовый документ (`.odt`).
    Odt,
    /// Электронная таблица (`.ods`).
    Ods,
    /// Презентация (`.odp`).
    Odp,
}

impl PackageKind {
    /// MIME-тип, который обязан лежать в `mimetype`.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Odt => ODT_MIME,
            Self::Ods => ODS_MIME,
            Self::Odp => ODP_MIME,
        }
    }

    /// Расширение пакета без точки.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Odt => "odt",
            Self::Ods => "ods",
            Self::Odp => "odp",
        }
    }

    /// Вид по содержимому `mimetype`; `None` — это не ODF или шаблон.
    #[must_use]
    pub fn from_mime(mime: &str) -> Option<Self> {
        match mime {
            ODT_MIME => Some(Self::Odt),
            ODS_MIME => Some(Self::Ods),
            ODP_MIME => Some(Self::Odp),
            _ => None,
        }
    }
}

/// Запись каталога: часть пакета и её media-type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Путь части внутри пакета; у корня — `/`.
    pub full_path: String,
    /// Media-type части, если объявлен.
    pub media_type: Option<String>,
}

/// Разобранный каталог ODF-пакета.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Вид пакета из `mimetype`.
    pub kind: PackageKind,
    /// Версия ODF из `manifest:version` (`1.2`), если объявлена.
    pub version: Option<String>,
    /// Записи каталога в порядке файла; первая обычно корень `/`.
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    /// Запись каталога по пути части.
    #[must_use]
    pub fn entry(&self, full_path: &str) -> Option<&ManifestEntry> {
        self.entries
            .iter()
            .find(|entry| entry.full_path == full_path)
    }

    /// Media-type корня пакета: у ODS это и есть `mimetype`.
    #[must_use]
    pub fn root_media_type(&self) -> Option<&str> {
        self.entry("/")?.media_type.as_deref()
    }
}

/// Прочитать `mimetype` и `META-INF/manifest.xml` пакета.
///
/// `mimetype` обязан быть первой записью архива — так требует ODF 1.2. Способ
/// сжатия не проверяется: [`Archive`] его не отдаёт (см. описание модуля).
///
/// # Errors
/// [`OdfError::Malformed`] — `mimetype` нет, он не первый или не UTF-8;
/// каталога нет или он не разбирается; media-type корня противоречит
/// `mimetype`. [`OdfError::Unsupported`] — `mimetype` не принадлежит ODF
/// (шаблоны `.ots`/`.ott`/`.otp` и посторонние ZIP сюда же).
/// [`OdfError::Core`] — архив не читается (ZIP, лимиты, XML).
pub fn read(archive: &mut Archive) -> Result<Manifest> {
    // `names()` отдаёт части в порядке центрального каталога — того же, в каком
    // их пишет любой генератор ODF. Сравнение до чтения: отсутствующий
    // `mimetype` должен дать Malformed, а не MissingPart из глубины архива.
    if archive.names().next().as_deref() != Some(MIMETYPE_PART) {
        return Err(OdfError::malformed(
            MIMETYPE_PART,
            "the package must start with an uncompressed `mimetype` part",
        ));
    }

    let raw = archive.part(MIMETYPE_PART)?;
    let mime = std::str::from_utf8(&raw)
        .map_err(|error| {
            OdfError::malformed(MIMETYPE_PART, format!("mimetype is not UTF-8: {error}"))
        })?
        .trim();
    let kind = PackageKind::from_mime(mime).ok_or_else(|| {
        OdfError::Unsupported(format!(
            "mimetype `{mime}` is not an ODF 1.2 document (ODT, ODS or ODP)"
        ))
    })?;

    if !archive.contains(MANIFEST_PART) {
        return Err(OdfError::malformed(
            MANIFEST_PART,
            "the package has no `META-INF/manifest.xml`",
        ));
    }
    let bytes = archive.part(MANIFEST_PART)?;
    let (version, entries) = parse(&bytes, MANIFEST_PART)?;

    // Корневая запись дублирует mimetype. Если она с ним спорит, пакет собран
    // не по спецификации, и непонятно, какому из двух источников верить.
    if let Some(root) = entries
        .iter()
        .find(|entry| entry.full_path == "/")
        .and_then(|entry| entry.media_type.as_deref())
    {
        if root != kind.mime() {
            return Err(OdfError::malformed(
                MANIFEST_PART,
                format!(
                    "the root media type `{root}` contradicts mimetype `{}`",
                    kind.mime()
                ),
            ));
        }
    }

    Ok(Manifest {
        kind,
        version,
        entries,
    })
}

/// Разобрать `META-INF/manifest.xml`.
///
/// Каталог — плоский список `manifest:file-entry`; вложенность частей
/// (например, `Pictures/`) задаётся путём, а не структурой XML, поэтому
/// достаточно одного прохода.
fn parse(bytes: &[u8], part: &str) -> Result<(Option<String>, Vec<ManifestEntry>)> {
    let mut reader = XmlReader::new(bytes, part);
    let mut version = None;
    let mut entries = Vec::new();

    while let Some(event) = reader.next_significant()? {
        let (Event::Start(element) | Event::Empty(element)) = event else {
            continue;
        };
        let attrs =
            doc_converter_core::xml::attributes(&element).map_err(|error| map_core(part, error))?;
        match element.local_name().as_ref() {
            b"manifest" => {
                if let Some(value) = find(&attrs, "version") {
                    version = Some(value.to_owned());
                }
            }
            b"file-entry" => {
                // Запись без пути ничего не адресует; молча пропускаем, как
                // посторонний атрибут: терять из-за неё пакет незачем.
                if let Some(full_path) = find(&attrs, "full-path") {
                    entries.push(ManifestEntry {
                        full_path: full_path.to_owned(),
                        media_type: find(&attrs, "media-type").map(str::to_owned),
                    });
                }
            }
            _ => {}
        }
    }

    Ok((version, entries))
}
