// Общий набор для генераторов фикстур DOCX: пространства имён, сборка пакета
// OPC и мелкие билдеры XML.
//
// Фиксутра описывается объектом FixtureSpec: полный набор частей архива плюс
// метаданные сайдкара. Генератор (scripts/generate_docx_fixtures.ts) только
// пишет это на диск — никакой логики OOXML в нём нет.

import { type Bytes, pngBytes } from '../docx-zip.js';

export const W_NS = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main';
export const R_NS = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships';
export const WP_NS = 'http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing';
export const A_NS = 'http://schemas.openxmlformats.org/drawingml/2006/main';
export const PIC_NS = 'http://schemas.openxmlformats.org/drawingml/2006/picture';
export const MC_NS = 'http://schemas.openxmlformats.org/markup-compatibility/2006';
export const WPS_NS = 'http://schemas.microsoft.com/office/word/2010/wordprocessingShape';
export const WPG_NS = 'http://schemas.microsoft.com/office/word/2010/wordprocessingGroup';
export const V_NS = 'urn:schemas-microsoft-com:vml';

export const CT_NS = 'http://schemas.openxmlformats.org/package/2006/content-types';
export const REL_NS = 'http://schemas.openxmlformats.org/package/2006/relationships';

/** Префиксы, которые `mc:Ignorable` разрешает не понимать потребителю. */
export const MC_IGNORABLE = 'w14 wp14 wps wpg mc';

export const XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n';

export const escapeXml = (text: string): string =>
    text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&apos;');

// ==================== FixtureSpec ====================

/** Описание одной фикстуры: пакет + метаданные сайдкара. */
export interface FixtureSpec {
    /** Путь без расширения, он же `metadata.name`: "styles/based_on_chain". */
    name: string;
    description: string;
    /**
     * Абзацы уровня `w:body`. Абзацы внутри `w:tc`, колонтитулов, сносок и
     * комментариев сюда НЕ входят — только прямые дети тела документа.
     */
    expectedParagraphs: number;
    expectedTables: number;
    expectedImages?: number;
    /** `WarningKind`-строки из ADR-0016, которые парсер обязан выдать. */
    expectedWarnings?: string[];
    /**
     * `dc:title` в `docProps/core.xml`; генератор кладёт то же значение в
     * `metadata.docTitle`. По умолчанию — `description`.
     */
    docTitle?: string;
    /** Доп. поля `metadata` (стили, нумерация, колонтитулы — что проверяет фикстура). */
    meta?: Record<string, unknown>;
    /** Доп. поля `content` сайдкара. */
    content?: Record<string, unknown>;
    /** Части пакета в порядке записи в ZIP: путь → содержимое. */
    parts: Array<[string, string | Bytes]>;
    /**
     * Готовые байты файла вместо сборки ZIP из `parts`. Нужно для битых
     * фикстур (обрезанный архив) — там ZIP собирается руками и ломается.
     */
    bytes?: Bytes;
}

// ==================== Билдеры XML ====================

export interface RunOptions {
    bold?: boolean;
    italic?: boolean;
    underline?: boolean;
    strike?: boolean;
    /** `w:rtl` — RTL-прогон (арабский/иврит). */
    rtl?: boolean;
    /** `w:rStyle w:val` */
    style?: string;
    /** Размер в полупунктах (`w:sz`). */
    size?: number;
    color?: string;
    font?: string;
   /** Доп. XML внутри `w:rPr`. */
    rPr?: string;
}

export interface ParagraphOptions {
    /** `w:pStyle w:val` */
    style?: string;
    /** `w:numPr` → `w:numId` */
    numId?: number;
    /** `w:numPr` → `w:ilvl`, по умолчанию 0 */
    ilvl?: number;
    /** `w:bidi` — RTL-абзац. */
    bidi?: boolean;
    /** `w:jc w:val` */
    jc?: string;
    /** `w:pStyle` с другим именем для RTL-стиля и т.п. */
    pPr?: string;
}

/** Прогон `<w:r>` с одним `<w:t>`. */
export const run = (text: string, opts: RunOptions = {}): string => {
    const props: string[] = [];
    if (opts.style !== undefined) props.push(`<w:rStyle w:val="${opts.style}"/>`);
    if (opts.font !== undefined) props.push(`<w:rFonts w:ascii="${opts.font}" w:hAnsi="${opts.font}" w:cs="${opts.font}"/>`);
    if (opts.bold) props.push('<w:b/>');
    if (opts.italic) props.push('<w:i/>');
    if (opts.underline) props.push('<w:u w:val="single"/>');
    if (opts.strike) props.push('<w:strike/>');
    if (opts.rtl) props.push('<w:rtl/>');
    if (opts.color !== undefined) props.push(`<w:color w:val="${opts.color}"/>`);
    if (opts.size !== undefined) props.push(`<w:sz w:val="${opts.size}"/>`);
    if (opts.rPr !== undefined) props.push(opts.rPr);
    const rPr = props.length > 0 ? `<w:rPr>${props.join('')}</w:rPr>` : '';
    return `<w:r>${rPr}<w:t xml:space="preserve">${escapeXml(text)}</w:t></w:r>`;
};

/** Абзац `<w:p>` из одного прогона. */
export const p = (text: string, opts: ParagraphOptions & RunOptions = {}): string => {
    const pPr: string[] = [];
    if (opts.style !== undefined) pPr.push(`<w:pStyle w:val="${opts.style}"/>`);
    if (opts.numId !== undefined) {
        pPr.push(`<w:numPr><w:ilvl w:val="${opts.ilvl ?? 0}"/><w:numId w:val="${opts.numId}"/></w:numPr>`);
    }
    if (opts.bidi) pPr.push('<w:bidi/>');
    if (opts.jc !== undefined) pPr.push(`<w:jc w:val="${opts.jc}"/>`);
    if (opts.pPr !== undefined) pPr.push(opts.pPr);
    return para(pPr.join(''), run(text, opts));
};

/** Абзац из готовых `w:pPr` и прогонов — когда одного `<w:t>` мало. */
export const para = (pPrXml: string, runsXml: string): string =>
    `<w:p>${pPrXml === '' ? '' : `<w:pPr>${pPrXml}</w:pPr>`}${runsXml}</w:p>`;

/** Абзац-заголовок: `w:pStyle w:val="HeadingN"`. */
export const h = (text: string, level: 1 | 2 | 3 = 1): string => p(text, { style: `Heading${level}` });

/** Ячейка `<w:tc>`; `tcPr` — готовый XML внутри `<w:tcPr>`. */
export const tc = (text: string, tcPr = ''): string =>
    `<w:tc>${tcPr === '' ? '' : `<w:tcPr>${tcPr}</w:tcPr>`}${p(text)}</w:tc>`;

/** Строка `<w:tr>` из готовых ячеек. */
export const tr = (cells: string, trPr = ''): string =>
    `<w:tr>${trPr === '' ? '' : `<w:trPr>${trPr}</w:trPr>`}${cells}</w:tr>`;

/** Таблица из текстовых ячеек; для сложных случаев собирайте XML вручную. */
export const table = (
    rows: string[][],
    opts: {
        /** Ширины столбцов в twips (dxa). */
        grid?: number[];
        borders?: boolean;
        /** `w:tblLook w:val` */
        tblLook?: string;
        styleId?: string;
        /** Сколько первых строк помечать как повторяющиеся на странице. */
        repeatHeaderRows?: number;
    } = {},
): string => {
    // w:tblGrid — сосед w:tblPr, а не его ребёнок: в CT_Tbl порядок жёсткий
    // (tblPr → tblGrid → tr), и вложенный grid ломает строгих читателей.
    const grid =
        opts.grid === undefined
            ? ''
            : `<w:tblGrid>${opts.grid.map((w) => `<w:gridCol w:w="${w}"/>`).join('')}</w:tblGrid>`;
    const props: string[] = ['<w:tblW w:w="0" w:type="auto"/>'];
    if (opts.styleId !== undefined) props.unshift(`<w:tblStyle w:val="${opts.styleId}"/>`);
    if (opts.tblLook !== undefined) props.push(`<w:tblLook w:val="${opts.tblLook}"/>`);
    if (opts.borders === true) {
        props.push(
            '<w:tblBorders>' +
                ['top', 'left', 'bottom', 'right', 'insideH', 'insideV']
                    .map((side) => `<w:${side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>`)
                    .join('') +
                '</w:tblBorders>',
        );
    }
    const rowsXml = rows
        .map((cells, index) => {
            const trPr = opts.repeatHeaderRows !== undefined && index < opts.repeatHeaderRows
                ? '<w:tblHeader/>'
                : '';
            return tr(cells.map((cell) => tc(cell)).join(''), trPr);
        })
        .join('');
    return `<w:tbl><w:tblPr>${props.join('')}</w:tblPr>${grid}${rowsXml}</w:tbl>`;
};

/** Разрыв страницы. */
export const pageBreak = (): string => para('<w:rPr/>', '<w:r><w:br w:type="page"/></w:r>');

export const sectionProps = (xml: string): string => `<w:sectPr>${xml}</w:sectPr>`;

// ==================== Рисунки ====================

/** Стабильный rId для i-й картинки (0-based) в списке `images`. */
export const imageRelId = (index: number): string => `rIdImg${index + 1}`;

/** `<a:blip r:embed>` + `<pic:pic>` — общий хвост inline и anchor. */
const pictureCore = (relId: string, name: string, cx: number, cy: number, picId: number): string =>
    `<pic:pic>` +
    `<pic:nvPicPr><pic:cNvPr id="${picId}" name="${escapeXml(name)}"/><pic:cNvPicPr/></pic:nvPicPr>` +
    `<pic:blipFill><a:blip r:embed="${relId}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>` +
    `<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="${cx}" cy="${cy}"/></a:xfrm>` +
    `<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>`;

/**
 * `wp:inline` — картинка в потоке текста.
 *
 * @param relId rId из `imageRelId`
 * @param opts размеры в EMU (914400 EMU = 1 дюйм) и имя
 */
export const inlineImage = (
    relId: string,
    opts: { cx?: number; cy?: number; name?: string; id?: number } = {},
): string => {
    const cx = opts.cx ?? 914400;
    const cy = opts.cy ?? 914400;
    const id = opts.id ?? 1;
    return (
        '<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">' +
        `<wp:extent cx="${cx}" cy="${cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/>` +
        `<wp:docPr id="${id}" name="${escapeXml(opts.name ?? `Picture ${id}`)}"/>` +
        '<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>' +
        `<a:graphic><a:graphicData uri="${PIC_NS}">${pictureCore(relId, opts.name ?? `image${id}.png`, cx, cy, id)}</a:graphicData></a:graphic>` +
        '</wp:inline></w:drawing>'
    );
};

/**
 * `wp:anchor` — плавающий рисунок.
 *
 * @param relId rId из `imageRelId`
 * @param opts размеры, обтекание (`wrap`) и позиция
 */
export const anchoredImage = (
    relId: string,
    opts: {
        cx?: number;
        cy?: number;
        name?: string;
        id?: number;
        /** behindDoc|0 — перед/за текстом */
        behindDoc?: boolean;
        /** Тип обтекания: square, tight, through, topAndBottom, none. */
        wrap?: 'square' | 'tight' | 'through' | 'topAndBottom' | 'none';
        /** Привязка по горизонтали: page, margin, column, character. */
        hRelative?: string;
        vRelative?: string;
        /** Смещение в EMU. */
        posX?: number;
        posY?: number;
    } = {},
): string => {
    const cx = opts.cx ?? 914400;
    const cy = opts.cy ?? 914400;
    const id = opts.id ?? 1;
    const wrap = opts.wrap ?? 'square';
    const wrapXml =
        wrap === 'none'
            ? '<wp:wrapNone/>'
            : wrap === 'topAndBottom'
              ? '<wp:wrapTopAndBottom/>'
              : `<wp:wrap${wrap === 'square' ? 'Square' : wrap === 'tight' ? 'Tight' : 'Through'}/>`;
    return (
        `<w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" ` +
        `relativeHeight="${id}" behindDoc="${opts.behindDoc === true ? 1 : 0}" locked="0" layoutInCell="1" allowOverlap="1">` +
        '<wp:simplePos x="0" y="0"/>' +
        `<wp:positionH relativeFrom="${opts.hRelative ?? 'column'}"><wp:posOffset>${opts.posX ?? 0}</wp:posOffset></wp:positionH>` +
        `<wp:positionV relativeFrom="${opts.vRelative ?? 'paragraph'}"><wp:posOffset>${opts.posY ?? 0}</wp:posOffset></wp:positionV>` +
        `<wp:extent cx="${cx}" cy="${cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/>${wrapXml}` +
        `<wp:docPr id="${id}" name="${escapeXml(opts.name ?? `Picture ${id}`)}"/>` +
        '<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>' +
        `<a:graphic><a:graphicData uri="${PIC_NS}">${pictureCore(relId, opts.name ?? `image${id}.png`, cx, cy, id)}</a:graphicData></a:graphic>` +
        '</wp:anchor></w:drawing>'
    );
};

// ==================== Части пакета ====================

/** `word/document.xml` с телом; `attr` — добавка к атрибутам `w:document`. */
export const documentXml = (body: string, attr = ''): string =>
    XML_DECL +
    `<w:document xmlns:w="${W_NS}" xmlns:r="${R_NS}" xmlns:wp="${WP_NS}" xmlns:a="${A_NS}" ` +
    `xmlns:pic="${PIC_NS}" xmlns:mc="${MC_NS}" xmlns:wps="${WPS_NS}" xmlns:wpg="${WPG_NS}" ` +
    `mc:Ignorable="${MC_IGNORABLE}"${attr}>` +
    `<w:body>${body}</w:body></w:document>`;

/** Значения по умолчанию для тела пустого документа Word. */
export const defaultSectPr: string =
    '<w:pgSz w:w="11906" w:h="16838"/>' +
    '<w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="709" w:footer="709" w:gutter="0"/>' +
    '<w:cols w:space="708"/><w:docGrid w:linePitch="360"/>';

/** `docDefaults` + Normal + Heading1..3 (+ их linked-стили) + TableNormal. */
export const defaultStylesXml = (): string =>
    XML_DECL +
    `<w:styles xmlns:w="${W_NS}" xmlns:r="${R_NS}">` +
    '<w:docDefaults>' +
    '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="SimSun" w:cs="Arial"/>' +
    '<w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="en-US" w:eastAsia="zh-CN" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>' +
    '<w:pPrDefault><w:pPr><w:spacing w:before="0" w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault>' +
    '</w:docDefaults>' +
    '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>' +
    '<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont">' +
    '<w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style>' +
    '<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/>' +
    '<w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/></w:style>' +
    headingStyle(1, 'Heading 1', 'Heading1Char', 32, '2f5496') +
    headingStyle(2, 'Heading 2', 'Heading2Char', 26, '2f5496') +
    headingStyle(3, 'Heading 3', 'Heading3Char', 24, '1f3763') +
    '</w:styles>';

const headingStyle = (level: number, name: string, linkId: string, size: number, color: string): string =>
    `<w:style w:type="paragraph" w:styleId="Heading${level}"><w:name w:val="${name}"/>` +
    '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:link w:val="' + linkId + '"/><w:uiPriority w:val="9"/>' +
    '<w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="' + (level - 1) + '"/></w:pPr>' +
    `<w:rPr><w:b/><w:color w:val="${color}"/><w:sz w:val="${size}"/></w:rPr></w:style>` +
    `<w:style w:type="character" w:customStyle="1" w:styleId="${linkId}"><w:name w:val="${name} Char"/>` +
    `<w:basedOn w:val="DefaultParagraphFont"/><w:link w:val="Heading${level}"/><w:uiPriority w:val="9"/>` +
    `<w:rPr><w:b/><w:color w:val="${color}"/><w:sz w:val="${size}"/></w:rPr></w:style>`;

export const defaultSettingsXml = (extra = ''): string =>
    XML_DECL +
    `<w:settings xmlns:w="${W_NS}" xmlns:r="${R_NS}" xmlns:mc="${MC_NS}" mc:Ignorable="${MC_IGNORABLE}">` +
    '<w:zoom w:percent="100"/><w:defaultTabStop w:val="708"/><w:characterSpacingControl w:val="doNotCompress"/>' +
    extra +
    '<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>' +
    '</w:settings>';

export interface PackageOptions {
    /** `dc:title` в `docProps/core.xml` (генератор потом подменяет его на docTitle). */
    title?: string;
    /** Тело `w:body` (без `<w:sectPr>` — добавьте его сами, если нужен). */
    body?: string;
    /** Готовый `word/document.xml` вместо сборки из `body`. */
    document?: string;
    /** Готовый `word/styles.xml`; `null` — не включать часть вовсе. */
    styles?: string | null;
    /** Доп. стили на верхнем уровне `w:styles` (вставляются перед `</w:styles>`). */
    stylesExtra?: string;
    numbering?: string;
    settings?: string;
    footnotes?: string;
    comments?: string;
    /** Колонтитулы: ключ — имя файла в `word/` ("header1.xml"), значение — XML. */
    headers?: Record<string, string>;
    footers?: Record<string, string>;
    /** Картинки: ключ — имя файла в `word/media/`, значение — байты PNG. */
    images?: Record<string, Bytes>;
    /** Доп. `<Relationship>` для `word/_rels/document.xml.rels`. */
    extraRels?: string;
    /** Готовый `word/_rels/document.xml.rels` целиком. */
    documentRels?: string;
    /** Готовый `_rels/.rels` целиком. */
    rootRels?: string;
    /** Готовый `[Content_Types].xml` целиком. */
    contentTypes?: string;
    /** Доп. `<Override>`/`<Default>` в `[Content_Types].xml`. */
    extraContentTypes?: string;
    /** Любые доп. части пакета. */
    extraParts?: Record<string, string | Bytes>;
}

const DEFAULT_CONTENT_TYPES =
    XML_DECL +
    `<Types xmlns="${CT_NS}">` +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="png" ContentType="image/png"/>' +
    '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>' +
    '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>' +
    '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>' +
    '<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>' +
    '</Types>';

const CORE_REL_TYPE = 'http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties';
const APP_REL_TYPE = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties';

/** Дата создания/изменения в docProps — фиксированная, иначе нет детерминизма. */
export const DOC_PROPS_DATE = '2026-01-01T00:00:00Z';
export const DOC_PROPS_CREATOR = 'doc-converter fixtures';
export const DOC_PROPS_APPLICATION = 'doc-converter fixture generator';

/**
 * `docProps/core.xml` — dc:title, dc:creator, cp:lastModifiedBy, cp:revision,
 * dcterms:created/modified.
 *
 * @param title значение `dc:title`
 */
export const corePropsXml = (title: string): string =>
    XML_DECL +
    '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" ' +
    'xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" ' +
    'xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">' +
    `<dc:title>${escapeXml(title)}</dc:title>` +
    `<dc:creator>${DOC_PROPS_CREATOR}</dc:creator>` +
    `<cp:lastModifiedBy>${DOC_PROPS_CREATOR}</cp:lastModifiedBy>` +
    '<cp:revision>1</cp:revision>' +
    `<dcterms:created xsi:type="dcterms:W3CDTF">${DOC_PROPS_DATE}</dcterms:created>` +
    `<dcterms:modified xsi:type="dcterms:W3CDTF">${DOC_PROPS_DATE}</dcterms:modified>` +
    '</cp:coreProperties>';

/** `docProps/app.xml` — Application/AppVersion. */
export const appPropsXml = (): string =>
    XML_DECL +
    '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" ' +
    'xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">' +
    `<Application>${DOC_PROPS_APPLICATION}</Application>` +
    '<AppVersion>1.0</AppVersion>' +
    '</Properties>';

const DEFAULT_TITLE = 'doc-converter fixture';

/** Убирает части из готового пакета (битые фикстуры). */
export const omit = (
    parts: Array<[string, string | Bytes]>,
    names: string[],
): Array<[string, string | Bytes]> => parts.filter(([name]) => !names.includes(name));

/** Подменяет `dc:title` в `docProps/core.xml` готового пакета. */
export const withDocTitle = (
    parts: Array<[string, string | Bytes]>,
    title: string,
): Array<[string, string | Bytes]> =>
    parts.map((entry) => (entry[0] === 'docProps/core.xml' ? [entry[0], corePropsXml(title)] : entry));

const CT_OVERRIDES: Record<string, string> = {
    'word/numbering.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml',
    'word/settings.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml',
    'word/footnotes.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml',
    'word/comments.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml',
};

const REL_TYPES: Record<string, string> = {
    styles: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles',
    numbering: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering',
    settings: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings',
    footnotes: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes',
    comments: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments',
    image: 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/image',
};

export const HEADER_REL_TYPE = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/header';
export const FOOTER_REL_TYPE = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer';
export const HYPERLINK_REL_TYPE = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink';

export const headerRelId = (index: number): string => `rIdHeader${index + 1}`;
export const footerRelId = (index: number): string => `rIdFooter${index + 1}`;

const overrideFor = (part: string): string | undefined => {
    if (CT_OVERRIDES[part] !== undefined) return CT_OVERRIDES[part];
    if (/^word\/header\d+\.xml$/.test(part)) {
        return 'application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml';
    }
    if (/^word\/footer\d+\.xml$/.test(part)) {
        return 'application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml';
    }
    return undefined;
};

/**
 * Собирает пакет DOCX: дефолтные части, автогенерация rels и Content_Types
 * для подсунутых частей, поверх — явные переопределения из опций.
 *
 * @param opts состав пакета
 * @returns части архива в порядке записи в ZIP
 */
export const packageParts = (opts: PackageOptions = {}): Array<[string, string | Bytes]> => {
    const documentBody = opts.body ?? `${para('', '')}<w:sectPr>${defaultSectPr}</w:sectPr>`;
    const document = opts.document ?? documentXml(documentBody);

    // Части получают предсказуемые rId: колонтитулы и картинки нумеруются
    // своими префиксами, чтобы автор фикстуры мог сослаться на них через
    // headerRelId/footerRelId/imageRelId, не заглядывая в этот список.
    const rels: string[] = [];
    let nextRel = 1;
    const addRel = (id: string, type: string, target: string): void => {
        rels.push(`<Relationship Id="${id}" Type="${type}" Target="${target}"/>`);
    };
    const stylesPresent = opts.styles !== null;
    if (stylesPresent) addRel(`rId${nextRel++}`, REL_TYPES.styles!, 'styles.xml');
    if (opts.numbering !== undefined) addRel(`rId${nextRel++}`, REL_TYPES.numbering!, 'numbering.xml');
    if (opts.settings !== undefined) addRel(`rId${nextRel++}`, REL_TYPES.settings!, 'settings.xml');
    if (opts.footnotes !== undefined) addRel(`rId${nextRel++}`, REL_TYPES.footnotes!, 'footnotes.xml');
    if (opts.comments !== undefined) addRel(`rId${nextRel++}`, REL_TYPES.comments!, 'comments.xml');
    Object.keys(opts.headers ?? {}).forEach((name, i) => addRel(headerRelId(i), HEADER_REL_TYPE, name));
    Object.keys(opts.footers ?? {}).forEach((name, i) => addRel(footerRelId(i), FOOTER_REL_TYPE, name));
    Object.keys(opts.images ?? {}).forEach((name, i) => addRel(imageRelId(i), REL_TYPES.image!, `media/${name}`));
    if (opts.extraRels !== undefined) rels.push(opts.extraRels);

    const documentRels =
        opts.documentRels ??
        XML_DECL + `<Relationships xmlns="${REL_NS}">${rels.join('')}</Relationships>`;

    const allPartNames = [
        ...(opts.numbering !== undefined ? ['word/numbering.xml'] : []),
        ...(opts.settings !== undefined ? ['word/settings.xml'] : []),
        ...(opts.footnotes !== undefined ? ['word/footnotes.xml'] : []),
        ...(opts.comments !== undefined ? ['word/comments.xml'] : []),
        ...Object.keys(opts.headers ?? {}).map((n) => `word/${n}`),
        ...Object.keys(opts.footers ?? {}).map((n) => `word/${n}`),
    ];
    const typeOverrides = allPartNames
        .map((part) => (overrideFor(part) !== undefined ? `<Override PartName="/${part}" ContentType="${overrideFor(part)!}"/>` : ''))
        .filter((xml) => xml !== '')
        .join('');
    const contentTypes =
        opts.contentTypes ??
        (typeOverrides === '' && opts.extraContentTypes === undefined
            ? DEFAULT_CONTENT_TYPES
            : DEFAULT_CONTENT_TYPES.replace(
                  '</Types>',
                  `${typeOverrides}${opts.extraContentTypes ?? ''}</Types>`,
              ));

    const entries: Array<[string, string | Bytes]> = [];
    entries.push(['[Content_Types].xml', contentTypes]);
    entries.push([
        '_rels/.rels',
        opts.rootRels ??
            XML_DECL +
                `<Relationships xmlns="${REL_NS}">` +
                '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>' +
                `<Relationship Id="rId2" Type="${CORE_REL_TYPE}" Target="docProps/core.xml"/>` +
                `<Relationship Id="rId3" Type="${APP_REL_TYPE}" Target="docProps/app.xml"/>` +
                '</Relationships>',
    ]);
    entries.push(['word/document.xml', document]);
    if (stylesPresent) {
        const styles = opts.styles ?? defaultStylesXml();
        entries.push([
            'word/styles.xml',
            opts.stylesExtra === undefined ? styles : styles.replace('</w:styles>', `${opts.stylesExtra}</w:styles>`),
        ]);
    }
    if (opts.numbering !== undefined) entries.push(['word/numbering.xml', opts.numbering]);
    if (opts.settings !== undefined) entries.push(['word/settings.xml', opts.settings]);
    if (opts.footnotes !== undefined) entries.push(['word/footnotes.xml', opts.footnotes]);
    if (opts.comments !== undefined) entries.push(['word/comments.xml', opts.comments]);
    for (const [name, xml] of Object.entries(opts.headers ?? {})) entries.push([`word/${name}`, xml]);
    for (const [name, xml] of Object.entries(opts.footers ?? {})) entries.push([`word/${name}`, xml]);
    for (const [name, bytes] of Object.entries(opts.images ?? {})) entries.push([`word/media/${name}`, bytes]);
    entries.push(['word/_rels/document.xml.rels', documentRels]);
    entries.push(['docProps/core.xml', corePropsXml(opts.title ?? DEFAULT_TITLE)]);
    entries.push(['docProps/app.xml', appPropsXml()]);
    for (const [name, value] of Object.entries(opts.extraParts ?? {})) entries.push([name, value]);
    return entries;
};

/**
 * Стандартная 1×1-картинка (PNG с шумом заданного размера).
 *
 * @param size сторона квадрата в пикселях
 * @param seed зерно PRNG
 */
export const noisePng = (size: number, seed: number): Bytes => pngBytes(size, size, seed);
