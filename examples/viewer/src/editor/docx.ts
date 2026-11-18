import type { CommandResult, EditorContext } from './commands.js';
import {
  clearFormatClipboard,
  align,
  applyList,
  copyFormat,
  findText,
  fontFamily,
  fontSize,
  getRun,
  getText,
  highlight,
  indent,
  lineSpacing,
  paragraphStyle,
  pasteFormat,
  replace,
  setRun,
  textColor,
  toggleRunProperty,
} from './docx_format.js';
import {
  addComment,
  insertBreak,
  insertCaption,
  insertChart,
  insertCover,
  insertCrossRef,
  insertLink,
  insertPicture,
  insertTable,
  insertToc,
  mailMerge,
} from './docx_insert.js';
import { pageMargins, pageOrientation, pageSize, styleSet, theme } from './docx_page.js';
import { DOCUMENT_PART, importElement, w } from './docx_shared.js';
import { children, descendants, direct, localName, readXml, writeXml } from './xml.js';

const CONTENT_TYPES = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>`;
const ROOT_RELATIONSHIPS = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>`;
const DOCUMENT_RELATIONSHIPS = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>`;
const EMPTY_DOCUMENT = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p/>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
      <w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="708" w:footer="708" w:gutter="0"/>
    </w:sectPr>
  </w:body>
</w:document>`;
const EMPTY_STYLES = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:rPrDefault>
    <w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="240" w:lineRule="auto"/></w:pPr></w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1">
    <w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>
    <w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr>
    <w:rPr><w:rFonts w:ascii="Calibri Light" w:hAnsi="Calibri Light"/><w:b/><w:color w:val="2F5496"/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr>
  </w:style>
</w:styles>`;

function newDocument(context: EditorContext): CommandResult {
  for (const path of Object.keys(context.zip.files)) context.zip.remove(path);
  context.zip.file('[Content_Types].xml', CONTENT_TYPES);
  context.zip.file('_rels/.rels', ROOT_RELATIONSHIPS);
  context.zip.file('word/_rels/document.xml.rels', DOCUMENT_RELATIONSHIPS);
  context.zip.file(DOCUMENT_PART, EMPTY_DOCUMENT);
  context.zip.file('word/styles.xml', EMPTY_STYLES);
  clearFormatClipboard();
  return { changed: true };
}

function removeNode(node: Element): void {
  node.parentNode?.removeChild(node);
}

function unwrap(node: Element): void {
  const parent = node.parentNode;
  if (!parent) return;
  while (node.firstChild) parent.insertBefore(node.firstChild, node);
  parent.removeChild(node);
}

function revisionNodes(document: Document, name: string): Element[] {
  return descendants(document, name);
}

function convertDeletedText(document: Document): void {
  for (const name of ['delText', 'delInstrText']) {
    for (const deleted of revisionNodes(document, name)) {
      const restored = w(document, name === 'delText' ? 't' : 'instrText', deleted.textContent ?? '');
      for (let index = 0; index < deleted.attributes.length; index += 1) {
        const attribute = deleted.attributes.item(index);
        if (attribute) restored.setAttributeNS(attribute.namespaceURI, attribute.name, attribute.value);
      }
      deleted.parentNode?.replaceChild(restored, deleted);
    }
  }
}

function acceptParagraphMarks(document: Document): void {
  for (const paragraph of descendants(document, 'p')) {
    const properties = direct(paragraph, 'pPr');
    const runProperties = properties ? direct(properties, 'rPr') : null;
    const deletion = runProperties ? direct(runProperties, 'del') : null;
    if (!deletion) continue;
    let next = paragraph.nextSibling;
    while (next && next.nodeType !== 1) next = next.nextSibling;
    if (next && localName(next) === 'p') {
      for (const child of children(next as Element)) {
        if (localName(child) !== 'pPr') paragraph.appendChild(child);
      }
      next.parentNode?.removeChild(next);
    }
    removeNode(deletion);
  }
}

function acceptTableDeletions(document: Document): void {
  for (const row of descendants(document, 'tr')) {
    const properties = direct(row, 'trPr');
    if (properties && direct(properties, 'del')) removeNode(row);
  }
  for (const cell of descendants(document, 'tc')) {
    const properties = direct(cell, 'tcPr');
    if (properties && direct(properties, 'cellDel')) removeNode(cell);
  }
}

async function acceptChanges(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  acceptTableDeletions(document);
  acceptParagraphMarks(document);
  for (const name of ['del', 'moveFrom']) for (const node of revisionNodes(document, name)) removeNode(node);
  for (const name of ['ins', 'moveTo']) for (const node of revisionNodes(document, name)) unwrap(node);
  for (const name of [
    'rPrChange', 'pPrChange', 'tblPrChange', 'tblGridChange', 'trPrChange', 'tcPrChange',
    'sectPrChange', 'numberingChange', 'cellIns', 'cellMerge',
    'moveFromRangeStart', 'moveFromRangeEnd', 'moveToRangeStart', 'moveToRangeEnd',
  ]) for (const node of revisionNodes(document, name)) removeNode(node);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

function rejectPropertyChanges(document: Document): void {
  for (const name of [
    'rPrChange', 'pPrChange', 'tblPrChange', 'tblGridChange', 'trPrChange', 'tcPrChange',
    'sectPrChange', 'numberingChange',
  ]) {
    for (const change of revisionNodes(document, name)) {
      const current = change.parentNode;
      if (!current || current.nodeType !== 1) continue;
      const currentElement = current as Element;
      const previous = children(change).find((node) => localName(node) === localName(currentElement));
      if (previous) {
        current.parentNode?.replaceChild(importElement(document, previous), currentElement);
      } else {
        removeNode(change);
      }
    }
  }
}

function rejectTableInsertions(document: Document): void {
  for (const row of descendants(document, 'tr')) {
    const properties = direct(row, 'trPr');
    if (properties && direct(properties, 'ins')) removeNode(row);
  }
  for (const cell of descendants(document, 'tc')) {
    const properties = direct(cell, 'tcPr');
    if (properties && direct(properties, 'cellIns')) removeNode(cell);
  }
}

async function rejectChanges(context: EditorContext): Promise<CommandResult> {
  const document = await readXml(context.zip, DOCUMENT_PART);
  rejectTableInsertions(document);
  rejectPropertyChanges(document);
  for (const name of ['ins', 'moveTo']) for (const node of revisionNodes(document, name)) removeNode(node);
  convertDeletedText(document);
  for (const name of ['del', 'moveFrom']) for (const node of revisionNodes(document, name)) unwrap(node);
  for (const name of [
    'cellDel', 'cellMerge', 'moveFromRangeStart', 'moveFromRangeEnd', 'moveToRangeStart', 'moveToRangeEnd',
  ]) for (const node of revisionNodes(document, name)) removeNode(node);
  writeXml(context.zip, DOCUMENT_PART, document);
  return { changed: true };
}

/** Выполняет адресную правку DOCX в исходном ZIP без round-trip через модель viewer'а. */
export async function executeDocx(context: EditorContext, command: string): Promise<CommandResult> {
  switch (command) {
    case 'new-document': return newDocument(context);
    case 'get-text': return getText(context);
    case 'get-run': return getRun(context);
    case 'set-run': return setRun(context);
    case 'copy-format': return copyFormat(context);
    case 'paste-format': return pasteFormat(context);
    case 'font-family': return fontFamily(context);
    case 'font-size': return fontSize(context);
    case 'text-color': return textColor(context);
    case 'highlight': return highlight(context);
    case 'toggle-bold': return toggleRunProperty(context, 'b');
    case 'toggle-italic': return toggleRunProperty(context, 'i');
    case 'toggle-underline': return toggleRunProperty(context, 'u');
    case 'toggle-strikethrough': return toggleRunProperty(context, 'strike');
    case 'toggle-subscript': return toggleRunProperty(context, 'subscript');
    case 'toggle-superscript': return toggleRunProperty(context, 'superscript');
    case 'bullets': return applyList(context, 'bullets');
    case 'numbering': return applyList(context, 'numbering');
    case 'multilevel-list': return applyList(context, 'multilevel-list');
    case 'line-spacing': return lineSpacing(context);
    case 'align': return align(context);
    case 'indent': return indent(context);
    case 'style': return paragraphStyle(context);
    case 'find': return findText(context);
    case 'replace': return replace(context);
    case 'insert-cover': return insertCover(context);
    case 'insert-break': return insertBreak(context);
    case 'insert-table': return insertTable(context);
    case 'insert-picture': return insertPicture(context);
    case 'insert-chart': return insertChart(context);
    case 'insert-drawing': return insertPicture(context);
    case 'theme': return theme(context);
    case 'style-set': return styleSet(context);
    case 'page-margins': return pageMargins(context);
    case 'page-orientation': return pageOrientation(context);
    case 'page-size': return pageSize(context);
    case 'insert-toc': return insertToc(context);
    case 'insert-link': return insertLink(context);
    case 'insert-caption': return insertCaption(context);
    case 'insert-cross-ref': return insertCrossRef(context);
    case 'mail-merge': return mailMerge(context);
    case 'add-comment': return addComment(context);
    case 'accept-changes': return acceptChanges(context);
    case 'reject-changes': return rejectChanges(context);
    default: throw new Error(`неизвестная DOCX-команда «${command}»`);
  }
}
