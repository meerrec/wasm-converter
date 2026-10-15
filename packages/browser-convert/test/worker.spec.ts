/**
 * Тесты входа конвертации в воркере: фабрика createConvertWorkerHandler гоняется без
 * настоящего воркера — `post` подменён шпионом, слушатель внизу worker.ts в node-окружении
 * не вешается (`self` нет). Рантайм x2t замокан целиком; X2tError в моке зеркалит реальный
 * класс из packages/x2t/src/x2t.ts — поля `code`/`stderr` с дефолтами -1/''.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@meerrec/x2t', () => ({
  loadX2t: vi.fn(),
  convert: vi.fn(),
  X2tError: class X2tError extends Error {
    readonly code: number;
    readonly stderr: string;

    constructor(message: string, code = -1, stderr = '') {
      super(message);
      this.name = 'X2tError';
      this.code = code;
      this.stderr = stderr;
    }
  },
}));

import { convert as convertWithX2t, loadX2t, X2tError } from '@meerrec/x2t';
import type { X2tModule } from '@meerrec/x2t';

import {
  createConvertWorkerHandler,
  type ConvertWorkerRequest,
  type ConvertWorkerResponse,
} from '../src/worker.js';

/** Модуль x2t в тестах — заглушка: рантайм сюда не заходит. */
const fakeModule = {} as unknown as X2tModule;

/** Запрос по умолчанию: URL'ы первого сообщения и кириллическое имя файла. */
const BASE_REQUEST: ConvertWorkerRequest = {
  kind: 'convert',
  id: 1,
  scriptUrl: '/x2t/x2t.js',
  wasmUrl: '/x2t/x2t.wasm',
  bytes: new Uint8Array([10, 20, 30]).buffer,
  from: 'docx',
  to: 'odt',
  fileName: 'отчёт.docx',
};

function request(overrides: Partial<ConvertWorkerRequest> = {}): ConvertWorkerRequest {
  return { ...BASE_REQUEST, ...overrides };
}

/** Шпион вместо postMessage воркера: складывает сообщения и списки переносимых буферов. */
function collect(): {
  messages: ConvertWorkerResponse[];
  transfers: Transferable[][];
  post: (message: ConvertWorkerResponse, transfer?: Transferable[]) => void;
} {
  const messages: ConvertWorkerResponse[] = [];
  const transfers: Transferable[][] = [];
  const post = (message: ConvertWorkerResponse, transfer?: Transferable[]): void => {
    messages.push(message);
    transfers.push(transfer ?? []);
  };
  return { messages, transfers, post };
}

/** Ждёт, пока асинхронный обработчик положит сообщение в шпион. */
async function nextMessage(collector: ReturnType<typeof collect>): Promise<ConvertWorkerResponse> {
  await vi.waitFor(() => expect(collector.messages.length).toBeGreaterThan(0));
  return collector.messages[0]!;
}

beforeEach(() => {
  vi.mocked(loadX2t).mockReset();
  vi.mocked(loadX2t).mockResolvedValue(fakeModule);
  vi.mocked(convertWithX2t).mockReset();
});

describe('createConvertWorkerHandler: успешная конвертация', () => {
  it('поднимает модуль с опциями первого сообщения и отдаёт result с переносом байтов', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({
      bytes: new Uint8Array([1, 2, 3]),
      code: 0,
      stderr: 'stderr строка',
    });
    const collector = collect();
    const handle = createConvertWorkerHandler(collector.post);

    handle({ data: request() });
    const message = await nextMessage(collector);

    expect(loadX2t).toHaveBeenCalledTimes(1);
    expect(loadX2t).toHaveBeenCalledWith({ scriptUrl: '/x2t/x2t.js', wasmUrl: '/x2t/x2t.wasm' });
    const [moduleArg, inputArg, optionsArg] = vi.mocked(convertWithX2t).mock.calls[0]!;
    expect(moduleArg).toBe(fakeModule);
    expect(inputArg).toEqual(new Uint8Array([10, 20, 30]));
    expect(optionsArg).toEqual({ from: 'docx', to: 'odt', fileName: 'отчёт.docx' });

    expect(message).toEqual({
      kind: 'result',
      id: 1,
      bytes: new Uint8Array([1, 2, 3]).buffer,
      code: 0,
      stderr: 'stderr строка',
      convertMs: expect.any(Number),
    });
    // Байты уехали через transfer — в списке переносимых тот же буфер, что в ответе.
    expect(message.kind).toBe('result');
    if (message.kind === 'result') {
      expect(message.bytes).toBeInstanceOf(ArrayBuffer);
      expect(collector.transfers[0]).toEqual([message.bytes]);
    }
  });

  it('без wasmUrl не передаёт это поле в loadX2t вовсе', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: new Uint8Array(0), code: 0, stderr: '' });
    const collector = collect();
    const handle = createConvertWorkerHandler(collector.post);

    handle({
      data: {
        kind: 'convert',
        id: 2,
        scriptUrl: '/x2t.js',
        bytes: new ArrayBuffer(0),
        from: 'docx',
        to: 'odt',
        fileName: 'a.docx',
      },
    });
    await nextMessage(collector);

    expect(loadX2t).toHaveBeenCalledTimes(1);
    const options = vi.mocked(loadX2t).mock.calls[0]![0]!;
    expect(options).toEqual({ scriptUrl: '/x2t.js' });
    expect('wasmUrl' in options).toBe(false);
  });

  it('второй запрос с другими URL не поднимает модуль заново и конвертирует', async () => {
    vi.mocked(convertWithX2t)
      .mockResolvedValueOnce({ bytes: new Uint8Array([1]), code: 0, stderr: '' })
      .mockResolvedValueOnce({ bytes: new Uint8Array([2]), code: 0, stderr: '' });
    const collector = collect();
    const handle = createConvertWorkerHandler(collector.post);

    handle({ data: request() });
    await nextMessage(collector);
    handle({ data: request({ id: 2, scriptUrl: '/другой/x2t.js', wasmUrl: '/другой/x2t.wasm' }) });
    await vi.waitFor(() => expect(collector.messages.length).toBe(2));

    expect(loadX2t).toHaveBeenCalledTimes(1);
    const second = collector.messages[1]!;
    expect(second).toMatchObject({ kind: 'result', id: 2 });
    if (second.kind === 'result') {
      expect(new Uint8Array(second.bytes)).toEqual(new Uint8Array([2]));
    }
  });

  it('ненулевой код конвертации не фильтруется', async () => {
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: new Uint8Array(0), code: 88, stderr: '' });
    const collector = collect();
    const handle = createConvertWorkerHandler(collector.post);

    handle({ data: request() });
    const message = await nextMessage(collector);

    expect(message).toMatchObject({ kind: 'result', id: 1, code: 88 });
  });
});

describe('createConvertWorkerHandler: отказы', () => {
  it('отказ loadX2t отдаёт error с message и полями ошибки', async () => {
    vi.mocked(loadX2t).mockRejectedValue(new X2tError('fetch failed', 404, 'stderr ошибки'));
    const collector = collect();
    const handle = createConvertWorkerHandler(collector.post);

    handle({ data: request() });
    const message = await nextMessage(collector);

    expect(convertWithX2t).not.toHaveBeenCalled();
    expect(message).toEqual({
      kind: 'error',
      id: 1,
      // stderr ошибки дописан отдельной строкой — поля stderr в ответе нет.
      message: 'fetch failed\nstderr ошибки',
      code: 404,
    });
  });

  it('отказ подъёма не кэшируется: следующий запрос пробует loadX2t снова и отдаёт result', async () => {
    vi.mocked(loadX2t)
      .mockRejectedValueOnce(new X2tError('fetch failed'))
      .mockResolvedValueOnce(fakeModule);
    vi.mocked(convertWithX2t).mockResolvedValue({ bytes: new Uint8Array([7]), code: 0, stderr: '' });
    const collector = collect();
    const handle = createConvertWorkerHandler(collector.post);

    handle({ data: request() });
    const first = await nextMessage(collector);
    expect(first).toMatchObject({ kind: 'error', id: 1 });

    // URL'ы второй попытки — из первого сообщения, а не из второго запроса.
    handle({ data: request({ id: 2, scriptUrl: '/другой/x2t.js' }) });
    await vi.waitFor(() => expect(collector.messages.length).toBe(2));

    expect(loadX2t).toHaveBeenCalledTimes(2);
    expect(vi.mocked(loadX2t).mock.calls[1]![0]).toEqual({
      scriptUrl: '/x2t/x2t.js',
      wasmUrl: '/x2t/x2t.wasm',
    });
    expect(collector.messages[1]).toMatchObject({ kind: 'result', id: 2, code: 0 });
  });

  it('runtime-ошибка convert без code/stderr отдаёт error с message и без code', async () => {
    vi.mocked(convertWithX2t).mockRejectedValue(new Error('x2t crashed: memory access out of bounds'));
    const collector = collect();
    const handle = createConvertWorkerHandler(collector.post);

    handle({ data: request() });
    const message = await nextMessage(collector);

    expect(message).toEqual({
      kind: 'error',
      id: 1,
      message: 'x2t crashed: memory access out of bounds',
    });
  });
});
