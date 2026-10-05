// Генератор фикстур DOCX (упрощённая версия)
// DOCX - это ZIP-архив с XML-файлами.
// Используем archiver для создания ZIP.
//
// Запуск: npx tsx scripts/generate_docx_fixtures.ts

import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { createWriteStream } from 'node:fs';

const ROOT = path.resolve(import.meta.dirname, '..');
const OUT_DIR = path.join(ROOT, 'test-fixtures', 'docx');

// Экранирование XML
const escapeXml = (text: string): string => {
    return text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&apos;');
};

// Создание ZIP-архива с использованием Node.js Child Process (zip команда)
const createZip = async (outputPath: string, files: Record<string, string>): Promise<void> => {
    const { exec } = await import('node:child_process');
    const { promisify } = await import('node:util');
    const execAsync = promisify(exec);
    
    // Создаем временную директорию
    const tempDir = path.join(ROOT, 'temp_docx_' + Date.now());
    await mkdir(tempDir, { recursive: true });
    
    // Пишем файлы
    for (const [filePath, content] of Object.entries(files)) {
        const fullPath = path.join(tempDir, filePath);
        await mkdir(path.dirname(fullPath), { recursive: true });
        await writeFile(fullPath, content);
    }
    
    // Архивируем
    const zipPath = path.resolve(outputPath);
    await execAsync(`cd "${tempDir}" && zip -r "${zipPath}" *`);
    
    // Удаляем временную директорию
    const { rm } = await import('node:fs/promises');
    await rm(tempDir, { recursive: true, force: true });
};

// Проверяем, есть ли команда zip
const checkZipCommand = async (): Promise<boolean> => {
    const { exec } = await import('node:child_process');
    const { promisify } = await import('node:util');
    const execAsync = promisify(exec);
    try {
        await execAsync('zip -v');
        return true;
    } catch {
        return false;
    }
};

// Базовая структура DOCX
const baseDocxFiles = (): Record<string, string> => ({
    '[Content_Types].xml': `<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>`,

    '_rels/.rels': `<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>`,

    'word/_rels/document.xml.rels': `<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>`,

    'word/styles.xml': `<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/></w:rPr></w:rPrDefault>
    <w:pPrDefault><w:pPr><w:spacing w:before="120" w:after="120"/></w:pPr></w:pPrDefault>
  </w:docDefaults>
</w:styles>`,
});

// Создание document.xml
const createDocumentXml = (content: string): string => {
    return `<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
${content}
  </w:body>
</w:document>`;
};

// Генерация параграфа
const p = (text: string, bold = false, italic = false): string => {
    const props = [];
    if (bold) props.push('<w:b/>');
    if (italic) props.push('<w:i/>');
    const rPr = props.length > 0 ? `<w:rPr>${props.join('')}</w:rPr>` : '';
    return `<w:p><w:r>${rPr}<w:t>${escapeXml(text)}</w:t></w:r></w:p>`;
};

// Генерация заголовка
const h = (text: string, level: 1 | 2 | 3 = 1): string => {
    return `<w:p><w:pPr><w:pStyle w:val="Heading${level}"/></w:pPr><w:r><w:t>${escapeXml(text)}</w:t></w:r></w:p>`;
};

// Генерация таблицы
const table = (rows: string[][]): string => {
    const trs = rows.map(row => {
        const tcs = row.map(cell => `<w:tc><w:p><w:r><w:t>${escapeXml(cell)}</w:t></w:r></w:p></w:tc>`).join('');
        return `<w:tr>${tcs}</w:tr>`;
    }).join('');
    return `<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>${trs}</w:tbl>`;
};

// Сохранение фикстуры
const saveFixture = async (name: string, docContent: string, metadata: any, contentData: any): Promise<void> => {
    const files = { ...baseDocxFiles(), 'word/document.xml': createDocumentXml(docContent) };
    await createZip(path.join(OUT_DIR, `${name}.docx`), files);
    await writeFile(path.join(OUT_DIR, `${name}.json`), JSON.stringify({ metadata, content: contentData }, null, 2));
    console.log(`  ✅ ${name}`);
};

// Генерация всех фикстур
async function main() {
    const hasZip = await checkZipCommand();
    if (!hasZip) {
        console.error('❌ Error: zip command not found. Please install zip utility.');
        console.log('   On macOS: brew install zip');
        console.log('   On Ubuntu: sudo apt-get install zip');
        process.exit(1);
    }
    
    console.log('🚀 Generating DOCX fixtures...');
    
    // Создаём директории
    const dirs = ['simple', 'formatting', 'tables', 'complex', 'edge_cases'];
    for (const dir of dirs) {
        await mkdir(path.join(OUT_DIR, dir), { recursive: true });
    }
    
    // ==================== SIMPLE ====================
    console.log('\n📝 Simple fixtures:');
    
    await saveFixture('simple/empty', '', {
        name: 'simple/empty',
        description: 'Пустой документ',
        expectedParagraphs: 0,
        expectedTables: 0
    }, { paragraphs: [], tables: [], images: [] });
    
    await saveFixture('simple/one_paragraph', p('Hello, World!'), {
        name: 'simple/one_paragraph',
        description: 'Один абзац',
        expectedParagraphs: 1,
        expectedTables: 0
    }, { paragraphs: [{ text: 'Hello, World!' }], tables: [], images: [] });
    
    for (let i = 0; i < 3; i++) {
        const paragraphs = Array.from({ length: 5 }, (_, j) => p(`Paragraph ${j + 1}`));
        await saveFixture(`simple/multiple_paragraphs_${i}`, paragraphs.join(''), {
            name: `simple/multiple_paragraphs_${i}`,
            description: '5 абзацев',
            expectedParagraphs: 5,
            expectedTables: 0
        }, { paragraphs: Array.from({ length: 5 }, (_, j) => ({ text: `Paragraph ${j + 1}` })), tables: [], images: [] });
    }
    
    // ==================== FORMATTING ====================
    console.log('\n🎨 Formatting fixtures:');
    
    await saveFixture('formatting/bold', p('Bold text', true), {
        name: 'formatting/bold',
        description: 'Жирный текст',
        expectedParagraphs: 1
    }, { paragraphs: [{ text: 'Bold text', bold: true }], tables: [] });
    
    await saveFixture('formatting/italic', p('Italic text', false, true), {
        name: 'formatting/italic',
        description: 'Курсив',
        expectedParagraphs: 1
    }, { paragraphs: [{ text: 'Italic text', italic: true }], tables: [] });
    
    for (let level = 1; level <= 3; level++) {
        await saveFixture(`formatting/heading_${level}`, h(`Heading ${level}`, level as 1 | 2 | 3), {
            name: `formatting/heading_${level}`,
            description: `Заголовок уровня ${level}`,
            expectedParagraphs: 1
        }, { paragraphs: [{ text: `Heading ${level}`, style: `Heading${level}` }], tables: [] });
    }
    
    // ==================== TABLES ====================
    console.log('\n📊 Table fixtures:');
    
    await saveFixture('tables/simple_2x2', table([['A1', 'A2'], ['B1', 'B2']]), {
        name: 'tables/simple_2x2',
        description: 'Таблица 2x2',
        expectedTables: 1
    }, { tables: [{ rows: 2, cols: 2, cells: [['A1', 'A2'], ['B1', 'B2']] }], paragraphs: [] });
    
    await saveFixture('tables/simple_3x3', table([['A1', 'A2', 'A3'], ['B1', 'B2', 'B3'], ['C1', 'C2', 'C3']]), {
        name: 'tables/simple_3x3',
        description: 'Таблица 3x3',
        expectedTables: 1
    }, { tables: [{ rows: 3, cols: 3, cells: [['A1', 'A2', 'A3'], ['B1', 'B2', 'B3'], ['C1', 'C2', 'C3']] }], paragraphs: [] });
    
    // ==================== COMPLEX ====================
    console.log('\n📜 Complex fixtures:');
    
    await saveFixture('complex/text_and_table', p('Text before') + table([['H1', 'H2'], ['C1', 'C2']]) + p('Text after'), {
        name: 'complex/text_and_table',
        description: 'Текст + таблица + текст',
        expectedParagraphs: 2,
        expectedTables: 1
    }, {
        paragraphs: [{ text: 'Text before' }, { text: 'Text after' }],
        tables: [{ rows: 2, cols: 2, cells: [['H1', 'H2'], ['C1', 'C2']] }]
    });
    
    // ==================== EDGE CASES ====================
    console.log('\n⚠️  Edge case fixtures:');
    
    await saveFixture('edge_cases/empty_paragraphs', p('') + p('Non-empty') + p(''), {
        name: 'edge_cases/empty_paragraphs',
        description: 'Пустые абзацы',
        expectedParagraphs: 3
    }, { paragraphs: ['', 'Non-empty', ''], tables: [] });
    
    const specialText = '<>&"\'Test&\'"<>';
    await saveFixture('edge_cases/special_chars', p(specialText), {
        name: 'edge_cases/special_chars',
        description: 'Специальные символы',
        expectedParagraphs: 1
    }, { paragraphs: [{ text: specialText }], tables: [] });
    
    await saveFixture('edge_cases/multilingual', p('English') + p('Русский') + p('中文'), {
        name: 'edge_cases/multilingual',
        description: 'Разные языки',
        expectedParagraphs: 3
    }, { paragraphs: [{ text: 'English' }, { text: 'Русский' }, { text: '中文' }], tables: [] });
    
    console.log('\n✅ All DOCX fixtures generated!');
    console.log(`📂 Saved to: ${OUT_DIR}`);
}

main().catch(console.error);
