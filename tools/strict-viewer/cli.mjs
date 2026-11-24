#!/usr/bin/env node
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { comparePng } from './compare.mjs';
import { writeReport } from './report.mjs';

function args(argv) {
  const result = {};
  for (let i = 0; i < argv.length; i += 2) result[argv[i].replace(/^--/, '')] = argv[i + 1];
  return result;
}
const options = args(process.argv.slice(2));
let report;
try {
  if (!options.reference || !options.actual || !options.output) throw new Error('Usage: cli.mjs --reference ref.png --actual actual.png --output directory [--id name] [--masks masks.json]');
  const id = options.id ?? path.basename(options.actual, '.png');
  let result;
  try { await readFile(options.reference); }
  catch { result = { id, status: 'missing-reference', reason: 'reference-not-found' }; }
  if (!result) {
    try { await readFile(options.actual); }
    catch { result = { id, status: 'capture-error', reason: 'actual-not-found' }; }
  }
  if (!result) {
    const masks = options.masks ? JSON.parse(await readFile(options.masks, 'utf8')) : [];
    result = { id, ...comparePng(await readFile(options.reference), await readFile(options.actual), { masks }) };
    if (result.diffPng) {
      await mkdir(options.output, { recursive: true });
      await writeFile(path.join(options.output, 'diff.png'), result.diffPng);
    }
    delete result.diffPng;
    result.images = { reference: path.relative(options.output, options.reference), actual: path.relative(options.output, options.actual), diff: result.status === 'visual-diff' ? 'diff.png' : null };
  }
  report = { schemaVersion: '1.0.0', baselineId: options.baseline ?? null, revision: options.revision ?? null, results: [result], summary: { [result.status]: 1 } };
  await writeReport(options.output, report);
  process.exitCode = result.status === 'match' ? 0 : ['visual-diff', 'page-mismatch'].includes(result.status) ? 1 : ['missing-reference', 'capture-error'].includes(result.status) ? 2 : 3;
} catch (error) {
  report = { schemaVersion: '1.0.0', results: [{ id: options.id ?? 'unknown', status: 'harness-error', reason: error.message }], summary: { 'harness-error': 1 } };
  if (options.output) await writeReport(options.output, report);
  else console.error(error);
  process.exitCode = 3;
}
