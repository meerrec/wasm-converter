/**
 * Публичное API пакета `@doc-converter/eurooffice`: типизированный клиент Embed API
 * редактора Euro-Office поверх iframe.
 *
 * Контракт — docs/embed-api.md репозитория ranuts/document; происхождение и границы
 * пакета описаны в README.
 */
export { createEditorClient, describeEditorError } from './protocol.js';
export type {
  EditorClient,
  EditorClientOptions,
  EditorCommand,
  EditorEvent,
  EditorEventListener,
  OpenBufferPayload,
  OpenFilePayload,
  OpenUrlPayload,
  ParsedEditorMessage,
  SavePayload,
  SetReadonlyPayload,
} from './protocol.js';
export type {
  DocumentDirtyChangedEvent,
  DocumentDirtyChangedPayload,
  DocumentErrorEvent,
  DocumentErrorPayload,
  DocumentOpenedEvent,
  DocumentOpenedPayload,
  DocumentReadonlyChangedEvent,
  DocumentReadonlyChangedPayload,
  DocumentReadyEvent,
  DocumentSavedEvent,
  DocumentSavedPayload,
  DocumentStateEvent,
  DocumentStatePayload,
  DocumentUnknownEvent,
} from './protocol.js';
