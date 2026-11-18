import type { ReactElement, ReactNode } from 'react';
import { Icon } from './icons.js';

/**
 * Титульная строка приложения в духе Office Online: светлая, с логотипом и
 * названием приложения слева, именем документа рядом и поиском справа.
 *
 * `children` — место страницы: контролы открытия файла, выбора фикстуры и всё
 * остальное, что относится к документу, а не к каркасу.
 */
export function TitleBar(props: {
  logo: ReactNode;
  app: string;
  documentName: string;
  children?: ReactNode;
  onSearch?: () => void;
  onUndo?: () => void;
  onRedo?: () => void;
  onSave?: () => void;
  canUndo?: boolean;
  canRedo?: boolean;
  saveDisabled?: boolean;
}): ReactElement {
  return (
    <header className="ofc-titlebar">
      <span className="ofc-titlebar__brand">
        {props.logo}
        <span className="ofc-titlebar__app">{props.app}</span>
      </span>
      <span className="ofc-titlebar__divider" />
      <span className="ofc-titlebar__doc">{props.documentName}</span>
      {props.children === undefined ? null : (
        <span className="ofc-titlebar__actions">{props.children}</span>
      )}
      <span className="ofc-titlebar__spacer" />
      {props.onUndo === undefined ? null : (
        <span className="ofc-titlebar__quick">
          <button type="button" title="Отменить (Ctrl+Z)" aria-label="Отменить" disabled={props.canUndo !== true} onClick={props.onUndo}><Icon name="undo" size={16} /></button>
          <button type="button" title="Повторить (Ctrl+Y)" aria-label="Повторить" disabled={props.canRedo !== true} onClick={props.onRedo}><Icon name="redo" size={16} /></button>
          <button type="button" title="Сохранить (Ctrl+S)" aria-label="Сохранить" disabled={props.saveDisabled === true} onClick={props.onSave}><Icon name="save" size={16} /></button>
        </span>
      )}
      {/* Поиск открывает ту же команду, что и «Найти» на вкладке «Главная». */}
      <button
        type="button"
        className="ofc-titlebar__search"
        disabled={props.onSearch === undefined}
        onClick={() => {
          props.onSearch?.();
        }}
      >
        <Icon name="search" size={16} />
        <span>Поиск</span>
      </button>
      <span className="ofc-titlebar__avatar" title="Учётная запись">
        <Icon name="account" size={16} />
      </span>
    </header>
  );
}
