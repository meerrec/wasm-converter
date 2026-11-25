// Подъём и остановка dev-сервера демо.
//
// Сервер поднимает сам оракул, а не Playwright: захват идёт вне тестового
// раннера, и один сервер живёт на всю сессию (см. `index.ts`). Чужой сервер на
// том же порту не переиспользуется: он мог быть поднят из другой ревизии и
// отдавать другой wasm — тогда baseline молча смешал бы две сборки движка.

import { spawn, type ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
import { deferred } from './deferred.js';

/** Сколько строк вывода ребёнка держим для диагностики. */
const LOG_RING_LINES = 200;
/** Сколько ждать ответа сервера. Сборка wasm на холодную бывает долгой. */
const STARTUP_TIMEOUT_MS = 180_000;
/** Сколько ждать между опросами готовности. */
const POLL_INTERVAL_MS = 250;
/** Сколько живёт `SIGTERM` до `SIGKILL`. */
const KILL_GRACE_MS = 5_000;

/** Запущенный сервер: адрес и идемпотентная остановка. */
export interface DevServer {
  /** Базовый URL без завершающего слэша. */
  readonly url: string;
  /** Остановить сервер вместе с деревом процессов. Повторный вызов безопасен. */
  stop(): Promise<void>;
}

/** Параметры запуска. */
export interface StartDevServerOptions {
  /** Порт; занятый порт — ошибка, а не повод переиспользовать чужой сервер. */
  port: number;
  /** Каталог, из которого запускается `pnpm` (корень монорепозитория). */
  cwd: string;
}

/** Кольцевой буфер вывода: хвост нужен в сообщении о сбое старта. */
class LogRing {
  private readonly lines: string[] = [];

  push(chunk: string): void {
    for (const line of chunk.split(/\r?\n/)) {
      if (line.length === 0) continue;
      this.lines.push(line);
      if (this.lines.length > LOG_RING_LINES) this.lines.shift();
    }
  }

  tail(): string {
    return this.lines.join('\n');
  }
}

/** Свободен ли порт: занятый порт означает чужой сервер, а не наш. */
function isPortFree(port: number): Promise<boolean> {
  const { promise, resolve } = deferred<boolean>();
  const probe = createServer();
  probe.once('error', () => {
    resolve(false);
  });
  probe.once('listening', () => {
    probe.close(() => {
      resolve(true);
    });
  });
  probe.listen(port, '127.0.0.1');
  return promise;
}

/** Отвечает ли сервер на `/` хоть каким-то HTTP-статусом. */
async function isServing(url: string): Promise<boolean> {
  try {
    const response = await fetch(url, { redirect: 'manual' });
    // Любой ответ значит, что Vite слушает: 404 на `/` тоже ответ.
    return response.status > 0;
  } catch {
    return false;
  }
}

/** Пауза между опросами готовности. */
function delay(ms: number): Promise<void> {
  const { promise, resolve } = deferred<void>();
  setTimeout(() => {
    resolve();
  }, ms);
  return promise;
}

/** Убить дерево процессов: сначала мягко, затем жёстко. */
async function killTree(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const pid = child.pid;
  if (pid === undefined) return;

  const exited = new Promise<void>((resolve) => {
    child.once('exit', () => {
      resolve();
    });
  });

  // `pnpm` запускает Vite отдельным процессом: сигнал одному `pnpm` оставил бы
  // Vite жить и держать порт. Группа процессов создаётся через `detached`.
  try {
    process.kill(-pid, 'SIGTERM');
  } catch {
    try {
      child.kill('SIGTERM');
    } catch {
      // Процесс уже умер — это и есть цель.
    }
  }

  const { promise: grace, resolve: graceDone } = deferred<void>();
  const timer = setTimeout(graceDone, KILL_GRACE_MS);
  await Promise.race([exited, grace]);
  clearTimeout(timer);

  if (child.exitCode === null && child.signalCode === null) {
    try {
      process.kill(-pid, 'SIGKILL');
    } catch {
      try {
        child.kill('SIGKILL');
      } catch {
        // Уже нет процесса — нечего убивать.
      }
    }
    await Promise.race([exited, delay(1_000)]);
  }
}

/**
 * Поднять dev-сервер демо и дождаться, пока он начнёт отвечать.
 *
 * @throws {Error} порт занят, процесс не запустился или сервер не ответил за
 * 180 с; в сообщении — хвост stdout/stderr ребёнка.
 */
export async function startDevServer(options: StartDevServerOptions): Promise<DevServer> {
  const { port, cwd } = options;
  const url = `http://localhost:${port}`;

  if (!(await isPortFree(port))) {
    throw new Error(
      `port ${port} is already in use: refusing to reuse a server this harness did not start ` +
        '(a stale server would serve a stale wasm build and poison the baseline)',
    );
  }

  const log = new LogRing();
  const child = spawn(
    'pnpm',
    ['-F', '@meerrec/example-viewer', 'dev', '--port', String(port), '--strictPort'],
    {
      cwd,
      // Своя группа процессов: `stop()` гасит дерево целиком, а не только pnpm.
      detached: true,
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, FORCE_COLOR: '0' },
    },
  );

  child.stdout?.setEncoding('utf8');
  child.stderr?.setEncoding('utf8');
  child.stdout?.on('data', (chunk: string) => {
    log.push(chunk);
  });
  child.stderr?.on('data', (chunk: string) => {
    log.push(chunk);
  });

  // Ошибка запуска приходит колбэком, поэтому живёт в объекте: локальная
  // переменная после `await` сузилась бы до начального `null`.
  const spawnState: { error: Error | null } = { error: null };
  child.once('error', (error) => {
    spawnState.error = error;
  });

  let stopped = false;
  const stop = async (): Promise<void> => {
    if (stopped) return;
    stopped = true;
    await killTree(child);
  };

  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  for (;;) {
    if (spawnState.error !== null) {
      await stop();
      throw new Error(`failed to spawn the dev server: ${spawnState.error.message}\n${log.tail()}`);
    }
    if (child.exitCode !== null) {
      await stop();
      throw new Error(
        `dev server exited with code ${String(child.exitCode)} before becoming ready\n${log.tail()}`,
      );
    }
    if (await isServing(url)) return { url, stop };
    if (Date.now() > deadline) {
      await stop();
      throw new Error(
        `dev server did not answer at ${url} within ${STARTUP_TIMEOUT_MS} ms\n${log.tail()}`,
      );
    }
    await delay(POLL_INTERVAL_MS);
  }
}
