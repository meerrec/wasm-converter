// Конфиг харнесса спайка LOWA: раздача страницы с COOP/COEP и зеркало чужих артефактов.
//
// Зачем COOP/COEP: LOWA — Emscripten-сборка с pthreads, а они стоят на SharedArrayBuffer,
// который браузер отдаёт только в cross-origin isolated окружении (runtime-contract §4).
// Обычный статический сервер заголовков не ставит, поэтому харнесс ходит через Vite.
//
// Зачем зеркало: браузер грузит ~50 МБ (soffice.wasm ~36 МБ br, soffice.data ~16 МБ br) на
// каждом холодном старте, а CDN отвечает `cache-control: max-age=180`. Плагин mirror кэширует
// байты в LOWA_CACHE_DIR и отдаёт их тем же content-encoding, что и CDN, — страница при этом
// грузит всё с собственного origin, то есть CORS/CORP не участвуют вовсе.
//
// Запуск: pnpm -C experiments/lowa-spike dev   (см. docs/sprint-13/harness.md)

import { createReadStream, existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs';
import { get as httpsGet } from 'node:https';
import { join } from 'node:path';

const MIRROR_PREFIX = '/lowa/';
const CDN = 'https://cdn.zetaoffice.net/zetaoffice_latest/';
// zeta.js на CDN нет — демо ZetaOffice везёт его у себя (MIT, файл не наш, поэтому только зеркало).
const DEMO_ASSETS = 'https://zetaoffice.net/demos/convertpdf/assets/vendor/zetajs/';

const UPSTREAM: Record<string, string> = {
  'soffice.js': CDN,
  'soffice.wasm': CDN,
  'soffice.data': CDN,
  'soffice.data.js.metadata': CDN,
  'zeta.js': DEMO_ASSETS,
};

const CONTENT_TYPES: Record<string, string> = {
  'soffice.js': 'text/javascript; charset=utf-8',
  'soffice.wasm': 'application/wasm',
  'soffice.data': 'application/octet-stream',
  'soffice.data.js.metadata': 'text/plain; charset=utf-8',
  'zeta.js': 'text/javascript; charset=utf-8',
};

const CACHE_DIR = process.env.LOWA_CACHE_DIR ?? '/tmp/lowa-harness-cache';

const ISOLATION_HEADERS: Record<string, string> = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
  'Cross-Origin-Resource-Policy': 'cross-origin',
  'Access-Control-Allow-Origin': '*',
};

type Cached = { contentType: string; contentEncoding?: string; bytes: number };

/** Скачивает один артефакт у upstream как есть, не давая Node распаковывать тело. */
function download(url: string): Promise<{ body: Buffer; contentEncoding?: string }> {
  return new Promise((resolve, reject) => {
    const req = httpsGet(
      url,
      // Просим только те кодировки, которые понимает Chromium; ответ отдаём браузеру как есть,
      // чтобы он распаковывал сам (иначе пришлось бы держать на диске 161 МБ распакованного wasm).
      { headers: { 'accept-encoding': 'br, gzip', 'user-agent': 'lowa-spike-harness' } },
      (res) => {
        const status = res.statusCode ?? 0;
        if (status === 301 || status === 302 || status === 307 || status === 308) {
          res.resume();
          const location = res.headers.location;
          if (!location) {
            reject(new Error(`redirect without location from ${url}`));
            return;
          }
          download(new URL(location, url).href).then(resolve, reject);
          return;
        }
        if (status !== 200) {
          res.resume();
          reject(new Error(`GET ${url} -> ${status}`));
          return;
        }
        const chunks: Buffer[] = [];
        res.on('data', (chunk: Buffer) => chunks.push(chunk));
        res.on('end', () => {
          const encoding = res.headers['content-encoding'];
          if (encoding !== undefined && encoding !== 'br' && encoding !== 'gzip' && encoding !== 'identity') {
            reject(new Error(`unexpected content-encoding ${encoding} from ${url}`));
            return;
          }
          resolve({
            body: Buffer.concat(chunks),
            ...(encoding === undefined || encoding === 'identity' ? {} : { contentEncoding: encoding }),
          });
        });
        res.on('error', reject);
      },
    );
    req.on('error', reject);
  });
}

/** Кладёт артефакт в кэш, если его там ещё нет; возвращает метаданные для отдачи. */
async function ensureCached(name: string): Promise<Cached> {
  const file = join(CACHE_DIR, name);
  const metaFile = `${file}.meta.json`;
  const type = CONTENT_TYPES[name] ?? 'application/octet-stream';
  if (existsSync(file) && existsSync(metaFile)) {
    const meta = JSON.parse(readFileSync(metaFile, 'utf8')) as Cached;
    if (meta.bytes === statSync(file).size) {
      return meta;
    }
  }
  mkdirSync(CACHE_DIR, { recursive: true });
  const upstream = UPSTREAM[name];
  if (upstream === undefined) {
    throw new Error(`no upstream for ${name}`);
  }
  const started = Date.now();
  const { body, contentEncoding } = await download(upstream + name);
  const tmp = `${file}.part`;
  writeFileSync(tmp, body);
  renameSync(tmp, file);
  const meta: Cached = { contentType: type, bytes: body.length, ...(contentEncoding ? { contentEncoding } : {}) };
  writeFileSync(metaFile, JSON.stringify(meta));
  console.log(
    `[lowa-mirror] ${name}: ${(body.length / 1048576).toFixed(1)} МБ` +
      `${contentEncoding ? ` (${contentEncoding})` : ''} за ${((Date.now() - started) / 1000).toFixed(1)} с`,
  );
  return meta;
}

/** Раздаёт закэшированные артефакты по /lowa/<имя> с заголовками изоляции. */
function lowaMirror() {
  return {
    name: 'lowa-mirror',
    configureServer(server: any) {
      server.middlewares.use((req: any, res: any, next: any) => {
        const url: string = req.url ?? '';
        if (!url.startsWith(MIRROR_PREFIX)) {
          next();
          return;
        }
        const name = decodeURIComponent(url.slice(MIRROR_PREFIX.length).split('?')[0]);
        if (UPSTREAM[name] === undefined) {
          res.statusCode = 404;
          res.end(`unknown artifact: ${name}`);
          return;
        }
        ensureCached(name)
          .then((meta) => {
            for (const [key, value] of Object.entries(ISOLATION_HEADERS)) {
              res.setHeader(key, value);
            }
            res.setHeader('Content-Type', meta.contentType);
            res.setHeader('Content-Length', String(meta.bytes));
            res.setHeader('Cache-Control', 'public, max-age=86400');
            if (meta.contentEncoding !== undefined) {
              res.setHeader('Content-Encoding', meta.contentEncoding);
            }
            if (req.method === 'HEAD') {
              res.end();
              return;
            }
            createReadStream(join(CACHE_DIR, name)).pipe(res);
          })
          .catch((error: Error) => {
            console.error(`[lowa-mirror] ${name}: ${error.message}`);
            res.statusCode = 502;
            res.end(`mirror failed: ${error.message}`);
          });
      });
    },
  };
}

export default {
  plugins: [lowaMirror()],
  server: {
    port: Number(process.env.LOWA_PORT ?? 5199),
    strictPort: true,
    // SharedArrayBuffer доступен только в cross-origin isolated окружении.
    headers: ISOLATION_HEADERS,
  },
  preview: {
    port: Number(process.env.LOWA_PORT ?? 5199),
    strictPort: true,
    headers: ISOLATION_HEADERS,
  },
};
