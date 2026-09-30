import type { WorkerResponse } from './protocol.js';

interface Pending {
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
}

export interface RpcHandle {
  call<T = unknown>(type: string, payload?: unknown, transfer?: Transferable[]): Promise<T>;
  notify(type: string, payload?: unknown, transfer?: Transferable[]): void;
  on(event: WorkerResponse['type'], handler: (msg: WorkerResponse) => void): () => void;
  dispose(): void;
}

export function createRpc(port: Worker | MessagePort): RpcHandle {
  const pending = new Map<number, Pending>();
  const listeners = new Map<string, Set<(msg: WorkerResponse) => void>>();
  let nextId = 1;

  const onMessage = (ev: MessageEvent<WorkerResponse>) => {
    const msg = ev.data;
    if (typeof msg?.id === 'number' && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id)!;
      pending.delete(msg.id);
      if (msg.type === 'error') reject(new Error(msg.message));
      else resolve('result' in msg ? msg.result : msg);
      return;
    }
    const set = listeners.get(msg.type);
    if (set) for (const h of set) h(msg);
  };

  port.addEventListener('message', onMessage as EventListener);

  return {
    call<T>(type: string, payload?: unknown, transfer: Transferable[] = []) {
      const id = nextId++;
      return new Promise<T>((resolve, reject) => {
        pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
        port.postMessage({ id, type, payload }, transfer);
      });
    },
    notify(type, payload, transfer = []) {
      port.postMessage({ type, payload }, transfer);
    },
    on(event, handler) {
      let set = listeners.get(event);
      if (!set) { set = new Set(); listeners.set(event, set); }
      set.add(handler);
      return () => set!.delete(handler);
    },
    dispose() {
      port.removeEventListener('message', onMessage as EventListener);
      for (const { reject } of pending.values()) reject(new Error('rpc disposed'));
      pending.clear();
      listeners.clear();
    },
  };
}
