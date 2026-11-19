import type {
  CellAddress,
  CellRange,
  SheetHit,
  SheetSelection,
  XlsxSelection,
} from '../protocol.js';
import {
  address,
  createSelection,
  fullColumns,
  fullRows,
  fullSheet,
  MAX_COL,
  MAX_ROW,
  moveWithinSelection,
  parseRange,
  primaryRange,
  rangeBetween,
  singleCell,
  type SelectionState,
} from './selection.js';

/** Операции вьюера, нужные автомату; книга остаётся за worker-границей. */
export interface SelectionHost {
  getSelection(): SelectionState | null;
  setSelection(state: SelectionState | null): void;
  hitTest(clientX: number, clientY: number): Promise<SheetHit>;
  normalize(selection: SheetSelection): Promise<SheetSelection>;
  navigate(active: CellAddress, command: string): Promise<CellAddress>;
  currentRegion(active: CellAddress): Promise<CellRange | null>;
  pageMove(active: CellAddress, horizontal: boolean, backward: boolean): Promise<CellAddress>;
  ensureVisible(active: CellAddress): Promise<void>;
  scrollBy(x: number, y: number): void;
  focus(): void;
}

export interface SelectionController {
  setSelection(reference: string | CellRange, add: boolean, scrollIntoView: boolean): Promise<void>;
  restoreSelection(selection: XlsxSelection): Promise<void>;
  clearSelection(): void;
  destroy(): void;
}

interface PendingPointer {
  pointerId: number;
  clientX: number;
  clientY: number;
  moved: boolean;
  released: boolean;
  cancelled: boolean;
}

interface Drag {
  id: number;
  pointerId: number;
  kind: 'cell' | 'row' | 'column';
  anchor: CellAddress;
  add: boolean;
  rangesBefore: CellRange[];
  clientX: number;
  clientY: number;
}

const isMac = (): boolean => /Mac|iPhone|iPad/.test(globalThis.navigator?.platform ?? '');
const additive = (event: { ctrlKey: boolean; metaKey: boolean }): boolean =>
  isMac() ? event.metaKey : event.ctrlKey;
const commandKey = (event: { ctrlKey: boolean; metaKey: boolean }): boolean => additive(event);

/** Мышь и клавиатура XLSX; React и модель книги в горячем пути не участвуют. */
export function createSelectionController(
  host: SelectionHost,
  canvas: HTMLCanvasElement,
  scroller: HTMLDivElement,
): SelectionController {
  let destroyed = false;
  let generation = 0;
  let drag: Drag | null = null;
  let pendingPointer: PendingPointer | null = null;
  let finishedDragId: number | null = null;
  let hitInFlight = false;
  let queuedPoint: { x: number; y: number } | null = null;
  let autoFrame: number | null = null;
  let ctrlAAll = false;
  const disposers = new Set<() => void>();

  const listen = <K extends keyof HTMLElementEventMap>(
    element: HTMLElement,
    type: K,
    handler: (event: HTMLElementEventMap[K]) => void,
  ): void => {
    element.addEventListener(type, handler);
    disposers.add(() => element.removeEventListener(type, handler));
  };

  const commit = (state: SelectionState, resetCtrlA = true): void => {
    if (resetCtrlA) ctrlAAll = false;
    host.setSelection(state);
  };

  const normalize = async (state: SelectionState): Promise<void> => {
    const seq = generation;
    try {
      const normalized = await host.normalize({ active: state.active, ranges: state.ranges });
      const current = host.getSelection();
      if (destroyed || seq !== generation || !current || current.sheetIndex !== state.sheetIndex) return;
      commit({ ...current, active: normalized.active, ranges: normalized.ranges }, false);
    } catch {
      // Повторное открытие книги делает запрос устаревшим; локальный диапазон
      // остаётся валидным и будет нормализован следующим действием.
    }
  };

  const apply = (state: SelectionState): Promise<void> => {
    ++generation;
    commit(state);
    return normalize(state);
  };

  const rangeForDrag = (current: Drag, hit: SheetHit): CellRange | null => {
    if (current.kind === 'cell' && hit.kind === 'cell') {
      return rangeBetween(current.anchor, address(hit.row, hit.col));
    }
    if (current.kind === 'row') {
      const row = hit.kind === 'cell' || hit.kind === 'row-header' ? hit.row : null;
      return row === null ? null : fullRows(current.anchor.row, row);
    }
    if (current.kind === 'column') {
      const col = hit.kind === 'cell' || hit.kind === 'column-header' ? hit.col : null;
      return col === null ? null : fullColumns(current.anchor.col, col);
    }
    return null;
  };

  const applyDrag = (current: Drag, hit: SheetHit): void => {
    const range = rangeForDrag(current, hit);
    const previous = host.getSelection();
    if (!range || !previous) return;
    const cursor = current.kind === 'cell' && hit.kind === 'cell'
      ? address(hit.row, hit.col)
      : current.anchor;
    const ranges = current.add
      ? [...current.rangesBefore, range]
      : [range];
    apply({ ...previous, cursor, ranges });
  };

  const stopAutoScroll = (): void => {
    if (autoFrame !== null) cancelAnimationFrame(autoFrame);
    autoFrame = null;
  };

  const updateDrag = (x: number, y: number): void => {
    const current = drag;
    if (!current || destroyed) return;
    current.clientX = x;
    current.clientY = y;
    queuedPoint = { x, y };
    if (hitInFlight) return;
    hitInFlight = true;
    const point = queuedPoint;
    queuedPoint = null;
    if (!point) return;
    void host.hitTest(point.x, point.y).then((hit) => {
      hitInFlight = false;
      const active = drag?.id === current.id;
      const finished = finishedDragId === current.id;
      if (destroyed || (!active && !finished)) return;
      applyDrag(current, hit);
      if (finished) finishedDragId = null;
      if (active && queuedPoint) updateDrag(queuedPoint.x, queuedPoint.y);
    }, () => {
      hitInFlight = false;
    });
  };

  const autoScroll = (): void => {
    autoFrame = null;
    const current = drag;
    if (!current || destroyed) return;
    const rect = scroller.getBoundingClientRect();
    const margin = 24;
    const speed = (distance: number): number => Math.min(32, Math.max(4, Math.ceil(distance / 3)));
    const dx = current.clientX < rect.left + margin
      ? -speed(rect.left + margin - current.clientX)
      : current.clientX > rect.right - margin
        ? speed(current.clientX - rect.right + margin)
        : 0;
    const dy = current.clientY < rect.top + margin
      ? -speed(rect.top + margin - current.clientY)
      : current.clientY > rect.bottom - margin
        ? speed(current.clientY - rect.bottom + margin)
        : 0;
    if (dx === 0 && dy === 0) return;
    host.scrollBy(dx, dy);
    updateDrag(current.clientX, current.clientY);
    autoFrame = requestAnimationFrame(autoScroll);
  };

  const beginAutoScroll = (): void => {
    if (autoFrame === null) autoFrame = requestAnimationFrame(autoScroll);
  };

  const finishDrag = (event?: PointerEvent): void => {
    stopAutoScroll();
    if (event && pendingPointer?.pointerId === event.pointerId) {
      pendingPointer.released = true;
      pendingPointer.cancelled = event.type === 'pointercancel';
    } else if (!event) {
      pendingPointer = null;
    }
    const current = drag;
    if (current) finishedDragId = current.id;
    drag = null;
    queuedPoint = null;
    if (current && event && canvas.hasPointerCapture(current.pointerId)) {
      canvas.releasePointerCapture(current.pointerId);
    }
  };

  const onPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0 || destroyed || !host.getSelection()) return;
    event.preventDefault();
    host.focus();
    ctrlAAll = false;
    finishedDragId = null;
    pendingPointer = {
      pointerId: event.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
      moved: false,
      released: false,
      cancelled: false,
    };
    const seq = ++generation;
    void host.hitTest(event.clientX, event.clientY).then((hit) => {
      const state = host.getSelection();
      if (destroyed || seq !== generation || !state || hit.kind === 'outside') return;
      const pending = pendingPointer?.pointerId === event.pointerId ? pendingPointer : null;
      pendingPointer = null;
      if (pending?.cancelled) return;
      const add = additive(event) || state.mode === 'add';
      const extend = event.shiftKey || state.mode === 'extend';
      if (hit.kind === 'select-all') {
        const next = createSelection(state.sheetIndex);
        apply({ ...next, ranges: [fullSheet()] });
        return;
      }
      if (hit.kind === 'cell' && extend) {
        const target = address(hit.row, hit.col);
        apply({
          ...state,
          cursor: target,
          ranges: [...state.ranges.slice(0, -1), rangeBetween(state.anchor, target)],
        });
        return;
      }
      if (hit.kind === 'row-header' && extend) {
        apply({
          ...state,
          ranges: [...state.ranges.slice(0, -1), fullRows(state.anchor.row, hit.row)],
        });
        return;
      }
      if (hit.kind === 'column-header' && extend) {
        apply({
          ...state,
          ranges: [...state.ranges.slice(0, -1), fullColumns(state.anchor.col, hit.col)],
        });
        return;
      }

      const kind = hit.kind === 'cell' ? 'cell' : hit.kind === 'row-header' ? 'row' : 'column';
      const anchor = hit.kind === 'cell'
        ? address(hit.row, hit.col)
        : hit.kind === 'row-header'
          ? address(hit.row, 0)
          : address(0, hit.col);
      const range = kind === 'cell' ? singleCell(anchor) : kind === 'row' ? fullRows(anchor.row, anchor.row) : fullColumns(anchor.col, anchor.col);
      const rangesBefore = add ? state.ranges : [];
      const next: SelectionState = {
        ...state,
        active: anchor,
        anchor,
        cursor: anchor,
        ranges: [...rangesBefore, range],
        mode: state.mode,
      };
      apply(next);
      const nextDrag: Drag = {
        id: generation,
        pointerId: event.pointerId,
        kind,
        anchor,
        add,
        rangesBefore,
        clientX: event.clientX,
        clientY: event.clientY,
      };
      if (pending?.released) {
        if (pending.moved) {
          void host.hitTest(pending.clientX, pending.clientY).then((lastHit) => {
            if (!destroyed && generation === nextDrag.id) applyDrag(nextDrag, lastHit);
          }).catch(() => undefined);
        }
        return;
      }
      if (!pending) return;
      drag = nextDrag;
      canvas.setPointerCapture(event.pointerId);
      if (pending.moved) updateDrag(pending.clientX, pending.clientY);
    }).catch(() => undefined);
  };

  const onPointerMove = (event: PointerEvent): void => {
    if (pendingPointer?.pointerId === event.pointerId) {
      pendingPointer.clientX = event.clientX;
      pendingPointer.clientY = event.clientY;
      pendingPointer.moved = true;
      event.preventDefault();
      return;
    }
    if (!drag || event.pointerId !== drag.pointerId) return;
    event.preventDefault();
    updateDrag(event.clientX, event.clientY);
    beginAutoScroll();
  };

  const onKeyDown = (event: KeyboardEvent): void => {
    const state = host.getSelection();
    if (!state || destroyed) return;
    const primary = commandKey(event);
    const extend = event.shiftKey || state.mode === 'extend';
    const key = event.key;

    if (key === 'Escape') {
      finishDrag();
      commit({ ...state, mode: 'normal' });
      event.preventDefault();
      return;
    }
    if (key === 'F8') {
      commit({ ...state, mode: event.shiftKey ? 'add' : state.mode === 'extend' ? 'normal' : 'extend' });
      event.preventDefault();
      return;
    }
    if (primary && key.toLowerCase() === 'a') {
      event.preventDefault();
      const selectAll = ctrlAAll;
      ctrlAAll = !ctrlAAll;
      const seq = ++generation;
      void host.currentRegion(state.active).then((range) => {
        const current = host.getSelection();
        if (!current || seq !== generation) return;
        const nextRange = selectAll || !range ? fullSheet() : range;
        commit({ ...current, cursor: current.active, ranges: [nextRange] }, false);
      }).catch(() => undefined);
      return;
    }
    if (key === ' ' && event.shiftKey && primary) {
      apply({ ...state, ranges: [fullSheet()] });
      event.preventDefault();
      return;
    }
    if (key === ' ' && event.shiftKey) {
      apply({ ...state, anchor: address(state.active.row, 0), cursor: address(state.active.row, MAX_COL), ranges: [fullRows(state.active.row, state.active.row)] });
      event.preventDefault();
      return;
    }
    if (key === ' ' && primary) {
      apply({ ...state, anchor: address(0, state.active.col), cursor: address(MAX_ROW, state.active.col), ranges: [fullColumns(state.active.col, state.active.col)] });
      event.preventDefault();
      return;
    }
    if (primary && key === 'Backspace') {
      void host.ensureVisible(state.active);
      event.preventDefault();
      return;
    }

    const direction = key === 'ArrowLeft' ? 'left'
      : key === 'ArrowRight' ? 'right'
        : key === 'ArrowUp' ? 'up'
          : key === 'ArrowDown' ? 'down' : null;
    const command = direction
      ? `${primary ? 'ctrl-' : ''}${direction}`
      : key === 'Home'
        ? primary ? 'ctrl-home' : 'home'
        : key === 'End' && primary
          ? 'ctrl-end'
          : null;
    if (command) {
      event.preventDefault();
      const seq = ++generation;
      void host.navigate(state.active, command).then((target) => {
        const current = host.getSelection();
        if (!current || seq !== generation) return;
        if (extend) {
          apply({
            ...current,
            cursor: target,
            ranges: [...current.ranges.slice(0, -1), rangeBetween(current.anchor, target)],
          });
        } else {
          apply(createSelection(current.sheetIndex, target));
        }
        void host.ensureVisible(target);
      }).catch(() => undefined);
      return;
    }

    if (key === 'PageUp' || key === 'PageDown') {
      event.preventDefault();
      const horizontal = event.altKey;
      const backward = key === 'PageUp';
      const seq = ++generation;
      void host.pageMove(state.active, horizontal, backward).then((target) => {
        const current = host.getSelection();
        if (!current || seq !== generation) return;
        if (extend) {
          apply({ ...current, cursor: target, ranges: [...current.ranges.slice(0, -1), rangeBetween(current.anchor, target)] });
        } else {
          apply(createSelection(current.sheetIndex, target));
        }
        void host.ensureVisible(target);
      }).catch(() => undefined);
      return;
    }

    if (key === 'Tab' || key === 'Enter') {
      event.preventDefault();
      const range = primaryRange(state);
      const single = range.start.row === range.end.row && range.start.col === range.end.col;
      const columnMajor = key === 'Enter';
      if (single) {
        const command = columnMajor
          ? event.shiftKey ? 'up' : 'down'
          : event.shiftKey ? 'left' : 'right';
        const seq = ++generation;
        void host.navigate(state.active, command).then((target) => {
          const current = host.getSelection();
          if (current && seq === generation) apply(createSelection(current.sheetIndex, target));
        }).catch(() => undefined);
      } else {
        const active = moveWithinSelection(state, event.shiftKey, columnMajor);
        commit({ ...state, active });
        void host.ensureVisible(active);
      }
    }
  };

  listen(canvas, 'pointerdown', onPointerDown);
  listen(canvas, 'pointermove', onPointerMove);
  listen(canvas, 'pointerup', finishDrag);
  listen(canvas, 'pointercancel', finishDrag);
  listen(scroller, 'keydown', onKeyDown);
  listen(scroller, 'blur', () => finishDrag());

  return {
    async setSelection(reference, add, scrollIntoView) {
      const current = host.getSelection();
      if (!current) return;
      const range = typeof reference === 'string' ? parseRange(reference) : reference;
      const active = range.start;
      const next: SelectionState = {
        ...current,
        active,
        anchor: range.start,
        cursor: range.end,
        ranges: add ? [...current.ranges, range] : [range],
        mode: 'normal',
      };
      await apply(next);
      if (scrollIntoView) await host.ensureVisible(active);
    },
    async restoreSelection(selection) {
      const current = host.getSelection();
      if (!current) return;
      const primary = selection.ranges[selection.ranges.length - 1] ?? singleCell(selection.active);
      await apply({
        ...current,
        active: selection.active,
        anchor: selection.active,
        cursor: primary.end,
        ranges: selection.ranges,
        mode: 'normal',
      });
      await host.ensureVisible(selection.active);
    },
    clearSelection() {
      ++generation;
      finishDrag();
      host.setSelection(null);
    },
    destroy() {
      destroyed = true;
      ++generation;
      finishDrag();
      for (const dispose of disposers) dispose();
      disposers.clear();
    },
  };
}


