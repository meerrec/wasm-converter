import { describe, expect, it } from 'vitest';

import {
  resolveExecutionPlan,
  resolveRenderTarget,
} from '../src/execution/plan.js';

/**
 * Матрица плана кадра: где рисуем и каким транспортом возим.
 *
 * Один и тот же чистый код спрашивают main (место рендера) и воркер
 * (транспорт), поэтому проверяются обе половины и все недопустимые пары.
 */
describe('resolveRenderTarget', () => {
  it('auto берёт воркер, когда OffscreenCanvas есть', () => {
    expect(
      resolveRenderTarget({ requested: undefined, canOffscreen: true, transport: undefined }),
    ).toBe('worker');
    expect(
      resolveRenderTarget({ requested: 'auto', canOffscreen: true, transport: 'auto' }),
    ).toBe('worker');
  });

  it('auto без OffscreenCanvas уходит на main', () => {
    expect(
      resolveRenderTarget({ requested: 'auto', canOffscreen: false, transport: undefined }),
    ).toBe('main');
  });

  it('явный main выбирает main даже при OffscreenCanvas', () => {
    expect(
      resolveRenderTarget({ requested: 'main', canOffscreen: true, transport: 'auto' }),
    ).toBe('main');
  });

  it('явный worker без OffscreenCanvas — ошибка, а не тихий main', () => {
    expect(() =>
      resolveRenderTarget({ requested: 'worker', canOffscreen: false, transport: undefined }),
    ).toThrow(/OffscreenCanvas.*worker/);
  });

  it('main вместе с sab — ошибка конфигурации в обоих вариантах запроса', () => {
    expect(() =>
      resolveRenderTarget({ requested: 'main', canOffscreen: true, transport: 'sab' }),
    ).toThrow(/sab/);
    // auto без OffscreenCanvas — тот же main-путь, значит тот же запрет.
    expect(() =>
      resolveRenderTarget({ requested: undefined, canOffscreen: false, transport: 'sab' }),
    ).toThrow(/sab/);
  });

  it('main с auto или bytes разрешён', () => {
    expect(
      resolveRenderTarget({ requested: 'main', canOffscreen: false, transport: 'auto' }),
    ).toBe('main');
    expect(
      resolveRenderTarget({ requested: 'main', canOffscreen: false, transport: 'bytes' }),
    ).toBe('main');
  });
});

describe('resolveExecutionPlan', () => {
  it('worker + auto + SAB даёт sab', () => {
    expect(
      resolveExecutionPlan({ render: 'worker', requested: undefined, sabAvailable: true }),
    ).toEqual({ render: 'worker', transport: 'sab' });
    expect(
      resolveExecutionPlan({ render: 'worker', requested: 'auto', sabAvailable: true }),
    ).toEqual({ render: 'worker', transport: 'sab' });
  });

  it('worker + auto без SAB даёт bytes: это штатный путь, не аварийный', () => {
    expect(
      resolveExecutionPlan({ render: 'worker', requested: 'auto', sabAvailable: false }),
    ).toEqual({ render: 'worker', transport: 'bytes' });
  });

  it('явный bytes не трогает SAB, даже когда он есть', () => {
    expect(
      resolveExecutionPlan({ render: 'worker', requested: 'bytes', sabAvailable: true }),
    ).toEqual({ render: 'worker', transport: 'bytes' });
  });

  it('явный sab без SAB — ошибка с подсказкой про COOP/COEP', () => {
    expect(() =>
      resolveExecutionPlan({ render: 'worker', requested: 'sab', sabAvailable: false }),
    ).toThrow(/COOP\/COEP/);
  });

  it('main всегда едет bytes, даже при auto и доступном SAB', () => {
    expect(
      resolveExecutionPlan({ render: 'main', requested: 'auto', sabAvailable: true }),
    ).toEqual({ render: 'main', transport: 'bytes' });
  });

  it('main + sab не представим', () => {
    expect(() =>
      resolveExecutionPlan({ render: 'main', requested: 'sab', sabAvailable: true }),
    ).toThrow(/sab/);
  });
});
