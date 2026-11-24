const { chromium } = require('../../examples/viewer/node_modules/@playwright/test');
const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../..');
const output = path.resolve(process.argv[2] ?? '/tmp/office-word');
const only = new Set((process.argv[3] ?? '').split(',').filter(Boolean));
const requiredVersion = process.env.OFFICE_VERSION || null;
const corpus = JSON.parse(require('node:fs').readFileSync(path.join(root, 'test-fixtures/strict-viewer/corpus.json'), 'utf8'));
const docs = corpus.documents.filter(d => d.kind === 'docx' && (!only.size || only.has(d.id)));
const sha = b => crypto.createHash('sha256').update(b).digest('hex');
const viewerUrl = p => `https://view.officeapps.live.com/op/view.aspx?src=${encodeURIComponent(`https://raw.githubusercontent.com/meerrec/wasm-converter/main/${p}`)}`;
async function capture(browser, doc) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, locale: 'ru-RU', timezoneId: 'UTC', colorScheme: 'light', acceptDownloads: true });
  const page = await context.newPage();
  try {
    await page.goto(viewerUrl(doc.path), { waitUntil: 'domcontentloaded', timeout: 120000 });
    let frame;
    for (let i = 0; i < 240 && !frame; i++) { frame = page.frames().find(f => f.url().includes('wordviewerframe')); if (!frame) await page.waitForTimeout(500); }
    if (!frame) throw new Error('Word viewer frame not found');
    await frame.locator('img.WACPageImg').first().waitFor({ timeout: 120000 });
    await frame.locator('img.WACPageImg').evaluateAll(async imgs => { await Promise.all(imgs.map(img => img.complete && img.naturalWidth ? Promise.resolve() : new Promise((resolve, reject) => { img.onload = resolve; img.onerror = reject; }))); });
    const images = await frame.locator('img.WACPageImg').evaluateAll(imgs => imgs.map(img => ({ src: img.src, width: img.naturalWidth, height: img.naturalHeight })));
    if (!images.length || images.some(i => !i.src.startsWith('data:image/png;base64,'))) throw new Error('Office pages are not PNG data URLs');
    const base = path.join(output, doc.id); await fs.rm(base, { recursive: true, force: true }); await fs.mkdir(base, { recursive: true });
    for (let i = 0; i < images.length; i++) await fs.writeFile(path.join(base, `raw-page-${i + 1}.png`), Buffer.from(images[i].src.split(',')[1], 'base64'));
    await frame.locator('#MoreViewerOptionsFlyoutAnchor').click();
    const pending = page.waitForResponse(r => (r.headers()['content-type'] || '').includes('application/pdf'), { timeout: 120000 }); pending.catch(() => {});
    await frame.getByRole('menuitem').filter({ hasText: /PDF/i }).click();
    const response = await pending, pdf = await response.body();
    await fs.writeFile(path.join(base, 'reference.pdf'), pdf);
    const applicationVersion = response.headers()['x-officeversion'];
    if (!applicationVersion) throw new Error('Office PDF response has no x-officeversion header');
    if (requiredVersion && applicationVersion !== requiredVersion) throw new Error(`Office build ${applicationVersion} does not match required ${requiredVersion}`);
    await fs.writeFile(path.join(base, 'capture.json'), JSON.stringify({ source: doc, pages: images.map((x, i) => ({ file: `raw-page-${i + 1}.png`, width: x.width, height: x.height })), applicationVersion, capturedAt: new Date().toISOString(), pdfSha256: sha(pdf) }, null, 2));
    console.log(`PASS ${doc.id}: ${images.length} pages, PDF ${pdf.length} bytes`);
  } finally { await context.close(); }
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || '/usr/bin/chromium' }); const failures = [];
  for (const doc of docs) {
    let error;
    for (let attempt = 1; attempt <= 6; attempt++) {
      try { await capture(browser, doc); error = null; break; }
      catch (e) { error = e; console.error(`FAIL ${doc.id} attempt ${attempt}: ${e.message}`); }
    }
    if (error) failures.push({ id: doc.id, error: error.message });
  }
  await browser.close(); await fs.writeFile(path.join(output, `capture-summary${only.size ? '-' + [...only].join('-') : ''}.json`), JSON.stringify({ failures }, null, 2));
  if (failures.length) process.exitCode = 2;
})().catch(e => { console.error(e); process.exit(3); });
