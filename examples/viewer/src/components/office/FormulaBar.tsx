import type { ReactElement, ReactNode } from 'react';
import { Icon } from './icons.js';

/**
 * Строка формул «ДваИкселя»: кнопка вызова функции и поле самой формулы.
 *
 * `cellRef` — узел с именем активной ячейки; раньше его вставляла страница
 * (`<span id="cell">`), теперь это имя живёт в строке состояния, поэтому поле
 * имени стало необязательным.
 */
export function FormulaBar(props: { cellRef?: ReactNode; formula?: string }): ReactElement {
  return (
    <div className="ofc-formulabar">
      {props.cellRef === undefined ? null : (
        <span className="ofc-formulabar__name">{props.cellRef}</span>
      )}
      {/* Кнопка недоступна, пока функцию некуда вставлять: строка формул
          показывает вьюер, а не редактирует книгу. */}
      <button type="button" className="ofc-formulabar__fx" disabled title="Вставить функцию">
        <Icon name="function" size={16} />
      </button>
      <span className="ofc-formulabar__formula">{props.formula ?? ''}</span>
    </div>
  );
}
