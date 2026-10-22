// Фикстуры категории a11y: структура для экранных дикторов и профиль
// производительности длинного документа.
//
// `a11y_audit` — 20 страниц с настоящей структурой: заголовки Heading 1/2,
// абзацы, нумерованный и маркированный списки, три таблицы с повторяющейся
// шапкой (`w:tblHeader`) и объединениями (`w:gridSpan`/`w:vMerge`), внешняя
// ссылка и поле PAGE, кириллица. По ней проверяются NVDA/VoiceOver и axe.
//
// `a11y_perf` — 200 страниц связного текста с таблицами и без картинок:
// профиль p95 fps и виртуализации. Разрывы страниц расставлены явно, поэтому
// число страниц не зависит от метрик шрифта: N разрывов дают N + 1 страницу.

import {
    HYPERLINK_REL_TYPE,
    type FixtureSpec,
    W_NS,
    XML_DECL,
    defaultSectPr,
    escapeXml,
    h,
    p,
    pageBreak,
    packageParts,
    para,
    run,
    sectionProps,
} from './kit.js';

/**
 * Тело из страниц: между соседними страницами — явный разрыв.
 *
 * Абзацы и таблицы считаются по построению: `expectedParagraphs` — только
 * прямые дети `w:body`, а `pageBreak()` из kit — отдельный `<w:p>`, поэтому он
 * тоже идёт в счёт.
 *
 * @param pages блоки каждой страницы в порядке документа
 * @returns XML тела, число абзацев и таблиц верхнего уровня
 */
const pagesBody = (
    pages: readonly (readonly string[])[],
): { xml: string; paragraphs: number; tables: number } => {
    const blocks: string[] = [];
    let paragraphs = 0;
    let tables = 0;
    pages.forEach((page, index) => {
        for (const block of page) {
            blocks.push(block);
            if (block.startsWith('<w:tbl')) tables += 1;
            else paragraphs += 1;
        }
        if (index < pages.length - 1) {
            blocks.push(pageBreak());
            paragraphs += 1;
        }
    });
    return { xml: blocks.join('') + sectionProps(defaultSectPr), paragraphs, tables };
};

// ==================== Нумерация ====================

/** Уровень списка: формат, текст маркера и шрифт символа. */
interface ListLevel {
    numFmt: 'decimal' | 'bullet';
    lvlText: string;
    /** `w:rFonts` маркера: у Word bullet-уровни набраны Symbol/Courier New. */
    font?: string;
    /** `w:ind w:left` в twips. */
    indent?: number;
}

const listLevel = (ilvl: number, level: ListLevel): string =>
    `<w:lvl w:ilvl="${ilvl}"><w:start w:val="1"/><w:numFmt w:val="${level.numFmt}"/>` +
    `<w:lvlText w:val="${level.lvlText}"/><w:lvlJc w:val="left"/>` +
    `<w:pPr><w:ind w:left="${level.indent ?? 720}" w:hanging="360"/></w:pPr>` +
    (level.font === undefined
        ? ''
        : `<w:rPr><w:rFonts w:ascii="${level.font}" w:hAnsi="${level.font}" w:hint="default"/></w:rPr>`) +
    '</w:lvl>';

/** Детерминированный nsid — Word пишет его в каждый abstractNum. */
const nsid = (id: number): string =>
    (0x1a2b3c4d + id * 0x00010001).toString(16).toUpperCase().padStart(8, '0');

const abstractNum = (id: number, levels: readonly ListLevel[]): string =>
    `<w:abstractNum w:abstractNumId="${id}"><w:nsid w:val="${nsid(id)}"/>` +
    '<w:multiLevelType w:val="hybridMultilevel"/>' +
    levels.map((level, index) => listLevel(index, level)).join('') +
    '</w:abstractNum>';

/** Два списка: десятичный (numId 1) и маркированный (numId 2). */
const numberingXml = (): string =>
    XML_DECL +
    `<w:numbering xmlns:w="${W_NS}">` +
    abstractNum(0, [
        { numFmt: 'decimal', lvlText: '%1.' },
        { numFmt: 'decimal', lvlText: '%2.', indent: 1440 },
    ]) +
    abstractNum(1, [
        { numFmt: 'bullet', lvlText: '&#xF0B7;', font: 'Symbol' },
        { numFmt: 'bullet', lvlText: 'o', font: 'Courier New', indent: 1440 },
    ]) +
    '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>' +
    '<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>' +
    '</w:numbering>';

/** Пункты списков ссылаются на List Paragraph — так это делает Word. */
const LIST_PARAGRAPH_STYLE =
    '<w:style w:type="paragraph" w:customStyle="1" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/>' +
    '<w:basedOn w:val="Normal"/><w:uiPriority w:val="34"/><w:qFormat/>' +
    '<w:pPr><w:contextualSpacing/><w:ind w:left="720"/></w:pPr></w:style>';

/** Стиль ссылки: синий цвет и подчёркивание, как в Word. */
const HYPERLINK_STYLE =
    '<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/>' +
    '<w:basedOn w:val="DefaultParagraphFont"/><w:uiPriority w:val="99"/><w:unhideWhenUsed/>' +
    '<w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>';

/** Пункт нумерованного списка (numId 1). */
const numbered = (text: string, ilvl = 0): string =>
    p(text, { style: 'ListParagraph', numId: 1, ilvl });

/** Пункт маркированного списка (numId 2). */
const bulleted = (text: string, ilvl = 0): string =>
    p(text, { style: 'ListParagraph', numId: 2, ilvl });

// ==================== Ссылка и поле ====================

/** Внешняя ссылка: rId не пересекается с авто-связями styles/numbering. */
const LINK_REL_ID = 'rIdA11yLink';
const LINK_TARGET = 'https://example.com/a11y';
const LINK_REL = `<Relationship Id="${LINK_REL_ID}" Type="${HYPERLINK_REL_TYPE}" Target="${LINK_TARGET}" TargetMode="External"/>`;

const link = (text: string): string =>
    `<w:hyperlink r:id="${LINK_REL_ID}" w:history="1">${run(text, { style: 'Hyperlink' })}</w:hyperlink>`;

/** Прогон с готовым содержимым (`w:fldChar`, `w:instrText`, `w:t`). */
const rawRun = (inner: string): string => `<w:r>${inner}</w:r>`;

/** Поле с кэшем: begin → instrText → separate → результат → end. */
const cachedField = (instruction: string, value: string): string =>
    rawRun('<w:fldChar w:fldCharType="begin"/>') +
    rawRun(`<w:instrText xml:space="preserve">${escapeXml(instruction)}</w:instrText>`) +
    rawRun('<w:fldChar w:fldCharType="separate"/>') +
    rawRun(`<w:t xml:space="preserve">${escapeXml(value)}</w:t>`) +
    rawRun('<w:fldChar w:fldCharType="end"/>');

// ==================== Таблицы ====================

const BORDER_SIDES = ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'] as const;

/** Рамки таблицы: без них аудитор видит набор абзацев, а не таблицу. */
const TABLE_BORDERS =
    '<w:tblBorders>' +
    BORDER_SIDES.map((side) => `<w:${side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>`).join('') +
    '</w:tblBorders>';

/** Ячейка с одним абзацем; `tcPr` — готовый XML внутри `w:tcPr`. */
const cell = (text: string, tcPr = ''): string =>
    `<w:tc>${tcPr === '' ? '' : `<w:tcPr>${tcPr}</w:tcPr>`}${p(text)}</w:tc>`;

/** Ячейка фиксированной ширины с необязательным объединением. */
const cellW = (text: string, width: number, extra = ''): string =>
    cell(text, `<w:tcW w:w="${width}" w:type="dxa"/>${extra}`);

/** Строка; `trPr` — готовый XML внутри `w:trPr` (`w:tblHeader`). */
const row = (cells: string, trPr = ''): string =>
    `<w:tr>${trPr === '' ? '' : `<w:trPr>${trPr}</w:trPr>`}${cells}</w:tr>`;

/** Таблица с `w:tblGrid` и рамками; grid обязан идти за `w:tblPr`. */
const tbl = (grid: readonly number[], rows: readonly string[]): string =>
    '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/>' +
    TABLE_BORDERS +
    '<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="0" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/>' +
    '</w:tblPr>' +
    `<w:tblGrid>${grid.map((width) => `<w:gridCol w:w="${width}"/>`).join('')}</w:tblGrid>` +
    rows.join('') +
    '</w:tbl>';

/** 4×3: шапка повторяется, в теле есть gridSpan и vMerge. */
const COMPONENT_TABLE = tbl(
    [3000, 3000, 3000],
    [
        row(
            cellW('Компонент', 3000) + cellW('Критерий', 3000) + cellW('Статус', 3000),
            '<w:tblHeader/>',
        ),
        row(cellW('Клавиатурная навигация', 6000, '<w:gridSpan w:val="2"/>') + cellW('Пройдено', 3000)),
        row(
            cellW('Экранный диктор', 3000, '<w:vMerge w:val="restart"/>') +
                cellW('NVDA', 3000) +
                cellW('Пройдено', 3000),
        ),
        row(cellW('', 3000, '<w:vMerge/>') + cellW('VoiceOver', 3000) + cellW('В работе', 3000)),
    ],
);

/** 4×2: шапка повторяется, последняя строка объединена по горизонтали. */
const CHECKS_TABLE = tbl(
    [4500, 4500],
    [
        row(cellW('Проверка', 4500) + cellW('Ожидание', 4500), '<w:tblHeader/>'),
        row(cellW('Порядок заголовков', 4500) + cellW('H1 и H2 без пропусков', 4500)),
        row(cellW('Имена элементов', 4500) + cellW('У ссылок и полей есть текст', 4500)),
        row(cellW('Все проверки пройдены', 9000, '<w:gridSpan w:val="2"/>')),
    ],
);

/** 3×2: шапка повторяется; строки перечисляют платформы. */
const PLATFORMS_TABLE = tbl(
    [4500, 4500],
    [
        row(cellW('Платформа', 4500) + cellW('Диктор', 4500), '<w:tblHeader/>'),
        row(cellW('Windows', 4500) + cellW('NVDA', 4500)),
        row(cellW('macOS', 4500) + cellW('VoiceOver', 4500)),
    ],
);

// ==================== a11y_audit ====================

/**
 * 20 страниц аудита. Разрывов 19: последняя страница заканчивается `w:sectPr`.
 * Содержимого на страницу немного, поэтому переполнение не добавляет листов.
 */
const AUDIT_PAGES: readonly (readonly string[])[] = [
    // 1. Титул: H1, ссылка и поле PAGE.
    [
        h('Доступность документа', 1),
        p('Этот документ проверяет структуру, которую видят экранные дикторы и автоматические аудиторы.'),
        para('', run('Полное руководство: ') + link('Руководство по доступности')),
        para('', run('Версия макета: ') + cachedField(' PAGE ', '1')),
    ],
    // 2. Абзацы под H2.
    [
        h('Структура заголовков', 2),
        p('Заголовок первого уровня задаёт раздел, второго — подраздел внутри него.'),
        p('Экранный диктор перечисляет заголовки в порядке документа и позволяет перейти к нужному разделу.'),
        p('Пропуск уровня ломает навигацию, поэтому иерархия в этом документе непрерывна.'),
    ],
    // 3. Нумерованный список.
    [
        h('Нумерованный список', 2),
        p('Порядок пунктов важен: диктор объявляет номер пункта и общее количество.'),
        numbered('Проверить иерархию заголовков.'),
        numbered('Прочитать текст с экранным диктором.'),
        numbered('Пройти по ссылкам клавишей Tab.'),
        numbered('Сверить объявления таблиц.'),
    ],
    // 4. Маркированный список.
    [
        h('Маркированный список', 2),
        p('Маркеры не задают порядок, но диктор всё равно объявляет список.'),
        bulleted('Заголовки и подзаголовки.'),
        bulleted('Основной текст и примечания.'),
        bulleted('Ссылки и поля.'),
    ],
    // 5. Таблица с gridSpan и vMerge.
    [
        h('Таблица с объединёнными ячейками', 2),
        p('Шапка помечена как заголовочная строка и повторяется, если таблица переносится на следующую страницу.'),
        COMPONENT_TABLE,
        p('Объединения по горизонтали и вертикали сохраняют связи между ячейками.'),
    ],
    // 6. Вторая таблица.
    [
        h('Таблица проверок', 2),
        p('Каждая строка описывает одну проверку и ожидаемый результат.'),
        CHECKS_TABLE,
    ],
    // 7. Третья таблица.
    [
        h('Таблица платформ', 2),
        p('Проверки выполняются на двух платформах с разными дикторами.'),
        PLATFORMS_TABLE,
        p('Расхождения записываются в журнал доступности.'),
    ],
    // 8. Второй H1.
    [
        h('Работа с экранным диктором', 1),
        p('Диктор читает документ в том порядке, в котором блоки записаны в теле.'),
        p('Визуальный порядок совпадает с логическим: таблицы не разрывают поток текста.'),
    ],
    // 9. H2 + нумерованный список.
    [
        h('NVDA', 2),
        p('На Windows проверка идёт в NVDA с включённым режимом обзора.'),
        numbered('Открыть документ и дождаться окончания разбора.'),
        numbered('Проверить список заголовков клавишей Insert+F7.'),
        numbered('Перейти к каждой таблице и прочитать шапку.'),
    ],
    // 10. H2 + вложенный маркированный список.
    [
        h('VoiceOver', 2),
        p('На macOS проверка идёт в VoiceOver с ротором по заголовкам и таблицам.'),
        bulleted('Убедиться, что H1 объявляется как заголовок первого уровня.'),
        bulleted('Проверить, что ссылка открывается клавишей Enter.'),
        bulleted('Пройти ротором по спискам.', 1),
        bulleted('Вернуться к началу документа.'),
    ],
    // 11. H2.
    [
        h('Порядок фокуса', 2),
        p('Фокус движется сверху вниз и слева направо, совпадая с порядком чтения.'),
        p('Ссылка получает фокус после текста, в котором она упомянута.'),
    ],
    // 12. H2.
    [
        h('Клавиатурные сокращения', 2),
        p('Все действия доступны с клавиатуры: мышь не обязательна.'),
        p('Tab перемещает фокус, Shift+Tab возвращает его назад.'),
        p('Enter активирует ссылку, Escape закрывает временные панели.'),
    ],
    // 13. Третий H1.
    [
        h('Клавиатурная навигация', 1),
        p('Раздел описывает перемещение по длинному документу без прокрутки мышью.'),
        p('Каждый интерактивный элемент достижим и имеет видимую рамку фокуса.'),
    ],
    // 14. H2 + нумерованный список.
    [
        h('Перемещение по документу', 2),
        p('Навигация строится на заголовках, списках и таблицах.'),
        numbered('Перейти к следующему заголовку.'),
        numbered('Перейти к следующему списку.'),
        numbered('Перейти к следующей таблице.'),
    ],
    // 15. H2.
    [
        h('Пропуск блоков', 2),
        p('Ссылки «перейти к содержимому» позволяют пропустить повторяющиеся блоки.'),
        p('В этом документе повторяющихся блоков нет, поэтому пропуск ведёт к первому заголовку.'),
    ],
    // 16. Четвёртый H1.
    [
        h('Контраст и масштабирование', 1),
        p('Текст остаётся читаемым при масштабе до 200 процентов без горизонтальной прокрутки.'),
        p('Цвет не единственный носитель смысла: статусы продублированы текстом.'),
    ],
    // 17. H2 + маркированный список.
    [
        h('Цветовые токены', 2),
        p('Палитра проверена на контраст основного текста и фона.'),
        bulleted('Основной текст: не ниже 4.5 к 1.'),
        bulleted('Крупный текст: не ниже 3 к 1.'),
        bulleted('Рамка фокуса: не ниже 3 к 1.'),
    ],
    // 18. H2.
    [
        h('Масштаб 200 процентов', 2),
        p('При увеличении текст переносится по словам, а таблицы не выходят за поля.'),
        p('Горизонтальная прокрутка страницы не появляется.'),
    ],
    // 19. H1 + нумерованный чек-лист.
    [
        h('Чек-лист проверки', 1),
        p('Итоговый список повторяет шаги приёмки.'),
        numbered('Проверить иерархию заголовков.'),
        numbered('Прочитать все таблицы с шапкой.'),
        numbered('Пройти по ссылкам и полям.'),
        numbered('Проверить контраст и масштаб.'),
        numbered('Записать результат в журнал.'),
    ],
    // 20. Финал: H2 и ссылка.
    [
        h('Итог', 2),
        p('Документ покрывает структуру, списки, таблицы, ссылки и масштабирование.'),
        p('Результаты проверки переносятся в общий отчёт доступности.'),
        para('', run('Стандарт: ') + link('Web Content Accessibility Guidelines')),
    ],
];

// ==================== a11y_perf ====================

/** Страниц в профиле производительности: N страниц требуют N − 1 разрывов. */
const PERF_PAGES = 200;
/** Абзацев на страницу: заведомо меньше листа, чтобы переполнение не росло. */
const PERF_PARAGRAPHS_PER_PAGE = 8;
/** Каждая двадцатая страница несёт таблицу: ровно 10 таблиц на 200 страниц. */
const PERF_TABLE_EVERY = 20;

/** Фразы связного текста; выбираются по индексу, без случайности. */
const PERF_SENTENCES: readonly string[] = [
    'Связный текст проверяет, как вьюер раскладывает длинный документ и удерживает частоту кадров при прокрутке.',
    'Абзацы идут непрерывным потоком, чтобы виртуализация страниц оставалась единственным способом уложиться в бюджет.',
    'Разрывы страниц расставлены явно, поэтому число листов не зависит от метрик шрифта и ширины полосы набора.',
    'Таблицы добавлены небольшими порциями: они дают раскладке другую работу, но не меняют порядок чтения.',
    'Кириллица занимает больше байтов, чем латиница, поэтому файл остаётся текстовым и хорошо сжимается.',
    'Повторный запуск генератора даёт те же байты: фразы, индексы и таймстампы фиксированы.',
    'Профиль нужен для замера p95 fps: длинная прокрутка не должна давать провалов кадров.',
    'Картинок в профиле нет: вес и раскладку определяет только текст и несколько таблиц.',
];

/**
 * Текст абзаца: три разные фразы подряд.
 *
 * Смещения 0, 3 и 6 по кругу из восьми фраз не совпадают, поэтому в абзаце
 * нет повторов, а последовательность детерминирована номером страницы.
 */
const perfParagraph = (page: number, index: number): string => {
    const base = (page * PERF_PARAGRAPHS_PER_PAGE + index) % PERF_SENTENCES.length;
    const phrase = (offset: number): string => PERF_SENTENCES[(base + offset) % PERF_SENTENCES.length]!;
    return `${phrase(0)} ${phrase(3)} ${phrase(6)}`;
};

/** Таблица профиля: та же шапка и короткие строки, чтобы страница не переполнялась. */
const PERF_TABLE = tbl(
    [3000, 3000, 3000],
    [
        row(cellW('Метрика', 3000) + cellW('Значение', 3000) + cellW('Условие', 3000), '<w:tblHeader/>'),
        row(cellW('p95 fps', 3000) + cellW('60', 3000) + cellW('Прокрутка без пропусков', 3000)),
        row(cellW('p99 fps', 3000) + cellW('55', 3000) + cellW('Плавная прокрутка', 3000)),
        row(cellW('Память', 3000) + cellW('64 МиБ', 3000) + cellW('Виртуализация страниц', 3000)),
        row(cellW('Время открытия', 3000) + cellW('2 с', 3000) + cellW('Разбор и первая раскладка', 3000)),
    ],
);

/** Страницы профиля: 8 абзацев, на каждой двадцатой ещё и таблица. */
const PERF_PAGE_BLOCKS: readonly (readonly string[])[] = Array.from(
    { length: PERF_PAGES },
    (_, index): string[] => {
        const page = index + 1;
        const blocks: string[] = [];
        for (let i = 0; i < PERF_PARAGRAPHS_PER_PAGE; i += 1) {
            blocks.push(p(perfParagraph(page, i)));
        }
        if (page % PERF_TABLE_EVERY === 0) blocks.push(PERF_TABLE);
        return blocks;
    },
);

// ==================== Фикстуры ====================

const AUDIT = pagesBody(AUDIT_PAGES);
const PERF = pagesBody(PERF_PAGE_BLOCKS);

export const a11yFixtures: FixtureSpec[] = [
    {
        name: 'a11y/a11y_audit',
        description:
            '20 страниц для NVDA/VoiceOver и axe: заголовки H1/H2, списки, таблицы с шапкой и объединениями, ссылка и поле PAGE',
        expectedParagraphs: AUDIT.paragraphs,
        expectedTables: AUDIT.tables,
        expectedImages: 0,
        // Число страниц выводится из разметки: разрывов на один меньше.
        meta: { pages: AUDIT_PAGES.length, pageBreaks: AUDIT_PAGES.length - 1 },
        parts: packageParts({
            body: AUDIT.xml,
            numbering: numberingXml(),
            stylesExtra: LIST_PARAGRAPH_STYLE + HYPERLINK_STYLE,
            extraRels: LINK_REL,
        }),
        content: {
            pages: AUDIT_PAGES.length,
            paragraphs: [
                { text: 'Доступность документа', style: 'Heading1' },
                { text: 'Структура заголовков', style: 'Heading2' },
                {
                    text: 'Полное руководство: Руководство по доступности',
                    hyperlink: LINK_TARGET,
                    relId: LINK_REL_ID,
                },
                { text: 'Версия макета: 1', field: 'PAGE' },
                { text: 'Проверить иерархию заголовков.', numId: 1, ilvl: 0 },
                { text: 'Заголовки и подзаголовки.', numId: 2, ilvl: 0 },
                { text: 'Пройти ротором по спискам.', numId: 2, ilvl: 1 },
            ],
            tables: [
                {
                    rows: 4,
                    cols: 3,
                    headerRows: 1,
                    gridSpan: [{ row: 1, col: 0, span: 2 }],
                    vMerge: [
                        { row: 2, col: 0, val: 'restart' },
                        { row: 3, col: 0, val: 'continue' },
                    ],
                    cells: [
                        ['Компонент', 'Критерий', 'Статус'],
                        ['Клавиатурная навигация', 'Пройдено'],
                        ['Экранный диктор', 'NVDA', 'Пройдено'],
                        ['', 'VoiceOver', 'В работе'],
                    ],
                },
                {
                    rows: 4,
                    cols: 2,
                    headerRows: 1,
                    gridSpan: [{ row: 3, col: 0, span: 2 }],
                    cells: [
                        ['Проверка', 'Ожидание'],
                        ['Порядок заголовков', 'H1 и H2 без пропусков'],
                        ['Имена элементов', 'У ссылок и полей есть текст'],
                        ['Все проверки пройдены'],
                    ],
                },
                {
                    rows: 3,
                    cols: 2,
                    headerRows: 1,
                    cells: [
                        ['Платформа', 'Диктор'],
                        ['Windows', 'NVDA'],
                        ['macOS', 'VoiceOver'],
                    ],
                },
            ],
            fields: [{ instruction: ' PAGE ', type: 'page', cached: '1', form: 'fldChar' }],
            rels: [
                {
                    id: LINK_REL_ID,
                    type: HYPERLINK_REL_TYPE,
                    target: LINK_TARGET,
                    external: true,
                },
            ],
            images: [],
        },
    },
    {
        name: 'a11y/a11y_perf',
        description:
            '200 страниц связного текста и таблиц без картинок — профиль p95 fps и виртуализации',
        expectedParagraphs: PERF.paragraphs,
        expectedTables: PERF.tables,
        expectedImages: 0,
        // Число страниц выводится из разметки: разрывов на один меньше.
        meta: { pages: PERF_PAGES, pageBreaks: PERF_PAGES - 1 },
        parts: packageParts({ body: PERF.xml }),
        content: {
            pages: PERF_PAGES,
            pageBreaks: PERF_PAGES - 1,
            paragraphsPerPage: PERF_PARAGRAPHS_PER_PAGE,
            paragraphs: [
                { text: perfParagraph(1, 0) },
                { text: perfParagraph(1, PERF_PARAGRAPHS_PER_PAGE - 1) },
                { text: perfParagraph(PERF_PAGES, 0) },
            ],
            tables: [{ rows: 5, cols: 3, headerRows: 1, count: PERF_PAGES / PERF_TABLE_EVERY }],
            images: [],
        },
    },
];
