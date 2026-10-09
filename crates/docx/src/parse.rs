//! Точка входа разбора: сборка `Document` из частей пакета (слайс S12).
//!
//! Порядок разбора здесь задаёт нумерацию узлов (ADR-0019 §2): таблицы модели
//! (`word/styles.xml`, `word/numbering.xml`, `word/settings.xml`) ID не получают,
//! тело документа получает их первым, за ним — сноски и колонтитулы. Сам
//! `Document` — корень с `NodeId(0)`, зарезервированным под него аллокатором.

use std::collections::BTreeMap;

use doc_converter_core::rels::{rels_part, RelMap};
use doc_converter_core::xml::XmlReader;
use doc_converter_core::zip_limits::ZipLimits;
use doc_converter_core::{Archive, NodeId, ParseWarning, WarningKind, CONTENT_TYPES, ROOT_RELS};
use quick_xml::events::Event;

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::{
    BlockItem, Body, Document, Footnote, HeaderFooter, Inline, NoteKind, NumberingTable, Paragraph,
    PartRef, Relationships, Settings, StyleId, StyleTable, Table,
};
use crate::settings::xml_error;
use crate::xml::local_name;
use crate::{comments, document, footnotes, metadata, numbering, rels, settings, styles};

// ---------------------------------------------------------------------------
// Ранние отказы по сигнатуре файла
// ---------------------------------------------------------------------------

/// Магия CFB-контейнера (OLE2).
const CFB_MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

/// Имена потоков шифрованного OOXML.
///
/// Зашифрованный `.docx` и бинарный `.doc` — один и тот же CFB-контейнер
/// (ADR-0016 §2), и по сигнатуре файла их не различить: форматы разводит каталог
/// потоков — у шифрованного пакета это `EncryptionInfo` и `EncryptedPackage`,
/// у `.doc` — `WordDocument`.
const ENCRYPTION_STREAMS: [&str; 2] = ["EncryptionInfo", "EncryptedPackage"];

/// Content type главной части `.docm`: макросы в v1 — non-goal (ADR-0016 §2).
const MACRO_ENABLED_MAIN: &[u8] = b"application/vnd.ms-word.document.macroEnabled.main+xml";

/// Префикс namespace'ов Strict OOXML (ADR-0017).
const STRICT_PREFIX: &[u8] = b"http://purl.oclc.org/ooxml/";

/// Сколько байт части просматривается в поисках признака формата.
///
/// Каталог CFB и объявления namespace'ов корневого элемента лежат в самом начале
/// файла, а главная часть бывает и на мегабайты: целиком её просматривать незачем.
const SIGNATURE_SCAN: usize = 8 * 1024;

/// Отвергает OLE-контейнер по сигнатуре файла.
///
/// Проверка идёт до ZIP: на CFB-байтах `zip` скажет только «не архив», а
/// пользователю нужно знать, что документ зашифрован или сохранён в старом
/// формате (ADR-0016 §2).
///
/// # Errors
/// [`Error::EncryptedDocument`] — CFB с потоками шифрования;
/// [`Error::LegacyFormat`] — прочий CFB, то есть `.doc`.
fn reject_ole_container(bytes: &[u8]) -> Result<()> {
    if !bytes.starts_with(&CFB_MAGIC) {
        return Ok(());
    }

    let window = scan_window(bytes);
    if ENCRYPTION_STREAMS
        .iter()
        .any(|name| contains(window, name.as_bytes()))
    {
        return Err(Error::EncryptedDocument);
    }
    Err(Error::LegacyFormat)
}

/// Проверяет, что часть не Strict OOXML.
///
/// У Strict другие namespace'ы (ADR-0017 §2), и модель Transitional описала бы
/// такой документ молча и неверно — пустой заготовкой; отказ с понятным
/// сообщением честнее.
///
/// # Errors
/// [`Error::StrictNotSupported`] — в начале части namespace Strict.
fn ensure_transitional(bytes: &[u8]) -> Result<()> {
    if contains(scan_window(bytes), STRICT_PREFIX) {
        return Err(Error::StrictNotSupported);
    }
    Ok(())
}

/// Начало части — столько байт, сколько нужно поиску признака формата.
#[must_use]
fn scan_window(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len().min(SIGNATURE_SCAN)]
}

/// Поиск подстроки по байтам: XML не обязан быть UTF-8, поэтому сравнение идёт
/// без `str`.
#[must_use]
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ---------------------------------------------------------------------------
// Части пакета
// ---------------------------------------------------------------------------

/// Стандартный путь главной части.
const DOCUMENT_PART: &str = "word/document.xml";

/// Где искать необязательную часть: хвост типа связи и стандартный путь.
#[derive(Clone, Copy)]
struct Part {
    /// Хвост типа связи (`/styles`): полный URI версионно-зависим, а хвост у
    /// Transitional и Strict один и тот же (см. [`rels::targets`]).
    suffix: &'static str,
    /// Стандартный путь части (`word/styles.xml`).
    standard: &'static str,
}

/// Главная часть пакета: связь `officeDocument` из `_rels/.rels`.
const OFFICE_DOCUMENT: Part = Part {
    suffix: "/officeDocument",
    standard: DOCUMENT_PART,
};
/// Таблица стилей.
const STYLES: Part = Part {
    suffix: "/styles",
    standard: "word/styles.xml",
};
/// Таблица нумерации.
const NUMBERING: Part = Part {
    suffix: "/numbering",
    standard: "word/numbering.xml",
};
/// Параметры документа.
const SETTINGS: Part = Part {
    suffix: "/settings",
    standard: "word/settings.xml",
};
/// Сноски.
const FOOTNOTES: Part = Part {
    suffix: "/footnotes",
    standard: "word/footnotes.xml",
};
/// Концевые сноски.
const ENDNOTES: Part = Part {
    suffix: "/endnotes",
    standard: "word/endnotes.xml",
};
/// Комментарии.
const COMMENTS: Part = Part {
    suffix: "/comments",
    standard: "word/comments.xml",
};

/// Связи пакета (`_rels/.rels`) — источник имени главной части.
///
/// Отсутствие root rels не фатально (ADR-0016 §2): главная часть берётся по
/// стандартному пути, а потерянные связи пакета становятся предупреждением.
/// Поэтому здесь не зовётся [`rels::load`]: у него часть-источник — имя части, а
/// для пакета она пустая, и в предупреждении оказался бы пустой `part` вместо
/// `_rels/.rels`.
///
/// # Errors
/// [`Error::Malformed`] / [`Error::XmlFatal`] — XML rels не читается;
/// [`Error::Core`] — часть не распаковывается; [`Error::TooManyWarnings`] —
/// предупреждений стало больше порога.
fn root_rels(archive: &mut Archive, ctx: &mut ParseCtx) -> Result<Relationships> {
    if !archive.contains(ROOT_RELS) {
        ctx.warn(
            WarningKind::MissingRels,
            ROOT_RELS,
            format!("no `{ROOT_RELS}`; the main part is taken from `{DOCUMENT_PART}`"),
        )?;
        return Ok(Relationships::default());
    }

    let bytes = read_part(archive, ROOT_RELS)?;
    let map = RelMap::parse(&bytes).map_err(|e| xml_error(ROOT_RELS, e))?;
    Ok(Relationships::from_map(map))
}

/// Имя главной части: `officeDocument` из root rels, иначе стандартный путь.
///
/// Имя берётся из связей, а не жёстко: пакет вправе назвать главную часть иначе.
#[must_use]
fn main_part(root_rels: &Relationships) -> String {
    // Источник — сам пакет (пустое имя): цели root rels разрешаются от его корня.
    rels::targets(root_rels, "", OFFICE_DOCUMENT.suffix)
        .into_iter()
        .find_map(|(_, part, _)| part)
        .unwrap_or_else(|| OFFICE_DOCUMENT.standard.to_owned())
}

/// Необязательная часть пакета: имя и байты, если часть есть.
///
/// Имя ищется сначала среди целей связей главной части — пакет вправе назвать
/// часть иначе, чем стандарт, — и только потом по стандартному пути. Ссылка на
/// отсутствующую часть становится предупреждением [`WarningKind::MissingPart`]
/// (ADR-0016 §2: «часть, на которую ссылаются, отсутствует»); части, на которую
/// не ссылаются и которой нет, просто нет: умолчания модели это не нарушает, а у
/// большинства документов нет ни `word/numbering.xml`, ни `word/settings.xml`.
///
/// # Errors
/// [`Error::XmlFatal`] / [`Error::Core`] — часть не читается;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn optional_part(
    archive: &mut Archive,
    rels: &Relationships,
    source_part: &str,
    spec: Part,
    ctx: &mut ParseCtx,
) -> Result<Option<(String, Vec<u8>)>> {
    for (_id, target, _rel) in rels::targets(rels, source_part, spec.suffix) {
        // Внешняя цель — ссылка, а не часть пакета.
        let Some(target) = target else { continue };

        if archive.contains(&target) {
            let bytes = read_part(archive, &target)?;
            return Ok(Some((target, bytes)));
        }
        ctx.warn(
            WarningKind::MissingPart,
            &target,
            format!("`{target}` is referenced by `{source_part}` but missing from the package"),
        )?;
    }

    if archive.contains(spec.standard) {
        let bytes = read_part(archive, spec.standard)?;
        return Ok(Some((spec.standard.to_owned(), bytes)));
    }
    Ok(None)
}

/// Имя необязательной части: её собственное, если часть есть, иначе стандартный
/// путь — по нему называются предупреждения обхода ссылок.
#[must_use]
fn part_name(part: Option<&(String, Vec<u8>)>, spec: Part) -> String {
    part.map_or_else(|| spec.standard.to_owned(), |(name, _)| name.clone())
}

/// Читает часть пакета, называя её в ошибке.
///
/// # Errors
/// [`Error::XmlFatal`] / [`Error::Malformed`] — ядро не разобрало часть;
/// [`Error::Core`] — часть не распаковывается или превысила лимиты (ADR-0015).
fn read_part(archive: &mut Archive, part: &str) -> Result<Vec<u8>> {
    archive.part(part).map_err(|e| xml_error(part, e))
}

/// Переносит предупреждения, собранные при открытии архива.
///
/// Symlink-части отсеиваются ещё в архиве (ADR-0015 §3), но знать о них должен
/// документ. [`Archive::take_warnings`] отдаёт готовые [`ParseWarning`], а у
/// [`ParseCtx`] вход для такого предупреждения один — [`ParseCtx::warn`]: ни узла,
/// ни пути по XML у symlink-предупреждения нет, терять при переносе нечего, а
/// порог ADR-0016 §6 оказывается общим для всех предупреждений документа.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn adopt_archive_warnings(archive: &mut Archive, ctx: &mut ParseCtx) -> Result<()> {
    for warning in archive.take_warnings() {
        let part = warning
            .location
            .as_ref()
            .map_or_else(String::new, |location| location.part.clone());
        ctx.warn(warning.kind, &part, warning.message)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Таблицы модели
// ---------------------------------------------------------------------------

/// Таблицы, на которые опирается модель.
struct Tables {
    /// Стили из `word/styles.xml`.
    styles: StyleTable,
    /// Нумерация из `word/numbering.xml`.
    numbering: NumberingTable,
    /// Параметры из `word/settings.xml`.
    settings: Settings,
}

/// Разбирает необязательные части таблиц.
///
/// Отсутствие любой из них не нарушение: у модели есть умолчания
/// ([`StyleTable::default`], [`NumberingTable::default`], [`Settings::default`]),
/// а предупреждается только ссылка на отсутствующую часть (ADR-0016 §2).
///
/// # Errors
/// [`Error::XmlFatal`] / [`Error::Malformed`] — часть не читается;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_tables(
    archive: &mut Archive,
    rels: &Relationships,
    document_part: &str,
    ctx: &mut ParseCtx,
) -> Result<Tables> {
    let styles_part = optional_part(archive, rels, document_part, STYLES, ctx)?;
    let numbering_part = optional_part(archive, rels, document_part, NUMBERING, ctx)?;
    let settings_part = optional_part(archive, rels, document_part, SETTINGS, ctx)?;

    let styles = match &styles_part {
        Some((part, bytes)) => styles::parse(bytes, ctx, part)?,
        None => StyleTable::default(),
    };
    let numbering = match &numbering_part {
        Some((part, bytes)) => numbering::parse(bytes, ctx, part)?,
        None => NumberingTable::default(),
    };
    let settings = match &settings_part {
        Some((part, bytes)) => settings::parse(bytes, ctx, part)?,
        None => Settings::default(),
    };

    Ok(Tables {
        styles,
        numbering,
        settings,
    })
}

/// Разбирает часть сносок, если она есть в пакете.
///
/// # Errors
/// [`Error::Malformed`] — в части нет контейнера сносок или поток оборвался;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_note_part(
    part: Option<&(String, Vec<u8>)>,
    rels: &Relationships,
    kind: NoteKind,
    ctx: &mut ParseCtx,
) -> Result<Vec<Footnote>> {
    match part {
        Some((name, bytes)) => footnotes::parse(bytes, rels, ctx, name, kind),
        None => Ok(Vec::new()),
    }
}

// ---------------------------------------------------------------------------
// Колонтитулы
// ---------------------------------------------------------------------------

/// Читает колонтитулы, названные ссылками секций.
///
/// Ключ карты — имя части (`word/header1.xml`): так на колонтитул ссылается
/// `w:headerReference`, и одна часть обслуживает все секции, которые на неё
/// ссылаются.
///
/// # Errors
/// [`Error::Malformed`] — в части нет `w:hdr`/`w:ftr` или поток оборвался;
/// [`Error::XmlFatal`] / [`Error::Core`] — часть не читается;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn load_frames(
    archive: &mut Archive,
    body: &Body,
    ctx: &mut ParseCtx,
) -> Result<(
    BTreeMap<String, HeaderFooter>,
    BTreeMap<String, HeaderFooter>,
)> {
    let mut headers = BTreeMap::new();
    let mut footers = BTreeMap::new();

    for (is_header, reference) in frame_refs(body) {
        // Пустая цель — не ссылка, внешняя — не часть пакета.
        if reference.external || reference.target.is_empty() {
            continue;
        }

        let frames = if is_header {
            &mut headers
        } else {
            &mut footers
        };
        if frames.contains_key(&reference.target) {
            continue;
        }
        if let Some(frame) = read_frame(archive, reference, ctx)? {
            frames.insert(reference.target.clone(), frame);
        }
    }

    Ok((headers, footers))
}

/// Ссылки на колонтитулы всех секций: `(верхний?, ссылка)`.
fn frame_refs(body: &Body) -> impl Iterator<Item = (bool, &PartRef)> {
    body.sections.iter().flat_map(|section| {
        [
            (true, section.header_default.as_ref()),
            (true, section.header_first.as_ref()),
            (true, section.header_even.as_ref()),
            (false, section.footer_default.as_ref()),
            (false, section.footer_first.as_ref()),
            (false, section.footer_even.as_ref()),
        ]
        .into_iter()
        .filter_map(|(is_header, reference)| reference.map(|reference| (is_header, reference)))
    })
}

/// Читает и разбирает часть колонтитула.
///
/// `None` — части нет в пакете; это предупреждение, а не отказ (ADR-0016 §2).
///
/// # Errors
/// [`Error::Malformed`] — в части нет `w:hdr`/`w:ftr` или поток оборвался;
/// [`Error::XmlFatal`] / [`Error::Core`] — часть не читается;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn read_frame(
    archive: &mut Archive,
    reference: &PartRef,
    ctx: &mut ParseCtx,
) -> Result<Option<HeaderFooter>> {
    let part = reference.target.as_str();
    if !archive.contains(part) {
        ctx.warn(
            WarningKind::MissingPart,
            part,
            format!("`{part}` is referenced by a header/footer reference but is missing"),
        )?;
        return Ok(None);
    }

    let bytes = read_part(archive, part)?;
    let rels = optional_rels(archive, part, ctx)?;
    let items = parse_frame(&bytes, &rels, ctx, part)?;
    // У колонтитула и его тела свои узлы модели, поэтому ID два.
    let id = ctx.id();
    let body_id = ctx.id();

    Ok(Some(HeaderFooter {
        id,
        part: part.to_owned(),
        body: Body {
            id: body_id,
            items,
            sections: Vec::new(),
        },
    }))
}

/// Связи части, если `_rels`-часть есть в пакете.
///
/// Колонтитул без связей — норма (ссылок и картинок в нём может не быть), и
/// [`WarningKind::MissingRels`] остаётся за главной частью, где потерянная связь
/// значит больше.
///
/// # Errors
/// [`Error::Malformed`] / [`Error::XmlFatal`] — XML rels не читается.
fn optional_rels(archive: &mut Archive, part: &str, ctx: &mut ParseCtx) -> Result<Relationships> {
    if !archive.contains(&rels_part(part)) {
        return Ok(Relationships::default());
    }
    rels::load(archive, part, ctx)
}

/// Разбирает часть колонтитула: блоки внутри `w:hdr`/`w:ftr`.
///
/// Тело колонтитула — те же блоки, что и у документа, поэтому читает его общий
/// [`document::parse_blocks`]; здесь нужно лишь дойти до корневого элемента части
/// и назвать путь для предупреждений.
///
/// # Errors
/// [`Error::Malformed`] — корневого элемента нет или поток оборвался;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_frame(
    bytes: &[u8],
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Vec<BlockItem>> {
    let mut reader = XmlReader::preserving(bytes, part);
    while let Some(event) = reader.next_significant()? {
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            _ => continue,
        };
        let path = match local_name(element.name().into_inner()) {
            b"hdr" => "w:hdr",
            b"ftr" => "w:ftr",
            _ => continue,
        };

        return if empty {
            Ok(Vec::new())
        } else {
            document::parse_blocks(&mut reader, rels, ctx, part, path)
        };
    }

    Err(Error::malformed(part, "`w:hdr`/`w:ftr` is missing"))
}

// ---------------------------------------------------------------------------
// Проверка ссылок модели
// ---------------------------------------------------------------------------

/// Имена частей, по которым разложены блоки модели.
///
/// Тела частей переезжают в [`Document`] по значению, а имена нужны обходу ссылок
/// и после сборки: `part` предупреждения обязан называть ту часть, где ссылка
/// записана. Колонтитулы называют свои части сами ([`HeaderFooter::part`]).
struct PartNames {
    /// Главная часть — блоки тела документа.
    document: String,
    /// Часть сносок.
    footnotes: String,
    /// Часть концевых сносок.
    endnotes: String,
    /// Часть комментариев.
    comments: String,
}

/// Проверяет ссылки модели на стили и нумерацию.
///
/// Промах не фатален (ADR-0016 §2): стиль игнорируется, список рендерится как
/// обычный абзац, — поэтому каждая ненайденная ссылка становится предупреждением.
/// Обход начинается с блоков документа (того, что видно), продолжается блоками
/// сносок, комментариев и колонтитулов.
///
/// Цепочки `basedOn` и `numId` → `abstractNum` здесь не проверяются: таблицы
/// стилей и нумерации валидируются при разборе их частей, и повторный обход дал
/// бы по предупреждению на каждый дефект вместо одного.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn validate_refs(document: &mut Document, parts: &PartNames, ctx: &mut ParseCtx) -> Result<()> {
    // Поля разводятся на отдельные заимствования: обходу нужны таблицы по `&`.
    let Document {
        body,
        styles,
        numbering,
        footnotes,
        endnotes,
        comments,
        headers,
        footers,
        ..
    } = document;

    let mut sources: Vec<(&str, &[BlockItem])> = vec![(parts.document.as_str(), &body.items)];
    sources.extend(
        footnotes
            .iter()
            .map(|note| (parts.footnotes.as_str(), note.body.as_slice())),
    );
    sources.extend(
        endnotes
            .iter()
            .map(|note| (parts.endnotes.as_str(), note.body.as_slice())),
    );
    sources.extend(
        comments
            .iter()
            .map(|comment| (parts.comments.as_str(), comment.body.as_slice())),
    );
    sources.extend(
        headers
            .values()
            .chain(footers.values())
            .map(|frame| (frame.part.as_str(), frame.body.items.as_slice())),
    );

    for (part, blocks) in sources {
        walk_blocks(blocks, styles, numbering, part, ctx)?;
    }

    Ok(())
}

/// Обходит блоки документа, проверяя ссылки на стили и нумерацию.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn walk_blocks(
    blocks: &[BlockItem],
    styles: &StyleTable,
    numbering: &NumberingTable,
    part: &str,
    ctx: &mut ParseCtx,
) -> Result<()> {
    for block in blocks {
        match block {
            BlockItem::Paragraph(paragraph) => {
                walk_paragraph(paragraph, styles, numbering, part, ctx)?;
            }
            BlockItem::Table(table) => walk_table(table, styles, numbering, part, ctx)?,
            // `w:sectPr` и неподдержанные блоки ссылок на таблицы не несут.
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
    Ok(())
}

/// Проверяет ссылки абзаца: стиль, нумерацию и содержимое.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn walk_paragraph(
    paragraph: &Paragraph,
    styles: &StyleTable,
    numbering: &NumberingTable,
    part: &str,
    ctx: &mut ParseCtx,
) -> Result<()> {
    if let Some(style) = &paragraph.style_ref {
        if !has_style(styles, style) {
            ctx.node_warn(
                WarningKind::MissingStyleRef,
                part,
                paragraph.id,
                format!("paragraph style `{style}` is not defined"),
            )?;
        }
    }

    // `w:numId="0"` снимает нумерацию: это не ссылка (модель хранит её как есть).
    if let Some(num_id) = paragraph.numbering_ref {
        if num_id.value() != 0 && !numbering.nums.contains_key(&num_id) {
            ctx.node_warn(
                WarningKind::MissingNumId,
                part,
                paragraph.id,
                format!("numbering `{}` is not defined", num_id.value()),
            )?;
        }
    }

    walk_runs(&paragraph.runs, styles, part, ctx)
}

/// Проверяет знаковые стили содержимого, спускаясь в ссылки и поля.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn walk_runs(
    inlines: &[Inline],
    styles: &StyleTable,
    part: &str,
    ctx: &mut ParseCtx,
) -> Result<()> {
    for inline in inlines {
        match inline {
            Inline::Run(run) => {
                if let Some(style) = &run.style_ref {
                    if !has_style(styles, style) {
                        ctx.node_warn(
                            WarningKind::MissingStyleRef,
                            part,
                            run.id,
                            format!("run style `{style}` is not defined"),
                        )?;
                    }
                }
            }
            Inline::Hyperlink(hyperlink) => walk_runs(&hyperlink.runs, styles, part, ctx)?,
            Inline::Field(field) => walk_runs(&field.result, styles, part, ctx)?,
            _ => {}
        }
    }
    Ok(())
}

/// Проверяет ссылки таблицы: её стиль и содержимое ячеек, включая вложенные
/// таблицы.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn walk_table(
    table: &Table,
    styles: &StyleTable,
    numbering: &NumberingTable,
    part: &str,
    ctx: &mut ParseCtx,
) -> Result<()> {
    if let Some(style) = &table.style_ref {
        if !styles.table.contains_key(style) {
            ctx.node_warn(
                WarningKind::MissingStyleRef,
                part,
                table.id,
                format!("table style `{style}` is not defined"),
            )?;
        }
    }

    for row in &table.rows {
        for cell in &row.cells {
            walk_blocks(&cell.items, styles, numbering, part, ctx)?;
        }
    }
    Ok(())
}

/// Есть ли стиль в таблице стилей.
///
/// `w:pStyle` и `w:rStyle` ищутся в обеих таблицах — абзацных и знаковых стилей:
/// вид стиля проверяет сама таблица, а ссылка на стиль другого вида не повод для
/// предупреждения. Важно одно: стиль в пакете есть.
#[must_use]
fn has_style(styles: &StyleTable, style: &StyleId) -> bool {
    styles.paragraph.contains_key(style) || styles.character.contains_key(style)
}

// ---------------------------------------------------------------------------
// Сборка
// ---------------------------------------------------------------------------

/// Разбирает пакет DOCX в модель.
///
/// Восстановимые нарушения разбор не прерывают: они копятся в
/// [`Document::warnings`] и попадают в лог (ADR-0016 §1, §4).
///
/// # Errors
/// [`Error::EncryptedDocument`] — зашифрованный OOXML; [`Error::LegacyFormat`] —
/// бинарный `.doc`; [`Error::MacroEnabledDocument`] — `.docm`;
/// [`Error::StrictNotSupported`] — Strict OOXML (ADR-0017);
/// [`Error::MissingPart`] — нет `[Content_Types].xml`;
/// [`Error::MissingDocumentXml`] — нет главной части; [`Error::XmlFatal`] /
/// [`Error::Malformed`] — часть не читается; [`Error::Core`] — распаковка и
/// лимиты ZIP (ADR-0015); [`Error::TooManyWarnings`] — предупреждений больше
/// порога (ADR-0016 §6).
pub fn parse_docx(bytes: &[u8], limits: ZipLimits) -> Result<Document> {
    reject_ole_container(bytes)?;

    let mut archive = Archive::open_with_limits(bytes.to_vec(), limits)?;
    let mut ctx = ParseCtx::new();
    adopt_archive_warnings(&mut archive, &mut ctx)?;

    // `[Content_Types].xml` обязателен: без него пакет не OPC. Проверяется именно
    // он, а не `Archive::validate_ooxml`: та требует ещё и `_rels/.rels`,
    // отсутствие которых здесь — предупреждение, а не отказ (см. `root_rels`).
    if !archive.contains(CONTENT_TYPES) {
        return Err(Error::MissingPart(CONTENT_TYPES.to_owned()));
    }
    let content_types = read_part(&mut archive, CONTENT_TYPES)?;
    if contains(&content_types, MACRO_ENABLED_MAIN) {
        return Err(Error::MacroEnabledDocument);
    }
    ensure_transitional(&content_types)?;

    let root_rels = root_rels(&mut archive, &mut ctx)?;
    let document_part = main_part(&root_rels);
    if !archive.contains(&document_part) {
        return Err(Error::MissingDocumentXml);
    }

    let body_xml = read_part(&mut archive, &document_part)?;
    ensure_transitional(&body_xml)?;
    let rels = rels::load(&mut archive, &document_part, &mut ctx)?;

    let tables = parse_tables(&mut archive, &rels, &document_part, &mut ctx)?;
    let metadata = metadata::parse(&mut archive, &mut ctx)?;

    let body = document::parse(&body_xml, &rels, &mut ctx, &document_part)?;

    let footnotes_part = optional_part(&mut archive, &rels, &document_part, FOOTNOTES, &mut ctx)?;
    let endnotes_part = optional_part(&mut archive, &rels, &document_part, ENDNOTES, &mut ctx)?;
    let comments_part = optional_part(&mut archive, &rels, &document_part, COMMENTS, &mut ctx)?;
    let footnotes = parse_note_part(footnotes_part.as_ref(), &rels, NoteKind::Footnote, &mut ctx)?;
    let endnotes = parse_note_part(endnotes_part.as_ref(), &rels, NoteKind::Endnote, &mut ctx)?;
    let comments = match &comments_part {
        Some((part, bytes)) => comments::parse(bytes, &rels, &mut ctx, part)?,
        None => Vec::new(),
    };

    let (headers, footers) = load_frames(&mut archive, &body, &mut ctx)?;

    let parts = PartNames {
        document: document_part,
        footnotes: part_name(footnotes_part.as_ref(), FOOTNOTES),
        endnotes: part_name(endnotes_part.as_ref(), ENDNOTES),
        comments: part_name(comments_part.as_ref(), COMMENTS),
    };
    let mut document = Document {
        id: NodeId::new(0),
        body,
        styles: tables.styles,
        numbering: tables.numbering,
        settings: tables.settings,
        metadata,
        rels,
        footnotes,
        endnotes,
        comments,
        headers,
        footers,
        warnings: Vec::new(),
    };

    validate_refs(&mut document, &parts, &mut ctx)?;

    // Предупреждения забираются вместе с аллокатором: ID выданы, но контекст
    // остаётся единственным местом, где собран их общий список (ADR-0016 §3).
    let (_, warnings) = ctx.into_parts();
    let warnings = warnings.into_vec();
    log_warnings(&warnings);
    document.warnings = warnings;
    Ok(document)
}

/// Логирует предупреждения разбора.
///
/// Предупреждения — не отказ (ADR-0016 §4), но в логе они быть обязаны: иначе
/// испорченный документ разбирается «молча и неверно».
fn log_warnings(warnings: &[ParseWarning]) {
    for warning in warnings {
        tracing::warn!(
            kind = %warning.kind.as_str(),
            part = %warning
                .location
                .as_ref()
                .map_or("", |location| location.part.as_str()),
            "{}",
            warning.message
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};
    use std::path::PathBuf;

    use super::*;
    use crate::model::RunContent;

    /// Минимальные части OPC-пакета: разбору хватает их.
    const CONTENT_TYPES_XML: &[u8] = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;
    const ROOT_RELS_XML: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    const DOCUMENT_XML: &[u8] = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello, World!</w:t></w:r></w:p></w:body></w:document>"#;
    const EMPTY_RELS_XML: &[u8] =
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;

    /// Байты фикстуры из `test-fixtures/docx`.
    fn fixture(relative: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx")
            .join(relative);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"))
    }

    /// Разбор фикстуры с лимитами по умолчанию.
    fn parse_fixture(relative: &str) -> Result<Document> {
        parse_docx(&fixture(relative), ZipLimits::default())
    }

    /// ZIP в памяти; `symlinks` — части-симлинки, у них отдельный вызов writer'а.
    fn zip(parts: &[(&str, &[u8])], symlinks: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default();
            for (name, data) in parts {
                writer.start_file(*name, options).unwrap();
                writer.write_all(data).unwrap();
            }
            for (name, target) in symlinks {
                writer.add_symlink(name, target, options).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    /// Минимальный пакет: content types, root rels, тело с одним абзацем и
    /// пустой список связей главной части.
    ///
    /// Пустые связи части обязательны: без `word/_rels/document.xml.rels` разбор
    /// предупреждает `MissingRels` (так и задумано), а тесты на этом хелпере
    /// проверяют другие вещи и на лишнее предупреждение спотыкались бы.
    fn minimal_zip(symlinks: &[(&str, &str)]) -> Vec<u8> {
        zip(
            &[
                ("[Content_Types].xml", CONTENT_TYPES_XML),
                ("_rels/.rels", ROOT_RELS_XML),
                ("word/document.xml", DOCUMENT_XML),
                ("word/_rels/document.xml.rels", EMPTY_RELS_XML),
            ],
            symlinks,
        )
    }

    /// Число абзацев тела — то, что в сайдкарах названо `expectedParagraphs`.
    ///
    /// В `items` попадает и завершающий `w:sectPr`, поэтому длину списка с числом
    /// абзацев отождествлять нельзя.
    fn paragraph_count(body: &Body) -> usize {
        body.items
            .iter()
            .filter(|item| matches!(item, BlockItem::Paragraph(_)))
            .count()
    }

    /// Текст абзаца — сцепка текстов его run'ов.
    fn paragraph_text(paragraph: &Paragraph) -> String {
        let mut text = String::new();
        for inline in &paragraph.runs {
            if let Inline::Run(run) = inline {
                for content in &run.content {
                    if let RunContent::Text(part) = content {
                        text.push_str(part);
                    }
                }
            }
        }
        text
    }

    /// Первый блок тела как абзац.
    fn first_paragraph(body: &Body) -> &Paragraph {
        match &body.items[0] {
            BlockItem::Paragraph(paragraph) => paragraph,
            other => panic!("ожидали абзац, пришло {other:?}"),
        }
    }

    fn part_ref(rel_id: &str, target: &str) -> PartRef {
        PartRef {
            rel_id: rel_id.to_owned(),
            target: target.to_owned(),
            external: false,
        }
    }

    // -----------------------------------------------------------------------
    // Битые фикстуры (сайдкары `broken/*.json`)
    // -----------------------------------------------------------------------

    #[test]
    fn truncated_zip_is_a_container_error() {
        let err =
            parse_fixture("broken/truncated_zip.docx").expect_err("обрезанный ZIP не открывается");
        assert!(matches!(err, Error::Core(_)), "{err}");
    }

    #[test]
    fn macro_enabled_document_is_rejected() {
        let err = parse_fixture("broken/macro_enabled.docx").expect_err("`.docm` — non-goal v1");
        assert!(matches!(err, Error::MacroEnabledDocument), "{err}");
    }

    #[test]
    fn missing_document_xml_is_fatal() {
        let err = parse_fixture("broken/no_document_xml.docx")
            .expect_err("без главной части разбирать нечего");
        assert!(matches!(err, Error::MissingDocumentXml), "{err}");
    }

    /// Root rels не обязательны: без них главная часть берётся по стандартному
    /// пути, а связи пакета теряются с предупреждением (так и в сайдкаре
    /// фикстуры — документ разбирается, `expectedParagraphs: 2`).
    #[test]
    fn missing_root_rels_warns_and_the_document_is_kept() {
        let document =
            parse_fixture("broken/missing_root_rels.docx").expect("root rels не обязательны");

        assert_eq!(document.warnings_by_kind(WarningKind::MissingRels).len(), 1);
        assert_eq!(paragraph_count(&document.body), 2);
        let location = document.warnings[0]
            .location
            .as_ref()
            .expect("часть названа");
        assert_eq!(location.part, ROOT_RELS);
    }

    #[test]
    fn cyclic_based_on_warns() {
        let document =
            parse_fixture("broken/cyclic_based_on.docx").expect("цикл `basedOn` не фатален");
        assert_eq!(
            document.warnings_by_kind(WarningKind::CyclicBasedOn).len(),
            1
        );
    }

    #[test]
    fn missing_style_and_abstract_num_warn() {
        let document = parse_fixture("broken/missing_style_and_abstract_num.docx")
            .expect("промахи ссылок не фатальны");

        // Стиль `NoSuchStyle` назван и абзацем, и run'ом.
        assert_eq!(
            document
                .warnings_by_kind(WarningKind::MissingStyleRef)
                .len(),
            2
        );
        // `numId="7"` есть в таблице, но ведёт на отсутствующий `abstractNum`.
        assert_eq!(
            document
                .warnings_by_kind(WarningKind::MissingAbstractNum)
                .len(),
            1
        );
        assert!(document
            .warnings_by_kind(WarningKind::MissingNumId)
            .is_empty());
    }

    // -----------------------------------------------------------------------
    // Разбор частей
    // -----------------------------------------------------------------------

    #[test]
    fn a_simple_document_is_parsed() {
        let document = parse_fixture("simple/one_paragraph.docx").expect("пакет разбирается");

        assert_eq!(
            document.id,
            NodeId::new(0),
            "корень документа — `NodeId(0)`"
        );
        assert_eq!(paragraph_count(&document.body), 1);
        assert_eq!(
            paragraph_text(first_paragraph(&document.body)),
            "Hello, World!"
        );
        assert!(document.warnings.is_empty(), "{:?}", document.warnings);
    }

    /// Отсутствие необязательных частей — не нарушение: у модели есть умолчания,
    /// а предупреждается только ссылка на отсутствующую часть (ADR-0016 §2).
    #[test]
    fn absent_optional_parts_are_silent() {
        let document = parse_docx(&minimal_zip(&[]), ZipLimits::default())
            .expect("минимальный пакет разбирается");

        assert_eq!(document.settings, Settings::default());
        assert!(document.numbering.nums.is_empty());
        assert!(document.styles.paragraph.is_empty());
        assert!(document.warnings.is_empty(), "{:?}", document.warnings);
    }

    #[test]
    fn notes_and_comments_are_parsed_from_their_parts() {
        let document = parse_fixture("notes/footnote_basic.docx").expect("пакет разбирается");
        assert!(
            !document.footnotes.is_empty(),
            "`word/footnotes.xml` разобран"
        );
        assert!(document.endnotes.is_empty());
        assert!(!document.body.items.is_empty());

        let document = parse_fixture("notes/comments_basic.docx").expect("пакет разбирается");
        assert!(
            !document.comments.is_empty(),
            "`word/comments.xml` разобран"
        );
    }

    /// `Document` собирается из тех же байтов одинаково: ID узлов и порядок
    /// предупреждений детерминированы (ADR-0019 §2).
    #[test]
    fn parsing_the_same_bytes_twice_gives_the_same_document() {
        let bytes = fixture("basic/sections.docx");
        let first = parse_docx(&bytes, ZipLimits::default()).expect("пакет разбирается");
        let second = parse_docx(&bytes, ZipLimits::default()).expect("пакет разбирается");

        assert_eq!(first, second);
    }

    // -----------------------------------------------------------------------
    // Колонтитулы
    // -----------------------------------------------------------------------

    #[test]
    fn a_frame_part_is_parsed_from_its_reference() {
        let mut archive =
            Archive::new(fixture("headers_footers/even_odd.docx")).expect("пакет открывается");
        let mut ctx = ParseCtx::new();
        let reference = part_ref("rIdHeader1", "word/header1.xml");

        let header = read_frame(&mut archive, &reference, &mut ctx)
            .expect("колонтитул читается")
            .expect("часть есть в пакете");

        assert_eq!(header.part, "word/header1.xml");
        assert_eq!(
            paragraph_text(first_paragraph(&header.body)),
            "Default header"
        );
        assert!(header.body.sections.is_empty());
        assert!(header.id != header.body.id, "у колонтитула и тела свои ID");
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    /// Колонтитул, на который ссылается документ, но которого нет в пакете, —
    /// предупреждение, а не отказ (ADR-0016 §2).
    #[test]
    fn a_missing_frame_part_warns() {
        let mut archive =
            Archive::new(fixture("headers_footers/even_odd.docx")).expect("пакет открывается");
        let mut ctx = ParseCtx::new();
        let reference = part_ref("rIdHeader9", "word/header9.xml");

        let frame =
            read_frame(&mut archive, &reference, &mut ctx).expect("отсутствие части не фатально");

        assert!(frame.is_none());
        assert_eq!(ctx.warnings().len(), 1);
        assert_eq!(ctx.warnings()[0].kind, WarningKind::MissingPart);
    }

    // -----------------------------------------------------------------------
    // Синтетические пакеты: ранние отказы и лимиты
    // -----------------------------------------------------------------------

    #[test]
    fn content_types_are_required() {
        let bytes = zip(
            &[
                ("_rels/.rels", ROOT_RELS_XML),
                ("word/document.xml", DOCUMENT_XML),
            ],
            &[],
        );

        let err = parse_docx(&bytes, ZipLimits::default()).expect_err("пакет без content types");

        assert!(
            matches!(&err, Error::MissingPart(part) if part == CONTENT_TYPES),
            "{err}"
        );
    }

    #[test]
    fn encrypted_container_is_rejected() {
        let mut bytes = CFB_MAGIC.to_vec();
        bytes.extend_from_slice(b"\x00\x00EncryptionInfo\x00\x00EncryptedPackage\x00");

        let err = parse_docx(&bytes, ZipLimits::default()).expect_err("шифрованный OOXML");

        assert!(matches!(err, Error::EncryptedDocument), "{err}");
    }

    #[test]
    fn legacy_doc_container_is_rejected() {
        let mut bytes = CFB_MAGIC.to_vec();
        bytes.extend_from_slice(b"\x00\x00WordDocument\x00\x01CompObj\x00");

        let err = parse_docx(&bytes, ZipLimits::default()).expect_err("бинарный `.doc`");

        assert!(matches!(err, Error::LegacyFormat), "{err}");
    }

    #[test]
    fn strict_package_is_rejected() {
        let strict = br#"<w:document xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main"><w:body/></w:document>"#;
        let bytes = zip(
            &[
                ("[Content_Types].xml", CONTENT_TYPES_XML),
                ("_rels/.rels", ROOT_RELS_XML),
                ("word/document.xml", strict),
            ],
            &[],
        );

        let err =
            parse_docx(&bytes, ZipLimits::default()).expect_err("Strict — non-goal (ADR-0017)");

        assert!(matches!(err, Error::StrictNotSupported), "{err}");
    }

    #[test]
    fn ratio_limits_reject_a_zip_bomb() {
        // 20 МиБ нулей deflate сжимает в сотни раз — классическая бомба.
        let bomb = vec![0u8; 20 * 1024 * 1024];
        let bytes = zip(
            &[
                ("[Content_Types].xml", CONTENT_TYPES_XML),
                ("_rels/.rels", ROOT_RELS_XML),
                ("word/document.xml", DOCUMENT_XML),
                ("word/media/bomb.bin", bomb.as_slice()),
            ],
            &[],
        );

        let err =
            parse_docx(&bytes, ZipLimits::default()).expect_err("лимиты ADR-0015 режут бомбу");
        assert!(
            matches!(
                err,
                Error::Core(doc_converter_core::Error::ZipRatioExceeded { ref name, .. })
                    if name == "word/media/bomb.bin"
            ),
            "{err}"
        );

        // Без лимитов тот же пакет разбирается: бомбу никто не распаковывает.
        let document =
            parse_docx(&bytes, ZipLimits::unlimited()).expect("без лимитов пакет разбирается");
        assert_eq!(document.body.items.len(), 1);
    }

    /// Symlink-часть архива (ADR-0015 §3) видна документу как предупреждение.
    #[test]
    fn symlink_parts_are_reported() {
        let bytes = minimal_zip(&[("word/evil.xml", "../../etc/passwd")]);

        let document = parse_docx(&bytes, ZipLimits::default()).expect("symlink не мешает разбору");

        assert_eq!(
            document.warnings_by_kind(WarningKind::SymlinkIgnored).len(),
            1
        );
        assert!(!document.warnings.is_empty());
    }
}
