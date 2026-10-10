import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { defineConfig, type Plugin } from 'vite';

const FIXTURES = path.resolve(import.meta.dirname, '../../test-fixtures/docx');
const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';

/**
 * Имена фикстур — пути от корня фикстур, с категорией (`simple/one_paragraph.docx`):
 * документы лежат по подкаталогам, а не в один уровень, как книги XLSX.
 * Отбор и порядок одинаковы у dev-сервера и сборки.
 */
async function fixtureNames(dir = FIXTURES, prefix = ''): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const names: string[] = [];
  for (const entry of entries) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      names.push(...(await fixtureNames(path.join(dir, entry.name), relative)));
    } else if (entry.name.endsWith('.docx')) {
      names.push(relative);
    }
  }
  return names.sort();
}

/** Путь к фикстуре по имени из запроса; `null` — имени нельзя доверять. */
function fixturePath(name: string): string | null {
  // Имя приходит из URL: `..` в нём вывел бы за пределы каталога фикстур.
  if (name.includes('..') || name.startsWith('/')) return null;
  const full = path.resolve(FIXTURES, name);
  return full.startsWith(`${FIXTURES}${path.sep}`) ? full : null;
}

/**
 * Отдаёт фикстуры по `/fixtures/<категория>/<имя>.docx`.
 *
 * Складывать сотню документов в `public/` незачем: они и так лежат в
 * репозитории, и примеру нужен только доступ на чтение во время разработки.
 */
function fixtures(): Plugin {
  return {
    name: 'doc-converter-docx-fixtures',
    configureServer(server) {
      server.middlewares.use('/fixtures', (req, res, next) => {
        const name = decodeURIComponent((req.url ?? '').split('?')[0] ?? '').replace(/^\/+/, '');
        if (name === 'index.json') {
          void fixtureNames().then((names) => {
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify(names));
          });
          return;
        }
        const full = name.endsWith('.docx') ? fixturePath(name) : null;
        if (!full) {
          next();
          return;
        }
        readFile(full)
          .then((body) => {
            res.setHeader('Content-Type', DOCX_MIME);
            res.end(body);
          })
          .catch(next);
      });
    },
    // В dev фикстуры отдаёт middleware выше, а в собранном виде их иначе не
    // было бы вовсе: на Pages пример открывает документы тем же путём `/fixtures/`.
    async generateBundle() {
      const names = await fixtureNames();
      for (const name of names) {
        this.emitFile({
          type: 'asset',
          fileName: `fixtures/${name}`,
          source: await readFile(path.join(FIXTURES, name)),
        });
      }
      this.emitFile({
        type: 'asset',
        fileName: 'fixtures/index.json',
        source: JSON.stringify(names),
      });
    },
  };
}

export default defineConfig({
  // Pages отдаёт проект по `/wasm-converter/`, значение базы приходит из
  // workflow; локально и в тестах база остаётся корнем.
  base: process.env.VITE_BASE ?? '/',
  plugins: [fixtures()],
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
  worker: { format: 'es' },
});
