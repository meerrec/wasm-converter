import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { defineConfig, type Plugin } from 'vite';

const FIXTURES = path.resolve(import.meta.dirname, '../../test-fixtures/xlsx');

/** Имена фикстур: один и тот же отбор и порядок в dev-сервере и в сборке. */
async function fixtureNames(): Promise<string[]> {
  const names = await readdir(FIXTURES);
  return names.filter((n) => n.endsWith('.xlsx')).sort();
}

/**
 * Отдаёт фикстуры по `/fixtures/<имя>.xlsx`.
 *
 * Складывать сотню книг в `public/` незачем: они и так лежат в репозитории, и
 * примеру нужен только доступ на чтение во время разработки.
 */
function fixtures(): Plugin {
  return {
    name: 'doc-converter-fixtures',
    configureServer(server) {
      server.middlewares.use('/fixtures', (req, res, next) => {
        const name = path.basename((req.url ?? '').split('?')[0] ?? '');
        if (name === 'index.json') {
          void fixtureNames().then((names) => {
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify(names));
          });
          return;
        }
        if (!name.endsWith('.xlsx')) {
          next();
          return;
        }
        readFile(path.join(FIXTURES, name))
          .then((body) => {
            res.setHeader('Content-Type', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet');
            res.end(body);
          })
          .catch(next);
      });
    },
    // В dev фикстуры отдаёт middleware выше, а в собранном виде их иначе не
    // было бы вовсе: на Pages пример открывает книги тем же путём `/fixtures/`.
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
