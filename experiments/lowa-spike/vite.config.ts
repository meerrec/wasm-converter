// Конфиг харнесса спайка LOWA: раздача страницы с COOP/COEP и зеркало чужих артефактов.
//
// Зачем COOP/COEP: LOWA — Emscripten-сборка с pthreads, а они стоят на SharedArrayBuffer,
// который браузер отдаёт только в cross-origin isolated окружении (runtime-contract §4).
// Обычный статический сервер заголовков не ставит, поэтому харнесс ходит через Vite.
//
// Зачем зеркало: браузер грузит ~50 МБ (soffice.wasm ~36 МБ br, soffice.data ~16 МБ br) на
// каждом холодном старте, а CDN отвечает `cache-control: max-age=180`. Плагин mirror отдаёт
// байты с того же origin и с тем же content-encoding, что и CDN, — страница при этом грузит всё
// с собственного origin, то есть CORS/CORP не участвуют вовсе.
//
// Источник байтов — репозиторий: public/lowa/ (Git LFS, `git lfs pull` после клона). Кэш в
// LOWA_CACHE_DIR остался резервом: он и файл с CDN докачает, и даст внятную причину, если
// копия в репозитории приехала не целиком.
//
// Запуск: pnpm -C experiments/lowa-spike dev   (см. docs/sprint-13/harness.md)

import { createReadStream, existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs';
import { get as httpsGet } from 'node:https';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const MIRROR_PREFIX = '/lowa/';
const CDN = 'https://cdn.zetaoffice.net/zetaoffice_latest/';
// zeta.js на CDN нет — демо ZetaOffice везёт его у себя (MIT, файл не наш, поэтому только зеркало).
const DEMO_ASSETS = 'https://zetaoffice.net/demos/convertpdf/assets/vendor/zetajs/';

type Artifact = {
  upstream: string;
  contentType: string;
  /** Копия в репозитории: ожидаемый размер и кодировка байтов на диске. */
  repo: { bytes: number; contentEncoding?: string };
};

// Единственная таблица артефактов: что отдавать и откуда брать. Копии в репозитории — ровно те
// байты, что отдаёт CDN, то есть сжатые brotli, поэтому кодировка объявлена статически: в
// отличие от кэша, рядом с ними нет meta.json, где она была бы записана.
const ARTIFACTS: Record<string, Artifact> = {
  'soffice.js': {
    upstream: CDN,
    contentType: 'text/javascript; charset=utf-8',
    repo: { bytes: 157_580, contentEncoding: 'br' },
  },
  'soffice.wasm': {
    upstream: CDN,
    contentType: 'application/wasm',
    repo: { bytes: 36_279_250, contentEncoding: 'br' },
  },
  'soffice.data': {
    upstream: CDN,
    contentType: 'application/octet-stream',
    repo: { bytes: 15_891_013, contentEncoding: 'br' },
  },
  'soffice.data.js.metadata': {
    upstream: CDN,
    contentType: 'text/plain; charset=utf-8',
    repo: { bytes: 215_180 },
  },
  'zeta.js': {
    upstream: DEMO_ASSETS,
    contentType: 'text/javascript; charset=utf-8',
    repo: { bytes: 8_645, contentEncoding: 'br' },
  },
};

/** Копии артефактов в репозитории — рядом с этим конфигом, так что путь не зависит от cwd. */
const REPO_DIR = fileURLToPath(new URL('./public/lowa/', import.meta.url));

const CACHE_DIR = process.env.LOWA_CACHE_DIR ?? '/tmp/lowa-harness-cache';

const ISOLATION_HEADERS: Record<string, string> = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
  'Cross-Origin-Resource-Policy': 'cross-origin',
  'Access-Control-Allow-Origin': '*',
};

type Cached = { contentType: string; contentEncoding?: string; bytes: number };

/** Что отдавать браузеру: файл на диске плюс заголовки к нему. */
type Served = Cached & { file: string; source: 'repo' | 'cache' };

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

/**
 * Копия из репозитория; null — брать нечего. Размер сверяется не для красоты: без
 * `git lfs pull` на диске лежит ~130-байтовый указатель, и отдать его как
 * `application/wasm` с `content-encoding: br` — значит уронить офис невнятной ошибкой
 * разбора вместо того, чтобы просто сходить в кэш за настоящими байтами.
 */
function fromRepo(name: string): Served | null {
  const artifact = ARTIFACTS[name];
  if (artifact === undefined) {
    return null;
  }
  const file = join(REPO_DIR, name);
  const { bytes, contentEncoding } = artifact.repo;
  if (!existsSync(file)) {
    return null;
  }
  const actual = statSync(file).size;
  if (actual !== bytes) {
    console.warn(
      `[lowa-mirror] ${name}: в репозитории ${actual} Б вместо ${bytes} Б ` +
        '(не выполнен git lfs pull?) — беру кэш',
    );
    return null;
  }
  return { file, source: 'repo', contentType: artifact.contentType, bytes: actual, ...(contentEncoding ? { contentEncoding } : {}) };
}

/** Кладёт артефакт в кэш, если его там ещё нет; возвращает метаданные для отдачи. */
async function ensureCached(name: string): Promise<Served> {
  const artifact = ARTIFACTS[name];
  if (artifact === undefined) {
    throw new Error(`no upstream for ${name}`);
  }
  const file = join(CACHE_DIR, name);
  const metaFile = `${file}.meta.json`;
  if (existsSync(file) && existsSync(metaFile)) {
    const meta = JSON.parse(readFileSync(metaFile, 'utf8')) as Cached;
    if (meta.bytes === statSync(file).size) {
      return { file, source: 'cache', ...meta };
    }
  }
  mkdirSync(CACHE_DIR, { recursive: true });
  const started = Date.now();
  const { body, contentEncoding } = await download(artifact.upstream + name);
  const tmp = `${file}.part`;
  writeFileSync(tmp, body);
  renameSync(tmp, file);
  const meta: Cached = {
    contentType: artifact.contentType,
    bytes: body.length,
    ...(contentEncoding ? { contentEncoding } : {}),
  };
  writeFileSync(metaFile, JSON.stringify(meta));
  console.log(
    `[lowa-mirror] ${name}: ${(body.length / 1048576).toFixed(1)} МБ` +
      `${contentEncoding ? ` (${contentEncoding})` : ''} за ${((Date.now() - started) / 1000).toFixed(1)} с с CDN`,
  );
  return { file, source: 'cache', ...meta };
}

/** Печатает источник артефакта один раз за процесс — чтобы в логе харнесса было видно, откуда байты. */
const reportedSources = new Set<string>();
function reportSource(name: string, served: Served): void {
  if (reportedSources.has(name)) {
    return;
  }
  reportedSources.add(name);
  console.log(
    `[lowa-mirror] ${name}: ${(served.bytes / 1048576).toFixed(1)} МБ` +
      `${served.contentEncoding ? ` (${served.contentEncoding})` : ''} из ` +
      `${served.source === 'repo' ? 'репозитория' : CACHE_DIR}`,
  );
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
        if (ARTIFACTS[name] === undefined) {
          res.statusCode = 404;
          res.end(`unknown artifact: ${name}`);
          return;
        }
        Promise.resolve(fromRepo(name) ?? ensureCached(name))
          .then((served: Served) => {
            reportSource(name, served);
            for (const [key, value] of Object.entries(ISOLATION_HEADERS)) {
              res.setHeader(key, value);
            }
            res.setHeader('Content-Type', served.contentType);
            res.setHeader('Content-Length', String(served.bytes));
            res.setHeader('Cache-Control', 'public, max-age=86400');
            if (served.contentEncoding !== undefined) {
              res.setHeader('Content-Encoding', served.contentEncoding);
            }
            if (req.method === 'HEAD') {
              res.end();
              return;
            }
            createReadStream(served.file).pipe(res);
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
