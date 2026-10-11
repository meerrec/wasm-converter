import type { ReactElement, ReactNode } from 'react';
import { Icon } from './icons.js';

/**
 * Строка формул «ДваИкселя»: поле имени ячейки, кнопка вызова функции и поле
 * самой формулы.
 *
 * `cellRef` — узел, а не строка: имя активной ячейки пишет вьюер мимо React, и
 * страница вставляет сюда свой `<span id="cell">`, за который он держится.
 */
export function FormulaBar(props: { cellRef: ReactNode; formula?: string }): ReactElement {
  return (
    <div className="ofc-formulabar">
      <span className="ofc-formulabar__name">{props.cellRef}</span>
      {/* Кнопка недоступна, пока функцию некуда вставлять: строка формул
          показывает вьюер, а не редактирует книгу. */}
      <button type="button" className="ofc-formulabar__fx" disabled title="Вставить функцию">
        <Icon name="function" size={16} />
      </button>
      <span className="ofc-formulabar__formula">{props.formula ?? ''}</span>
    </div>
  );
}
