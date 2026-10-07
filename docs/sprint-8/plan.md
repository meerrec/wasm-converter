# Спринт 8: Парсинг DOCX + fuzz

**Файл:** `docs/sprint-8/plan.md`
**Версия:** 2.0 (пересмотр от 07.10.2026)
**Длительность:** 2 недели (10 рабочих дней) + 2 дня резерва
**Команда:** R1, R2 (Rust), QA, DevOps (частично)
**Статус:** план утверждён к старту после закрытия ADR 0013, 0014, 0015, 0016, 0017, 0019, 0020
**Блокирует:** Спринт 9 (layout DOCX), Спринт 10 (рендер + PDF DOCX)

## 1. Цель

> Разобрать DOCX в нормализованную модель, пригодную для layout в Спринте 9, с fuzz-инфраструктурой с первого дня, дифференциальной проверкой против mammoth.js и зафиксированными ADR по каскаду стилей, лимитам ZIP, политике ошибок и стабильным идентификаторам узлов.

Три ключевых слова:

1. **Нормализованная** — модель несёт `RawPPr`/`RawRPr` + ссылки на стили, каскад резолвится в Спринте 9 (ADR-0013).
2. **Пригодная для layout** — модель содержит всё, что нужно Спринту 9: секции, floats, списки, таблицы с conditional formats, `NodeId` (ADR-0019).
3. **Дифференциальная** — 50 фикстур прогоняются через mammoth.js, расхождения документируются.

## 2. Definition of Ready

### 2.1. ADR приняты

| ADR | Тема | Статус |
|---|---|---|
| ADR-0013 | Каскад стилей DOCX | принят |
| ADR-0014 | `mc:AlternateContent` | принят |
| ADR-0015 | Лимиты ZIP для OOXML | принят |
| ADR-0016 | Политика ошибок парсинга DOCX | принят |
| ADR-0017 | Strict vs Transitional OOXML | принят (non-goal: Strict) |
| ADR-0019 | NodeId | принят |
| ADR-0020 | Границы парсера и writer | принят (writer — non-goal v1) |

### 2.2. Инфраструктура

- `cargo-fuzz` установлен, `crates/fuzz/` создан.
- `cargo-llvm-cov` в CI, порог задан.
- `cargo-deny` + `cargo-audit` в CI.
- `mammoth` установлен как dev-зависимость.
- `scripts/diff-mammoth.ts` — заготовка.
- `test-fixtures/docx/` — 75 новых фикстур готовы.

### 2.3. Фикстуры

| Категория | Количество | Что проверяет |
|---|---|---|
| Базовые | 10 | Текст, параграфы, runs |
| Стили | 5 | `basedOn`, `link`, `docDefaults`, `default` |
| Нумерация | 5 | `abstractNum`, `num`, `lvlOverride`, `startOverride` |
| Таблицы | 5 | Вложенные, `gridSpan`, `vMerge`, `tblLook`, borders |
| Изображения | 5 | `wp:inline`, `wp:anchor`, wrap, position |
| `mc:AlternateContent` | 5 | Choice/Fallback, фигуры, текстовые поля |
| Track changes | 5 | `w:ins`, `w:del` (парсятся как Unknown) |
| Поля | 5 | `fldSimple`, `instrText`, `fldChar` |
| RTL | 5 | Арабский, иврит |
| CJK | 5 | Китайский, японский, корейский |
| Колонтитулы | 5 | `first`, `even`, `default`, `titlePg` |
| Сноски/комментарии | 5 | `footnotes.xml`, `comments.xml` |
| Битые | 5 | Отсутствующий rels, циклический `basedOn`, битый ZIP |
| Большие | 5 | 10+ МиБ, для бюджетов |
| **Итого** | **75** | |

## 3. Модель

### 3.1. Дерево

```
Document {
    id: NodeId,
    body: Body,
    styles: StyleTable,
    numbering: NumberingTable,
    settings: Settings,
    metadata: Metadata,
    rels: Relationships,
    warnings: Vec<ParseWarning>,
}

Body {
    id: NodeId,
    items: Vec<BlockItem>,
    sections: Vec<Section>,
}

enum BlockItem {
    Paragraph(Paragraph),
    Table(Table),
    SectPr(SectionProperties),
    Unknown { id: NodeId, xml: String },
}

Paragraph {
    id: NodeId,
    ppr: RawPPr,
    mark_rpr: RawRPr,
    runs: Vec<Inline>,
    style_ref: Option<StyleId>,
    numbering_ref: Option<NumId>,
    section_break: Option<SectionProperties>,
}

enum Inline {
    Run(Run),
    Hyperlink(Hyperlink),
    Bookmark(Bookmark),
    Field(Field),
    Break(BreakKind),
    Tab,
    Symbol { font: String, char: char },
    Drawing(InlineOrAnchor),
    Unknown { id: NodeId, xml: String },
}

Run {
    id: NodeId,
    rpr: RawRPr,
    style_ref: Option<StyleId>,
    content: Vec<RunContent>,
}

enum RunContent {
    Text(String),
    Tab,
    Break(BreakKind),
    Symbol { font: String, char: char },
    Drawing(InlineOrAnchor),
}

struct InlineOrAnchor {
    id: NodeId,
    inline: Option<InlineImage>,
    anchor: Option<Anchor>,
}

Anchor {
    id: NodeId,
    horizontal: PositionH,
    vertical: PositionV,
    wrap: WrapKind,
    behind_text: bool,
    relative_from: RelFromH / RelFromV,
    image: InlineImage,
}

Table {
    id: NodeId,
    style_ref: Option<StyleId>,
    grid: Vec<GridCol>,
    rows: Vec<Row>,
    layout: TableLayout,
    width: Option<TableWidth>,
    borders: TableBorders,
    look: TableLook,
}

Row {
    id: NodeId,
    cells: Vec<Cell>,
    height: Option<RowHeight>,
    cant_split: bool,
    header: bool,
}

Cell {
    id: NodeId,
    grid_span: u32,
    v_merge: VMerge,
    width: Option<CellWidth>,
    margins: CellMargins,
    v_align: VerticalAlign,
    borders: CellBorders,
    items: Vec<BlockItem>,
}

Section {
    id: NodeId,
    properties: SectionProperties,
    header_default: Option<PartRef>,
    header_first: Option<PartRef>,
    header_even: Option<PartRef>,
    footer_default: Option<PartRef>,
    footer_first: Option<PartRef>,
    footer_even: Option<PartRef>,
    title_pg: bool,
    page_size: PageSize,
    orientation: Orientation,
    margins: Margins,
    columns: Columns,
}

StyleTable {
    doc_defaults: DocDefaults,
    paragraph: HashMap<StyleId, ParagraphStyle>,
    character: HashMap<StyleId, CharacterStyle>,
    table: HashMap<StyleId, TableStyle>,
    numbering: HashMap<StyleId, NumberingStyle>,
    defaults: DefaultStyleIds,
}

NumberingTable {
    abstract: HashMap<AbstractNumId, AbstractNum>,
    nums: HashMap<NumId, Num>,
}

Settings {
    default_tab_stop: Option<Twips>,
    even_and_odd_headers: bool,
    footnote_pr: FootnotePr,
    endnote_pr: EndnotePr,
    compat: CompatSettings,
    character_spacing_control: CharacterSpacingControl,
}
```

### 3.2. Сырые свойства

```rust
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RawPPr {
    pub style: Option<StyleId>,
    pub num_pr: Option<NumPr>,
    pub spacing: Option<Spacing>,
    pub ind: Option<Ind>,
    pub jc: Option<Justification>,
    pub keep_next: Option<Toggle>,
    pub keep_lines: Option<Toggle>,
    pub page_break_before: Option<Toggle>,
    pub widow_control: Option<Toggle>,
    pub outline_lvl: Option<u8>,
    pub p_bdr: Option<ParagraphBorders>,
    pub shd: Option<Shading>,
    pub tabs: Vec<TabStop>,
    pub r_pr: Option<RawRPr>,
    pub sect_pr: Option<SectionProperties>,
    pub unknown: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RawRPr {
    pub style: Option<StyleId>,
    pub r_fonts: Option<RFonts>,
    pub b: Option<Toggle>,
    pub i: Option<Toggle>,
    pub caps: Option<Toggle>,
    pub small_caps: Option<Toggle>,
    pub strike: Option<Toggle>,
    pub dstrike: Option<Toggle>,
    pub vanish: Option<Toggle>,
    pub outline: Option<Toggle>,
    pub shadow: Option<Toggle>,
    pub emboss: Option<Toggle>,
    pub imprint: Option<Toggle>,
    pub color: Option<Color>,
    pub sz: Option<HalfPoint>,
    pub sz_cs: Option<HalfPoint>,
    pub highlight: Option<Highlight>,
    pub u: Option<Underline>,
    pub vert_align: Option<VerticalAlign>,
    pub spacing: Option<Spacing>,
    pub position: Option<HalfPoint>,
    pub unknown: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Toggle {
    On,
    Off,
    Inherit,
}
```

### 3.3. NodeId

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct NodeId(u64);

pub struct NodeIdAllocator { next: u64 }

impl NodeIdAllocator {
    pub fn new() -> Self { Self { next: 1 } }
    pub fn alloc(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }
}
```

Свойства:

- уникален в пределах одного открытия документа;
- детерминирован (один вход → одни ID);
- не переживает переоткрытие;
- не выставляется в публичный API;
- сериализуется в JSON для snapshot-тестов.

Обратный индекс `HashMap<NodeId, NodeId>` (parent) строится после парсинга, если нужен.

### 3.4. Сериализация

`Document` реализует `serde::Serialize` и `Deserialize`. Round-trip `Model → JSON → Model` lossless. Property-тест на 100 фикстурах. `Model → DOCX` не реализуется (ADR-0020).

## 4. Задачи по дням

### День 1

**R1: Fuzz-инфраструктура.** `crates/fuzz/`, цели: `core_zip`, `core_xml_reader`, `core_rels`, `docx_document`, `docx_styles`, `docx_numbering`, `docx_rels`, `xlsx_worksheet`. Corpus: 100 фикстур + 20 реальных документов. `.github/workflows/fuzz.yml`: ночной прогон 15 мин на цель. Проверки: падение, OOM, зависание > 30 с, превышение лимитов ZIP.

**R2: Модель — структуры.** `crates/docx/src/model/`, все структуры §3, `NodeId`, `NodeIdAllocator`, `Toggle`, `BreakKind`, `WrapKind`, `VMerge`, `TableLayout`, `TableLook`, `RawPPr`, `RawRPr`, `#[derive(Serialize, Deserialize)]`.

### День 2

**R1: ZIP-лимиты (ADR-0015).** `OoxmlArchive::open_with_limits`, `part`, `rels_for`. Path traversal, ratio check, per-part 64 МиБ, per-archive 256 МиБ, symlink игнорируется. Тесты: zip bomb, zip slip, битый central directory, превышение ratio.

**R2: Модель — валидация.** `StyleTable::validate`, `NumberingTable::validate`, циклический `basedOn` → `ParseWarning::CyclicBasedOn`, отсутствующий `basedOn` → `ParseWarning::MissingStyleRef`, отсутствующий `abstractNum` → `ParseWarning::MissingAbstractNum`.

### День 3

**R1: XML-хелперы.** `XmlReader` namespace-aware, BOM, DTD, `mc:AlternateContent` (ADR-0014), игнорируемые `w:rsid*`, `w:proofErr`, `w:lastRenderedPageBreak`, `bookmarkStart`/`End` → `Bookmark`.

**R2: `document.rs` — часть 1.** `w:body`, `w:p`, `w:pPr`, `w:rPr`, `w:r`, `w:t`, `w:br`, `w:cr`, `w:tab`, `w:sym`, `w:drawing`, `w:wp:inline`, `w:wp:anchor`.

### День 4

**R1: `document.rs` — часть 2.** `w:hyperlink`, `w:bookmarkStart/End`, `w:fldSimple`, `w:instrText`, `w:fldChar`, `w:ins`, `w:del` (Unknown), `w:sectPr`, `w:tbl`, `w:tr`, `w:tc`, `w:tblGrid`.

**R2: `styles.rs`.** `w:styles`, `w:docDefaults`, `w:style`, `w:basedOn`, `w:next`, `w:link`, `w:default`, `w:pPr`, `w:rPr`. Защита от циклического `basedOn` (глубина ≤ 32).

### День 5

**R1: `numbering.rs`.** `w:abstractNum`, `w:num`, `w:lvl`, `w:lvlOverride`, `w:startOverride`, `w:lvlText`, `w:numFmt`, `w:suff`, `w:picBullet`, `w:lvlRestart`, `w:multiLevelType`.

**R2: `settings.rs`, `rels.rs`.** `w:defaultTabStop`, `w:evenAndOddHeaders`, `w:footnotePr`, `w:endnotePr`, `w:compat`, `w:characterSpacingControl`, rels для `document.xml`, `header*.xml`, `footer*.xml`, `footnotes.xml`, `comments.xml`.

### День 6

**R1: `footnotes.rs`, `comments.rs`.** `footnotes.xml` → `Vec<Footnote>`, `comments.xml` → `Vec<Comment>`, каждая сущность получает `NodeId`.

**R2: Normalize toggle.** `Toggle::On`/`Off`/`Inherit`, тесты на `w:val="0"`, `w:val="false"`, отсутствие `w:val`, обработка `w:b` внутри `w:pPr/w:rPr`.

### День 7

**R1: Интеграция — `parse_docx`.** Открытие ZIP с лимитами, проверка `[Content_Types].xml`, чтение `document.xml`, `styles.xml`, `numbering.xml`, `settings.xml`, рекурсивный обход с `NodeIdAllocator`, валидация ссылок, возврат `Document` с warnings.

**R2: `mc:AlternateContent`.** `resolve_alternate_content()` — выбор первой поддерживаемой `mc:Choice`, если ни одна не поддержана — `Unknown`, `mc:Fallback` игнорируется. Тесты на 5 фикстурах.

### День 8

**R1: Property-тесты.** Round-trip `Model → JSON → Model` lossless, `proptest` на `NodeId` уникальность, toggle round-trip, `style_ref` резолвится после `validate()`, порядок обхода детерминирован.

**R2: Дифференциальный тест vs mammoth.** `scripts/diff-mammoth.ts`: прогон 50 фикстур через mammoth.js, сравнение плоского текста, структуры абзацев, списков, таблиц, гиперссылок. Расхождения классифицируются. Порог ≥ 95%. Отчёт `docs/sprint-8/diff-report.md`.

### День 9

**R1: Бюджеты памяти.** Тесты `test_paragraph_stays_small` (≤ 256 байт), `test_run_stays_small` (≤ 128), `test_cell_stays_small` (≤ 192). Замер на 100 фикстурах. Отчёт `docs/sprint-8/memory-report.md`.

**R2: Fuzz — прогон и отчёт.** Локальный прогон 1 час на каждую цель, исправление находок, issue upstream при находке в `quick-xml`. Отчёт `docs/sprint-8/fuzz-report.md`.

### День 10

**R1: Покрытие и CI.** `cargo-llvm-cov` в CI, порог ≥ 85% для `crates/docx`, ≥ 80% для нового кода `crates/core`, gate в CI. Отчёт `docs/sprint-8/coverage-report.md`.

**R2: Документация и отчёт.** `docs/sprint-8/report.md`, обновление статусов ADR, ретро.

## 5. Бюджеты

| Метрика | Бюджет | Как измеряется |
|---|---|---|
| `openDocx` 50 МБ (native) | < 1,5 с | `criterion` |
| `openDocx` 50 МБ (wasm) | < 4 с | `criterion` |
| Память на абзац | ≤ 256 байт | `test_paragraph_stays_small` |
| Память на run | ≤ 128 байт | `test_run_stays_small` |
| Память на ячейку таблицы | ≤ 192 байт | `test_cell_stays_small` |
| Пиковая память на 50 МБ DOCX с изображениями | ≤ 200 МиБ | замер + `performance.memory` |
| Покрытие `crates/docx` | ≥ 85% | `cargo-llvm-cov` |
| Покрытие нового кода `crates/core` | ≥ 80% | `cargo-llvm-cov` |
| Fuzz без падений | 24 ч на цель | CI-артефакт |
| Differential vs mammoth | ≥ 95% на 50 фикстурах | `scripts/diff-mammoth.ts` |
| ZIP-лимиты | все тесты зелёные | unit |

## 6. DoD

- 100/100 фикстур парсятся без фатальных ошибок.
- 50 из них — differential vs mammoth ≥ 95%, расхождения задокументированы.
- 50 МБ DOCX < 1,5 с native на фиксированном профиле.
- Покрытие ≥ 85% для `crates/docx`, ≥ 80% для нового кода `crates/core`, gate в CI.
- Fuzz 24 ч без падений, OOM, зависаний > 30 с.
- ZIP-лимиты из ADR-0015 покрыты тестами.
- Политика ошибок из ADR-0016 покрыта тестами.
- ADR 0013–0020 приняты и лежат в `docs/adr/`.
- `cargo-deny` + `cargo-audit` — чисто.
- Модель расширена: `Break`, `Tab`, `Symbol`, `Field`, `Anchor`, `Unknown`, `TableLook`, `Section`, `RawPPr`, `RawRPr`.
- `Inline` — enum, не «run с флагами».
- `Toggle` — тринстейт, покрыт тестами.
- `NodeId` детерминирован, не выставляется в публичный API.
- Round-trip `Model → JSON → Model` lossless на 100 фикстурах.
- Writer в DOCX не реализуется (ADR-0020).
- Non-goals зафиксированы в README.

## 7. Non-goals

- Strict OOXML — только Transitional (ADR-0017).
- Track changes — `w:ins`/`w:del` парсятся как `Unknown` с сохранением XML.
- VML — только `mc:Choice`, `mc:Fallback` игнорируется.
- TOC — поле парсится, содержимое не генерируется.
- Уравнения OMML, SmartArt, embedded OLE — `Unknown`.
- Encrypted DOCX — ошибка `EncryptedDocument`.
- `.docm` — ошибка `MacroEnabledDocument`.
- `.doc` (бинарный) — ошибка `LegacyFormat`.
- Writer в DOCX — ADR-0020.
- Command API, undo/redo, diff/patch — ADR-0018.
- Резолвинг каскада стилей — Спринт 9.
- Layout, pagination — Спринт 9.

## 8. Риски

| Риск | Вероятность | Влияние | Митигация |
|---|---|---|---|
| Модель недостаточна для Спринта 9 | Высокая | Высокое | ADR-0013, `RawPPr`/`RawRPr` |
| `numbering.xml` сложнее ожидаемого | Высокая | Среднее | 5 фикстур, фокус R2 в день 5 |
| Fuzz найдёт падения в `quick-xml` | Средняя | Низкое | upstream fix или workaround |
| Differential < 95% | Средняя | Среднее | классификация расхождений |
| ZIP-лимиты слишком строгие | Низкая | Низкое | конфигурируемы |
| Не хватит 2 недель | Средняя | Высокое | резерв 2 дня; вынос `footnotes`/`comments` в Спринт 9 |
| `mc:AlternateContent` — неожиданные ветки | Средняя | Среднее | 5 фикстур, политика `Unknown` |

## 9. Артефакты

### Код

- `crates/fuzz/`, `.github/workflows/fuzz.yml`.
- `crates/docx/src/model/`.
- `crates/docx/src/{document,styles,numbering,settings,rels,footnotes,comments}.rs`.
- `crates/core/src/zip_limits.rs`.
- `crates/core/src/error_policy.rs`.
- `crates/core/src/node_id.rs`.
- `scripts/diff-mammoth.ts`.

### Документация

- ADR 0013–0020.
- `docs/sprint-8/{report,diff-report,fuzz-report,memory-report,coverage-report,bench-profile}.md`.

### Фикстуры

- `test-fixtures/docx/` — 75+ новых.

## 10. Definition of Ready для Спринта 9

- Модель несёт `RawPPr`/`RawRPr` для каждого абзаца и run.
- `StyleTable` полный: `docDefaults`, paragraph, character, table, numbering.
- `NumberingTable` полный: `abstractNum`, `num`, `lvlOverride`.
- `Section` с колонтитулами и page setup.
- `Anchor` для floats.
- Все ссылки резолвятся, циклические — warning.
- Differential vs mammoth ≥ 95%.
- Покрытие ≥ 85% для `crates/docx`.
- Fuzz 24 ч без падений.
- ADR 0013–0020 в `docs/adr/`.
- `NodeId` работает, snapshot-тесты используют его.
- Round-trip `Model → JSON → Model` lossless.

## 11. Коммуникация

- Дейли: 15 минут, R1 и R2 синхронизируются.
- Ревью ADR: до старта спринта, на неделе 15.
- Ревью модели: день 2, до начала парсеров. QA участвует.
- Демо: день 10, fuzz-находки и differential-отчёт.
- Ретро: день 10.

## 12. Итог

Отличия от исходной версии:

1. Модель расширена: `Break`, `Tab`, `Symbol`, `Field`, `Anchor`, `Unknown`, `TableLook`, `Section`, `RawPPr`, `RawRPr`, `NodeId`.
2. Добавлена дифференциальная проверка vs mammoth — DoD стал проверяемым.
3. Зафиксированы ADR 0013–0020.

Стоимость: +2 дня резерва, +1 день на differential. Выгода: Спринт 9 не потребует переписывания модели, Спринт 10 не покажет расхождение с Word.
