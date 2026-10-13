// Копирует не-JS ресурсы src/viewer/ в dist/ — tsc переносит только .ts.
// Вызывается из npm run build, чтобы dist/ был полным (main.js + style.css).
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const SPIKE_DIR = resolve(SCRIPT_DIR, '..');
const DIST_DIR = join(SPIKE_DIR, 'dist');

mkdirSync(DIST_DIR, { recursive: true });
copyFileSync(join(SPIKE_DIR, 'src', 'viewer', 'style.css'), join(DIST_DIR, 'style.css'));
console.log('копия стилей: dist/style.css');
