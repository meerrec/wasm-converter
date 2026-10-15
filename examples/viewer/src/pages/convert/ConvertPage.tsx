import type { ReactElement } from 'react';
import { Link } from 'react-router-dom';
import { useConvertPage } from './use_convert_page.js';
import './convert.css';

/**
 * Страница «Конвертер»: конвертация OOXML без сервера через x2t плюс витрина
 * пакета `@meerrec/browser-convert` — медиа-экстракция и препроцессоры.
 *
 * Конвертация — единственная секция, которой нужен x2t: медиа и препроцессинг
 * работают на чистом JS и переживают недоступный рантайм. Рантайм (~39 МБ)
 * поднимается лениво, при первой конвертации.
 */

/**
 * Принятые файлы: конвертируются форматы обоих семейств, а `pptx` здесь ради
 * витрины — его берут не конвертация, а медиа-экстракция и препроцессор
 * notes/docProps.
 */
const ACCEPT = '.docx,.odt,.rtf,.txt,.xlsx,.ods,.csv,.pptx';

/** Расшифровки кодов x2t — короткие, чтобы строка не расплывалась. */
const CODE_NOTES: Readonly<Record<number, string>> = {
  0: 'успешно',
  80: 'цель требует движка раскладки (pdf/html/png/jpg) — его в wasm нет',
  88: 'кросс-семейная конвертация не поддержана',
};

/** Страница «Конвертер». */
export default function ConvertPage(): ReactElement {
  const page = useConvertPage();
  return (
    <div className="conv-app">
      {/* Шапка — общий `header` из styles.css; переключателя страниц в примере
          нет, поэтому ссылки на соседей живут хотя бы здесь. */}
      <header>
        <h1>Конвертер</h1>
        <nav className="conv-nav">
          <Link to="/docx">Слово</Link>
          <Link to="/xlsx">ДваИксель</Link>
        </nav>
        <p className="conv-lead">
          OOXML-конвертация без сервера: wasm-сборка конвертера ONLYOFFICE (x2t) исполняется в
          воркере, рантайм ~39 МБ поднимается лениво — при первой конвертации.
        </p>
      </header>
      <main className="conv-main">
        <section className="conv-section">
          <h2>Конвертация</h2>
          <div className="conv-row">
            <label>
              Файл
              <input id="convert-file" type="file" accept={ACCEPT} onChange={page.onFileChange} />
            </label>
            <label>
              Целевой формат
              <select
                id="convert-target"
                value={page.target}
                disabled={page.targets.length === 0 || page.busy}
                onChange={(event) => {
                  page.onTargetChange(event.target.value);
                }}
              >
                {page.targets.length === 0 ? (
                  <option value="">—</option>
                ) : (
                  page.targets.map((format) => (
                    <option key={format} value={format}>
                      {format}
                    </option>
                  ))
                )}
              </select>
            </label>
            <button
              id="convert-run"
              type="button"
              disabled={!page.canConvert}
              onClick={page.onConvert}
            >
              {page.busy ? 'В работе…' : 'Конвертировать'}
            </button>
          </div>
          {page.fileName === null ? (
            <p className="conv-note">
              Выберите docx/odt/rtf/txt или xlsx/ods/csv — цели строятся по семейству файла.
            </p>
          ) : page.family === null ? (
            <p className="conv-note">
              {page.fileName}: конвертация этого формата не поддержана — доступны docx/odt/rtf/txt
              и xlsx/ods/csv. Секции «Медиа» и «Препроцессинг» ниже работают, если формат им
              знаком.
            </p>
          ) : (
            <p className="conv-note">
              {page.fileName}: семейство {page.familyLabel} — цели: {page.targets.join(', ')}.
            </p>
          )}
          {page.error !== null && <p className="conv-error">Ошибка: {page.error}</p>}
          {page.result !== null && (
            <>
              <p id="convert-status" className="conv-status">
                код {page.result.code} — {CODE_NOTES[page.result.code] ?? 'неизвестный код'} ·{' '}
                {Math.round(page.result.convertMs)} мс
              </p>
              {page.result.stderr.trim() !== '' && (
                <pre id="convert-stderr" className="conv-detail">
                  {page.result.stderr.trim()}
                </pre>
              )}
              {page.result.code === 0 && (
                <button id="convert-download" type="button" onClick={page.onDownloadResult}>
                  Скачать {page.result.fileName}
                </button>
              )}
            </>
          )}
        </section>
        <section className="conv-section">
          <h2>Медиа</h2>
          <div className="conv-row">
            <button
              id="media-show"
              type="button"
              disabled={!page.canExtractMedia}
              onClick={page.onShowMedia}
            >
              {page.mediaBusy ? 'Извлекаю…' : 'Показать медиа'}
            </button>
            <span className="conv-note">
              Картинки и вложения из docx/xlsx/pptx: object URL-ы, отзываются при повторном
              извлечении, смене файла и уходе со страницы.
            </span>
          </div>
          {page.mediaError !== null && <p className="conv-error">Ошибка: {page.mediaError}</p>}
          {page.media !== null &&
            (page.media.length === 0 ? (
              <p id="media-empty" className="conv-note">
                Медиа в этом файле не найдено.
              </p>
            ) : (
              <ul id="media-gallery" className="conv-media">
                {page.media.map((item) => (
                  <li key={item.name} className="conv-media__item">
                    {item.mime.startsWith('image/') ? (
                      <img src={item.url} alt={item.name} />
                    ) : (
                      <span className="conv-media__mime">{item.mime}</span>
                    )}
                    <span className="conv-media__name">{item.name}</span>
                  </li>
                ))}
              </ul>
            ))}
        </section>
        <section className="conv-section">
          <h2>Препроцессинг</h2>
          {page.preprocessor === null ? (
            <p className="conv-note">
              Препроцессоры есть у xlsx (снять двойное экранирование переносов), pptx (фиксы
              notes/docProps) и docx (развернуть ruby) — выберите файл одного из этих типов.
            </p>
          ) : (
            <div className="conv-row">
              <button
                id={`preprocess-${page.preprocessor.id}`}
                type="button"
                disabled={page.preprocessBusy}
                onClick={page.onPreprocess}
              >
                {page.preprocessor.label}
              </button>
            </div>
          )}
          {page.preprocessError !== null && (
            <p className="conv-error">Ошибка: {page.preprocessError}</p>
          )}
          {page.preprocessed !== null && (
            <>
              <p id="preprocess-status" className="conv-status">
                {page.preprocessed.changed
                  ? `изменено (было ${page.preprocessed.before} Б → стало ${page.preprocessed.after} Б)`
                  : 'без изменений'}
              </p>
              <button id="preprocess-download" type="button" onClick={page.onDownloadPreprocessed}>
                Скачать результат
              </button>
            </>
          )}
        </section>
      </main>
    </div>
  );
}
