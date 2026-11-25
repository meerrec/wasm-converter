// Фикстуры приоритетного состава, которых не было: пагинация (`keepNext`,
// widow/orphan), колонки, концевые сноски, Office Math, SmartArt/OLE и
// системный шрифт с fallback-политикой.
//
// Почему отдельным модулем, а не в существующих: эти фикстуры описывают
// возможности, которых движок либо не умеет, либо умеет частично. Держать их
// рядом с «работающими» — значит путать регрессионный набор с набором
// ограничений.
//
// Math, SmartArt и OLE в первой сертификации — `unsupported-visible`: их
// присутствие обязано попадать в отчёт, а не исчезать молча. Код отчёта
// (`docx-math-unsupported`, `docx-smartart-excluded`, `docx-ole-excluded`)
// кладётся в `meta`, а не в `expectedWarnings`: там только `WarningKind`
// парсера по ADR-0016.

import {
    type FixtureSpec,
    R_NS,
    W_NS,
    XML_DECL,
    defaultSectPr,
    escapeXml,
    packageParts,
    p,
    para,
    run,
    sectionProps,
} from './kit.js';

/** Пространство имён Office Math. */
const M_NS = 'http://schemas.openxmlformats.org/officeDocument/2006/math';

/** Тело из блоков + финальный `w:sectPr` (обязан быть последним в `w:body`). */
const body = (...blocks: string[]): string => blocks.join('') + sectionProps(defaultSectPr);

/** Абзац с произвольным `w:pPr` и прогонами. */
const withProps = (pPr: string, runs: string): string => `<w:p><w:pPr>${pPr}</w:pPr>${runs}</w:p>`;

/** Прогон с указанной гарнитурой. */
const fontRun = (text: string, font: string): string =>
    `<w:r><w:rPr><w:rFonts w:ascii="${font}" w:hAnsi="${font}" w:cs="${font}"/></w:rPr>` +
    `<w:t xml:space="preserve">${escapeXml(text)}</w:t></w:r>`;

// ==================== Колонки ====================

/** `w:sectPr` с несколькими колонками: `w:cols` идёт после `w:pgMar`, до `w:docGrid`. */
const columnsSectPr = (num: number, equalWidth: boolean, sep: boolean): string =>
    '<w:pgSz w:w="11906" w:h="16838"/>' +
    '<w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="709" w:footer="709" w:gutter="0"/>' +
    `<w:cols w:num="${num}"${equalWidth ? '' : ' w:equalWidth="0"'} w:space="708"${sep ? ' w:sep="1"' : ''}>` +
    (equalWidth
        ? ''
        : '<w:col w:w="2551" w:space="708"/><w:col w:w="5102" w:space="708"/><w:col w:w="2551"/>') +
    '</w:cols><w:docGrid w:linePitch="360"/>';

const columnsTwo: FixtureSpec = {
    name: 'columns/two_equal',
    description: 'Две равные колонки с разрывом колонки и разделителем',
    expectedParagraphs: 6,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body:
            p('Первая колонка: текст идёт сверху вниз и переносится в следующую колонку только по разрыву.') +
            p('Второй абзац первой колонки.') +
            para('', '<w:r><w:br w:type="column"/></w:r>') +
            p('Вторая колонка после явного разрыва колонки.') +
            p('Ещё абзац второй колонки.') +
            p('Последний абзац.') +
            `<w:sectPr>${columnsSectPr(2, true, true)}</w:sectPr>`,
    }),
    meta: { columns: { count: 2, equalWidth: true, separator: true } },
};

const columnsThreeUnequal: FixtureSpec = {
    name: 'columns/three_unequal',
    description: 'Три колонки разной ширины без выравнивания',
    expectedParagraphs: 6,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body:
            p('Узкая колонка.') +
            para('', '<w:r><w:br w:type="column"/></w:r>') +
            p('Широкая центральная колонка: её ширина вдвое больше соседних.') +
            para('', '<w:r><w:br w:type="column"/></w:r>') +
            p('Третья колонка.') +
            p('И ещё одна строка в третьей.') +
            `<w:sectPr>${columnsSectPr(3, false, false)}</w:sectPr>`,
    }),
    meta: { columns: { count: 3, equalWidth: false, widths: [2551, 5102, 2551] } },
};

// ==================== Пагинация ====================

/**
 * Содержимое `w:pPr` с `keepNext`/`keepLines`/`widowControl`.
 *
 * Возвращает только детей `w:pPr`: обёртку добавляет `withProps`, иначе
 * получается `w:pPr` внутри `w:pPr` — невалидный OOXML и предупреждение
 * `unknown_element` на каждом таком абзаце.
 */
const keepProps = (opts: { keepNext?: boolean; keepLines?: boolean; widowControl?: boolean }): string =>
    (opts.keepNext === true ? '<w:keepNext/>' : '') +
    (opts.keepLines === true ? '<w:keepLines/>' : '') +
    (opts.widowControl === false ? '<w:widowControl w:val="0"/>' : '<w:widowControl/>');

const keepNextChain: FixtureSpec = {
    name: 'pagination/keep_next_chain',
    description: 'Цепочка абзацев с keepNext: заголовок не отрывается от следующего абзаца',
    expectedParagraphs: 12,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            ...Array.from({ length: 6 }, (_, index) => p(`Заполняющий абзац ${index + 1}.`)),
            withProps(keepProps({ keepNext: true, keepLines: true }), run('Заголовок, который переезжает со следующим абзацем')),
            withProps(keepProps({ keepNext: true }), run('Второй абзац цепочки keepNext.')),
            p('Первый обычный абзац после цепочки.'),
            p('Второй обычный абзац.'),
            p('Третий обычный абзац.'),
            p('Четвёртый обычный абзац.'),
        ),
    }),
    meta: { keepNext: [7, 8], keepLines: [7] },
};

const widowOrphan: FixtureSpec = {
    name: 'pagination/widow_orphan',
    description: 'Widow/orphan: два абзаца с разной политикой на границе страниц',
    expectedParagraphs: 10,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            ...Array.from({ length: 5 }, (_, index) => p(`Абзац перед разрывом ${index + 1}.`)),
            // Один абзац оставляет вдову (последняя строка на новой странице),
            // второй запрещает её: пара нужна, чтобы разница была наглядна.
            withProps(keepProps({ widowControl: true }), run(
                'Этот абзац содержит достаточно строк, чтобы уместиться целиком или перенестись с контролем висячих строк. '.repeat(4),
            )),
            withProps(keepProps({ widowControl: false }), run(
                'А этот абзац отключает контроль висячих строк и потому оставляет одну строку на следующей странице. '.repeat(4),
            )),
            p('Абзац после пары.'),
            p('Ещё один абзац.'),
            p('Последний абзац.'),
        ),
    }),
    meta: { widowControl: { enabled: 1, disabled: 1 } },
};

// ==================== Концевые сноски ====================

/** Часть `word/endnotes.xml`. */
const endnotesXml = (...notes: string[]): string =>
    XML_DECL + `<w:endnotes xmlns:w="${W_NS}" xmlns:r="${R_NS}">${notes.join('')}</w:endnotes>`;

/** Концевая сноска с текстом. */
const endnote = (id: number, text: string): string =>
    `<w:endnote w:id="${id}"><w:p><w:pPr><w:pStyle w:val="EndnoteText"/></w:pPr>` +
    '<w:r><w:rPr><w:rStyle w:val="EndnoteReference"/></w:rPr><w:endnoteRef/></w:r>' +
    `<w:r><w:t xml:space="preserve"> ${escapeXml(text)}</w:t></w:r></w:p></w:endnote>`;

/** Служебная концевая сноска-разделитель. */
const separatorEndnote = (type: 'separator' | 'continuationSeparator', id: number, element: string): string =>
    `<w:endnote w:type="${type}" w:id="${id}"><w:p>` +
    '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>' +
    `<w:r><w:${element}/></w:r></w:p></w:endnote>`;

/** Прогон со ссылкой на концевую сноску. */
const endnoteRef = (id: number): string =>
    `<w:r><w:rPr><w:rStyle w:val="EndnoteReference"/></w:rPr><w:endnoteReference w:id="${id}"/></w:r>`;

/** Стили концевых сносок: в defaultStylesXml их нет. */
const ENDNOTE_STYLES =
    '<w:style w:type="paragraph" w:styleId="EndnoteText"><w:name w:val="endnote text"/>' +
    '<w:basedOn w:val="Normal"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/>' +
    '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>' +
    '<w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>' +
    '<w:style w:type="character" w:styleId="EndnoteReference"><w:name w:val="endnote reference"/>' +
    '<w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/>' +
    '<w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>';

const endnoteBasic: FixtureSpec = {
    name: 'notes/endnote_basic',
    description: 'Концевая сноска: endnotes.xml и w:endnoteReference в тексте',
    expectedParagraphs: 2,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: [
        ...packageParts({
            body: body(
                para('', run('Текст с концевой сноской') + endnoteRef(1)),
                p('Второй абзац после ссылки.'),
            ),
            stylesExtra: ENDNOTE_STYLES,
        }).filter(([name]) => name !== 'word/document.xml'),
        // Концевые сноски — отдельная часть; добавляем её к документу вручную,
        // потому что kit знает только о footnotes/comments.
        [
            'word/document.xml',
            XML_DECL +
                `<w:document xmlns:w="${W_NS}" xmlns:r="${R_NS}">` +
                `<w:body>${body(
                    para('', run('Текст с концевой сноской') + endnoteRef(1)),
                    p('Второй абзац после ссылки.'),
                )}</w:body></w:document>`,
        ],
        ['word/endnotes.xml', endnotesXml(
            separatorEndnote('separator', -1, 'separator'),
            separatorEndnote('continuationSeparator', 0, 'continuationSeparator'),
            endnote(1, 'Текст концевой сноски.'),
        )],
    ],
    content: { endnotes: [{ id: 1, text: 'Текст концевой сноски.' }] },
};

// ==================== Office Math ====================

/** Простое уравнение: x = (a + b) / 2. */
const OMML_RUN_A = '<m:r><m:t>a</m:t></m:r>';
const OMML_RUN_B = '<m:r><m:t>b</m:t></m:r>';
const OMML_RUN_X = '<m:r><m:t>x</m:t></m:r>';

const mathSimple: FixtureSpec = {
    name: 'math/omml_simple',
    description: 'Office Math: дробь и сумма в m:oMathPara',
    expectedParagraphs: 2,
    expectedTables: 0,
    expectedImages: 0,
    // `expectedWarnings` — только `WarningKind` парсера (ADR-0016). Код отчёта
    // `docx-math-unsupported` живёт в `meta`, не здесь.
    expectedWarnings: [],
    parts: [
        ...packageParts({
            body: body(
                p('Формула перед абзацем.'),
                `<w:p><m:oMathPara xmlns:m="${M_NS}"><m:oMath>` +
                    `${OMML_RUN_X}<m:r><m:t>=</m:t></m:r>` +
                    '<m:f><m:num>' +
                    `${OMML_RUN_A}<m:r><m:t>+</m:t></m:r>${OMML_RUN_B}` +
                    '</m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f>' +
                    '</m:oMath></m:oMathPara></w:p>',
            ),
        }).filter(([name]) => name !== 'word/document.xml'),
        [
            'word/document.xml',
            XML_DECL +
                `<w:document xmlns:w="${W_NS}" xmlns:r="${R_NS}" xmlns:m="${M_NS}">` +
                `<w:body>${body(
                    p('Формула перед абзацем.'),
                    `<w:p><m:oMathPara><m:oMath>` +
                        `${OMML_RUN_X}<m:r><m:t>=</m:t></m:r>` +
                        '<m:f><m:num>' +
                        `${OMML_RUN_A}<m:r><m:t>+</m:t></m:r>${OMML_RUN_B}` +
                        '</m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f>' +
                        '</m:oMath></m:oMathPara></w:p>',
                )}</w:body></w:document>`,
        ],
    ],
    content: { math: [{ kind: 'fraction', text: 'x = (a + b) / 2' }] },
};

// ==================== SmartArt и OLE ====================

/** Части SmartArt: минимальный `dataModel` с одним узлом. */
const SMARTART_DATA_MODEL =
    XML_DECL +
    '<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" ' +
    'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">' +
    '<dgm:ptLst><dgm:pt modelId="1"><dgm:prSet/><dgm:spPr/><dgm:t>' +
    '<a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Процесс</a:t></a:r></a:p>' +
    '</dgm:t></dgm:pt></dgm:ptLst><dgm:cxnLst/><dgm:bg/><dgm:whole/></dgm:dataModel>';

const smartArt: FixtureSpec = {
    name: 'embedded/smartart_basic',
    description: 'SmartArt: ссылка на diagrams/dataModel в w:drawing',
    expectedParagraphs: 2,
    expectedTables: 0,
    expectedImages: 0,
    // `docx-smartart-excluded` — код отчёта совместимости, не `WarningKind`.
    expectedWarnings: [],
    parts: [
        ...packageParts({
            body: body(
                p('Абзац перед диаграммой.'),
            ),
            extraRels:
                '<Relationship Id="rIdDgm1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="diagrams/data1.xml"/>',
        }).filter(([name]) => name !== 'word/document.xml'),
        [
            'word/document.xml',
            XML_DECL +
                `<w:document xmlns:w="${W_NS}" xmlns:r="${R_NS}" ` +
                'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" ' +
                'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">' +
                '<w:body>' +
                p('Абзац перед диаграммой.') +
                // SmartArt приходит как w:drawing с dgm:relIds — именно эта
                // ссылка и означает «здесь диаграмма», а не картинка.
                '<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">' +
                '<wp:extent cx="5486400" cy="3200400"/><wp:docPr id="1" name="Diagram 1"/>' +
                '<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram">' +
                '<dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" ' +
                'r:dm="rIdDgm1" r:lo="rIdDgm2" r:qs="rIdDgm3" r:cs="rIdDgm4"/>' +
                '</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>' +
                sectionProps(defaultSectPr) +
                '</w:body></w:document>',
        ],
        ['word/diagrams/data1.xml', SMARTART_DATA_MODEL],
    ],
    content: { smartArt: [{ nodes: ['Процесс'] }] },
};

const oleObject: FixtureSpec = {
    name: 'embedded/ole_object',
    description: 'OLE-объект: w:object с VML-превью и embedding',
    expectedParagraphs: 1,
    expectedTables: 0,
    expectedImages: 0,
    // `docx-ole-excluded` — код отчёта совместимости, не `WarningKind`.
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            '<w:p><w:r><w:object>' +
                '<v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="ole1" type="#_x0000_t75" ' +
                'style="width:120pt;height:80pt"><v:imagedata r:id="rIdOle" o:title=""/></v:shape>' +
                '</w:object></w:r></w:p>',
        ),
        extraRels:
            '<Relationship Id="rIdOle" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="embeddings/oleObject1.bin"/>',
        extraParts: { 'word/embeddings/oleObject1.bin': new Uint8Array([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) },
    }),
    content: { ole: [{ progId: 'Package', preview: true }] },
};

// ==================== Шрифты ====================

const systemFontFallback: FixtureSpec = {
    name: 'fonts/system_font_fallback',
    description: 'Системные шрифты вне пакета: Times New Roman, Wingdings, отсутствующий в системе',
    expectedParagraphs: 3,
    expectedTables: 0,
    expectedImages: 0,
    expectedWarnings: [],
    parts: packageParts({
        body: body(
            para('', fontRun('Times New Roman — системный шрифт, вне версионированного пакета. ', 'Times New Roman')),
            para('', fontRun('Wingdings: символы заменяются, если гарнитуры нет. ', 'Wingdings') + run('← обычный текст рядом')),
            para('', fontRun('Гарнитуры нет ни в пакете, ни в системе: ', 'This Font Does Not Exist') + run('текст обязан остаться видимым через fallback.')),
        ),
    }),
    meta: {
        fonts: {
            outsidePackage: ['Times New Roman', 'Wingdings', 'This Font Does Not Exist'],
            fallbackPolicy: 'Отсутствующая гарнитура заменяется шрифтом по умолчанию; текст не исчезает.',
        },
    },
    content: { paragraphs: 3 },
};

/** Шесть фикстур приоритетного состава, которых не было. */
export const strictViewerFixtures: FixtureSpec[] = [
    columnsTwo,
    columnsThreeUnequal,
    keepNextChain,
    widowOrphan,
    endnoteBasic,
    mathSimple,
    smartArt,
    oleObject,
    systemFontFallback,
];
