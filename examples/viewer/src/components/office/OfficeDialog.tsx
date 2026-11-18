import { useEffect, useRef, type ReactElement, type ReactNode } from 'react';
import { Icon } from './icons.js';

/** Модальный диалог ленты с нативным фокусом, Escape и возвратом фокуса кнопке. */
export function OfficeDialog(props: {
  title: string;
  children: ReactNode;
  onClose(): void;
  actions?: ReactNode;
}): ReactElement {
  const ref = useRef<HTMLDialogElement | null>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return undefined;
    dialog.showModal();
    const cancel = (event: Event): void => {
      event.preventDefault();
      props.onClose();
    };
    dialog.addEventListener('cancel', cancel);
    return () => {
      dialog.removeEventListener('cancel', cancel);
      dialog.close();
    };
  }, []);

  return (
    <dialog ref={ref} className="ofc-dialog" aria-label={props.title} onClose={props.onClose}>
      <div className="ofc-dialog__title">
        <h2>{props.title}</h2>
        <button type="button" aria-label="Закрыть" title="Закрыть" onClick={props.onClose}>
          <Icon name="cancel" size={18} />
        </button>
      </div>
      <div className="ofc-dialog__body">{props.children}</div>
      {props.actions === undefined ? null : <div className="ofc-dialog__actions">{props.actions}</div>}
    </dialog>
  );
}

/** Справка по локальному редактору и его клавишам. */
export function HelpDialog(props: { onClose(): void }): ReactElement {
  return (
    <OfficeDialog title="Справка" onClose={props.onClose}>
      <p>Документ изменяется локально: исходный ZIP сохраняется, а команды патчат только нужные XML-part’ы.</p>
      <ul>
        <li><kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>Z</kbd> — отменить, <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>Y</kbd> — повторить.</li>
        <li><kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>S</kbd> — сохранить исходный OOXML.</li>
        <li><kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>F</kbd> — найти текст, <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>P</kbd> — печать.</li>
      </ul>
      <p>Клик по ячейке или тексту выбирает цель правки. Правки попадают в кадр, PDF и сохранённый файл после повторного открытия документа движком.</p>
    </OfficeDialog>
  );
}
