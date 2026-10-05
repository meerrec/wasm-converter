import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { defineConfig, type Plugin } from 'vite';

const FIXTURES = path.resolve(import.meta.dirname, '../../test-fixtures/xlsx');

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
          void readdir(FIXTURES).then((names) => {
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify(names.filter((n) => n.endsWith('.xlsx')).sort()));
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
  };
}

export default defineConfig({
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
