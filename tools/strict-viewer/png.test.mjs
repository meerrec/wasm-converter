import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { decodePng, encodePng } from './png.mjs';
import { comparePng } from './compare.mjs';

const image = (pixels, width = 1, height = 1) => encodePng({ width, height, pixels: Buffer.from(pixels) });
const white = [255, 255, 255, 255];

test('PNG RGBA round-trip', () => {
  const source = image([...white, 1, 2, 3, 4], 2);
  assert.deepEqual([...decodePng(source).pixels], [...white, 1, 2, 3, 4]);
});

test('zero out-of-tolerance pixels: delta 2 passes, delta 3 and alpha 1 fail', () => {
  assert.equal(comparePng(image(white), image([253, 255, 255, 255])).status, 'match');
  assert.equal(comparePng(image(white), image([252, 255, 255, 255])).status, 'visual-diff');
  const alpha = comparePng(image(white), image([255, 255, 255, 254]));
  assert.equal(alpha.status, 'visual-diff'); assert.equal(alpha.alphaMismatch, true);
});

test('only declared in-bounds masks hide differences', () => {
  const masked = comparePng(image(white), image([0, 0, 0, 255]), { masks: [{ x: 0, y: 0, width: 1, height: 1 }] });
  assert.equal(masked.status, 'match'); assert.equal(masked.comparedPixels, 0);
  assert.throws(() => comparePng(image(white), image(white), { masks: [{ x: 0, y: 0, width: 2, height: 1 }] }));
});

test('dimension mismatch cannot become a match', () => {
  const result = comparePng(image(white), image([...white, ...white], 2));
  assert.equal(result.status, 'visual-diff'); assert.equal(result.reason, 'dimension-mismatch');
});

test('identical images have diagnostic SSIM 1', () => {
  const pixels = [...white, ...white];
  assert.equal(comparePng(image(pixels, 2), image(pixels, 2)).ssim, 1);
});

test('CLI separates exit codes 0, 1 and 2 and writes reports', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'strict-viewer-'));
  const reference = path.join(directory, 'reference.png'), actual = path.join(directory, 'actual.png');
  const cli = fileURLToPath(new URL('./cli.mjs', import.meta.url));
  await writeFile(reference, image(white)); await writeFile(actual, image(white));
  const run = (name) => spawnSync(process.execPath, [cli, '--reference', reference, '--actual', actual, '--output', path.join(directory, name)], { encoding: 'utf8' });
  assert.equal(run('match').status, 0);
  await writeFile(actual, image([0, 0, 0, 255])); assert.equal(run('diff').status, 1);
  const missing = spawnSync(process.execPath, [cli, '--reference', path.join(directory, 'none.png'), '--actual', actual, '--output', path.join(directory, 'missing')]);
  assert.equal(missing.status, 2);
  const report = JSON.parse(await readFile(path.join(directory, 'diff', 'report.json'), 'utf8'));
  assert.equal(report.results[0].status, 'visual-diff');
});
