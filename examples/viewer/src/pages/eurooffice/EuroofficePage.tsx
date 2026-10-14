import type { ReactElement } from 'react';
import { ACCEPT, useEuroofficePage } from './use_eurooffice_page.js';
import './eurooffice.css';

/**
 * Страница «Euro-Office»: просмотр документа во встроенном редакторе — оболочка
 * в стиле его сайта вокруг iframe `editor.html?embed=1`.
 *
 * Файл читает страница и отдаёт редактору байтами в режиме только просмотра,
 * поэтому странице не нужны ни CORS, ни серверный адрес документа, а правки в
 * редакторе выключены. Состояние и обработчики — в `useEuroofficePage`, здесь
 * остаются разметка и контракт id (`#viewer-open`, `#viewer-file`,
 * `#viewer-doc-name`, `#viewer-status`, `#viewer-frame`, `#viewer-drop-overlay`).
 */
export default function EuroofficePage(): ReactElement {
  const page = useEuroofficePage();
  return (
    <div className="eo-app">
      {/* Шапка тёмная всегда — как хром редактора: тема страницы на неё не влияет. */}
      <header className="eo-toolbar">
        <div className="eo-brand">
          <span className="eo-logo">E</span>
          <span className="eo-wordmark">Euro-Office</span>
        </div>
        <div className="eo-doc-name" id="viewer-doc-name" title={page.docName}>
          {page.docName}
        </div>
        <div className="eo-actions">
          <button className="eo-open" id="viewer-open" type="button" onClick={page.onOpenClick}>
            Открыть документ
          </button>
          <span id="viewer-status" data-state={page.state}>
            {page.statusText}
          </span>
        </div>
      </header>
      {/* Поле скрыто: диалог открывает кнопка тулбара, а выбор читает хук —
          тем же путём, что и брошенный файл. */}
      <input
        ref={page.fileInputRef}
        id="viewer-file"
        type="file"
        accept={ACCEPT}
        onChange={page.onFileChange}
        hidden
      />
      <main className="eo-frame-wrap">
        {/* `src` проставляет хук — после того, как встанет корневой воркер
            изоляции; без атрибута рамка ждёт и никуда не уходит. */}
        <iframe ref={page.frameRef} id="viewer-frame" src={page.frameSrc} title="Редактор Euro-Office" />
        <div
          className={page.dropActive ? 'eo-drop-overlay visible' : 'eo-drop-overlay'}
          id="viewer-drop-overlay"
        >
          Перетащите документ сюда — он откроется в режиме просмотра
        </div>
      </main>
    </div>
  );
}
