import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig, type Plugin } from 'vite';

const DOCX_ROOT = path.resolve(import.meta.dirname, '../../test-fixtures/docx');
const XLSX_ROOT = path.resolve(import.meta.dirname, '../../test-fixtures/xlsx');
const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';
const XLSX_MIME = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';

/**
 * Имена фикстур DOCX — пути от корня фикстур, с категорией (`simple/one_paragraph.docx`):
 * документы лежат по подкаталогам, а не в один уровень, как книги XLSX.
 * Отбор и порядок одинаковы у dev-сервера и сборки.
 */
async function docxFixtureNames(dir = DOCX_ROOT, prefix = ''): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const names: string[] = [];
  for (const entry of entries) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      names.push(...(await docxFixtureNames(path.join(dir, entry.name), relative)));
    } else if (entry.name.endsWith('.docx')) {
      names.push(relative);
    }
  }
  return names.sort();
}

/**
 * Имена фикстур XLSX: книги лежат одним уровнем, поэтому каталог обходится
 * плоско. Отбор и порядок одинаковы у dev-сервера и сборки.
 */
async function xlsxFixtureNames(): Promise<string[]> {
  const names = await readdir(XLSX_ROOT);
  return names.filter((n) => n.endsWith('.xlsx')).sort();
}

/** Корень фикстур: каталог на диске, MIME его файлов и отбор имён. */
interface FixtureRoot {
  readonly dir: string;
  readonly mime: string;
  readonly names: () => Promise<string[]>;
}

const ROOTS: Record<string, FixtureRoot> = {
  docx: { dir: DOCX_ROOT, mime: DOCX_MIME, names: () => docxFixtureNames() },
  xlsx: { dir: XLSX_ROOT, mime: XLSX_MIME, names: xlsxFixtureNames },
};

/** Путь к фикстуре по имени из запроса; `null` — имени нельзя доверять. */
function fixturePath(root: string, name: string): string | null {
  // Имя приходит из URL: `..` в нём вывел бы за пределы каталога фикстур.
  if (name.includes('..') || name.startsWith('/')) return null;
  const full = path.resolve(root, name);
  return full.startsWith(`${root}${path.sep}`) ? full : null;
}

/**
 * Отдаёт фикстуры обоих примеров: `/fixtures/docx/<категория>/<имя>.docx` и
 * `/fixtures/xlsx/<имя>.xlsx`.
 *
 * Складывать сотни документов и книг в `public/` незачем: они и так лежат в
 * репозитории, и примеру нужен только доступ на чтение во время разработки.
 */
function fixtures(): Plugin {
  return {
    name: 'doc-converter-fixtures',
    configureServer(server) {
      server.middlewares.use('/fixtures', (req, res, next) => {
        const name = decodeURIComponent((req.url ?? '').split('?')[0] ?? '').replace(/^\/+/, '');
        const slash = name.indexOf('/');
        const kind = slash < 0 ? '' : name.slice(0, slash);
        const relative = slash < 0 ? '' : name.slice(slash + 1);
        // Неизвестный корень — не наша фикстура: пусть запрос обработает дальше.
        const root = Object.hasOwn(ROOTS, kind) ? ROOTS[kind] : undefined;
        if (!root) {
          next();
          return;
        }
        if (relative === 'index.json') {
          void root.names().then((names) => {
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify(names));
          });
          return;
        }
        // Расширение чужого корня (`/fixtures/xlsx/…docx`) — тоже неизвестный
        // случай: имени такого файла в корне нет, отдаём запрос дальше.
        const full = relative.endsWith(`.${kind}`) ? fixturePath(root.dir, relative) : null;
        if (!full) {
          next();
          return;
        }
        readFile(full)
          .then((body) => {
            res.setHeader('Content-Type', root.mime);
            res.end(body);
          })
          .catch(next);
      });
    },
    // В dev фикстуры отдаёт middleware выше, а в собранном виде их иначе не
    // было бы вовсе: на Pages пример открывает документы тем же путём `/fixtures/`.
    async generateBundle() {
      const entries = Object.entries(ROOTS);
      for (const [kind, root] of entries) {
        const names = await root.names();
        for (const name of names) {
          this.emitFile({
            type: 'asset',
            fileName: `fixtures/${kind}/${name}`,
            source: await readFile(path.join(root.dir, name)),
          });
        }
        this.emitFile({
          type: 'asset',
          fileName: `fixtures/${kind}/index.json`,
          source: JSON.stringify(names),
        });
      }
    },
  };
}

/**
 * Возвращает точке входа `@doc-converter/browser-convert/worker` сайд-эффекты.
 *
 * Пакет объявлен как `sideEffects: false`, но его `/worker` — как раз голый
 * сайд-эффект: модуль подписывает воркер на сообщения и ничего не экспортирует
 * наружу. В сборке Vite верит `sideEffects: false` и выбрасывает модуль целиком —
 * чанк воркера получается пустым, а конвертация молча не работает. Здесь резолв
 * делегируется дальше по конвейеру, и только флаг сайд-эффектов переопределяется:
 * пакету следовало бы объявить исключение самому.
 *
 * Подключается в `worker.plugins`: воркеры собираются отдельным прогоном Rollup,
 * и плагины из общего списка `plugins` в него не попадают.
 */
function keepConvertWorkerSideEffects(): Plugin {
  return {
    name: 'doc-converter-keep-convert-worker-side-effects',
    enforce: 'pre',
    async resolveId(source, importer, options) {
      if (source !== '@doc-converter/browser-convert/worker') return null;
      // skipSelf: без него резолв вернулся бы в этот же плагин и зациклился.
      const resolved = await this.resolve(source, importer, { ...options, skipSelf: true });
      if (resolved === null) return null;
      return { id: resolved.id, moduleSideEffects: true };
    },
  };
}

export default defineConfig({
  // Pages отдаёт проект по `/wasm-converter/`, значение базы приходит из
  // workflow; локально и в тестах база остаётся корнем.
  base: process.env.VITE_BASE ?? '/',
  plugins: [react(), fixtures()],
  server: {
    // SharedArrayBuffer доступен только в cross-origin isolated окружении.
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
  preview: {
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
  worker: {
    format: 'es',
    plugins: () => [keepConvertWorkerSideEffects()],
  },
});
