#!/usr/bin/env node
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { comparePng } from './compare.mjs';
import { writeReport } from './report.mjs';

const [planPath, captureRoot, output] = process.argv.slice(2);
if (!planPath || !captureRoot || !output) {
  console.error('Usage: compare-corpus.mjs capture-plan.json actual-output report-output');
  process.exitCode = 3;
} else {
  const results = [];
  try {
    const plan = JSON.parse(await readFile(planPath, 'utf8'));
    let actualResults;
    try { actualResults = JSON.parse(await readFile(path.join(captureRoot, 'capture-summary.json'), 'utf8')).results; }
    catch (error) { throw new Error(`Capture summary is unavailable: ${error.message}`); }
    const byId = new Map(actualResults.map((r) => [r.id, r]));
    for (const { manifestPath, manifest } of plan.documents) {
      const expected = manifest.captures;
      for (const capture of expected) {
        const actual = byId.get(capture.id);
        if (actual?.status === 'page-mismatch') {
          results.push({ id: capture.id, status: 'page-mismatch', reason: actual.error, stage: actual.stage, actualPageCount: actual.actualPageCount });
          continue;
        }
        if (!actual || actual.status === 'capture-error') {
          results.push({ id: capture.id, status: 'capture-error', reason: actual?.error ?? 'capture result is missing', stage: actual?.stage });
          continue;
        }
        const referencePath = path.resolve(path.dirname(manifestPath), capture.image.uri);
        let referenceBytes;
        try { referenceBytes = await readFile(referencePath); }
        catch { results.push({ id: capture.id, status: 'missing-reference', reason: referencePath }); continue; }
        try {
          const result = comparePng(referenceBytes, await readFile(actual.image.path), { masks: (capture.masks ?? []).map((m) => m.rect) });
          const diffName = `${capture.id}.diff.png`;
          const hasDiff = Boolean(result.diffPng);
          if (hasDiff) { await mkdir(output, { recursive: true }); await writeFile(path.join(output, diffName), result.diffPng); }
          delete result.diffPng;
          results.push({ ...result, id: capture.id, kind: capture.kind, pageNumber: capture.pageNumber, sheet: capture.sheet, images: { reference: path.relative(output, referencePath), actual: path.relative(output, actual.image.path), diff: hasDiff ? diffName : null } });
        } catch (error) { results.push({ id: capture.id, status: 'harness-error', reason: error.message }); }
      }
      const actualForDocument = expected
        .map((capture) => byId.get(capture.id))
        .filter((result) => result?.status === 'captured');
      if (manifest.source.format === 'docx') {
        const actualPageCount = actualForDocument[0]?.actualPageCount;
        if (actualPageCount !== undefined && actualPageCount !== expected.length) {
          results.push({ id: `${manifest.source.path}:pages`, status: 'page-mismatch', reason: `Expected ${expected.length} DOCX pages, actual ${actualPageCount}` });
        }
      }
      if (manifest.source.format === 'xlsx') {
        const actualPageCount = actualForDocument[0]?.actualPageCount;
        if (manifest.view.mode === 'print-preview' && actualPageCount !== undefined && actualPageCount !== expected.length) {
          results.push({ id: `${manifest.source.path}:sheet-${manifest.view.activeSheet.index}:pages`, status: 'page-mismatch', reason: `Expected ${expected.length} XLSX print pages, actual ${actualPageCount}` });
        }
        const expectedOrder = expected.map((c) => `${c.sheet?.index ?? ''}:${c.pageNumber ?? ''}:${c.kind}`).join('|');
        const actualOrder = expected.map((c) => byId.get(c.id)).filter(Boolean).map((c) => {
          const capture = expected.find((item) => item.id === c.id);
          return `${capture.sheet?.index ?? ''}:${capture.pageNumber ?? ''}:${capture.kind}`;
        }).join('|');
        if (expectedOrder !== actualOrder) results.push({ id: `${manifest.source.path}:sheet-order`, status: 'page-mismatch', reason: 'XLSX sheet/print-page order differs' });
      }
    }
    const summary = {};
    for (const result of results) summary[result.status] = (summary[result.status] ?? 0) + 1;
    await writeReport(output, { schemaVersion: '1.0.0', baselineId: plan.baselineId, revision: process.env.GITHUB_SHA ?? null, results, summary });
    process.exitCode = summary['harness-error'] ? 3 : summary['missing-reference'] || summary['capture-error'] ? 2 : summary['visual-diff'] || summary['page-mismatch'] ? 1 : 0;
  } catch (error) {
    await writeReport(output, { schemaVersion: '1.0.0', results: [{ id: 'corpus', status: 'harness-error', reason: error.message }], summary: { 'harness-error': 1 } });
    process.exitCode = 3;
  }
}
