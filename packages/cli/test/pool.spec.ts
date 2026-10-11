import { beforeEach, describe, expect, it } from 'vitest';

import type { ConvertJob } from '../src/convert.js';
import { WorkerPool, type WorkerLike } from '../src/pool.js';
import type { WorkerRequest, WorkerResponse } from '../src/worker-protocol.js';

/** Воркер-двойник: отвечает асинхронно, чтобы пул успел раздать очередь. */
class FakeWorker implements WorkerLike {
  static instances: FakeWorker[] = [];
  readonly #listeners = new Map<string, ((payload: never) => void)[]>();
  terminated = false;

  constructor(private readonly handler: (job: ConvertJob) => Promise<unknown>) {
    FakeWorker.instances.push(this);
  }

  postMessage(message: WorkerRequest): void {
    void this.handler(message.job).then(
      (bytes) =>
        this.#emit('message', {
          id: message.id,
          ok: true,
          result: { input: message.job.input, output: message.job.output, bytes: bytes as number, ms: 1 },
        } satisfies WorkerResponse),
      (error: unknown) => this.#emit('message', { id: message.id, ok: false, error: String(error) } satisfies WorkerResponse),
    );
  }

  on(event: 'message', listener: (message: WorkerResponse) => void): void;
  on(event: 'error', listener: (error: Error) => void): void;
  on(event: 'exit', listener: (code: number) => void): void;
  on(event: string, listener: (payload: never) => void): void {
    const listeners = this.#listeners.get(event) ?? [];
    listeners.push(listener);
    this.#listeners.set(event, listeners);
  }

  terminate(): Promise<number> {
    this.terminated = true;
    return Promise.resolve(0);
  }

  /** Сымитировать падение воркера — то, что Node сообщает событием `exit`. */
  exit(code: number): void {
    this.#emit('exit', code);
  }

  #emit(event: string, payload: unknown): void {
    for (const listener of this.#listeners.get(event) ?? []) listener(payload as never);
  }
}

const job = (name: string): ConvertJob => ({ input: name, output: `${name}.pdf`, format: 'pdf', pdf: {} });

describe('WorkerPool', () => {
  beforeEach(() => {
    FakeWorker.instances = [];
  });

  it('раздаёт задачи по воркерам и собирает ответы', async () => {
    const seen = new Map<string, number>();
    const pool = new WorkerPool(
      3,
      () =>
        new FakeWorker(async (task) => {
          seen.set(task.input, (seen.get(task.input) ?? 0) + 1);
          return 100;
        }),
    );

    const results = await Promise.all([job('a.docx'), job('b.docx'), job('c.docx'), job('d.docx')].map((task) => pool.run(task)));
    await pool.close();

    expect(results.map((result) => result.output)).toEqual(['a.docx.pdf', 'b.docx.pdf', 'c.docx.pdf', 'd.docx.pdf']);
    expect(seen.size).toBe(4);
  });

  it('ошибка одного файла не мешает остальным', async () => {
    const pool = new WorkerPool(2, () => new FakeWorker(async (task) => Promise.reject(new Error(`boom ${task.input}`))));

    const outcomes = await Promise.allSettled([job('a.docx'), job('b.docx')].map((task) => pool.run(task)));
    await pool.close();

    expect(outcomes.map((outcome) => outcome.status)).toEqual(['rejected', 'rejected']);
    expect(String((outcomes[0] as PromiseRejectedResult).reason)).toContain('boom a.docx');
  });

  it('заменяет воркер, упавший во время задачи', async () => {
    // `stall.docx` не отвечает никогда — воркер «умирает» с задачей в руках.
    const pool = new WorkerPool(1, () =>
      new FakeWorker(async (task) => (task.input.startsWith('stall') ? new Promise<never>(() => {}) : 1)),
    );
    const crashed = pool.run(job('stall.docx'));
    await Promise.resolve();
    FakeWorker.instances[0]!.exit(1);

    await expect(crashed).rejects.toThrow(/exited with code 1/);

    const next = await pool.run(job('b.docx'));
    await pool.close();

    expect(FakeWorker.instances.length).toBe(2);
    expect(next.input).toBe('b.docx');
  });

  it('после close задачи не принимаются', async () => {
    const pool = new WorkerPool(1, () => new FakeWorker(async () => 1));
    await pool.close();
    await expect(pool.run(job('a.docx'))).rejects.toThrow(/closed/);
  });
});
