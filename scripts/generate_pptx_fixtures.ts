// Генератор фикстур PPTX.
//
// ZIP собирается детерминированным writer'ом (scripts/docx-zip.ts) с
// фиксированными таймстампами и порядком записей, поэтому повторный запуск
// даёт побайтово те же файлы: `git status --porcelain test-fixtures/pptx`
// после второго прогона пуст.
//
// Запуск:
//   npx tsx scripts/generate_pptx_fixtures.ts
//
// Фикстуры маленькие (слайды с текстом, геометрией, картинкой и диаграммой):
// они покрывают разбор/раскладку/рендер/PDF и e2e демо; больших дек нет —
// для PPTX нет гейта памяти, как у DOCX.

import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { buildZip, pngBytes, type Bytes } from './docx-zip.js';

const ROOT = path.resolve(import.meta.dirname, '..');
const OUT_DIR = path.join(ROOT, 'test-fixtures', 'pptx');

/** Запись пакета: путь внутри ZIP и содержимое. */
type Part = readonly [name: string, data: string | Uint8Array];

/** Презентация: имя файла, части пакета. */
interface Deck {
    name: string;
    parts: Part[];
}

const CONTENT_TYPES = (extra: string): string =>
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Default Extension="png" ContentType="image/png"/>' +
    extra +
    '</Types>';

const CHART_TYPE = '<Override PartName="/ppt/charts/chart1.xml" ' +
    'ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>';

const ROOT_RELS =
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>' +
    '</Relationships>';

/** presentation.xml: список мастеров/слайдов и размер слайда. */
const PRESENTATION = (slideRels: string, sizeCx = '12192000', sizeCy = '6858000'): string =>
    '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" ' +
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">' +
    '<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>' +
    `<p:sldIdLst>${slideRels}</p:sldIdLst>` +
    `<p:sldSz cx="${sizeCx}" cy="${sizeCy}"/>` +
    '</p:presentation>';

/** presentation.xml.rels: мастер, тема, слайды. */
const PRESENTATION_RELS = (slideTargets: string[]): string =>
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>' +
    slideTargets.map(
        (target, index) =>
            `<Relationship Id="rId${index + 2}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="${target}"/>`,
    ).join('') +
    '</Relationships>';

/** Мастер с плейсхолдерами титула и тела — от них наследуются рамки слайдов. */
const MASTER =
    '<p:sldMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" ' +
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" ' +
    'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">' +
    '<p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></p:bgPr></p:bg>' +
    '<p:spTree>' +
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title Placeholder 1"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>' +
    '<p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr>' +
    '<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t/></a:r></a:p></p:txBody></p:sp>' +
    '<p:sp><p:nvSpPr><p:cNvPr id="3" name="Text Placeholder 2"/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>' +
    '<p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="10515600" cy="4451125"/></a:xfrm></p:spPr>' +
    '<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t/></a:r></a:p></p:txBody></p:sp>' +
    '</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>' +
    '</p:sldMaster>';

const MASTER_RELS =
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>' +
    '</Relationships>';

const THEME =
    '<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme">' +
    '<a:themeElements>' +
    '<a:clrScheme name="Office">' +
    '<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>' +
    '<a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>' +
    '<a:dk2><a:srgbClr val="44516A"/></a:dk2>' +
    '<a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>' +
    '<a:accent1><a:srgbClr val="4472C4"/></a:accent1>' +
    '<a:accent2><a:srgbClr val="ED7D31"/></a:accent2>' +
    '<a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>' +
    '<a:accent4><a:srgbClr val="FFC000"/></a:accent4>' +
    '<a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>' +
    '<a:accent6><a:srgbClr val="70AD47"/></a:accent6>' +
    '<a:hlink><a:srgbClr val="0563C1"/></a:hlink>' +
    '<a:folHlink><a:srgbClr val="954F72"/></a:folHlink>' +
    '</a:clrScheme>' +
    '<a:fontScheme name="Office">' +
    '<a:majorFont><a:latin typeface="Calibri Light"/></a:majorFont>' +
    '<a:minorFont><a:latin typeface="Calibri"/></a:minorFont>' +
    '</a:fontScheme>' +
    '</a:themeElements></a:theme>';

const LAYOUT =
    '<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" ' +
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">' +
    '<p:cSld><p:spTree/></p:cSld></p:sldLayout>';

const LAYOUT_RELS =
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>' +
    '</Relationships>';

/** Слайд: имя, список шейпов (`p:sp`/`p:pic`/`p:graphicFrame`). */
const SLIDE = (name: string, shapes: string): string =>
    '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" ' +
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" ' +
    'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">' +
    `<p:cSld name="${name}"><p:spTree>${shapes}</p:spTree></p:cSld></p:sld>`;

/** Текстовый шейп с прогонами (`runs` — строка `a:r`-элементов). */
const TEXT_SHAPE = (id: number, name: string, ph: string, xfrm: string, paragraphs: string): string =>
    `<p:sp><p:nvSpPr><p:cNvPr id="${id}" name="${name}"/>` +
    `<p:nvPr>${ph}</p:nvPr></p:nvSpPr>` +
    `<p:spPr><a:xfrm>${xfrm}</a:xfrm></p:spPr>` +
    `<p:txBody><a:bodyPr/>${paragraphs}</p:txBody></p:sp>`;

const TITLE_SHAPE = (id: number, text: string): string =>
    TEXT_SHAPE(
        id,
        `Title ${id}`,
        '<p:ph type="title"/>',
        '<a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/>',
        `<a:p><a:r><a:rPr lang="en-US" sz="4400" b="1"/><a:t>${text}</a:t></a:r></a:p>`,
    );

const BODY_SHAPE = (id: number, paragraphs: string): string =>
    TEXT_SHAPE(
        id,
        `Content ${id}`,
        '<p:ph type="body" idx="1"/>',
        '<a:off x="838200" y="1825625"/><a:ext cx="10515600" cy="4451125"/>',
        paragraphs,
    );

const PARAGRAPH = (text: string, opts: { bullet?: string; level?: number; bold?: boolean; italic?: boolean; size?: string } = {}): string => {
    const pPr = opts.bullet !== undefined || opts.level !== undefined
        ? `<a:pPr${opts.level !== undefined ? ` lvl="${opts.level}"` : ''}>${opts.bullet !== undefined ? `<a:buChar char="${opts.bullet}"/>` : ''}</a:pPr>`
        : '';
    const rPr = opts.bold === true || opts.italic === true || opts.size !== undefined
        ? `<a:rPr lang="en-US"${opts.size !== undefined ? ` sz="${opts.size}"` : ''}${opts.bold === true ? ' b="1"' : ''}${opts.italic === true ? ' i="1"' : ''}/>`
        : '';
    return `<a:p>${pPr}${rPr ? `<a:r>${rPr}<a:t>${text}</a:t></a:r>` : `<a:r><a:t>${text}</a:t></a:r>`}</a:p>`;
};

/** Шейп-геометрия: прямоугольник, скруглённый или эллипс с заливкой/контуром. */
const GEOMETRY_SHAPE = (id: number, name: string, prst: string, xfrm: string, fill: string, stroke: string): string =>
    `<p:sp><p:nvSpPr><p:cNvPr id="${id}" name="${name}"/><p:nvPr/></p:nvSpPr>` +
    `<p:spPr><a:xfrm>${xfrm}</a:xfrm><a:prstGeom prst="${prst}"><a:avLst/></a:prstGeom>` +
    `<a:solidFill><a:srgbClr val="${fill}"/></a:solidFill>` +
    `<a:ln w="12700"><a:solidFill><a:srgbClr val="${stroke}"/></a:solidFill></a:ln>` +
    `</p:spPr><p:txBody><a:bodyPr/><a:p/></p:txBody></p:sp>`;

/** Картинка `p:pic` по ссылке `rId` на media-часть. */
const PICTURE_SHAPE = (id: number, name: string, relId: string, xfrm: string): string =>
    `<p:pic><p:nvPicPr><p:cNvPr id="${id}" name="${name}"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>` +
    `<p:blipFill><a:blip r:embed="${relId}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>` +
    `<p:spPr><a:xfrm>${xfrm}</a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>`;

/** Диаграмма `p:graphicFrame` по ссылке `rId` на chart-часть. */
const CHART_SHAPE = (id: number, name: string, relId: string, xfrm: string): string =>
    '<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr ' +
    `id="${id}" name="${name}"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>` +
    `<p:xfrm>${xfrm}</p:xfrm>` +
    '<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">' +
    '<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">' +
    `<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="${relId}"/>` +
    '</a:graphicData></a:graphic></p:graphicFrame>';

const CHART =
    '<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" ' +
    'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">' +
    '<c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Sales</a:t></a:r></a:p></c:rich></c:tx></c:title>' +
    '<c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>' +
    '<c:ser><c:idx val="0"/><c:order val="0"/>' +
    '<c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>2024</c:v></c:pt></c:strCache></c:strRef></c:tx>' +
    '<c:cat><c:strRef><c:f>Sheet1!$A$2:$A$3</c:f><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat>' +
    '<c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt></c:numCache></c:numRef></c:val>' +
    '</c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>';

/** Сборка пакета: общий каркас + слайды и их relationships. */
const deck = (
  name: string,
  slides: Part[],
  extra: 'image' | 'chart' | null = null,
  extraParts: Part[] = [],
): Part[] => {
  const slideNames = slides.map(([part]) => part);
  const slideTargets = slideNames.map((part) => part.replace('ppt/', ''));
  const slideRelEntries = slideNames.map((part) => {
    const number = part.match(/slide(\d+)\.xml/)![1]!;
    return [part.replace('.xml', '.xml.rels').replace('slides/', 'slides/_rels/'), slideRels(number, extra)] as Part;
  });
  return [
    ['[Content_Types].xml', CONTENT_TYPES(extra === 'chart' ? CHART_TYPE : '')],
    ['_rels/.rels', ROOT_RELS],
    ['ppt/presentation.xml', PRESENTATION(slideNames.map((_, index) => `<p:sldId id="${256 + index}" r:id="rId${index + 2}"/>`).join(''))],
    ['ppt/_rels/presentation.xml.rels', PRESENTATION_RELS(slideTargets)],
    ['ppt/slideMasters/slideMaster1.xml', MASTER],
    ['ppt/slideMasters/_rels/slideMaster1.xml.rels', MASTER_RELS],
    ['ppt/theme/theme1.xml', THEME],
    ['ppt/slideLayouts/slideLayout1.xml', LAYOUT],
    ['ppt/slideLayouts/_rels/slideLayout1.xml.rels', LAYOUT_RELS],
    ...slideRelEntries,
    ...slides,
    ...extraParts,
  ];
};

/** Relationships слайда: лэйаут, картинка, диаграмма. */
const slideRels = (number: string, extra: 'image' | 'chart' | null): string => {
  const entries = [
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>',
  ];
  if (extra === 'image') {
    entries.push(
      '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>',
    );
  }
  if (extra === 'chart') {
    entries.push(
      '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>',
    );
  }
  return `<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${entries.join('')}</Relationships>`;
};

/** Два слайда: титул + тело. Базовый сценарий просмотра и экспорта. */
const twoSlides = (): Deck => ({
    name: 'simple/two_slides.pptx',
    parts: deck('simple/two_slides.pptx', [
        ['ppt/slides/slide1.xml', SLIDE('Deck 1', TITLE_SHAPE(2, 'Hello Slides') + BODY_SHAPE(3, PARAGRAPH('First bullet') + PARAGRAPH('Second bullet', { bullet: '•' })))],
        ['ppt/slides/slide2.xml', SLIDE('Deck 2', TITLE_SHAPE(2, 'Slide two') + BODY_SHAPE(3, PARAGRAPH('Another line')))],
    ]),
});

/** Один слайд с геометрией: rect, roundRect, ellipse. */
const geometry = (): Deck => ({
    name: 'shapes/geometry.pptx',
    parts: deck('shapes/geometry.pptx', [
        ['ppt/slides/slide1.xml', SLIDE('Shapes', TITLE_SHAPE(2, 'Geometry') +
            GEOMETRY_SHAPE(3, 'Rectangle 3', 'rect', '<a:off x="838200" y="2286000"/><a:ext cx="2743200" cy="1371600"/>', '4472C4', '000000') +
            GEOMETRY_SHAPE(4, 'Rounded Rectangle 4', 'roundRect', '<a:off x="4023360" y="2286000"/><a:ext cx="2743200" cy="1371600"/>', 'ED7D31', '000000') +
            GEOMETRY_SHAPE(5, 'Oval 5', 'ellipse', '<a:off x="7208520" y="2286000"/><a:ext cx="2743200" cy="1371600"/>', '70AD47', '000000'))],
    ]),
});

/** Один слайд с картинкой. */
const picture = (): Deck => ({
    name: 'images/picture.pptx',
    parts: deck('images/picture.pptx', [
        ['ppt/slides/slide1.xml', SLIDE('Picture', TITLE_SHAPE(2, 'Image') +
            PICTURE_SHAPE(3, 'Picture 3', 'rId2', '<a:off x="2743200" y="2000250"/><a:ext cx="6705600" cy="3429000"/>'))],
    ], 'image', [
        ['ppt/media/image1.png', pngBytes(320, 180, 7)],
    ]),
});

/** Один слайд с диаграммой. */
const chart = (): Deck => ({
    name: 'charts/bar.pptx',
    parts: deck('charts/bar.pptx', [
        ['ppt/slides/slide1.xml', SLIDE('Chart', TITLE_SHAPE(2, 'Sales') +
            CHART_SHAPE(3, 'Chart 3', 'rId2', '<a:off x="1371600" y="2000250"/><a:ext cx="9448800" cy="3429000"/>'))],
    ], 'chart', [
        ['ppt/charts/chart1.xml', CHART],
    ]),
});

/** Один слайд со списками: уровни, маркеры, начертания. */
const bullets = (): Deck => ({
    name: 'text/bullets.pptx',
    parts: deck('text/bullets.pptx', [
        ['ppt/slides/slide1.xml', SLIDE('Bullets', TITLE_SHAPE(2, 'Outline') + BODY_SHAPE(3,
            PARAGRAPH('Bold lead', { bold: true }) +
            PARAGRAPH('Level one', { bullet: '•' }) +
            PARAGRAPH('Level two', { bullet: '–', level: 1 }) +
            PARAGRAPH('Italic tail', { italic: true })))],
    ]),
});

const DECKS: readonly Deck[] = [twoSlides(), geometry(), picture(), chart(), bullets()];

/** Валидация: уникальные имена, префиксы категорий. */
const validate = (decks: readonly Deck[]): void => {
    const seen = new Set<string>();
    for (const deck of decks) {
        if (seen.has(deck.name)) throw new Error(`duplicate fixture name: ${deck.name}`);
        if (!deck.name.includes('/')) throw new Error(`fixture without category prefix: ${deck.name}`);
        seen.add(deck.name);
        const partNames = new Set<string>();
        for (const [part] of deck.parts) {
            if (partNames.has(part)) throw new Error(`duplicate part ${part} in ${deck.name}`);
            partNames.add(part);
        }
    }
};

const main = async (): Promise<void> => {
    validate(DECKS);
    await rm(OUT_DIR, { recursive: true, force: true });
    await mkdir(OUT_DIR, { recursive: true });
    for (const deck of DECKS) {
        const bytes = buildZip(deck.parts);
        const file = path.join(OUT_DIR, deck.name);
        await mkdir(path.dirname(file), { recursive: true });
        await writeFile(file, bytes as Uint8Array);
    }
    console.log(`OK: ${DECKS.length} фикстур PPTX -> test-fixtures/pptx/`);
};

main().catch((error) => {
    console.error(error);
    process.exit(1);
});
