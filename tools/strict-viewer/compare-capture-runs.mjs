#!/usr/bin/env node
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { decodePng } from './png.mjs';

const [firstRoot, secondRoot, output] = process.argv.slice(2);
if (!firstRoot || !secondRoot || !output) {
  console.error('Usage: compare-capture-runs.mjs FIRST_OUTPUT SECOND_OUTPUT REPORT.json');
  process.exit(3);
}
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const first = JSON.parse(await readFile(path.join(firstRoot, 'capture-summary.json'), 'utf8'));
const second = JSON.parse(await readFile(path.join(secondRoot, 'capture-summary.json'), 'utf8'));
const left = new Map(first.results.map(result => [result.id, result]));
const right = new Map(second.results.map(result => [result.id, result]));
const ids = [...new Set([...left.keys(), ...right.keys()])].sort();
const results = [];
async function findPdfs(root, directory = root, found = new Map()) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) await findPdfs(root, absolute, found);
    else if (entry.name === 'actual.pdf') found.set(path.relative(root, absolute), absolute);
  }
  return found;
}
const [firstPdfs, secondPdfs] = await Promise.all([findPdfs(firstRoot), findPdfs(secondRoot)]);
const pdfRelativePaths = [...new Set([...firstPdfs.keys(), ...secondPdfs.keys()])].sort();
for (const id of ids) {
  const a = left.get(id), b = right.get(id);
  if (!a || !b) {
    results.push({ id, status: 'different', reason: 'capture-result-missing' });
    continue;
  }
  if (a.status !== b.status || a.actualPageCount !== b.actualPageCount) {
    results.push({ id, status: 'different', reason: 'status-or-page-count', first: { status: a.status, pages: a.actualPageCount }, second: { status: b.status, pages: b.actualPageCount } });
    continue;
  }
  if (!a.image) {
    results.push({ id, status: 'identical', reason: 'same-non-image-result', resultStatus: a.status, actualPageCount: a.actualPageCount });
    continue;
  }
  const [aBytes, bBytes] = await Promise.all([readFile(a.image.path), readFile(b.image.path)]);
  const item = { id, firstSha256: sha(aBytes), secondSha256: sha(bBytes), bytes: aBytes.length };
  if (aBytes.equals(bBytes)) {
    item.status = 'identical'; item.reason = 'png-bytes';
  } else {
    const aPng = decodePng(aBytes), bPng = decodePng(bBytes);
    const same = aPng.width === bPng.width && aPng.height === bPng.height && Buffer.from(aPng.pixels).equals(Buffer.from(bPng.pixels));
    item.status = same ? 'identical' : 'different';
    item.reason = same ? 'png-pixels-different-encoding' : 'png-pixels';
    item.dimensions = { first: [aPng.width, aPng.height], second: [bPng.width, bPng.height] };
  }
  results.push(item);
}
const pdfs = [];
for (const relativePath of pdfRelativePaths) {
  const aPath = firstPdfs.get(relativePath), bPath = secondPdfs.get(relativePath);
  if (!aPath || !bPath) {
    pdfs.push({ path: relativePath, status: 'different', reason: 'pdf-missing', first: Boolean(aPath), second: Boolean(bPath) });
    continue;
  }
  const [a, b] = await Promise.all([readFile(aPath), readFile(bPath)]);
  pdfs.push({ path: relativePath, firstSha256: sha(a), secondSha256: sha(b), status: a.equals(b) ? 'identical' : 'different' });
}
const summary = {
  captures: ids.length,
  identicalCaptures: results.filter(result => result.status === 'identical').length,
  differentCaptures: results.filter(result => result.status !== 'identical').length,
  pdfs: pdfs.length,
  identicalPdfs: pdfs.filter(result => result.status === 'identical').length,
  differentPdfs: pdfs.filter(result => result.status !== 'identical').length,
};
const report = { schemaVersion: '1.0.0', repeatable: summary.differentCaptures === 0 && summary.differentPdfs === 0, summary, results, pdfs };
await mkdir(path.dirname(path.resolve(output)), { recursive: true });
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report.summary));
process.exitCode = report.repeatable ? 0 : 1;
