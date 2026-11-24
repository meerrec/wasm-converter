import { test } from '@playwright/test';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { boot } from '../helpers/boot.js';
import { frameId, waitForFrame } from '../helpers/docx_frame.js';

const plan = JSON.parse(await readFile(process.env.STRICT_CAPTURE_PLAN!, 'utf8'));
const output = path.resolve(process.env.STRICT_CAPTURE_OUTPUT ?? 'strict-capture-output');
const results: any[] = [];

async function exportFrame(page: import('@playwright/test').Page, pdf: boolean): Promise<Buffer> {
  const values = await page.evaluate(async (asPdf) => {
    const bridge = window.docConverter;
    if (!bridge || bridge.kind === 'pptx') throw new Error('DOCX/XLSX viewer is not available');
    const bytes = asPdf ? await bridge.viewer.exportPdf() : await bridge.viewer.exportPng();
    return Array.from(bytes);
  }, pdf);
  return Buffer.from(values as number[]);
}

async function stablePng(page: import('@playwright/test').Page): Promise<Buffer> {
  await page.evaluate(() => document.fonts.ready);
  let previous = '';
  let stableFrames = 0;
  for (let attempt = 0; attempt < 60; attempt += 1) {
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
    stableFrames = signature === previous ? stableFrames + 1 : 1;
    // Готовность четырёх начертаний обеспечивает renderer до первого кадра.
    // Здесь ждём только завершения обычной очереди paint, а не «прогрева» шрифтов.
    if (stableFrames >= 5) return png;
    previous = signature;
    await page.waitForTimeout(100);
  }
  throw new Error('Frame did not become stable');
}

for (const document of plan.documents) {
  const { manifest } = document;
  test(`${manifest.source.path}:sheet-${manifest.view.activeSheet?.index ?? 0}:${manifest.view.mode}`, async ({ page }) => {
    const startedAt = new Date().toISOString();
    const base = { source: manifest.source, referenceManifest: document.manifestPath, startedAt };
    const directories = new Map<string, string>();
    let errors: string[] = [];
    for (const capture of manifest.captures) {
      const directory = path.join(output, manifest.source.format, capture.id);
      directories.set(capture.id, directory); await mkdir(directory, { recursive: true });
    }
    const record = async (capture: any, result: any) => {
      const value = { id: capture.id, ...base, ...result, finishedAt: new Date().toISOString() };
      results.push(value); await writeFile(path.join(directories.get(capture.id)!, 'actual.json'), `${JSON.stringify(value, null, 2)}\n`);
    };
    try {
      if (manifest.environment.applicationZoomPercent !== 100 || manifest.environment.browserZoomPercent !== 100) throw new Error('Non-100% zoom requires an explicitly verified viewer zoom adapter');
      await page.setViewportSize(manifest.environment.viewportCss);
      errors = await boot(page, manifest.source.format === 'docx' ? '/docx' : '/xlsx');
      const fixture = manifest.source.path.replace(/^test-fixtures\/(docx|xlsx)\//, '');
      await page.selectOption('#fixture', fixture);
      let actualPageCount: number | undefined;
      if (manifest.source.format === 'docx') {
        await page.waitForFunction(() => window.docConverter?.kind === 'docx' && window.docConverter.viewer.pageCount > 0, undefined, { timeout: 60_000 });
        actualPageCount = await page.evaluate(() => window.docConverter?.kind === 'docx' ? window.docConverter.viewer.pageCount : 0);
      } else {
        await page.waitForFunction(() => document.querySelector('#status')?.textContent?.includes('лист'), undefined, { timeout: 60_000 });
        await page.selectOption('#sheet', String(manifest.view.activeSheet.index));
      }
      if (manifest.view.mode !== 'normal') {
        const work = directories.get(manifest.captures[0].id)!;
        const bytes = await exportFrame(page, true);
        const pdfPath = path.join(work, 'actual.pdf'); await writeFile(pdfPath, bytes);
        const info = execFileSync('pdfinfo', [pdfPath], { encoding: 'utf8' });
        actualPageCount = Number(info.match(/^Pages:\s+(\d+)/m)?.[1]);
        const rasterDpi = manifest.source.format === 'docx' && manifest.view.mode === 'print-layout' ? '96' : '144';
        execFileSync('pdftoppm', ['-r', rasterDpi, '-png', pdfPath, path.join(work, 'raster')], { stdio: 'pipe', timeout: 600_000 });
        for (const capture of manifest.captures) {
          const pageNumber = capture.pageNumber ?? 1;
          if (pageNumber > actualPageCount!) {
            await record(capture, { status: 'page-mismatch', stage: 'complete', actualPageCount, error: `Page mismatch: requested ${pageNumber}, actual ${actualPageCount}` }); continue;
          }
          const rasterName = `raster-${String(pageNumber).padStart(String(actualPageCount).length, '0')}.png`;
          const png = await readFile(path.join(work, rasterName));
          const target = path.join(directories.get(capture.id)!, 'actual.png'); await writeFile(target, png);
          const runtime = await page.evaluate(() => ({ userAgent: navigator.userAgent, dpr: devicePixelRatio, viewport: { width: innerWidth, height: innerHeight } }));
          await record(capture, { status: 'captured', stage: 'complete', actualPageCount, image: { path: target, bytes: png.length }, runtime, errors });
        }
      } else {
        let currentDocxPage = 1;
        for (const capture of manifest.captures) {
          try {
            if (manifest.source.format === 'docx') {
              if (capture.pageNumber > actualPageCount!) throw new Error(`DOCX page mismatch: requested ${capture.pageNumber}, actual ${actualPageCount}`);
              while (currentDocxPage < capture.pageNumber) {
                const before = await frameId(page); await page.click('#page-next'); await waitForFrame(page, before); currentDocxPage += 1;
              }
            } else {
              await page.locator('#grid').setChecked(manifest.view.showGridLines);
              await page.locator('#headers').setChecked(manifest.view.showHeadings);
              await page.evaluate(({ x, y }) => { const scroller = document.querySelector('#viewer > div') as HTMLElement; scroller.scrollLeft = x; scroller.scrollTop = y; }, capture.scroll ?? { x: 0, y: 0 });
            }
            const png = await stablePng(page), target = path.join(directories.get(capture.id)!, 'actual.png'); await writeFile(target, png);
            const runtime = await page.evaluate(() => ({ userAgent: navigator.userAgent, dpr: devicePixelRatio, viewport: { width: innerWidth, height: innerHeight } }));
            await record(capture, { status: 'captured', stage: 'complete', actualPageCount, image: { path: target, bytes: png.length }, runtime, errors });
          } catch (error) { await record(capture, { status: 'capture-error', stage: 'render', actualPageCount, error: error instanceof Error ? error.message : String(error) }); }
        }
      }
    } catch (error) {
      const status = await page.locator('#status').textContent().catch(() => null);
      for (const capture of manifest.captures) if (!results.some(r => r.id === capture.id)) await record(capture, { status: 'capture-error', stage: 'open', error: error instanceof Error ? error.message : String(error), viewerStatus: status, errors });
    }
    const failed = results.filter(result => result.startedAt === startedAt && result.status !== 'captured' && result.status !== 'page-mismatch');
    if (failed.length > 0) throw new Error(`Capture failed: ${failed.slice(0, 10).map(result => `${result.id}: ${result.error}`).join('; ')}${failed.length > 10 ? `; and ${failed.length - 10} more` : ''}`);
  });
}

test.afterAll(async () => {
  await mkdir(output, { recursive: true });
  const summary = { schemaVersion: '1.0.0', baselineId: plan.baselineId, results };
  await writeFile(path.join(output, 'capture-summary.json'), `${JSON.stringify(summary, null, 2)}\n`);
});
