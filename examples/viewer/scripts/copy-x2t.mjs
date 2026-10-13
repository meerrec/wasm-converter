// Копирует артефакты x2t в `public/x2t/` перед dev и сборкой: страница
// «Конвертер» отдаёт их статикой, Vite кладёт `public/` в сборку как есть.
//
// Сами артефакты лежат в репозитории под Git LFS (`packages/x2t/vendor/`): в
// свежем клоне без `git lfs pull` на их месте пойнтеры. Тогда не копируем ничего
// и не роняем сборку — страница конвертера честно скажет, что рантайм недоступен.
import { copyFile, mkdir, open, rm, stat } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const VIEWER_DIR = resolve(SCRIPT_DIR, '..');

// Источник по умолчанию — вендорный каталог репозитория; env-переменная нужна,
// чтобы прогнать ветку «LFS не выкачан» на временном каталоге с пойнтерами.
const VENDOR_DIR = process.env.X2T_VENDOR_DIR
  ? resolve(process.env.X2T_VENDOR_DIR)
  : resolve(VIEWER_DIR, '../../packages/x2t/vendor');
const TARGET_DIR = join(VIEWER_DIR, 'public', 'x2t');

/** Файлы x2t, нужные странице: обвязка и её wasm-модуль (грузятся лениво). */
const FILES = ['x2t.js', 'x2t.wasm'];

/** Начало пойнтера Git LFS: настоящие артефакты так не начинаются. */
const LFS_POINTER_PREFIX = 'version https://git-lfs.github.com/spec/v1';

/** Меньше этого размера файл — точно не настоящий артефакт. */
const MIN_REAL_BYTES = 1024;

/** Начинается ли файл с пойнтера LFS; читается только голова, не весь 38-МБ файл. */
async function startsWithLfsPointer(path) {
  const handle = await open(path, 'r');
  try {
    const head = Buffer.alloc(LFS_POINTER_PREFIX.length);
    const { bytesRead } = await handle.read(head, 0, head.length, 0);
    return head.subarray(0, bytesRead).toString('utf8').startsWith(LFS_POINTER_PREFIX);
  } finally {
    await handle.close();
  }
}

/** Состояние источника: настоящий ли он и его мерки — по ним сверяется копия. */
async function sourceState(name) {
  const path = join(VENDOR_DIR, name);
  const info = await stat(path).catch(() => null);
  if (info === null || !info.isFile()) {
    return { name, path, real: false };
  }
  const real = info.size >= MIN_REAL_BYTES && !(await startsWithLfsPointer(path));
  return { name, path, real, size: info.size, mtimeMs: info.mtimeMs };
}

const sources = await Promise.all(FILES.map((name) => sourceState(name)));

if (!sources.every((source) => source.real)) {
  // Протухшие копии убираются молча: их место в `public/` источник не занимает,
  // и такая копия уехала бы в сборку и на Pages как ни в чём не бывало.
  await rm(TARGET_DIR, { recursive: true, force: true });
  console.warn(
    'артефакты x2t не выкачаны из Git LFS — страница конвертера покажет недоступность',
  );
} else {
  await mkdir(TARGET_DIR, { recursive: true });
  for (const source of sources) {
    const target = join(TARGET_DIR, source.name);
    const copy = await stat(target).catch(() => null);
    // Копия актуальна, если совпал размер, а время правки не старше источника.
    if (copy !== null && copy.size === source.size && copy.mtimeMs >= source.mtimeMs) {
      continue;
    }
    await copyFile(source.path, target);
  }
  console.log(`артефакты x2t: ${FILES.join(', ')} → ${relative(process.cwd(), TARGET_DIR)}`);
}
