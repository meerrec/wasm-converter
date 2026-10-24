import type { ReactElement, ReactNode } from 'react';
import { Icon } from './icons.js';

/**
 * Строка состояния приложения: слева — то, что относится к документу
 * (страница, подсказка), в центре — метрики кадров, справа — управление
 * видом и сообщения.
 *
 * `children` уезжает в середину: там стоят счётчики вьюера, которые меняются
 * десятки раз в секунду и пишутся в DOM мимо React.
 */
export function StatusBar(props: { left?: ReactNode; right?: ReactNode; children?: ReactNode }): ReactElement {
  return (
    <footer className="ofc-statusbar">
      <span className="ofc-side ofc-side--left">{props.left}</span>
      <span className="ofc-statusbar__meters">{props.children}</span>
      <span className="ofc-side ofc-side--right">{props.right}</span>
    </footer>
  );
}

/** Значение масштаба и его подпись в списке — как в ленте Office. */
export interface StatusZoomOption {
  value: string;
  label: string;
}

/**
 * Масштаб в строке состояния: «−», список процентов и «+». Список остаётся
 * обычным `<select>` с id `#zoom` — на нём держится контракт примера, — а
 * кнопки лишь выбирают соседнее значение того же списка, поэтому состояние
 * всегда одно и синхронно.
 */
export function StatusZoom(props: {
  id: string;
  options: readonly StatusZoomOption[];
  value: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}): ReactElement {
  const index = props.options.findIndex((option) => option.value === props.value);
  const prev = index > 0 ? props.options[index - 1] : undefined;
  const next = index >= 0 && index < props.options.length - 1 ? props.options[index + 1] : undefined;
  return (
    <span className="ofc-zoom" title="Масштаб">
      <button
        type="button"
        className="ofc-zoom__btn"
        aria-label="Уменьшить масштаб"
        disabled={props.disabled === true || prev === undefined}
        onClick={() => {
          if (prev !== undefined) props.onChange(prev.value);
        }}
      >
        −
      </button>
      <select
        id={props.id}
        className="ofc-zoom__select"
        aria-label="Масштаб"
        title="Масштаб"
        value={props.value}
        disabled={props.disabled === true}
        onChange={(event) => {
          props.onChange(event.target.value);
        }}
      >
        {props.options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <button
        type="button"
        className="ofc-zoom__btn"
        aria-label="Увеличить масштаб"
        disabled={props.disabled === true || next === undefined}
        onClick={() => {
          if (next !== undefined) props.onChange(next.value);
        }}
      >
        +
      </button>
    </span>
  );
}
