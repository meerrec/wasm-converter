// Генератор фикстур DOCX вручную (без внешних библиотек).
// DOCX - это ZIP-архив с XML-файлами.
//
// Запуск:
//   npx tsx scripts/generate_docx_fixtures_manual.ts
//
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import path from 'node:path';
import { create } from 'archiver';
import { createWriteStream } from 'node:fs';

const ROOT = path.resolve(import.meta.dirname, '..');
const OUT_DIR = path.join(ROOT, 'test-fixtures', 'docx');

// Утилиты
const randomText = (length: number = 50): string => {
    const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz 0123456789.!?,;:\n';
    return Array.from({ length }, () => chars.charAt(Math.floor(Math.random() * chars.length))).join('');
};

const randomWord = (): string => {
    const words = ['Lorem', 'ipsum', 'dolor', 'sit', 'amet', 'consectetur', 'adipiscing', 'elit'];
    return words[Math.floor(Math.random() * words.length)];
};

const randomSentence = (wordsCount: number = 10): string => {
    return Array.from({ length: wordsCount }, () => randomWord()).join(' ') + '.';
};

// Экранирование XML
const escapeXml = (text: string): string => {
    return text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&apos;');
};

// Типы для эталонных данных
interface FixtureMetadata {
    name: string;
    description: string;
    expectedPages?: number;
    expectedParagraphs: number;
    expectedTables: number;
    expectedImages: number;
    hasHeaders: boolean;
    hasFooters: boolean;
    hasFootnotes: boolean;
    hasComments: boolean;
}

interface FixtureContent {
    paragraphs: Array<{
        text: string;
        style?: string;
        bold?: boolean;
        italic?: boolean;
        underline?: boolean;
    }>;
    tables: Array<{
        rows: number;
        cols: number;
        cells: string[][];
    }>;
    images: Array<{ altText: string }>;
}

interface FixtureData {
    metadata: FixtureMetadata;
    content: FixtureContent;
}

// Функция для создания ZIP-архива с DOCX-файлами
const createDocxZip = async (outputPath: string, files: Record<string, string>): Promise<void> => {
    return new Promise((resolve, reject) => {
        const output = createWriteStream(outputPath);
        const archive = create('zip', {});
        
        output.on('close', () => resolve());
        archive.on('error', (err) => reject(err));
        
        archive.pipe(output);
        
        for (const [filePath, content] of Object.entries(files)) {
            archive.append(content, { name: filePath });
        }
        
        archive.finalize();
    });
};

// Функция для создания базовой структуры DOCX
const createDocxStructure = (): Record<string, string> => {
    return {
        // [Content_Types].xml
        '[Content_Types].xml': `<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="docx" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
  <Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
  <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-package.extended-properties+xml"/>
</Types>`,

        // _rels/.rels
        '_rels/.rels': `<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>`,

        // docProps/core.xml
        'docProps/core.xml': `<?xml version="1.0" encoding="UTF-8"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" 
                    xmlns:dc="http://purl.org/dc/elements/1.1/" 
                    xmlns:dcterms="http://purl.org/dc/terms/" 
                    xmlns:dcmitype="http://purl.org/dc/dcmitype/" 
                    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:creator>Test User</dc:creator>
  <cp:lastModifiedBy>Test User</cp:lastModifiedBy>
  <dcterms:created xsi:type="dcterms:W3CDTF">2026-10-05T00:00:00Z</dcterms:created>
  <dcterms:modified xsi:type="dcterms:W3CDTF">2026-10-05T00:00:00Z</dcterms:modified>
</cp:coreProperties>`,

        // docProps/app.xml
        'docProps/app.xml': `<?xml version="1.0" encoding="UTF-8"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" 
           xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">
  <Application>Microsoft Word</Application>
  <DocSecurity>0</DocSecurity>
  <ScaleCrop>true</ScaleCrop>
  <HeadingPairs>
    <vt:vector size="2" baseType="variant">
      <vt:variant>
        <vt:lpstr>Title</vt:lpstr>
      </vt:variant>
      <vt:variant>
        <vt:i4>1</vt:i4>
      </vt:variant>
    </vt:vector>
  </HeadingPairs>
  <TitlesOfParts>
    <vt:vector size="1" baseType="lpstr">
      <vt:lpstr>Title</vt:lpstr>
    </vt:vector>
  </TitlesOfParts>
  <LinksUpToDate>false</LinksUpToDate>
  <SharedDoc>false</SharedDoc>
  <HyperlinksChanged>false</HyperlinksChanged>
  <AppVersion>16.0000</AppVersion>
</Properties>`,

        // word/_rels/document.xml.rels
        'word/_rels/document.xml.rels': `<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>
</Relationships>`,

        // word/styles.xml
        'word/styles.xml': `<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault>
      <w:rPr>
        <w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:cs="Times New Roman"/>
        <w:sz w:val="22"/>
        <w:szCs w:val="24"/>
      </w:rPr>
    </w:rPrDefault>
    <w:pPrDefault>
      <w:pPr>
        <w:spacing w:before="120" w:after="120"/>
      </w:pPr>
    </w:pPrDefault>
  </w:docDefaults>
</w:styles>`,

        // word/numbering.xml
        'word/numbering.xml': `<?xml version="1.0" encoding="UTF-8"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
</w:numbering>`,

        // word/settings.xml
        'word/settings.xml': `<?xml version="1.0" encoding="UTF-8"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:zoom w:percent="100"/>
  <w:compat>
    <w:compatSetting w:name="compatMode" w:uri="http://schemas.microsoft.com/office/word" w:val="16"/>
  </w:compat>
</w:settings>`,
    };
};

// Функция для добавления документа
const addDocument = (files: Record<string, string>, content: string): Record<string, string> => {
    return {
        ...files,
        'word/document.xml': `<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" 
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    ${content}
  </w:body>
</w:document>`,
    };
};

// Функция для создания параграфа
const createParagraph = (text: string, bold: boolean = false, italic: boolean = false, underline: boolean = false): string => {
    let props = '';
    if (bold || italic || underline) {
        let rPr = '';
        if (bold) rPr += '<w:b/>';
        if (italic) rPr += '<w:i/>';
        if (underline) rPr += '<w:u w:val="single"/>';
        if (rPr) {
            props = `<w:rPr>${rPr}</w:rPr>`;
        }
    }
    
    return `<w:p>
  <w:r>
    ${props}
    <w:t>${escapeXml(text)}</w:t>
  </w:r>
</w:p>`;
};

// Функция для создания заголовка
const createHeading = (text: string, level: number = 1): string => {
    const styleId = `Heading${level}`;
    return `<w:p>
  <w:pPr>
    <w:pStyle w:val="${styleId}"/>
  </w:pPr>
  <w:r>
    <w:t>${escapeXml(text)}</w:t>
  </w:r>
</w:p>`;
};

// Функция для создания таблицы
const createTable = (rows: string[][]): string => {
    let tableXml = '<w:tbl>
  <w:tblPr>
    <w:tblW w:w="0" w:type="auto"/>
    <w:tblBorders/>
  </w:tblPr>';
    
    for (const row of rows) {
        tableXml += '<w:tr>';
        for (const cell of row) {
            tableXml += `<w:tc>
    <w:p>
      <w:r>
        <w:t>${escapeXml(cell)}</w:t>
      </w:r>
    </w:p>
  </w:tc>`;
        }
        tableXml += '</w:tr>';
    }
    
    tableXml += '</w:tbl>';
    return tableXml;
};

// Функция для сохранения фикстуры
const saveFixture = async (name: string, files: Record<string, string>, metadata: FixtureMetadata, content: FixtureContent): Promise<void> => {
    const filepath = path.join(OUT_DIR, name);
    
    // Сохраняем DOCX
    await createDocxZip(`${filepath}.docx`, files);
    
    // Сохраняем JSON-эталон
    const data: FixtureData = { metadata, content };
    await writeFile(`${filepath}.json`, JSON.stringify(data, null, 2));
    
    console.log(`✅ Created: ${name}.docx + ${name}.json`);
};

// ── Генерация фикстур ──

// Простые документы
const generateSimpleFixtures = async () => {
    console.log('\n📝 Generating simple fixtures...');
    
    // 1. Пустой документ
    {
        const base = createDocxStructure();
        const files = addDocument(base, '');
        const metadata: FixtureMetadata = {
            name: 'simple/empty',
            description: 'Пустой документ',
            expectedParagraphs: 0,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('simple/empty', files, metadata, {
            paragraphs: [],
            tables: [],
            images: [],
        });
    }
    
    // 2. Один абзац
    {
        const base = createDocxStructure();
        const content = createParagraph('Hello, World!');
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'simple/one_paragraph',
            description: 'Документ с одним абзацем',
            expectedParagraphs: 1,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('simple/one_paragraph', files, metadata, {
            paragraphs: [{ text: 'Hello, World!' }],
            tables: [],
            images: [],
        });
    }
    
    // 3. Несколько абзацев
    for (let i = 0; i < 5; i++) {
        const base = createDocxStructure();
        const paragraphs = [];
        let content = '';
        for (let j = 0; j < 5; j++) {
            const text = randomSentence(10);
            content += createParagraph(text);
            paragraphs.push({ text });
        }
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: `simple/multiple_paragraphs_${i}`,
            description: `Документ с 5 абзацами (вариант ${i})`,
            expectedParagraphs: 5,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture(`simple/multiple_paragraphs_${i}`, files, metadata, {
            paragraphs,
            tables: [],
            images: [],
        });
    }
};

// Абзацы с форматированием
const generateFormattingFixtures = async () => {
    console.log('\n🎨 Generating formatting fixtures...');
    
    // 1. Жирный текст
    {
        const base = createDocxStructure();
        const content = `<w:p>
  <w:r>
    <w:rPr>
      <w:b/>
    </w:rPr>
    <w:t>Bold text</w:t>
  </w:r>
  <w:r>
    <w:t> normal text</w:t>
  </w:r>
</w:p>`;
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'formatting/bold_text',
            description: 'Абзац с жирным текстом',
            expectedParagraphs: 1,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('formatting/bold_text', files, metadata, {
            paragraphs: [{ text: 'Bold text normal text', bold: true }],
            tables: [],
            images: [],
        });
    }
    
    // 2. Курсив
    {
        const base = createDocxStructure();
        const content = `<w:p>
  <w:r>
    <w:rPr>
      <w:i/>
    </w:rPr>
    <w:t>Italic text</w:t>
  </w:r>
  <w:r>
    <w:t> normal text</w:t>
  </w:r>
</w:p>`;
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'formatting/italic_text',
            description: 'Абзац с курсивом',
            expectedParagraphs: 1,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('formatting/italic_text', files, metadata, {
            paragraphs: [{ text: 'Italic text normal text', italic: true }],
            tables: [],
            images: [],
        });
    }
    
    // 3. Подчёркивание
    {
        const base = createDocxStructure();
        const content = `<w:p>
  <w:r>
    <w:rPr>
      <w:u w:val="single"/>
    </w:rPr>
    <w:t>Underlined text</w:t>
  </w:r>
  <w:r>
    <w:t> normal text</w:t>
  </w:r>
</w:p>`;
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'formatting/underline_text',
            description: 'Абзац с подчёркиванием',
            expectedParagraphs: 1,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('formatting/underline_text', files, metadata, {
            paragraphs: [{ text: 'Underlined text normal text', underline: true }],
            tables: [],
            images: [],
        });
    }
};

// Заголовки
const generateHeadingFixtures = async () => {
    console.log('\n📚 Generating heading fixtures...');
    
    for (let level = 1; level <= 3; level++) {
        const base = createDocxStructure();
        const content = createHeading(`Heading Level ${level}`, level) + createParagraph(randomSentence(10));
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: `formatting/heading_level_${level}`,
            description: `Документ с заголовком уровня ${level}`,
            expectedParagraphs: 2,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture(`formatting/heading_level_${level}`, files, metadata, {
            paragraphs: [
                { text: `Heading Level ${level}`, style: `Heading${level}` },
                { text: randomSentence(10) },
            ],
            tables: [],
            images: [],
        });
    }
};

// Таблицы
const generateTableFixtures = async () => {
    console.log('\n📊 Generating table fixtures...');
    
    // 1. Простая таблица 2x2
    {
        const base = createDocxStructure();
        const content = createTable([['Header 1', 'Header 2'], ['Cell 1', 'Cell 2']]);
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'tables/simple_table',
            description: 'Простая таблица 2x2',
            expectedParagraphs: 0,
            expectedTables: 1,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('tables/simple_table', files, metadata, {
            paragraphs: [],
            tables: [{
                rows: 2,
                cols: 2,
                cells: [['Header 1', 'Header 2'], ['Cell 1', 'Cell 2']],
            }],
            images: [],
        });
    }
    
    // 2. Таблица 3x3
    {
        const base = createDocxStructure();
        const content = createTable([['A1', 'A2', 'A3'], ['B1', 'B2', 'B3'], ['C1', 'C2', 'C3']]);
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'tables/table_3x3',
            description: 'Таблица 3x3',
            expectedParagraphs: 0,
            expectedTables: 1,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('tables/table_3x3', files, metadata, {
            paragraphs: [],
            tables: [{
                rows: 3,
                cols: 3,
                cells: [['A1', 'A2', 'A3'], ['B1', 'B2', 'B3'], ['C1', 'C2', 'C3']],
            }],
            images: [],
        });
    }
    
    // 3. Большая таблица 5x5
    {
        const base = createDocxStructure();
        const rows = [];
        for (let i = 0; i < 5; i++) {
            const row = [];
            for (let j = 0; j < 5; j++) {
                row.push(`R${i}C${j}`);
            }
            rows.push(row);
        }
        const content = createTable(rows);
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'tables/table_5x5',
            description: 'Таблица 5x5',
            expectedParagraphs: 0,
            expectedTables: 1,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('tables/table_5x5', files, metadata, {
            paragraphs: [],
            tables: [{ rows: 5, cols: 5, cells: rows }],
            images: [],
        });
    }
    
    // 4. Таблица с текстом и параграфами
    {
        const base = createDocxStructure();
        const content = createParagraph('Text before table') + 
                       createTable([['A', 'B'], ['C', 'D']]) +
                       createParagraph('Text after table');
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'tables/table_with_text',
            description: 'Таблица с текстом до и после',
            expectedParagraphs: 2,
            expectedTables: 1,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('tables/table_with_text', files, metadata, {
            paragraphs: [
                { text: 'Text before table' },
                { text: 'Text after table' },
            ],
            tables: [{
                rows: 2,
                cols: 2,
                cells: [['A', 'B'], ['C', 'D']],
            }],
            images: [],
        });
    }
};

// Многостраничные документы
const generateComplexFixtures = async () => {
    console.log('\n📜 Generating complex fixtures...');
    
    for (let i = 1; i <= 3; i++) {
        const base = createDocxStructure();
        const paragraphs = [];
        let content = '';
        
        // Заголовок
        content += createHeading(`Document ${i}`, 1);
        paragraphs.push({ text: `Document ${i}`, style: 'Heading1' });
        
        // Абзацы
        for (let j = 0; j < i * 20; j++) {
            const text = randomSentence(15);
            content += createParagraph(text);
            paragraphs.push({ text });
        }
        
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: `complex/long_document_${i}`,
            description: `Документ с ${i * 20 + 1} абзацами`,
            expectedParagraphs: i * 20 + 1,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture(`complex/long_document_${i}`, files, metadata, {
            paragraphs,
            tables: [],
            images: [],
        });
    }
};

// Крайние случаи
const generateEdgeCaseFixtures = async () => {
    console.log('\n⚠️  Generating edge case fixtures...');
    
    // 1. Очень длинный абзац
    {
        const base = createDocxStructure();
        const longText = randomText(2000);
        const content = createParagraph(longText);
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'edge_cases/very_long_paragraph',
            description: 'Очень длинный абзац (2000 символов)',
            expectedParagraphs: 1,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('edge_cases/very_long_paragraph', files, metadata, {
            paragraphs: [{ text: longText.substring(0, 50) + '...' }],
            tables: [],
            images: [],
        });
    }
    
    // 2. Пустые абзацы
    {
        const base = createDocxStructure();
        const content = createParagraph('') + createParagraph('Non-empty') + createParagraph('');
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'edge_cases/empty_paragraphs',
            description: 'Документ с пустыми абзацами',
            expectedParagraphs: 3,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('edge_cases/empty_paragraphs', files, metadata, {
            paragraphs: ['', 'Non-empty', ''],
            tables: [],
            images: [],
        });
    }
    
    // 3. Специальные символы
    {
        const base = createDocxStructure();
        const specialText = '<>&"\'Hello World&'"<>';
        const content = createParagraph(specialText);
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'edge_cases/special_chars',
            description: 'Абзац со специальными символами',
            expectedParagraphs: 1,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('edge_cases/special_chars', files, metadata, {
            paragraphs: [{ text: specialText }],
            tables: [],
            images: [],
        });
    }
    
    // 4. разные языки
    {
        const base = createDocxStructure();
        const content = createParagraph('English text') + 
                       createParagraph('Русский текст') +
                       createParagraph('中文文字');
        const files = addDocument(base, content);
        const metadata: FixtureMetadata = {
            name: 'edge_cases/multilingual',
            description: 'Документ на разных языках',
            expectedParagraphs: 3,
            expectedTables: 0,
            expectedImages: 0,
            hasHeaders: false,
            hasFooters: false,
            hasFootnotes: false,
            hasComments: false,
        };
        await saveFixture('edge_cases/multilingual', files, metadata, {
            paragraphs: [
                { text: 'English text' },
                { text: 'Русский текст' },
                { text: '中文文字' },
            ],
            tables: [],
            images: [],
        });
    }
};

// ── Hauptfunktion ──
async function main() {
    console.log('🚀 Starting DOCX fixtures generation (manual)...');
    console.log(`Output directory: ${OUT_DIR}`);
    
    try {
        // Создаём директории
        const dirs = ['simple', 'complex', 'tables', 'images', 'formatting', 'edge_cases'];
        for (const dir of dirs) {
            await mkdir(path.join(OUT_DIR, dir), { recursive: true });
        }
        
        // Генерируем фикстуры
        await generateSimpleFixtures();
        await generateFormattingFixtures();
        await generateHeadingFixtures();
        await generateTableFixtures();
        await generateComplexFixtures();
        await generateEdgeCaseFixtures();
        
        console.log('\n✅ All fixtures generated successfully!');
        console.log(`📂 Fixtures saved to: ${OUT_DIR}`);
        
        // Подсчёт статистики
        const files = await readdirRecursive(OUT_DIR);
        const docxFiles = files.filter(f => f.endsWith('.docx'));
        const jsonFiles = files.filter(f => f.endsWith('.json'));
        console.log(`📊 Generated ${docxFiles.length} DOCX files and ${jsonFiles.length} JSON files`);
        
    } catch (error) {
        console.error('❌ Error generating fixtures:', error);
        process.exit(1);
    }
}

// Вспомогательная функция для рекурсивного чтения директорий
async function readdirRecursive(dir: string): Promise<string[]> {
    const entries = await readdir(dir, { withFileTypes: true });
    const files = await Promise.all(
        entries.map(async entry => {
            const fullPath = path.join(dir, entry.name);
            if (entry.isDirectory()) {
                return readdirRecursive(fullPath);
            } else {
                return [fullPath];
            }
        })
    );
    return files.flat();
}

// Запуск
main().catch(console.error);

// Polyfill для Node.js
import { readdir } from 'node:fs/promises';
import archiver from 'archiver';
