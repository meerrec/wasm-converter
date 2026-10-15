/**
 * Воркер харнесса: здесь живёт модуль x2t и происходит вся конвертация.
 *
 * Воркер модульный в обоих режимах (`{ type: 'module' }` на странице и `worker.format: 'es'`
 * в vite.config.ts): в dev Vite отдаёт воркеры без сборки, поэтому классический воркер упал бы
 * на первом же ESM-импорте. Модульному воркеру `importScripts` недоступен — glue x2t приезжает
 * через `fetch` и косвенный `eval` (см. `loadX2t` в `packages/x2t/src/x2t.ts`); проверено
 * прогоном. Обвязка при этом подтягивается динамическим `import()`, и он же инлайнится в бандл
 * воркера на сборке (`worker.rollupOptions` в vite.config.ts), так что воркер остаётся одним
 * файлом.
 *
 * Пакет подключён относительным путём, а не по имени `@meerrec/x2t`: харнесс лежит вне
 * pnpm-workspace, и имя пакета в нём не резолвится.
 *
 * Модуль поднимается по первому запросу конвертации (`ensureBoot`), а не при создании воркера:
 * страницу открывают и ради просмотра готового `.docx`, которому x2t не нужен.
 */
import type { X2tModule } from '../../../packages/x2t/src/index.js';
import { PROTOCOL, type MainToWorker, type WorkerToMain } from './protocol.js';

/** Тип модуля-обвязки: берём из динамического импорта, чтобы не импортировать его статически. */
type Wrapper = typeof import('../../../packages/x2t/src/index.js');

/**
 * Артефакты лежат в `packages/x2t/vendor/` (Git LFS) и раздаются как `publicDir` харнесса,
 * то есть файлами в корне — отсюда `/x2t.js`, а не `/x2t/x2t.js` (см. vite.config.ts).
 */
const SCRIPT_URL = '/x2t.js';
const WASM_URL = '/x2t.wasm';

let wrapperPromise: Promise<Wrapper> | null = null;
let x2t: X2tModule | null = null;
let bootPromise: Promise<X2tModule> | null = null;

function getWrapper(): Promise<Wrapper> {
  wrapperPromise ??= import('../../../packages/x2t/src/index.js');
  return wrapperPromise;
}

function post(message: WorkerToMain, transfer: Transferable[] = []): void {
  self.postMessage(message, transfer);
}

function describe(error: unknown): string {
  return error instanceof Error ? `${error.name}: ${error.message}` : String(error);
}

async function boot(): Promise<X2tModule> {
  const startedAt = performance.now();
  post({ protocol: PROTOCOL, kind: 'log', line: 'поднимаю модуль x2t…' });
  const { loadX2t } = await getWrapper();
  const module = await loadX2t({
    scriptUrl: SCRIPT_URL,
    wasmUrl: WASM_URL,
    onPrint: (line: string) => post({ protocol: PROTOCOL, kind: 'log', line }),
  });
  post({ protocol: PROTOCOL, kind: 'ready', bootMs: performance.now() - startedAt });
  return module;
}

/**
 * Подъём модуля по первому требованию: 38 МБ были бы ценой входа на страницу даже тогда, когда
 * её открыли ради просмотра готового `.docx` — а он идёт своим вьюером (`src/viewer.ts`) и в x2t
 * не заходит. Неудача не кэшируется: следующая конвертация попробует поднять модуль снова, иначе
 * один сетевой сбой навсегда ломал бы конвертацию (тот же расчёт, что у ленивого DOCX-модуля в
 * `packages/core/src/worker/worker.ts`).
 */
function ensureBoot(): Promise<X2tModule> {
  if (x2t !== null) return Promise.resolve(x2t);
  bootPromise ??= boot().then(
    (module) => {
      x2t = module;
      return module;
    },
    (error: unknown) => {
      bootPromise = null;
      throw error;
    },
  );
  return bootPromise;
}

/** Расширение файла без точки, в нижнем регистре — по нему конвертер выбирает входной формат. */
function extensionOf(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  return dot === -1 ? '' : fileName.slice(dot + 1).toLowerCase();
}

async function onConvert(request: MainToWorker): Promise<void> {
  const from = extensionOf(request.fileName);
  if (from === '') {
    post({ protocol: PROTOCOL, kind: 'error', message: `file has no extension: ${request.fileName}` });
    return;
  }

  const stem = request.fileName.slice(0, -(from.length + 1));
  const outputName = `${stem}.${request.to}`;
  const startedAt = performance.now();
  try {
    const module = await ensureBoot();
    const { convert } = await getWrapper();
    const result = await convert(module, new Uint8Array(request.bytes), {
      from,
      to: request.to,
      fileName: request.fileName,
    });
    post(
      {
        protocol: PROTOCOL,
        kind: 'result',
        id: request.id,
        outputName,
        code: result.code,
        stderr: result.stderr,
        convertMs: performance.now() - startedAt,
        bytes: result.bytes.buffer,
      },
      [result.bytes.buffer],
    );
  } catch (error) {
    // Сюда попадают и отказы рантайма (abort), и наши X2tError — с кодом и stderr, если они есть.
    const code = error instanceof Error && 'code' in error ? Number(error.code) : -1;
    const stderr = error instanceof Error && 'stderr' in error ? String(error.stderr) : '';
    post(
      {
        protocol: PROTOCOL,
        kind: 'result',
        id: request.id,
        outputName,
        code,
        stderr: stderr === '' ? describe(error) : stderr,
        convertMs: performance.now() - startedAt,
        bytes: new ArrayBuffer(0),
      },
      [],
    );
  }
}

self.addEventListener('message', (event: MessageEvent<MainToWorker>) => {
  const message = event.data;
  if (message.protocol !== PROTOCOL) {
    post({ protocol: PROTOCOL, kind: 'error', message: `unexpected protocol: ${message.protocol}` });
    return;
  }
  void onConvert(message);
});
