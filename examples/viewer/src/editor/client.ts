import editorWorkerUrl from './ooxml_editor.worker.ts?worker&url';
import type {
  EditorArgs,
  EditorFormat,
  EditorRequest,
  EditorResponse,
  EditorResult,
} from './types.js';

interface Pending {
  resolve(result: EditorResult): void;
  reject(error: Error): void;
}

/** Omit, дистрибутивный по членам union: иначе поля веток теряются. */
type DistributiveOmit<T, K extends keyof any> = T extends any ? Omit<T, K> : never;


/**
 * URL модуля воркера.
 *
 * В сборке — чанк от `?worker&url`. В dev суффикс `?worker_file` заставляет
 * Vite добавить воркеру `import "/@fs/…/vite/dist/client/env.mjs"`, а WebKit
 * под COEP такой импорт не грузит: воркер умирает до первого сообщения.
 * Без суффикса dev-сервер отдаёт тот же модуль, но без инъекции env; имя файла
 * собирается в рантайме, чтобы Vite не подменил литерал ассетом.
 */
function editorWorkerSource(): string {
  if (import.meta.env.DEV) {
    const file = 'ooxml_editor.worker.ts';
    return new URL(`./${file}`, import.meta.url).href;
  }
  return editorWorkerUrl;
}

/** Клиент editor-worker: адресные промисы и гарантированное завершение при закрытии. */
export class OoxmlEditor {
  private readonly worker: Worker;
  private readonly pending = new Map<number, Pending>();
  private nextId = 1;
  private closed = false;

  constructor() {
    this.worker = new Worker(editorWorkerSource(), { type: 'module', name: 'ooxml-editor' });
    this.worker.addEventListener('message', (event: MessageEvent<EditorResponse>) => {
      const message = event.data;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.type === 'error') pending.reject(new Error(message.message));
      else pending.resolve(message);
    });
    this.worker.addEventListener('error', (event) => {
      // Сообщение воркера в WebKit часто пустое: имя файла и строка объясняют
      // отказ лучше, чем общая фраза.
      const detail = [event.message, event.filename, event.lineno]
        .filter((part) => part !== '' && part !== 0 && part !== undefined)
        .join(' ');
      this.rejectAll(new Error(detail || 'OOXML editor worker failed'));
    });
  }

  private request(message: DistributiveOmit<EditorRequest, 'id'>, transfer: Transferable[] = []): Promise<EditorResult> {
    if (this.closed) return Promise.reject(new Error('редактор уже закрыт'));
    const id = this.nextId++;
    return new Promise<EditorResult>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ ...message, id } as EditorRequest, transfer);
    });
  }

  private rejectAll(error: Error): void {
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
  }

  open(format: EditorFormat, bytes: ArrayBuffer, name: string): Promise<EditorResult> {
    return this.request({ type: 'open', format, bytes, name }, [bytes]);
  }

  create(format: EditorFormat, bytes: ArrayBuffer, name: string): Promise<EditorResult> {
    return this.request({ type: 'create', format, bytes, name }, [bytes]);
  }

  execute(command: string, args: EditorArgs = {}): Promise<EditorResult> {
    return this.request({ type: 'execute', command, args });
  }

  save(): Promise<EditorResult> {
    return this.request({ type: 'save' });
  }

  undo(): Promise<EditorResult> {
    return this.request({ type: 'undo' });
  }

  redo(): Promise<EditorResult> {
    return this.request({ type: 'redo' });
  }

  destroy(): void {
    if (this.closed) return;
    this.closed = true;
    this.worker.postMessage({ type: 'close' } satisfies EditorRequest);
    this.worker.terminate();
    this.rejectAll(new Error('редактор закрыт'));
  }
}
