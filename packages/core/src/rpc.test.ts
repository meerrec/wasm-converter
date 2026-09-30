import { describe, it, expect } from 'vitest';
import { createRpc } from './rpc.js';

describe('rpc', () => {
  it('resolves call() when worker replies ok', async () => {
    const { port1, port2 } = new MessageChannel();
    port1.onmessage = (ev) => {
      const { id } = ev.data as { id: number };
      port1.postMessage({ id, type: 'ok', result: 'pong' });
    };
    const rpc = createRpc(port2 as unknown as Worker);
    port2.start();
    const out = await rpc.call<string>('echo');
    expect(out).toBe('pong');
    rpc.dispose();
  });

  it('rejects on error', async () => {
    const { port1, port2 } = new MessageChannel();
    port1.onmessage = (ev) => {
      const { id } = ev.data as { id: number };
      port1.postMessage({ id, type: 'error', message: 'boom' });
    };
    const rpc = createRpc(port2 as unknown as Worker);
    port2.start();
    await expect(rpc.call('any')).rejects.toThrow('boom');
    rpc.dispose();
  });
});
