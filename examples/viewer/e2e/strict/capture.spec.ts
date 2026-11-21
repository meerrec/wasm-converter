import { test } from '@playwright/test';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { boot } from '../helpers/boot.js';
import { frameId, waitForFrame } from '../helpers/docx_frame.js';

const plan = JSON.parse(await readFile(process.env.STRICT_CAPTURE_PLAN!, 'utf8'));
const output = path.resolve(process.env.STRICT_CAPTURE_OUTPUT ?? 'strict-capture-output');
const results: unknown[] = [];

async function exportFrame(page: import('@playwright/test').Page, pdf: boolean): Promise<Buffer> {
  const values = await page.evaluate(async (asPdf) => {
    const bridge = window.docConverter;
    if (!bridge || bridge.kind === 'pptx') throw new Error('DOCX/XLSX viewer is not available');
    const bytes = asPdf ? await bridge.viewer.exportPdf() : await bridge.viewer.exportPng();
    return Array.from(bytes);
  }, pdf);
  return Buffer.from(values);
}

async function stablePng(page: import('@playwright/test').Page): Promise<Buffer> {
  await page.evaluate(() => document.fonts.ready);
  let previous = '';
  for (let attempt = 0; attempt < 30; attempt += 1) {
    const png = await exportFrame(page, false);
    const signature = await page.evaluate(async (bytes) => {
      const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext('2d')!;
      context.drawImage(bitmap, 0, 0);
      const data = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
      let hash = 0x811c9dc5;
      for (const byte of data) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0;
      return `${bitmap.width}x${bitmap.height}:${hash}`;
    }, [...png]);
    if (signature === previous) return png;
    previous = signature;
    await page.waitForTimeout(100);
  }
  throw new Error('Frame did not become stable');
}

for (const document of plan.documents) {
  const { manifest } = document;
  for (const capture of manifest.captures) {
    test(`${manifest.source.path}:${capture.id}`, async ({ page }) => {
      const startedAt = new Date().toISOString();
      const directory = path.join(output, manifest.source.format, capture.id);
      await mkdir(directory, { recursive: true });
      let stage = 'open';
      try {
        if (manifest.environment.applicationZoomPercent !== 100 || manifest.environment.browserZoomPercent !== 100) {
          throw new Error('Non-100% zoom requires an explicitly verified viewer zoom adapter');
        }
        await page.setViewportSize(manifest.environment.viewportCss);
        const errors = await boot(page, manifest.source.format === 'docx' ? '/docx' : '/xlsx');
        const fixture = manifest.source.path.replace(/^test-fixtures\/(docx|xlsx)\//, '');
        await page.selectOption('#fixture', fixture);
        if (manifest.source.format === 'docx') {
          await page.waitForFunction(() => window.docConverter?.kind === 'docx' && window.docConverter.viewer.pageCount > 0, undefined, { timeout: 30_000 });
          const pageCount = await page.evaluate(() => window.docConverter?.kind === 'docx' ? window.docConverter.viewer.pageCount : 0);
          if (capture.kind !== 'docx-page' || capture.pageNumber > pageCount) throw new Error(`DOCX page mismatch: requested ${capture.pageNumber}, actual ${pageCount}`);
          if (capture.pageNumber > 1) {
            for (let current = 1; current < capture.pageNumber; current += 1) {
              const before = await frameId(page); await page.click('#page-next'); await waitForFrame(page, before);
            }
          }
        } else {
          await page.waitForFunction(() => document.querySelector('#status')?.textContent?.includes('лист'), undefined, { timeout: 30_000 });
          const sheetIndex = capture.sheet?.index ?? manifest.view.activeSheet.index;
          await page.selectOption('#sheet', String(sheetIndex + 1));
          if (capture.kind === 'xlsx-screen') {
            await page.locator('#grid').setChecked(manifest.view.showGridLines);
            await page.locator('#headers').setChecked(manifest.view.showHeadings);
            if (capture.scroll) {
              await page.evaluate(({ x, y }) => {
                const scroller = document.querySelector('#viewer > div') as HTMLElement;
                scroller.scrollLeft = x; scroller.scrollTop = y;
              }, capture.scroll);
            }
          }
        }
        stage = 'render';
        const pdf = manifest.view.mode === 'pdf' || manifest.view.mode === 'print-preview';
        let png: Buffer;
        if (pdf) {
          const bytes = await exportFrame(page, true);
          const pdfPath = path.join(directory, 'actual.pdf');
          await writeFile(pdfPath, bytes);
          execFileSync('pdftoppm', ['-f', String(capture.pageNumber ?? 1), '-singlefile', '-r', '144', '-png', pdfPath, path.join(directory, 'actual')], { stdio: 'pipe' });
          png = await readFile(path.join(directory, 'actual.png'));
        } else png = await stablePng(page);
        await writeFile(path.join(directory, 'actual.png'), png);
        const runtime = await page.evaluate(() => ({ userAgent: navigator.userAgent, dpr: devicePixelRatio, viewport: { width: innerWidth, height: innerHeight } }));
        const result = { id: capture.id, status: 'captured', stage: 'complete', source: manifest.source, referenceManifest: document.manifestPath, image: { path: path.join(directory, 'actual.png'), bytes: png.length }, runtime, errors, startedAt, finishedAt: new Date().toISOString() };
        results.push(result);
        await writeFile(path.join(directory, 'actual.json'), `${JSON.stringify(result, null, 2)}\n`);
      } catch (error) {
        const result = { id: capture.id, status: 'capture-error', stage, source: manifest.source, referenceManifest: document.manifestPath, error: error instanceof Error ? error.message : String(error), startedAt, finishedAt: new Date().toISOString() };
        results.push(result);
        await writeFile(path.join(directory, 'actual.json'), `${JSON.stringify(result, null, 2)}\n`);
      }
    });
  }
}

test.afterAll(async () => {
  await mkdir(output, { recursive: true });
  const summary = { schemaVersion: '1.0.0', baselineId: plan.baselineId, results };
  await writeFile(path.join(output, 'capture-summary.json'), `${JSON.stringify(summary, null, 2)}\n`);
});
