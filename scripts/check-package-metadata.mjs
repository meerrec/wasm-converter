#!/usr/bin/env node
// Проверка метаданных публикации npm-пакетов @meerrec/*.
// Запуск: node scripts/check-package-metadata.mjs
// Зависимостей нет — только встроенные модули Node.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const packages = [
  'core',
  'wasm',
  'wasm-docx',
  'wasm-pdf',
  'cli',
  'react',
  'vue',
  'svelte',
];

// Единые для всех восьми пакетов значения: монорепозиторий meerrec/wasm-converter.
const expected = {
  repositoryType: 'git',
  repositoryUrl: 'git+https://github.com/meerrec/wasm-converter.git',
  homepage: 'https://github.com/meerrec/wasm-converter#readme',
  bugsUrl: 'https://github.com/meerrec/wasm-converter/issues',
  license: 'Apache-2.0',
};

let failed = 0;
let checked = 0;

function check(label, actual, wanted) {
  checked += 1;
  if (actual === wanted) return;
  failed += 1;
  console.error(
    `FAIL ${label}: ожидалось ${JSON.stringify(wanted)}, получено ${JSON.stringify(actual)}`,
  );
}

const rootLicense = readFileSync(join(root, 'LICENSE'));

for (const pkgName of packages) {
  const dir = join(root, 'packages', pkgName);
  const manifest = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
  const label = `@meerrec/${pkgName}`;
  const repository = manifest.repository ?? {};

  check(`${label}: repository.type`, repository.type, expected.repositoryType);
  check(`${label}: repository.url`, repository.url, expected.repositoryUrl);
  check(`${label}: repository.directory`, repository.directory, `packages/${pkgName}`);
  check(`${label}: homepage`, manifest.homepage, expected.homepage);
  check(`${label}: bugs.url`, manifest.bugs?.url, expected.bugsUrl);
  check(`${label}: license`, manifest.license, expected.license);

  let packageLicense;
  try {
    packageLicense = readFileSync(join(dir, 'LICENSE'));
  } catch {
    failed += 1;
    console.error(`FAIL ${label}: packages/${pkgName}/LICENSE не читается (файла нет?)`);
    continue;
  }
  if (!packageLicense.equals(rootLicense)) {
    failed += 1;
    console.error(
      `FAIL ${label}: packages/${pkgName}/LICENSE не совпадает с корневым LICENSE побайтово`,
    );
  }

  console.log(`OK   ${label}`);
}

if (failed > 0) {
  console.error(`\nПровалено проверок: ${failed} из ${checked}. Метаданные публикации не готовы.`);
  process.exit(1);
}

console.log(
  `\nOK: ${packages.length} пакетов — repository, homepage, bugs, license и LICENSE совпадают с корневыми.`,
);
