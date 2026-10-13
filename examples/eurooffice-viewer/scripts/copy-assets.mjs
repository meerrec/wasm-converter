// Копирует не-JS ресурсы src/viewer/ в dist/: Vite собирает в бандл только скрипт.
// Стили кладём копией, а не через `import './style.css'`: имя CSS-файла в lib-режиме
// Vite выводит из имени пакета, а viewer.html ждёт постоянный `./dist/style.css`.
// Вызывается из `pnpm run build`, чтобы dist/ был полным (main.js + style.css).
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const EXAMPLE_DIR = resolve(SCRIPT_DIR, '..');
const DIST_DIR = join(EXAMPLE_DIR, 'dist');

mkdirSync(DIST_DIR, { recursive: true });
copyFileSync(join(EXAMPLE_DIR, 'src', 'viewer', 'style.css'), join(DIST_DIR, 'style.css'));
console.log('копия стилей: dist/style.css');
