const { chromium } = require('../../examples/viewer/node_modules/@playwright/test');
const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const zlib = require('node:zlib');
const root = path.resolve(__dirname, '../..');
const output = path.resolve(process.argv[2] ?? '/tmp/office-excel');
const only = new Set((process.argv[3] ?? '').split(',').filter(Boolean));
const corpus = JSON.parse(require('node:fs').readFileSync(path.join(root, 'test-fixtures/strict-viewer/corpus.json'), 'utf8'));
const docs = corpus.documents.filter(d => d.kind === 'xlsx' && (!only.size || only.has(d.id)));
const sha = b => crypto.createHash('sha256').update(b).digest('hex');
const xmlDecode = s => s.replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
const viewerUrl = p => `https://view.officeapps.live.com/op/view.aspx?src=${encodeURIComponent(`https://raw.githubusercontent.com/meerrec/wasm-converter/main/${p}`)}`;
async function waitPrintPage(frame, page, expected) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const active = frame.locator(`div[class^="page_"][aria-label="Page ${expected}."][tabindex="0"]`);
    if (await active.count()) { await page.waitForTimeout(250); return; }
    await page.waitForTimeout(100);
  }
  const labels = await frame.locator('div[class^="page_"]').evaluateAll(elements => elements.map(element => ({ label: element.getAttribute('aria-label'), tabIndex: element.getAttribute('tabindex') })));
  const controls = await frame.locator('button, [role="button"]').evaluateAll(elements => elements.filter(element => {
    const rect = element.getBoundingClientRect(); return rect.width > 0 && rect.height > 0;
  }).map(element => ({ text: element.textContent?.trim(), label: element.getAttribute('aria-label'), title: element.getAttribute('title') })));
  const inputs = await frame.locator('input:visible').evaluateAll(elements => elements.map(element => ({ type: element.type, value: element.value, label: element.getAttribute('aria-label'), title: element.getAttribute('title'), id: element.id, name: element.name, placeholder: element.getAttribute('placeholder') })));
  throw new Error(`Print page did not reach ${expected}: ${JSON.stringify({ labels, controls, inputs })}`);
}
async function workbookSheets(file) {
  const bytes = await fs.readFile(file), central = bytes.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  const count = bytes.readUInt16LE(central + 10); let p = bytes.readUInt32LE(central + 16);
  for (let i = 0; i < count; i++) {
    const method = bytes.readUInt16LE(p + 10), compressed = bytes.readUInt32LE(p + 20), nl = bytes.readUInt16LE(p + 28), el = bytes.readUInt16LE(p + 30), cl = bytes.readUInt16LE(p + 32), local = bytes.readUInt32LE(p + 42), name = bytes.subarray(p + 46, p + 46 + nl).toString();
    if (name === 'xl/workbook.xml') {
      const start = local + 30 + bytes.readUInt16LE(local + 26) + bytes.readUInt16LE(local + 28), data = bytes.subarray(start, start + compressed), xml = (method === 8 ? zlib.inflateRawSync(data) : data).toString();
      return [...xml.matchAll(/<sheet\b[^>]*\bname="([^"]+)"[^>]*\bstate="([^"]+)"[^>]*\/?\s*>|<sheet\b[^>]*\bname="([^"]+)"[^>]*\/?\s*>/g)].map((m, index) => ({ index, name: xmlDecode(m[1] ?? m[3]), hidden: m[2] && m[2] !== 'visible' })).filter(s => !s.hidden);
    }
    p += 46 + nl + el + cl;
  }
  throw new Error('xl/workbook.xml not found');
}
async function captureStableNormal(frame, page) {
  const read = () => frame.evaluate(() => {
    const cs = [...document.querySelectorAll('canvas.ewr-sheettable')];
    const out = document.createElement('canvas');
    out.width = 1440; out.height = 682;
    const ctx = out.getContext('2d');
    ctx.fillStyle = 'white'; ctx.fillRect(0, 0, out.width, out.height);
    const rects = cs.map(c => c.getBoundingClientRect());
    const minX = Math.min(...rects.map(r => r.x)), minY = Math.min(...rects.map(r => r.y));
    cs.forEach(c => { const r = c.getBoundingClientRect(); ctx.drawImage(c, r.x - minX, r.y - minY); });
    return out.toDataURL('image/png');
  });
  const deadline = Date.now() + 15000;
  let previous = await read(), stable = 0;
  while (Date.now() < deadline) {
    await page.waitForTimeout(200);
    const current = await read();
    stable = current === previous ? stable + 1 : 0;
    // Office animates the active row-header tint in canvas. A few identical
    // frames can occur inside that animation, so require two settled seconds.
    if (stable === 10) return current;
    previous = current;
  }
  throw new Error('Excel Normal canvas did not become stable');
}
async function capture(browser, doc) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, locale: 'ru-RU', timezoneId: 'UTC', colorScheme: 'light' });
  const page = await context.newPage(), fonts = new Map();
  page.on('response', async response => { const url = response.url(), type = response.headers()['content-type'] ?? ''; if (/font|\.woff2?(?:\?|$)/i.test(type + url)) { try { const body = await response.body(); fonts.set(sha(body), { family: new URL(url).pathname.split('/').pop() || 'Office web font', style: 'normal', weight: 400, sha256: sha(body), source: 'Office Online response: ' + new URL(url).origin }); } catch {} } });
  try {
    await page.goto(viewerUrl(doc.path), { waitUntil: 'domcontentloaded', timeout: 120000 });
    let frame; for (let i = 0; i < 240 && !frame; i++) { frame = page.frames().find(f => f.url().includes('xlviewerinternal')); if (!frame) await page.waitForTimeout(Number(process.env.PAGE_DELAY || 150)); }
    if (!frame) throw new Error('Excel viewer frame not found');
    await frame.locator('canvas.ewr-sheettable').first().waitFor({ timeout: 120000 }); await page.waitForTimeout(8000);
    const applicationVersion = await frame.locator('input[id$="postOpenWorkbookContextJson"]').evaluate(input => JSON.parse(input.value).BuildVersion);
    if (!applicationVersion) throw new Error('Excel viewer BuildVersion is unavailable');
    const sheets = await workbookSheets(path.join(root, doc.path)), base = path.join(output, doc.id);
    await fs.rm(base, { recursive: true, force: true }); await fs.mkdir(base, { recursive: true });
    const results = [];
    for (const sheet of sheets) {
      if (sheet.index > 0) { let tab = frame.getByRole('tab', { name: sheet.name, exact: true }); if (!await tab.count()) tab = frame.getByText(sheet.name, { exact: true }).last(); if (!await tab.count()) throw new Error(`Sheet tab not found: ${sheet.name}`); await tab.click(); await page.waitForTimeout(4000); }
      await page.keyboard.press('Control+Home');
      await frame.locator('canvas.ewr-sheettable').first().click({ position: { x: 60, y: 50 }, force: true });
      await page.mouse.move(1400, 30);
      const normal = await captureStableNormal(frame, page);
      const normalFile = `sheet-${sheet.index + 1}-normal.png`; await fs.writeFile(path.join(base, normalFile), Buffer.from(normal.split(',')[1], 'base64'));
      await frame.getByRole('button', { name: /^(Файл|File)$/ }).first().click({ noWaitAfter: true, force: true }); await frame.locator('#FileMenuFlyoutLauncherDropdown').getByText(/^(Печать|Print)$/).click({ force: true });
      const printCanvas = frame.locator('canvas[class^="canvasFixingExtraWhitePage"]'); await printCanvas.first().waitFor({ timeout: 120000 }); await frame.getByText(/(?:Показано|Отображение) .*(?:всего|общего числа) \d+|Showing .*of \d+/).first().waitFor({ timeout: 120000 }); await page.waitForTimeout(2000);
      const body = await frame.locator('body').innerText(), total = Number((body.match(/(?:всего|общего числа) (\d+)/i) ?? body.match(/of (\d+) pages/i) ?? [])[1] ?? 1), pages = []; console.log(`PAGES ${doc.id} ${sheet.name}: ${total}`);
      const lastPage = process.env.COUNT_ONLY ? 0 : Math.min(total, Number(process.env.MAX_PAGE) || total);
      for (let pageNumber = 1; pageNumber <= lastPage; pageNumber++) {
        const canvas = frame.locator('canvas[class^="canvasFixingExtraWhitePage"]:visible').first(); await canvas.waitFor({ timeout: 30000 });
        const displayPage = (pageNumber - 1) % 100 + 1;
        if (pageNumber > 1) {
          if (displayPage === 1) {
            const range = frame.locator('#pageRangeInput');
            await range.fill(`${pageNumber}-${Math.min(pageNumber + 99, total)}`);
            await range.press('Enter');
          } else {
            const target = frame.locator(`[aria-label="Page ${displayPage}."]`);
            await target.scrollIntoViewIfNeeded(); await target.focus();
          }
        }
        await waitPrintPage(frame, page, displayPage);
        const data = await canvas.evaluate(c => ({ data: c.toDataURL('image/png'), width: c.width, height: c.height })), file = `sheet-${sheet.index + 1}-print-page-${pageNumber}.png`; await fs.writeFile(path.join(base, file), Buffer.from(data.data.split(',')[1], 'base64')); pages.push({ file, width: data.width, height: data.height });
      }
      results.push({ ...sheet, normalFile, pages }); console.log(`PASS ${doc.id} ${sheet.name}: normal + ${pages.length} print pages${pages.length === total ? '' : ` of ${total}`}`);
      await frame.getByRole('button', { name: /Вернуться к книге|Back to workbook/ }).first().click({ force: true }); await page.waitForTimeout(2000);
    }
    await fs.writeFile(path.join(base, 'capture.json'), JSON.stringify({ source: doc, sheets: results, applicationVersion, capturedAt: new Date().toISOString(), browserVersion: browser.version(), fonts: [...fonts.values()] }, null, 2));
  } finally { await context.close(); }
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || '/usr/bin/chromium' });
  const failures = [];
  for (const doc of docs) {
    let error;
    for (let attempt = 1; attempt <= 3; attempt++) {
      try { await capture(browser, doc); error = null; break; }
      catch (e) { error = e; console.error(`FAIL ${doc.id} attempt ${attempt}: ${e.message}`); }
    }
    if (error) failures.push({ id: doc.id, error: error.message });
  }
  await browser.close();
  await fs.writeFile(path.join(output, `capture-summary${only.size ? '-' + [...only].join('-') : ''}.json`), JSON.stringify({ failures }, null, 2));
  if (failures.length) process.exitCode = 2;
})().catch(e => { console.error(e); process.exit(3); });
