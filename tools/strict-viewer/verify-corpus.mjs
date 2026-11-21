#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { sha256 } from './png.mjs';

try {
  const file = process.argv[2] ?? 'test-fixtures/strict-viewer/corpus.json';
  const corpus = JSON.parse(await readFile(file, 'utf8'));
  if (corpus.status !== 'candidates' || corpus.baselineId !== null) throw new Error('Corpus must not claim an accepted baseline');
  if (corpus.documents.length !== 40) throw new Error('Exactly 40 candidate documents are required');
  const ids = new Set();
  for (const document of corpus.documents) {
    if (ids.has(document.id)) throw new Error(`Duplicate id ${document.id}`);
    ids.add(document.id);
    if (document.referenceStatus !== 'pending') throw new Error(`${document.id}: reference is not pending`);
    const bytes = await readFile(path.resolve(document.path));
    if (sha256(bytes) !== document.sha256) throw new Error(`${document.id}: source digest mismatch`);
  }
  for (const kind of ['docx', 'xlsx']) if (corpus.documents.filter((d) => d.kind === kind).length !== 20) throw new Error(`Expected 20 ${kind}`);
  console.log('PASS: 20 DOCX + 20 XLSX source digests verified; Office references remain pending');
} catch (error) {
  console.error(error.message); process.exitCode = 3;
}
