import type { ReactElement, ReactNode } from 'react';
import { Icon, type IconName } from './icons.js';

/**
 * Лента Office: вкладки, группы команд и элементы внутри групп.
 *
 * Компоненты намеренно «глупые» — состояние (активная вкладка, выбранный
 * шрифт, нажатые переключатели) держит страница: одна и та же лента служит и
 * «Слову», и «ДваИкселю», а набор команд у них разный.
 */

/** Вкладка ленты: `id` — то, что страница вернёт из `onSelect`. */
export interface RibbonTab {
  id: string;
  label: string;
}

/**
 * Вкладки ленты. Первая — «Файл» (акцентная), активная подсвечена белым с
 * синей полосой снизу.
 */
export function RibbonTabs(props: {
  tabs: RibbonTab[];
  activeId: string;
  onSelect: (id: string) => void;
}): ReactElement {
  return (
    <div className="ofc-tabs" role="tablist">
      {props.tabs.map((tab, index) => {
        // Акцент «Файла» задан позицией, а не именем: так вкладку можно
        // переименовать, не потеряв её вид.
        const file = index === 0 ? ' ofc-tab--file' : '';
        const active = tab.id === props.activeId ? ' ofc-tab--active' : '';
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={tab.id === props.activeId}
            className={`ofc-tab${file}${active}`}
            onClick={() => {
              props.onSelect(tab.id);
            }}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * Группа команд ленты с подписью снизу — как «Буфер обмена» или «Шрифт».
 * `grow` растягивает группу на свободное место (для галерей и полей).
 */
export function RibbonGroup(props: {
  label: string;
  children: ReactNode;
  grow?: boolean;
}): ReactElement {
  return (
    <div className={`ofc-group${props.grow === true ? ' ofc-group--grow' : ''}`}>
      <div className="ofc-group__items">{props.children}</div>
      <div className="ofc-group__label">{props.label}</div>
    </div>
  );
}

/** Подпись под крупной кнопкой: разбивается на две строки, как в Office. */
function LargeLabel(props: { text: string }): ReactElement {
  return <span className="ofc-btn__label">{props.text}</span>;
}

/**
 * Кнопка ленты: крупная (иконка и подпись) или мелкая (только иконка, смысл —
 * в подсказке `title`).
 */
export function RibbonButton(props: {
  icon: IconName;
  label: string;
  large?: boolean;
  disabled?: boolean;
  active?: boolean;
  title?: string;
  id?: string;
  onClick?: () => void;
}): ReactElement {
  const large = props.large === true;
  const classes = ['ofc-btn', large ? 'ofc-btn--large' : 'ofc-btn--small'];
  if (props.active === true) classes.push('ofc-btn--active');
  const className = classes.join(' ');
  return (
    <button
      type="button"
      className={className}
      id={props.id}
      // Подсказка по умолчанию — сама подпись: у мелкой кнопки другого текста нет.
      title={props.title ?? props.label}
      aria-label={props.label}
      aria-pressed={props.active}
      disabled={props.disabled === true}
      onClick={props.onClick}
    >
      <Icon name={props.icon} size={large ? 24 : 18} />
      {large ? <LargeLabel text={props.label} /> : null}
    </button>
  );
}

/**
 * Выпадающий список ленты: гарнитура, размер, формат числа.
 *
 * `onChange` передаётся обработчику всегда, даже когда он не задан: без него
 * React считает `<select>` с `value` только для чтения и ругается в консоль.
 *
 * `id` добавлен к зафиксированному набору пропсов: он нужен списку масштаба,
 * за которым держится страница (`#zoom` в id-контракте примера). Пропс
 * необязательный, поэтому остальные списки ленты о нём не знают.
 */
export function RibbonSelect(props: {
  options: string[];
  value: string;
  ariaLabel: string;
  disabled?: boolean;
  width?: number;
  title?: string;
  id?: string;
  onChange?: (v: string) => void;
}): ReactElement {
  return (
    <span
      className="ofc-select"
      style={props.width === undefined ? undefined : { width: `${props.width}px` }}
    >
      <select
        className="ofc-select__input"
        id={props.id}
        aria-label={props.ariaLabel}
        title={props.title ?? props.ariaLabel}
        value={props.value}
        disabled={props.disabled === true}
        onChange={(event) => {
          props.onChange?.(event.target.value);
        }}
      >
        {props.options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
      <Icon name="chevron-down" size={12} />
    </span>
  );
}

/** Кнопка-переключатель (полужирный, курсив, выравнивание, сетка). */
export function RibbonToggle(props: {
  icon: IconName;
  label: string;
  active?: boolean;
  disabled?: boolean;
  title?: string;
  onClick?: () => void;
}): ReactElement {
  return (
    <button
      type="button"
      className={`ofc-toggle${props.active === true ? ' ofc-toggle--active' : ''}`}
      title={props.title ?? props.label}
      aria-label={props.label}
      aria-pressed={props.active ?? false}
      disabled={props.disabled === true}
      onClick={props.onClick}
    >
      <Icon name={props.icon} size={18} />
    </button>
  );
}

/**
 * Плитка галереи стилей: `sample` — образец текста, `className` — класс с
 * оформлением образца (заголовок крупнее и синий и т.п.).
 */
export function RibbonTile(props: {
  title: string;
  sample: string;
  className?: string;
  disabled?: boolean;
}): ReactElement {
  return (
    <button
      type="button"
      className={`ofc-tile${props.className === undefined ? '' : ` ${props.className}`}`}
      title={props.title}
      aria-label={props.title}
      disabled={props.disabled === true}
    >
      <span className="ofc-tile__sample">{props.sample}</span>
    </button>
  );
}
