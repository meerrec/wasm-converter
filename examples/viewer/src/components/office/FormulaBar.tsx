import type { ReactElement, ReactNode } from 'react';
import { Icon } from './icons.js';

/**
 * Строка формул «ДваИкселя»: кнопка вызова функции и поле самой формулы.
 *
 * `cellRef` — узел с именем активной ячейки; раньше его вставляла страница
 * (`<span id="cell">`), теперь это имя живёт в строке состояния, поэтому поле
 * имени стало необязательным.
 */
export function FormulaBar(props: {
  cellRef?: ReactNode;
  formula?: string;
  disabled?: boolean;
  onFormulaChange?: (value: string) => void;
  onInsertFunction?: () => void;
}): ReactElement {
  return (
    <div className="ofc-formulabar">
      {props.cellRef === undefined ? null : (
        <span className="ofc-formulabar__name">{props.cellRef}</span>
      )}
      <button type="button" className="ofc-formulabar__fx" disabled={props.disabled === true} title="Вставить функцию" onClick={props.onInsertFunction}>
        <Icon name="function" size={16} />
      </button>
      <input
        className="ofc-formulabar__formula"
        aria-label="Строка формул"
        title="Формула или значение ячейки; Enter применяет правку"
        value={props.formula ?? ''}
        disabled={props.disabled === true}
        onChange={(event) => props.onFormulaChange?.(event.currentTarget.value)}
      />
    </div>
  );
}
