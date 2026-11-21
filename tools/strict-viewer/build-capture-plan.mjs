#!/usr/bin/env node
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';

const [output, ...manifestPaths] = process.argv.slice(2);
if (!output || manifestPaths.length === 0) throw new Error('Usage: build-capture-plan.mjs plan.json manifest.json [...]');
const documents = [];
for (const manifestPath of manifestPaths) {
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  if (manifest.schemaVersion !== '1.0.0' || manifest.reference?.provider !== 'office-online') throw new Error(`${manifestPath}: not an Office Online schema 1.0.0 manifest`);
  documents.push({ manifestPath: path.resolve(manifestPath), manifest });
}
const first = documents[0].manifest;
for (const { manifestPath, manifest } of documents) {
  if (JSON.stringify(manifest.environment) !== JSON.stringify(first.environment)) throw new Error(`${manifestPath}: run one pinned environment per capture plan`);
  if (manifest.corpus.baselineId !== first.corpus.baselineId) throw new Error(`${manifestPath}: mixed baseline IDs`);
}
const plan = { schemaVersion: '1.0.0', baselineId: first.corpus.baselineId, environment: first.environment, documents };
await mkdir(path.dirname(path.resolve(output)), { recursive: true });
await writeFile(output, `${JSON.stringify(plan, null, 2)}\n`);
console.log(`Capture plan: ${documents.length} documents, baseline ${plan.baselineId}`);
