/**
 * Тесты клиента Embed API: очередь команд до `document:ready`, матчинг ответов
 * по id, отмена устаревших команд (`supersede`), transferables и разбор payload.
 *
 * Редактора здесь нет: `postMessage` iframe'а — записывающая заглушка, а входящие
 * сообщения подаются событием `message` на `window` с теми самыми `origin` и
 * `source`, которые клиент проверяет, — иначе тест проверял бы мок, а не отсев.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createEditorClient, describeEditorError } from '../src/index.js';
import type {
  DocumentErrorEvent,
  DocumentSavedEvent,
  DocumentStateEvent,
  EditorClient,
  EditorEvent,
} from '../src/index.js';

/** Origin страницы в jsdom; клиент по умолчанию считает origin'ом редактора его же. */
const PAGE_ORIGIN = window.location.origin;

/** Команда, как её увидел iframe: аргументы `postMessage` целиком, без потерь. */
interface SentCommand {
  readonly id: string;
  readonly type: string;
  readonly payload: Record<string, unknown>;
  /** Второй аргумент `postMessage` — targetOrigin. */
  readonly origin: string;
  /** Третий аргумент `postMessage` — transferables. */
  readonly transfer: readonly Transferable[];
}

interface Harness {
  readonly client: EditorClient;
  /** Команды, дошедшие до iframe, в порядке отправки. */
  readonly sent: SentCommand[];
  /** События, доставленные слушателю клиента, в порядке доставки. */
  readonly events: EditorEvent[];
  /** Событие редактора: `message` на window; `from` переопределяет origin/source. */
  readonly emit: (data: unknown, from?: { origin?: string; source?: unknown }) => void;
}

/** Клиент поверх мока iframe: `contentWindow.postMessage` пишет вызовы в `sent`. */
function createHarness(options?: { origin?: string }): Harness {
  const sent: SentCommand[] = [];
  const target = {
    postMessage(
      message: { id: string; type: string; payload: Record<string, unknown> },
      origin: string,
      transfer?: Transferable[],
    ): void {
      sent.push({ ...message, origin, transfer: transfer ?? [] });
    },
  };
  const frame = { contentWindow: target } as unknown as HTMLIFrameElement;
  const client = createEditorClient(frame, options);
  const events: EditorEvent[] = [];
  client.onEvent((event) => events.push(event));
  const editorOrigin = options?.origin ?? PAGE_ORIGIN;
  const emit = (data: unknown, from: { origin?: string; source?: unknown } = {}): void => {
    window.dispatchEvent(
      new MessageEvent('message', {
        data,
        origin: from.origin ?? editorOrigin,
        source: (from.source ?? target) as MessageEventSource,
      }),
    );
  };
  return { client, sent, events, emit };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('очередь до document:ready', () => {
  it('копит команды до ready и отправляет их после, сохраняя порядок', () => {
    const { client, sent, events, emit } = createHarness();
    const bytes = new Uint8Array([1, 2, 3]);

    client.openBuffer('отчёт.docx', bytes, true);
    client.setReadonly(false);
    expect(sent).toHaveLength(0);

    emit({ type: 'document:ready' });
    expect(events.map((event) => event.type)).toEqual(['document:ready']);
    expect(sent.map((command) => command.type)).toEqual(['document:open-buffer', 'document:set-readonly']);
    expect(sent[0]!.payload['fileName']).toBe('отчёт.docx');
    expect(sent[0]!.payload['buffer']).toBe(bytes);
    expect(sent[0]!.payload['readonly']).toBe(true);
    expect(sent[1]!.payload).toEqual({ readonly: false });
    // id — свой, у каждой команды свой: по нему матчится ответ.
    expect(sent[0]!.id).toMatch(/^eo-/);
    expect(sent[0]!.id).not.toBe(sent[1]!.id);
  });

  it('после ready отправляет команду сразу, без очереди', () => {
    const { client, sent, emit } = createHarness();
    emit({ type: 'document:ready' });

    client.getState();
    expect(sent.map((command) => command.type)).toEqual(['document:get-state']);
  });
});

describe('origin редактора', () => {
  it('по умолчанию шлёт команды на собственный origin страницы', () => {
    const { client, sent, emit } = createHarness();
    emit({ type: 'document:ready' });
    client.getState();

    expect(sent[0]!.origin).toBe(PAGE_ORIGIN);
  });

  it('с options.origin шлёт на него и слушает только его', () => {
    const editorOrigin = 'https://editor.example';
    const { client, sent, events, emit } = createHarness({ origin: editorOrigin });

    emit({ type: 'document:ready' }); // origin по умолчанию в emit — editorOrigin
    expect(events).toHaveLength(1);
    client.getState();
    expect(sent[0]!.origin).toBe(editorOrigin);

    emit({ type: 'document:ready' }, { origin: 'https://evil.example' });
    expect(events).toHaveLength(1);
  });
});

describe('отсев посторонних сообщений', () => {
  it('игнорирует сообщение от чужого source', () => {
    const { events, emit } = createHarness();
    emit({ type: 'document:ready' }, { source: {} });

    expect(events).toEqual([]);
  });

  it('игнорирует сообщение не формата редактора', () => {
    const { events, emit } = createHarness();
    for (const data of ['строка', 42, null, { type: 'другое:событие' }, { payload: {} }]) {
      emit(data);
    }

    expect(events).toEqual([]);
  });

  it('логирует неизвестное document:* и не доставляет его как событие', () => {
    const debug = vi.spyOn(console, 'debug').mockImplementation(() => {});
    const { events, emit } = createHarness();

    emit({ type: 'document:future-thing', payload: { шаг: 1 } });

    expect(events).toEqual([]);
    expect(debug).toHaveBeenCalledWith('событие редактора:', 'document:future-thing', { шаг: 1 });
  });
});

describe('матчинг ответов по id', () => {
  it('доставляет ответ на отправленную команду и не доставляет повторный', () => {
    const { client, sent, events, emit } = createHarness();
    emit({ type: 'document:ready' });
    client.openBuffer('a.docx', new Uint8Array([1]), true);
    const id = sent[0]!.id;

    emit({ type: 'document:opened', id, payload: { fileName: 'a.docx' } });
    expect(events.map((event) => event.type)).toEqual(['document:ready', 'document:opened']);

    // Повторный ответ с тем же id — уже не ответ: команда снята с учёта.
    emit({ type: 'document:opened', id, payload: { fileName: 'a.docx' } });
    expect(events.map((event) => event.type)).toEqual(['document:ready', 'document:opened']);
  });

  it('не доставляет запоздалый ответ отменённой команды (supersede)', () => {
    const { client, sent, events, emit } = createHarness();
    emit({ type: 'document:ready' });
    client.openBuffer('первый.docx', new Uint8Array([1]), true);
    const stale = sent[0]!.id;
    client.openBuffer('второй.docx', new Uint8Array([2]), true);
    const current = sent[1]!.id;
    expect(stale).not.toBe(current);

    emit({ type: 'document:opened', id: stale, payload: { fileName: 'первый.docx' } });
    expect(events.map((event) => event.type)).toEqual(['document:ready']);

    emit({ type: 'document:opened', id: current, payload: { fileName: 'второй.docx' } });
    expect(events.map((event) => event.type)).toEqual(['document:ready', 'document:opened']);
  });

  it('отменяет только команды того же рода: открытие не снимает сохранение', () => {
    const { client, sent, events, emit } = createHarness();
    emit({ type: 'document:ready' });
    client.openBuffer('a.docx', new Uint8Array([1]), false);
    const staleOpen = sent[0]!.id;
    client.save({ targetExt: 'docx' });
    const saving = sent[1]!.id;
    client.openBuffer('b.docx', new Uint8Array([2]), false);
    const currentOpen = sent[2]!.id;

    emit({ type: 'document:saved', id: saving, payload: { fileName: 'a.docx' } });
    emit({ type: 'document:opened', id: staleOpen, payload: { fileName: 'a.docx' } });
    emit({ type: 'document:opened', id: currentOpen, payload: { fileName: 'b.docx' } });

    expect(events.map((event) => event.type)).toEqual(['document:ready', 'document:saved', 'document:opened']);
  });

  it('доставляет событие без id как есть: широковещательный ready', () => {
    const { events, emit } = createHarness();
    emit({ type: 'document:ready', payload: {} });

    expect(events).toEqual([{ type: 'document:ready', payload: {} }]);
  });

  it('доставляет событие с чужим id как есть: id может не вернуться', () => {
    const { events, emit } = createHarness();
    emit({ type: 'document:opened', id: 'чужой-id', payload: { fileName: 'a.docx' } });

    expect(events).toEqual([{ type: 'document:opened', id: 'чужой-id', payload: { fileName: 'a.docx' } }]);
  });
});

describe('transferables', () => {
  it('передаёт подложку байтов, когда срез покрывает её целиком', () => {
    const { client, sent, emit } = createHarness();
    emit({ type: 'document:ready' });
    const bytes = new Uint8Array([1, 2, 3]);

    client.openBuffer('a.docx', bytes, false);

    expect(sent[0]!.transfer).toHaveLength(1);
    expect(sent[0]!.transfer[0]).toBe(bytes.buffer);
  });

  it('передаёт `ArrayBuffer` целиком', () => {
    const { client, sent, emit } = createHarness();
    emit({ type: 'document:ready' });
    const buffer = new ArrayBuffer(8);

    client.openBuffer('a.docx', buffer, false);

    expect(sent[0]!.transfer).toHaveLength(1);
    expect(sent[0]!.transfer[0]).toBe(buffer);
  });

  it('копирует срез буфера: transfer отдал бы редактору чужие байты', () => {
    const { client, sent, emit } = createHarness();
    emit({ type: 'document:ready' });
    const slice = new Uint8Array([1, 2, 3, 4]).subarray(1, 3);

    client.openBuffer('a.docx', slice, false);

    expect(sent[0]!.payload['buffer']).toBe(slice);
    expect(sent[0]!.transfer).toEqual([]);
  });

  it('не transfer\'ит команды без байтов', () => {
    const { client, sent, emit } = createHarness();
    emit({ type: 'document:ready' });

    client.setReadonly(true);
    client.save();

    expect(sent.map((command) => command.transfer)).toEqual([[], []]);
  });
});

describe('остальные команды', () => {
  it('формирует тип и payload каждого метода', () => {
    const { client, sent, emit } = createHarness();
    emit({ type: 'document:ready' });
    const file = new File([new Uint8Array([1])], 'a.docx');

    client.openFile(file, false);
    client.openUrl('https://example.test/a.docx', 'a.docx', true);
    client.setReadonly(true);
    client.save();
    client.save({ targetExt: 'docx', returnOriginalOnTimeout: true });
    client.getState();

    expect(sent.map((command) => command.type)).toEqual([
      'document:open-file',
      'document:open-url',
      'document:set-readonly',
      'document:save',
      'document:save',
      'document:get-state',
    ]);
    expect(sent[0]!.payload['file']).toBe(file);
    expect(sent[0]!.payload['readonly']).toBe(false);
    expect(sent[1]!.payload).toEqual({
      url: 'https://example.test/a.docx',
      fileName: 'a.docx',
      readonly: true,
    });
    expect(sent[2]!.payload).toEqual({ readonly: true });
    expect(sent[3]!.payload).toEqual({});
    expect(sent[4]!.payload).toEqual({ targetExt: 'docx', returnOriginalOnTimeout: true });
    expect(sent[5]!.payload).toEqual({});
  });
});

describe('разбор payload событий', () => {
  it('приводит код ошибки, пришедший строкой, к числу и сохраняет сообщение', () => {
    const { events, emit } = createHarness();

    emit({ type: 'document:error', payload: { code: '-85', message: 'содержимое не совпадает' } });

    const event = events[0] as DocumentErrorEvent;
    expect(event.type).toBe('document:error');
    expect(event.payload.code).toBe(-85);
    expect(event.payload.message).toBe('содержимое не совпадает');
  });

  it('приводит флаги состояния к булевым значениям', () => {
    const { events, emit } = createHarness();

    emit({ type: 'document:state', payload: { readonly: 1, hasDocument: 'да', dirty: true } });

    const event = events[0] as DocumentStateEvent;
    expect(event.payload).toEqual({ readonly: false, hasDocument: false, dirty: true });
  });

  it('отдаёт выгруженный файл и не выдумывает отсутствующие поля', () => {
    const { events, emit } = createHarness();
    const file = new File([new Uint8Array([1])], 'выгрузка.docx');

    emit({ type: 'document:saved', payload: { fileName: 'выгрузка.docx', file, dirty: false } });
    emit({ type: 'document:saved', payload: {} });

    const saved = events[0] as DocumentSavedEvent;
    expect(saved.payload.file).toBe(file);
    expect(saved.payload.dirty).toBe(false);
    expect((events[1] as DocumentSavedEvent).payload).toEqual({});
  });
});

describe('onEvent', () => {
  it('возвращает отписку, не задевающую других слушателей', () => {
    const { client, emit } = createHarness();
    const first = vi.fn();
    const second = vi.fn();
    const off = client.onEvent(first);
    client.onEvent(second);

    emit({ type: 'document:dirty-changed', payload: { dirty: true } });
    expect(first).toHaveBeenCalledTimes(1);

    off();
    emit({ type: 'document:dirty-changed', payload: { dirty: false } });

    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(2);
  });
});

describe('describeEditorError', () => {
  it('расшифровывает известные коды', () => {
    expect(describeEditorError(-82)).toContain('сконвертировать');
    expect(describeEditorError(-85)).toContain('расширением файла');
  });

  it('на неизвестный код отдаёт фолбэк с кодом, а не пустую строку', () => {
    expect(describeEditorError(42)).toBe('неизвестная ошибка редактора (код 42)');
    expect(describeEditorError(-999)).toContain('-999');
  });
});
