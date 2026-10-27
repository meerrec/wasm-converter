/// <reference lib="webworker" />
// Тонкий запуск воркера формата: общее ядро плюс форматный обработчик.
//
// Встроенные воркеры (`xlsx_worker`/`docx_worker`/`pptx_worker`) исторически
// сами подписываются на `onmessage` и держат switch по форматным сообщениям.
// Новый формат приносит только `extra`-обработчик: всё, что ядро знает
// (init/open/append/end-stream/cancel/render/export-png/close), уже разобрано
// `WorkerCore`, а форматные ответы (`hit-test`, `export-pdf`, `text-layer`, …)
// остаются в его файле.

import type { InMsg } from '../protocol.js';
import { WorkerCore } from './worker_core.js';

/**
 * Обработчик форматных сообщений. Возвращать ничего не нужно: ответ воркер
 * отправляет сам через `post`.
 */
export type PluginMessageHandler = (msg: InMsg) => void | Promise<void>;

/**
 * Подписать воркер на общий протокол и форматные сообщения.
 *
 * @param core — ядро формата с `FormatHooks`
 * @param handle — разбор сообщений, которые ядро не обработало
 */
export function startPluginWorker(core: WorkerCore, handle: PluginMessageHandler): void {
  self.onmessage = async (ev: MessageEvent<InMsg>) => {
    const msg = ev.data;
    if (await core.handle(msg)) return;
    await handle(msg);
  };
}
