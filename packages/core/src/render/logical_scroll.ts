// Полный лист не помещается в CSS при большом зуме, поэтому DOM-прокрутка —
// только транспорт. Rust и hit-test видят независимые логические координаты.

/** Безопасный запас до браузерных ограничений на размеры CSS-элементов. */
export const MAX_DOM_SCROLL_CSS_PX = 8_000_000;

/** Одна ось прокрутки с тождественными краями и сжатой серединой. */
export interface LogicalAxis {
  readonly logicalMax: number;
  readonly domMax: number;
  toDom(logical: number): number;
  toLogical(dom: number): number;
}

const clamp = (value: number, maximum: number): number =>
  Math.min(Math.max(Number.isFinite(value) ? value : 0, 0), Math.max(0, maximum));

/**
 * Собрать отображение оси. На краях масштаб 1:1, середина сжимается линейно:
 * ползунок остаётся непрерывным, а колесо не ускоряется глобально по всему листу.
 */
export function createLogicalAxis(contentCssPx: number, viewportCssPx: number): LogicalAxis {
  const logicalMax = Math.max(0, contentCssPx - Math.max(0, viewportCssPx));
  const domMax = Math.min(logicalMax, MAX_DOM_SCROLL_CSS_PX);
  const edge = Math.min(domMax / 4, logicalMax / 4);
  const logicalMiddle = Math.max(1, logicalMax - edge * 2);
  const domMiddle = Math.max(1, domMax - edge * 2);

  return {
    logicalMax,
    domMax,
    toDom(logical) {
      const value = clamp(logical, logicalMax);
      if (value <= edge) return value;
      if (value >= logicalMax - edge) return domMax - (logicalMax - value);
      return edge + ((value - edge) * domMiddle) / logicalMiddle;
    },
    toLogical(dom) {
      const value = clamp(dom, domMax);
      if (value <= edge) return value;
      if (value >= domMax - edge) return logicalMax - (domMax - value);
      return edge + ((value - edge) * logicalMiddle) / domMiddle;
    },
  };
}

/** Две оси и фактический размер распорки в CSS-пикселях. */
export interface LogicalScroll {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  configure(contentWidth: number, contentHeight: number, zoom: number, viewportWidth: number, viewportHeight: number): void;
  syncFromDom(domX: number, domY: number): void;
  scrollTo(x: number, y: number): [number, number];
}

/** Состояние логической прокрутки; DOM-значения не являются источником истины. */
export function createLogicalScroll(): LogicalScroll {
  let x = 0;
  let y = 0;
  let axisX = createLogicalAxis(0, 0);
  let axisY = createLogicalAxis(0, 0);
  let width = 0;
  let height = 0;

  return {
    get x() { return x; },
    get y() { return y; },
    get width() { return width; },
    get height() { return height; },
    configure(contentWidth, contentHeight, zoom, viewportWidth, viewportHeight) {
      axisX = createLogicalAxis(contentWidth * zoom, viewportWidth);
      axisY = createLogicalAxis(contentHeight * zoom, viewportHeight);
      x = clamp(x, axisX.logicalMax);
      y = clamp(y, axisY.logicalMax);
      width = axisX.domMax + viewportWidth;
      height = axisY.domMax + viewportHeight;
    },
    syncFromDom(domX, domY) {
      x = axisX.toLogical(domX);
      y = axisY.toLogical(domY);
    },
    scrollTo(nextX, nextY) {
      x = clamp(nextX, axisX.logicalMax);
      y = clamp(nextY, axisY.logicalMax);
      return [axisX.toDom(x), axisY.toDom(y)];
    },
  };
}
