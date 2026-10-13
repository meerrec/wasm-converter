/**
 * Клиент Embed API редактора Euro-Office: команды шлются iframe'у
 * `/editor.html?embed=1` через postMessage, события приходят обратно.
 * Контракт — docs/embed-api.md репозитория ranuts/document; реализация
 * проверена по бандлу assets/editor-*.js: команды принимаются только от
 * непосредственного родителя, ответные события повторяют id команды
 * (`T("document:opened", …, t.id)`), `document:ready` — широковещательное.
 *
 * Файл читает родитель и отдаёт байтами (`document:open-buffer`) — CORS не
 * нужен ни на файл, ни на расширение.
 */

/** Идентификатор команды: уникален в пределах сессии, генерируется клиентом. */
type CommandId = string;

// ── Команды (родитель → iframe) ────────────────────────────────────────────

/** Payload `document:open-buffer` — открыть документ байтами из родителя. */
export interface OpenBufferPayload {
  /** Имя файла: по нему редактор определяет формат. */
  fileName: string;
  /** Содержимое файла целиком. */
  buffer: ArrayBuffer | Uint8Array;
  /** Только просмотр: в этом режиме `document:save` отвечает ошибкой. */
  readonly: boolean;
}

/** Payload `document:open-file` — открыть `File` из `<input type="file">`. */
export interface OpenFilePayload {
  file: File;
  readonly: boolean;
}

/**
 * Payload `document:open-url` — открыть документ по URL. Файл тянет сам
 * браузер, поэтому ответ сервера должен пропускать CORS.
 */
export interface OpenUrlPayload {
  url: string;
  fileName: string;
  readonly?: boolean;
  /** Заголовки запроса (например, авторизация). */
  fetchOptions?: { headers?: Record<string, string> };
}

/** Payload `document:set-readonly` — переключить режим только просмотра. */
export interface SetReadonlyPayload {
  readonly: boolean;
}

/** Payload `document:save` — выгрузить документ (ответ — `document:saved`). */
export interface SavePayload {
  /** Целевой формат: XLSX, DOCX, PPTX, CSV; по умолчанию — формат документа. */
  targetExt?: string;
  /** Вернуть исходный файл, если редактор не успел выгрузить правки. */
  returnOriginalOnTimeout?: boolean;
}

/** Команды, которые умеет слать клиент. */
export type EditorCommand =
  | { type: 'document:open-buffer'; payload: OpenBufferPayload }
  | { type: 'document:open-file'; payload: OpenFilePayload }
  | { type: 'document:open-url'; payload: OpenUrlPayload }
  | { type: 'document:set-readonly'; payload: SetReadonlyPayload }
  | { type: 'document:save'; payload: SavePayload }
  | { type: 'document:get-state'; payload: Record<string, never> };

// ── События (iframe → родитель) ────────────────────────────────────────────

/** Общая форма события: id повторяет id команды, ответом на которую оно является. */
interface EditorEventOf<T extends string, P> {
  type: T;
  /** У широковещательного `document:ready` id отсутствует. */
  id?: string;
  payload: P;
}

/** Payload `document:opened` — документ открыт. */
export interface DocumentOpenedPayload {
  /** Имя открытого документа (редактор может не вернуть — берём своё). */
  fileName?: string;
}

/** Payload `document:saved` — выгрузка готова, файл лежит в `file`. */
export interface DocumentSavedPayload {
  fileName?: string;
  file?: File;
  /** Правки, сделанные во время выгрузки, в файл не попали. */
  dirty?: boolean;
}

/** Payload `document:state` — ответ на `document:get-state`. */
export interface DocumentStatePayload {
  readonly: boolean;
  hasDocument: boolean;
  dirty: boolean;
}

/** Payload `document:dirty-changed` — флаг несохранённых правок сменился. */
export interface DocumentDirtyChangedPayload {
  dirty: boolean;
}

/** Payload `document:readonly-changed` — режим только просмотра сменился. */
export interface DocumentReadonlyChangedPayload {
  readonly: boolean;
}

/** Payload `document:error` — операция не удалась. */
export interface DocumentErrorPayload {
  /** Код ошибки (например, -82/-85); может отсутствовать. */
  code?: number;
  /** Человекочитаемое сообщение редактора. */
  message?: string;
}

/** Неизвестное событие `document:*` — доставляется как есть, молча не теряем. */
export interface DocumentUnknownEvent {
  type: string;
  id?: string;
  payload: Record<string, unknown>;
}

/** Именованные события — удобно сужать в switch и импортировать типами. */
export type DocumentReadyEvent = EditorEventOf<'document:ready', Record<string, never>>;
export type DocumentOpenedEvent = EditorEventOf<'document:opened', DocumentOpenedPayload>;
export type DocumentSavedEvent = EditorEventOf<'document:saved', DocumentSavedPayload>;
export type DocumentStateEvent = EditorEventOf<'document:state', DocumentStatePayload>;
export type DocumentDirtyChangedEvent = EditorEventOf<'document:dirty-changed', DocumentDirtyChangedPayload>;
export type DocumentReadonlyChangedEvent = EditorEventOf<'document:readonly-changed', DocumentReadonlyChangedPayload>;
export type DocumentErrorEvent = EditorEventOf<'document:error', DocumentErrorPayload>;

/** События редактора, которые доставляет клиент (известные типы). */
export type EditorEvent =
  | DocumentReadyEvent
  | DocumentOpenedEvent
  | DocumentSavedEvent
  | DocumentStateEvent
  | DocumentDirtyChangedEvent
  | DocumentReadonlyChangedEvent
  | DocumentErrorEvent;

/** Результат разбора: известное событие, неизвестное `document:*` или постороннее сообщение. */
export type ParsedEditorMessage =
  | { kind: 'event'; event: EditorEvent }
  | { kind: 'unknown'; unknown: DocumentUnknownEvent }
  | null;

// ── Расшифровка ошибок ─────────────────────────────────────────────────────

/** Коды ошибок редактора и их расшифровки (см. README пакета). */
const EDITOR_ERROR_HINTS = new Map<number, string>([
  [-82, 'не удалось сконвертировать: файл повреждён, зашифрован или вариант формата не поддержан'],
  [-85, 'содержимое не совпадает с расширением файла'],
]);

/**
 * Человекочитаемая расшифровка кода ошибки редактора.
 * Неизвестный код — общая фраза с кодом, чтобы ошибка не была молчаливой.
 */
export function describeEditorError(code: number): string {
  return EDITOR_ERROR_HINTS.get(code) ?? `неизвестная ошибка редактора (код ${code})`;
}

// ── Разбор входящих сообщений ──────────────────────────────────────────────

/** Число из payload, которое редактор мог прислать и строкой (`"-82"`). */
function toNumber(raw: unknown): number | undefined {
  if (typeof raw === 'number' && Number.isFinite(raw)) return raw;
  if (typeof raw === 'string' && raw.trim() !== '') {
    const parsed = Number(raw);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function toOpenedPayload(raw: Record<string, unknown>): DocumentOpenedPayload {
  return typeof raw['fileName'] === 'string' ? { fileName: raw['fileName'] } : {};
}

function toSavedPayload(raw: Record<string, unknown>): DocumentSavedPayload {
  const payload: DocumentSavedPayload = {};
  if (typeof raw['fileName'] === 'string') payload.fileName = raw['fileName'];
  if (raw['file'] instanceof File) payload.file = raw['file'];
  if (typeof raw['dirty'] === 'boolean') payload.dirty = raw['dirty'];
  return payload;
}

function toErrorPayload(raw: Record<string, unknown>): DocumentErrorPayload {
  const payload: DocumentErrorPayload = {};
  if (typeof raw['message'] === 'string') payload.message = raw['message'];
  const code = toNumber(raw['code']);
  if (code !== undefined) payload.code = code;
  return payload;
}

function toStatePayload(raw: Record<string, unknown>): DocumentStatePayload {
  return {
    readonly: raw['readonly'] === true,
    hasDocument: raw['hasDocument'] === true,
    dirty: raw['dirty'] === true,
  };
}

function toDirtyPayload(raw: Record<string, unknown>): DocumentDirtyChangedPayload {
  return { dirty: raw['dirty'] === true };
}

function toReadonlyPayload(raw: Record<string, unknown>): DocumentReadonlyChangedPayload {
  return { readonly: raw['readonly'] === true };
}

/** Событие без поля id, когда его нет: exactOptionalPropertyTypes запрещает явный undefined. */
function eventOf<T extends string, P>(type: T, id: string | undefined, payload: P): EditorEventOf<T, P> {
  return id === undefined ? { type, payload } : { type, id, payload };
}

/**
 * Сообщение iframe → событие редактора. Отсев по origin/source делает клиент,
 * тут — только форма сообщения. Неизвестное `document:*` возвращается как
 * `{ kind: 'unknown' }` — клиент его логирует, чтобы не терять молча;
 * посторонние сообщения — null.
 */
function parseEditorMessage(data: unknown): ParsedEditorMessage {
  if (typeof data !== 'object' || data === null) return null;
  const raw = data as Record<string, unknown>;
  if (typeof raw['type'] !== 'string' || !raw['type'].startsWith('document:')) return null;
  const type = raw['type'];
  const id = typeof raw['id'] === 'string' ? raw['id'] : undefined;
  const rawPayload = raw['payload'];
  const payload = typeof rawPayload === 'object' && rawPayload !== null ? (rawPayload as Record<string, unknown>) : {};

  switch (type) {
    case 'document:ready':
      return { kind: 'event', event: eventOf(type, id, {}) };
    case 'document:opened':
      return { kind: 'event', event: eventOf(type, id, toOpenedPayload(payload)) };
    case 'document:saved':
      return { kind: 'event', event: eventOf(type, id, toSavedPayload(payload)) };
    case 'document:state':
      return { kind: 'event', event: eventOf(type, id, toStatePayload(payload)) };
    case 'document:dirty-changed':
      return { kind: 'event', event: eventOf(type, id, toDirtyPayload(payload)) };
    case 'document:readonly-changed':
      return { kind: 'event', event: eventOf(type, id, toReadonlyPayload(payload)) };
    case 'document:error':
      return { kind: 'event', event: eventOf(type, id, toErrorPayload(payload)) };
    default:
      // Будущие события документа: не интерпретируем, но и не теряем молча.
      return { kind: 'unknown', unknown: eventOf(type, id, payload) };
  }
}

// ── Клиент ─────────────────────────────────────────────────────────────────

/** Подписчик на события редактора. */
export type EditorEventListener = (event: EditorEvent) => void;

/** Опции клиента. */
export interface EditorClientOptions {
  /** Origin редактора; по умолчанию — наш собственный (iframe раздаётся тем же сервером). */
  readonly origin?: string;
}

/** Клиент Embed API поверх iframe `/editor.html?embed=1`. */
export interface EditorClient {
  /** Открыть документ байтами: файл читает родитель. */
  openBuffer(fileName: string, buffer: ArrayBuffer | Uint8Array, readonly: boolean): void;
  /** Открыть `File` из файлового поля. */
  openFile(file: File, readonly: boolean): void;
  /** Открыть документ по URL (нужен CORS на ответ). */
  openUrl(url: string, fileName: string, readonly: boolean): void;
  /** Переключить режим только просмотра. */
  setReadonly(readonly: boolean): void;
  /** Выгрузить документ (ответ — `document:saved`). */
  save(options?: SavePayload): void;
  /** Запросить текущее состояние (ответ — `document:state`). */
  getState(): void;
  /** Подписка на события; возвращает функцию отписки. */
  onEvent(listener: EditorEventListener): () => void;
}

/**
 * Создаёт клиент Embed API для iframe редактора.
 *
 * Гарантии:
 * - события принимаются только от `frame.contentWindow` и origin'а редактора;
 * - команды, посланные до `document:ready`, ждут его в очереди и уходят после;
 * - id команд уникальны (счётчик); ответные события матчатся по id: ответ
 *   команды, отменённой новым открытием/сохранением, не доставляется.
 *   Событие без id или с чужим id доставляется как есть — редактор может не
 *   вернуть id, и ответ потерять дороже, чем показать.
 */
export function createEditorClient(frame: HTMLIFrameElement, options?: EditorClientOptions): EditorClient {
  const origin = options?.origin ?? window.location.origin;
  const listeners = new Set<EditorEventListener>();
  let ready = false;
  let serial = 0;
  /** Команды до `document:ready`: уйдут по готовности. */
  const queue: Array<{ id: CommandId; command: EditorCommand }> = [];
  /** Отправленные команды без ответа: id → команда. */
  const pending = new Map<CommandId, EditorCommand>();

  /** Префикс наших id — по нему отличаем свой запоздавший ответ от чужого id. */
  const ID_PREFIX = 'eo-';

  const nextId = (): CommandId => {
    serial += 1;
    return `${ID_PREFIX}${Date.now().toString(36)}-${serial.toString(36)}`;
  };

  /** Ответ с нашим id, которого нет в pending, — отменённой команде: не доставляем. */
  const isStaleResponse = (id: string): boolean => id.startsWith(ID_PREFIX) && !pending.has(id);

  /**
   * Новое открытие/сохранение/запрос состояния отменяет предыдущие
   * неотвеченные команды того же рода: их запоздавшие ответы не должны
   * выглядеть ответами на последнюю команду.
   */
  const supersede = (command: EditorCommand): void => {
    const sameKind = (other: EditorCommand): boolean =>
      command.type.startsWith('document:open-') ? other.type.startsWith('document:open-') : other.type === command.type;
    for (const [id, other] of pending) {
      if (sameKind(other)) pending.delete(id);
    }
  };

  const post = (id: CommandId, command: EditorCommand): void => {
    const target = frame.contentWindow;
    if (target === null) return; // iframe ещё не загружен (к этому моменту очередь пуста)
    const transfer: Transferable[] = [];
    if (command.type === 'document:open-buffer') {
      // Крупный файл передаём без копирования: transfer'им подложку байтов.
      const buffer = command.payload.buffer;
      const underlying = buffer instanceof ArrayBuffer ? buffer : buffer.buffer;
      if (underlying instanceof ArrayBuffer && underlying.byteLength === buffer.byteLength) {
        transfer.push(underlying);
      }
    }
    pending.set(id, command);
    target.postMessage({ id, type: command.type, payload: command.payload }, origin, transfer);
  };

  const send = (command: EditorCommand): void => {
    const id = nextId();
    supersede(command);
    if (!ready) {
      queue.push({ id, command });
      return;
    }
    post(id, command);
  };

  const dispatch = (event: EditorEvent): void => {
    for (const listener of listeners) listener(event);
  };

  const handleMessage = (event: MessageEvent): void => {
    if (event.origin !== origin) return;
    if (event.source !== frame.contentWindow) return;
    const parsed = parseEditorMessage(event.data);
    if (parsed === null) return;
    if (parsed.kind === 'unknown') {
      // Будущие события документа: не интерпретируем, но и не теряем молча.
      console.debug('событие редактора:', parsed.unknown.type, parsed.unknown.payload);
      return;
    }
    const editorEvent = parsed.event;

    if (editorEvent.type === 'document:ready') {
      ready = true;
      dispatch(editorEvent);
      const queued = queue.splice(0);
      for (const item of queued) post(item.id, item.command);
      return;
    }

    // Ответные события матчатся по id отправленной команды.
    if (editorEvent.id !== undefined) {
      if (pending.has(editorEvent.id)) {
        pending.delete(editorEvent.id);
      } else if (isStaleResponse(editorEvent.id)) {
        return;
      }
    }
    dispatch(editorEvent);
  };

  window.addEventListener('message', handleMessage);

  return {
    openBuffer: (fileName, buffer, readonly) => send({ type: 'document:open-buffer', payload: { fileName, buffer, readonly } }),
    openFile: (file, readonly) => send({ type: 'document:open-file', payload: { file, readonly } }),
    openUrl: (url, fileName, readonly) => send({ type: 'document:open-url', payload: { url, fileName, readonly } }),
    setReadonly: (readonly) => send({ type: 'document:set-readonly', payload: { readonly } }),
    save: (options) => {
      const payload: SavePayload = options ?? {};
      send({ type: 'document:save', payload });
    },
    getState: () => send({ type: 'document:get-state', payload: {} }),
    onEvent: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}
