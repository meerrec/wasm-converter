import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createSelectionController } from '../src/render/selection_controller.js';
import { address, createSelection, fullSheet, type SelectionState } from '../src/render/selection.js';
import type { CellAddress, SheetHit } from '../src/protocol.js';
import type { SelectionHost } from '../src/render/selection_controller.js';

class FakeElement {
  readonly listeners = new Map<string, Set<(event: unknown) => void>>();
  readonly style: Record<string, string> = {};
  focused = false;

  addEventListener(type: string, handler: (event: unknown) => void): void {
    const handlers = this.listeners.get(type) ?? new Set();
    this.listeners.set(type, handlers);
    handlers.add(handler);
  }
  removeEventListener(type: string, handler: (event: unknown) => void): void {
    this.listeners.get(type)?.delete(handler);
  }
  dispatch(type: string, event: Record<string, unknown>): void {
    for (const handler of [...(this.listeners.get(type) ?? [])]) {
      handler({ preventDefault: vi.fn(), ...event });
    }
  }
  setAttribute(): void {}
  append(): void {}
  focus(): void { this.focused = true; }
  setPointerCapture(): void {}
  releasePointerCapture(): void {}
  hasPointerCapture(): boolean { return false; }
  getBoundingClientRect(): DOMRect {
    return { left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600 } as DOMRect;
  }
}

const flush = async (): Promise<void> => {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
};

function makeHost() {
  let state: SelectionState | null = createSelection(0);
  const host: SelectionHost = {
    getSelection: () => state,
    setSelection: vi.fn((next) => { state = next; }),
    hitTest: vi.fn(async (x: number, y: number): Promise<SheetHit> => ({ kind: 'cell', row: y, col: x })),
    normalize: vi.fn(async (selection) => selection),
    navigate: vi.fn(async (active: CellAddress, command: string) => {
      if (command === 'ctrl-home') return address(0, 0);
      if (command === 'ctrl-end') return address(99, 9);
      const [match, direction, count] = /^steps-(left|right|up|down)-(\d+)$/.exec(command) ?? [];
      if (match) {
        const amount = Number(count);
        if (direction === 'right') return address(active.row, active.col + amount);
        if (direction === 'left') return address(active.row, active.col - amount);
        if (direction === 'down') return address(active.row + amount, active.col);
        return address(active.row - amount, active.col);
      }
      if (command === 'right') return address(active.row, active.col + 1);
      if (command === 'left') return address(active.row, active.col - 1);
      if (command === 'down') return address(active.row + 1, active.col);
      if (command === 'up') return address(active.row - 1, active.col);
      if (command === 'home') return address(active.row, 0);
      return active;
    }),
    currentRegion: vi.fn(async () => ({ start: address(0, 0), end: address(1, 1) })),
    pageMove: vi.fn(async (active: CellAddress) => address(active.row + 20, active.col)),
    ensureVisible: vi.fn(async () => undefined),
    scrollBy: vi.fn(),
    focus: vi.fn(),
  };
  return { host, getState: () => state };
}

let canvas: FakeElement;
let scroller: FakeElement;

beforeEach(() => {
  canvas = new FakeElement();
  scroller = new FakeElement();
  vi.stubGlobal('requestAnimationFrame', () => 1);
  vi.stubGlobal('cancelAnimationFrame', () => undefined);
});

afterEach(() => vi.unstubAllGlobals());

describe('автомат выделения', () => {
  it('выделяет ячейку и протягивает прямоугольник', async () => {
    const { host, getState } = makeHost();
    const controller = createSelectionController(host, canvas as unknown as HTMLCanvasElement, scroller as unknown as HTMLDivElement);
    canvas.dispatch('pointerdown', { button: 0, pointerId: 1, clientX: 1, clientY: 1, ctrlKey: false, metaKey: false, shiftKey: false });
    await flush();
    expect(getState()?.active).toEqual(address(1, 1));
    canvas.dispatch('pointermove', { pointerId: 1, clientX: 3, clientY: 2 });
    await flush();
    expect(getState()?.ranges[0]).toEqual({ start: address(1, 1), end: address(2, 3) });
    canvas.dispatch('pointerup', { pointerId: 1 });
    controller.destroy();
  });

  it('добавляет несмежный диапазон через Ctrl и расширяет через Shift+Arrow', async () => {
    const { host, getState } = makeHost();
    const controller = createSelectionController(host, canvas as unknown as HTMLCanvasElement, scroller as unknown as HTMLDivElement);
    canvas.dispatch('pointerdown', { button: 0, pointerId: 1, clientX: 1, clientY: 1, ctrlKey: false, metaKey: false, shiftKey: false });
    await flush();
    canvas.dispatch('pointerup', { pointerId: 1 });
    canvas.dispatch('pointerdown', { button: 0, pointerId: 2, clientX: 3, clientY: 3, ctrlKey: true, metaKey: false, shiftKey: false });
    await flush();
    expect(getState()?.ranges).toHaveLength(2);
    scroller.dispatch('keydown', { key: 'ArrowRight', shiftKey: true, ctrlKey: false, metaKey: false, altKey: false });
    await flush();
    expect(getState()?.ranges[1]?.end).toEqual(address(3, 4));
    controller.destroy();
  });

  it('Ctrl+A выбирает область данных, повторно — весь лист', async () => {
    const { host, getState } = makeHost();
    const controller = createSelectionController(host, canvas as unknown as HTMLCanvasElement, scroller as unknown as HTMLDivElement);
    const event = { key: 'a', shiftKey: false, ctrlKey: true, metaKey: false, altKey: false };
    scroller.dispatch('keydown', event);
    await flush();
    expect(getState()?.ranges[0]).toEqual({ start: address(0, 0), end: address(1, 1) });
    scroller.dispatch('keydown', event);
    await flush();
    expect(getState()?.ranges[0]).toEqual(fullSheet());
    controller.destroy();
  });

  it('два быстрых Ctrl+A не ждут ответ первого запроса', async () => {
    const { host, getState } = makeHost();
    const controller = createSelectionController(host, canvas as unknown as HTMLCanvasElement, scroller as unknown as HTMLDivElement);
    const event = { key: 'a', shiftKey: false, ctrlKey: true, metaKey: false, altKey: false };
    scroller.dispatch('keydown', event);
    scroller.dispatch('keydown', event);
    await flush();
    expect(getState()?.ranges[0]).toEqual(fullSheet());
    controller.destroy();
  });

  it('поддерживает Name Box, PageDown и Ctrl+End', async () => {
    const { host, getState } = makeHost();
    const controller = createSelectionController(host, canvas as unknown as HTMLCanvasElement, scroller as unknown as HTMLDivElement);
    await controller.setSelection('XFD1048576', false, true);
    expect(getState()?.active).toEqual(address(1_048_575, 16_383));
    expect(host.ensureVisible).toHaveBeenCalled();
    scroller.dispatch('keydown', { key: 'PageDown', shiftKey: false, ctrlKey: false, metaKey: false, altKey: false });
    await flush();
    expect(host.pageMove).toHaveBeenCalled();
    scroller.dispatch('keydown', { key: 'End', shiftKey: false, ctrlKey: true, metaKey: false, altKey: false });
    await flush();
    expect(getState()?.active).toEqual(address(99, 9));
    controller.destroy();
  });

  it('не принимает ответ после destroy', async () => {
    const { host, getState } = makeHost();
    const before = getState();
    let resolveHit: ((hit: SheetHit) => void) | undefined;
    host.hitTest = vi.fn(() => new Promise<SheetHit>((resolve) => { resolveHit = resolve; }));
    const controller = createSelectionController(host, canvas as unknown as HTMLCanvasElement, scroller as unknown as HTMLDivElement);
    canvas.dispatch('pointerdown', { button: 0, pointerId: 1, clientX: 5, clientY: 5, ctrlKey: false, metaKey: false, shiftKey: false });
    controller.destroy();
    resolveHit?.({ kind: 'cell', row: 5, col: 5 });
    await flush();
    expect(getState()).toBe(before);
  });
});
