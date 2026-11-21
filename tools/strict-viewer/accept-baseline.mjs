#!/usr/bin/env node
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { sha256 } from './png.mjs';

const options = Object.fromEntries(process.argv.slice(2).filter((_, i) => i % 2 === 0).map((arg, i) => [arg.replace(/^--/, ''), process.argv.slice(2)[i * 2 + 1]]));
try {
  if (!options.actual || !options.candidate || !options.approval || options['accept-visual'] !== 'yes') {
    throw new Error('Explicit use: accept-baseline.mjs --actual actual.png --candidate new-reference.png --approval approvals.json --accept-visual yes');
  }
  const approval = JSON.parse(await readFile(options.approval, 'utf8'));
  if (!approval.maintainer || !approval.independentQa || approval.maintainer === approval.independentQa || approval.captureAuthor === approval.independentQa) {
    throw new Error('Two distinct approvals are required; independent QA must not be the capture author');
  }
  const actual = await readFile(options.actual);
  if (approval.actualSha256 !== sha256(actual)) throw new Error('Approval does not match actual artifact digest');
  await mkdir(path.dirname(options.candidate), { recursive: true });
  await writeFile(options.candidate, actual, { flag: 'wx' });
  console.log(JSON.stringify({ status: 'candidate-created', path: options.candidate, sha256: sha256(actual), approvals: [approval.maintainer, approval.independentQa] }));
} catch (error) {
  console.error(error.message);
  process.exitCode = 3;
}
