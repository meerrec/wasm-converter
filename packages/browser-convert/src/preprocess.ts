/**
 * Препроцессоры OOXML перед конвертацией x2t: точечные фиксы файлов, которые
 * сторонние продюсеры записывают не по спецификации. Каждая функция без изменений
 * возвращает исходные байты, не пересобирая архив.
 */
import { listZipEntries, readZipEntry, rewriteZip } from './zip.js';
import type { ZipEntry } from './zip.js';

const UTF8_DECODER = new TextDecoder('utf-8');
const UTF8_ENCODER = new TextEncoder();

/**
 * Общий адаптер пересборки XML-записей: разжатые байты декодируются как UTF-8,
 * transform получает (xml, имя записи); null — запись остаётся без изменений.
 */
async function rewriteXmlEntries(
  bytes: Uint8Array,
  filter: (entry: ZipEntry) => boolean,
  transform: (xml: string, name: string) => string | null,
): Promise<Uint8Array> {
  return rewriteZip(bytes, {
    filter,
    transform: (data, entry) => {
      const replaced = transform(UTF8_DECODER.decode(data), entry.name);
      return replaced === null ? null : replaced;
    },
  });
}

/** Записи XML внутри пакета xl (sharedStrings, worksheets и т. п.). */
const isXlsxXml = (entry: ZipEntry): boolean => entry.name.startsWith('xl/') && entry.name.endsWith('.xml');

/**
 * Снять двойное экранирование переноса строки по всем `xl/*.xml`: `&amp;#10;` → `&#10;`.
 * Без изменений возвращает исходные байты (архив не пересобирается).
 */
export async function preprocessXlsxLineBreaks(xlsxBytes: Uint8Array): Promise<Uint8Array> {
  let needsFix = false;
  for (const entry of listZipEntries(xlsxBytes)) {
    if (!isXlsxXml(entry)) {
      continue;
    }
    const data = await readZipEntry(xlsxBytes, entry);
    if (data === null) {
      continue;
    }
    if (UTF8_DECODER.decode(data).includes('&amp;#10;')) {
      needsFix = true;
      break;
    }
  }
  if (!needsFix) {
    return xlsxBytes;
  }
  return rewriteXmlEntries(xlsxBytes, isXlsxXml, (xml) => {
    const replaced = xml.replaceAll('&amp;#10;', '&#10;');
    return replaced === xml ? null : replaced;
  });
}

/** Записи XML заметок докладчика (notesSlides) и мастеров заметок (notesMasters). */
const isPptxNotesXml = (entry: ZipEntry): boolean =>
  (entry.name.startsWith('ppt/notesSlides/') || entry.name.startsWith('ppt/notesMasters/')) &&
  entry.name.endsWith('.xml');

/** Атрибут showMasterPhAnim целиком — notes-класс SDK зовёт для него несуществующий метод. */
const SHOW_MASTER_PH_ANIM_RE = / showMasterPhAnim="[^"]*"/g;

/** Минимальный docProps/app.xml для pptx, где его нет. */
const PPTX_APP_XML =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Microsoft Office PowerPoint</Application></Properties>';

/** Минимальный docProps/core.xml для pptx, где его нет. */
const PPTX_CORE_XML =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title/><dc:creator/></cp:coreProperties>';

const RELS_PATH = '_rels/.rels';
const APP_PATH = 'docProps/app.xml';
const CORE_PATH = 'docProps/core.xml';
const APP_RELATIONSHIP_TYPE =
  'http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties';
const CORE_RELATIONSHIP_TYPE =
  'http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties';

/** Следующий свободный rIdN по встречающимся в тексте `"rIdN"`; без rId — rId1. */
function nextRelationshipId(relationships: string): string {
  let max = 0;
  for (const match of relationships.matchAll(/"rId(\d+)"/g)) {
    const value = Number(match[1]);
    if (value > max) {
      max = value;
    }
  }
  return `rId${max + 1}`;
}

/** Вставляет Relationship перед `</Relationships>`. */
function insertRelationship(relationships: string, type: string, target: string): string {
  const fragment = `<Relationship Id="${nextRelationshipId(relationships)}" Type="${type}" Target="${target}"/>`;
  return relationships.replace('</Relationships>', fragment + '</Relationships>');
}

/**
 * Фиксы PPTX: срез `showMasterPhAnim` в notesSlides/notesMasters, инжект недостающих
 * `docProps/app.xml` и `docProps/core.xml` с отношениями. Без изменений возвращает
 * исходные байты.
 */
export async function preprocessPptx(pptxBytes: Uint8Array): Promise<Uint8Array> {
  const entries = listZipEntries(pptxBytes);
  const needsApp = !entries.some((entry) => entry.name === APP_PATH);
  const needsCore = !entries.some((entry) => entry.name === CORE_PATH);

  // Изменённые notes-записи вычисляются заранее: если фиксов нет вовсе, архив не пересобирается.
  const notesChanges = new Map<string, string>();
  for (const entry of entries) {
    if (!isPptxNotesXml(entry)) {
      continue;
    }
    const data = await readZipEntry(pptxBytes, entry);
    if (data === null) {
      continue;
    }
    const xml = UTF8_DECODER.decode(data);
    const fixed = xml.replace(SHOW_MASTER_PH_ANIM_RE, '');
    if (fixed !== xml) {
      notesChanges.set(entry.name, fixed);
    }
  }

  // Отношения к недостающим docProps добавляются только в существующий _rels/.rels
  // с закрывающим тегом; тип, уже упомянутый в файле, не дублируется.
  const relsEntry = entries.find((entry) => entry.name === RELS_PATH);
  const relsData = relsEntry === undefined ? null : await readZipEntry(pptxBytes, relsEntry);
  const relsText = relsData === null ? null : UTF8_DECODER.decode(relsData);
  let needsAppRel = false;
  let needsCoreRel = false;
  if (relsText !== null && relsText.includes('</Relationships>')) {
    needsAppRel = needsApp && !relsText.includes('extended-properties');
    needsCoreRel = needsCore && !relsText.includes('core-properties');
  }

  if (!needsApp && !needsCore && notesChanges.size === 0) {
    return pptxBytes;
  }

  let patchedRels: string | null = null;
  if (relsText !== null && (needsAppRel || needsCoreRel)) {
    let text = relsText;
    if (needsAppRel) {
      text = insertRelationship(text, APP_RELATIONSHIP_TYPE, APP_PATH);
    }
    if (needsCoreRel) {
      text = insertRelationship(text, CORE_RELATIONSHIP_TYPE, CORE_PATH);
    }
    patchedRels = text;
  }

  const inject: Array<{ name: string; data: Uint8Array }> = [];
  if (needsApp) {
    inject.push({ name: APP_PATH, data: UTF8_ENCODER.encode(PPTX_APP_XML) });
  }
  if (needsCore) {
    inject.push({ name: CORE_PATH, data: UTF8_ENCODER.encode(PPTX_CORE_XML) });
  }

  const needsRelsPatch = patchedRels !== null;
  return rewriteZip(pptxBytes, {
    filter: (entry) => (needsRelsPatch && entry.name === RELS_PATH) || notesChanges.has(entry.name),
    transform: (_data, entry) => (entry.name === RELS_PATH ? patchedRels : notesChanges.get(entry.name) ?? null),
    inject,
  });
}

/** `<w:ruby>…</w:ruby>` целиком, нежадно — до первого закрывающего тега. */
const RUBY_TAG_RE = /<w:ruby(?:\s[^>]*)?>[\s\S]*?<\/w:ruby>/g;
/** Содержимое первого `<w:rubyBase>…</w:rubyBase>` в совпадении ruby. */
const RUBY_BASE_RE = /<w:rubyBase(?:\s[^>]*)?>([\s\S]*?)<\/w:rubyBase>/;

/**
 * Чистая XML-трансформация: `<w:ruby>…</w:ruby>` → `'</w:r>'` + содержимое
 * `<w:rubyBase>` + `'<w:r>'` (закрыть текущий run, вставить базовые run'ы, снова
 * открыть run — чтобы не вкладывать runs). null, если изменений нет.
 */
export function unwrapRubyXml(xml: string): string | null {
  const unwrapped = xml.replace(RUBY_TAG_RE, (match) => {
    const base = RUBY_BASE_RE.exec(match)?.[1] ?? '';
    return `</w:r>${base}<w:r>`;
  });
  return unwrapped === xml ? null : unwrapped;
}

/** Записи XML внутри пакета word (document, headers, footnotes и т. п.). */
const isDocxXml = (entry: ZipEntry): boolean => entry.name.startsWith('word/') && entry.name.endsWith('.xml');

/**
 * Развернуть `<w:ruby>` в базовый текст по `word/*.xml` (импортёр x2t роняет весь ruby
 * вместе с базовым словом). Дешёвый гейт: если в `word/document.xml` нет `'<w:ruby'` —
 * вернуть исходные байты.
 */
export async function preprocessDocxRuby(docxBytes: Uint8Array): Promise<Uint8Array> {
  const document = await readZipEntry(docxBytes, 'word/document.xml');
  if (document === null || !UTF8_DECODER.decode(document).includes('<w:ruby')) {
    return docxBytes;
  }
  return rewriteXmlEntries(docxBytes, isDocxXml, (xml) => unwrapRubyXml(xml));
}
