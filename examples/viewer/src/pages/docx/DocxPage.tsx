import { useState, type ChangeEvent, type ReactElement } from 'react';
import { FrameMeters } from '../../components/FrameMeters.js';
import {
  DrawingPad,
  HelpDialog,
  Icon,
  LoadingOverlay,
  LogoWord,
  RibbonButton,
  RibbonGroup,
  RibbonSelect,
  RibbonTabs,
  RibbonTile,
  RibbonToggle,
  Ruler,
  StatusBar,
  StatusZoom,
  SpellcheckDialog,
  TitleBar,
  type IconName,
  type RibbonTab,
  type StatusZoomOption,
} from '../../components/office/index.js';
import { useDocxPage, type ExportKind } from './use_docx_page.js';

/**
 * Страница DOCX («Слово»): каркас Office вокруг вьюера.
 *
 * Работает ровно то, что делает вьюер: открытие документа, выбор фикстуры,
 * перетаскивание файла, переход по страницам, зум и экспорт. Остальные команды
 * ленты — правдоподобный, но недоступный фон; состояние и вызовы вьюера живут
 * в `useDocxPage`, здесь остаются разметка и контракт id (`#viewer`,
 * `#page-prev`, `#page-info`, `#page-next`, `#pages`, `#status`, `#zoom`,
 * `#export-*`, `#edit-run`, счётчики кадров).
 */

/** Вкладки ленты — как в Word. */
const TABS: RibbonTab[] = [
  { id: 'file', label: 'Файл' },
  { id: 'home', label: 'Главная' },
  { id: 'insert', label: 'Вставка' },
  { id: 'draw', label: 'Рисование' },
  { id: 'design', label: 'Дизайн' },
  { id: 'layout', label: 'Макет' },
  { id: 'refs', label: 'Ссылки' },
  { id: 'mail', label: 'Рассылки' },
  { id: 'review', label: 'Рецензирование' },
  { id: 'view', label: 'Вид' },
  { id: 'help', label: 'Справка' },
];

/** Гарнитуры и размеры — те, что предлагает Word по умолчанию. */
const FONTS = ['Calibri', 'Cambria', 'Times New Roman', 'Arial'];
const SIZES = ['8', '9', '10', '11', '12', '14', '16', '18', '24'];

/** Масштабы зума: те же, что у вьюера, и в том же виде, что в Office. */
const ZOOM = [
  { value: 0.5, label: '50 %' },
  { value: 0.75, label: '75 %' },
  { value: 1, label: '100 %' },
  { value: 1.5, label: '150 %' },
  { value: 2, label: '200 %' },
];

/** Значения списка масштаба в подвале: подписи Office и есть значения `#zoom`. */
const ZOOM_OPTIONS: readonly StatusZoomOption[] = ZOOM.map((option) => ({
  value: option.label,
  label: option.label,
}));

/** Множитель по подписи из списка; отбор идёт по своему же списку. */
function zoomByLabel(label: string): number {
  const found = ZOOM.find((option) => option.label === label);
  return found === undefined ? 1 : found.value;
}

/** Подпись текущего масштаба — её показывают лента и строка состояния. */
function zoomLabel(value: number): string {
  const found = ZOOM.find((option) => option.value === value);
  return found === undefined ? '100 %' : found.label;
}

/** Группа команд вкладки-заглушки. */
interface RibbonSpec {
  label: string;
  buttons: { icon: IconName; label: string; command: string }[];
}

/**
 * Команды остальных вкладок: набор правдоподобный, но всё недоступно —
 * вкладка переключается, лента не пустует, а делать ничего не обещает.
 */
const OTHER_RIBBONS: Record<string, RibbonSpec[]> = {
  file: [{ label: 'Документ', buttons: [
    { icon: 'file', label: 'Создать', command: 'create' },
    { icon: 'save', label: 'Сохранить', command: 'save' },
    { icon: 'print', label: 'Печать', command: 'print' },
    { icon: 'share', label: 'Поделиться', command: 'share' },
  ] }],
  insert: [
    { label: 'Страницы', buttons: [
      { icon: 'file', label: 'Титульная страница', command: 'insert-cover' },
      { icon: 'page-setup', label: 'Разрыв', command: 'insert-break' },
    ] },
    { label: 'Таблицы', buttons: [{ icon: 'table', label: 'Таблица', command: 'insert-table' }] },
    { label: 'Иллюстрации', buttons: [
      { icon: 'picture', label: 'Рисунок', command: 'insert-picture' },
      { icon: 'chart', label: 'Диаграмма', command: 'insert-chart' },
    ] },
  ],
  draw: [{ label: 'Перо', buttons: [
    { icon: 'format-painter', label: 'Перо', command: 'pen' },
    { icon: 'highlight', label: 'Маркер', command: 'marker' },
    { icon: 'clear', label: 'Ластик', command: 'eraser' },
  ] }],
  design: [{ label: 'Стили документа', buttons: [
    { icon: 'styles', label: 'Темы', command: 'theme' },
    { icon: 'format', label: 'Набор стилей', command: 'style-set' },
  ] }],
  layout: [
    { label: 'Параметры страницы', buttons: [
      { icon: 'page-setup', label: 'Поля', command: 'page-margins' },
      { icon: 'sheet', label: 'Ориентация', command: 'page-orientation' },
      { icon: 'text', label: 'Размер', command: 'page-size' },
    ] },
    { label: 'Абзац', buttons: [
      { icon: 'indent-decrease', label: 'Отступ слева', command: 'indent-left' },
      { icon: 'indent-increase', label: 'Отступ справа', command: 'indent-right' },
    ] },
  ],
  refs: [
    { label: 'Оглавление', buttons: [
      { icon: 'multilevel-list', label: 'Оглавление', command: 'insert-toc' },
      { icon: 'link', label: 'Ссылка', command: 'insert-link' },
    ] },
    { label: 'Названия', buttons: [
      { icon: 'cell', label: 'Вставить название', command: 'insert-caption' },
      { icon: 'text', label: 'Перекрёстная ссылка', command: 'insert-cross-ref' },
    ] },
  ],
  mail: [{ label: 'Начало слияния', buttons: [
    { icon: 'file', label: 'Начать слияние', command: 'mail-start' },
    { icon: 'select', label: 'Выбрать получателей', command: 'mail-recipients' },
  ] }],
  review: [
    { label: 'Рецензирование', buttons: [
      { icon: 'comment', label: 'Создать примечание', command: 'add-comment' },
      { icon: 'review', label: 'Принять', command: 'accept-changes' },
      { icon: 'cancel', label: 'Отклонить', command: 'reject-changes' },
    ] },
    { label: 'Правописание', buttons: [
      { icon: 'spell-check', label: 'Орфография', command: 'spellcheck' },
      { icon: 'find', label: 'Область проверки', command: 'find' },
    ] },
  ],
  view: [
    { label: 'Режимы просмотра', buttons: [
      { icon: 'view', label: 'Режим чтения', command: 'reading' },
      { icon: 'cell', label: 'Разметка страницы', command: 'page-layout' },
    ] },
    { label: 'Отображение', buttons: [
      { icon: 'ruler', label: 'Линейка', command: 'ruler' },
      { icon: 'grid', label: 'Сетка', command: 'grid' },
      { icon: 'paragraph-mark', label: 'Знаки абзацев', command: 'paragraph-marks' },
    ] },
  ],
  help: [{ label: 'Справка', buttons: [
    { icon: 'help', label: 'Справка', command: 'help' },
    { icon: 'comment', label: 'Отзыв', command: 'feedback' },
  ] }],
};

/** Пропсы ленты «Главная»: всё, что доступно от вьюера и страницы. */
interface HomeRibbonProps {
  ready: boolean;
  fixtures: string[];
  fixture: string;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onFixtureChange: (event: ChangeEvent<HTMLSelectElement>) => void;
  canPrev: boolean;
  canNext: boolean;
  pageCount: number;
  onStepPage: (delta: number) => void;
  onExport: (kind: ExportKind) => void;
  exportDisabled: (kind: ExportKind) => boolean;
  /** Правка текста: документ открыт — поле `#edit-run` принимает ввод. */
  opened: boolean;
  editText: string;
  editTarget: string | null;
  onEditText: (text: string) => void;
  onEditRun: () => void;
  onCommand: (command: string, args?: Record<string, unknown>) => void;
}

/** Лента «Главная»: та же, что в Word, и так же разложена по группам. */
function HomeRibbon(props: HomeRibbonProps): ReactElement {
  const [fontFamily, setFontFamily] = useState('Calibri');
  const [fontSize, setFontSize] = useState('11');
  const [toggles, setToggles] = useState<Record<string, boolean>>({});
  const run = (command: string, args: Record<string, unknown> = {}, toggle = false): void => {
    props.onCommand(command, args);
    if (toggle) setToggles((current) => ({ ...current, [command]: !current[command] }));
  };
  return (
    <div className="ofc-ribbon-body">
      {/* Открыть — первое, что делает страница: файл с диска или фикстура из
          списка. Без вьюера открывать нечего, поэтому контролы ждут его. */}
      <RibbonGroup label="Открыть">
        <div className="ofc-open">
          <label className="ofc-file" title="Открыть документ с диска">
            <input
              id="file"
              type="file"
              accept=".docx"
              disabled={!props.ready}
              onChange={props.onFileChange}
            />
            <Icon name="file" size={14} />
            Открыть
          </label>
          <select
            id="fixture"
            title="Фикстура"
            value={props.fixture}
            disabled={!props.ready}
            onChange={props.onFixtureChange}
          >
            <option value="">— фикстура —</option>
            {props.fixtures.map((name) => (
              <option key={name} value={name}>
                {/* Категорию видно в значении, а в подписи она только мешает:
                    документы в списке различает имя файла. */}
                {name.replace(/\.docx$/, '')}
              </option>
            ))}
          </select>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Буфер обмена">
        <RibbonButton icon="paste" label="Вставить" large disabled={!props.opened} onClick={() => run('paste')} />
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="cut" label="Вырезать" disabled={!props.opened} onClick={() => run('cut')} />
            <RibbonButton icon="copy" label="Копировать" disabled={!props.opened} onClick={() => run('copy')} />
          </div>
          <div className="ofc-row">
            <RibbonButton icon="format-painter" label="Формат по образцу" disabled={!props.opened} onClick={() => run('format-painter')} />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Шрифт">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonSelect options={FONTS} value={fontFamily} ariaLabel="Гарнитура" width={96} disabled={!props.opened} onChange={(value) => { setFontFamily(value); run('font-family', { value }); }} />
            <RibbonSelect options={SIZES} value={fontSize} ariaLabel="Размер шрифта" width={44} disabled={!props.opened} onChange={(value) => { setFontSize(value); run('font-size', { value: Number(value) }); }} />
            <RibbonButton icon="text-color" label="Цвет текста" disabled={!props.opened} onClick={() => { const value = window.prompt('Цвет текста RRGGBB', '000000'); if (value) run('text-color', { value }); }} />
            <RibbonButton icon="highlight" label="Выделение цветом" disabled={!props.opened} onClick={() => { const value = window.prompt('Цвет выделения', 'yellow'); if (value) run('highlight', { value }); }} />
          </div>
          <div className="ofc-row">
            <RibbonToggle icon="bold" label="Полужирный" active={toggles['toggle-bold'] ?? false} disabled={!props.opened} onClick={() => run('toggle-bold', {}, true)} />
            <RibbonToggle icon="italic" label="Курсив" active={toggles['toggle-italic'] ?? false} disabled={!props.opened} onClick={() => run('toggle-italic', {}, true)} />
            <RibbonToggle icon="underline" label="Подчёркнутый" active={toggles['toggle-underline'] ?? false} disabled={!props.opened} onClick={() => run('toggle-underline', {}, true)} />
            <RibbonToggle icon="strikethrough" label="Зачёркнутый" active={toggles['toggle-strikethrough'] ?? false} disabled={!props.opened} onClick={() => run('toggle-strikethrough', {}, true)} />
            <RibbonToggle icon="subscript" label="Подстрочный знак" active={toggles['toggle-subscript'] ?? false} disabled={!props.opened} onClick={() => run('toggle-subscript', {}, true)} />
            <RibbonToggle icon="superscript" label="Надстрочный знак" active={toggles['toggle-superscript'] ?? false} disabled={!props.opened} onClick={() => run('toggle-superscript', {}, true)} />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Абзац">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="bullets" label="Маркированный список" disabled={!props.opened} onClick={() => run('bullets', {}, true)} />
            <RibbonButton icon="numbering" label="Нумерованный список" disabled={!props.opened} onClick={() => run('numbering', {}, true)} />
            <RibbonButton icon="multilevel-list" label="Многоуровневый список" disabled={!props.opened} onClick={() => run('multilevel-list', {}, true)} />
            <RibbonButton icon="line-spacing" label="Междустрочный интервал" disabled={!props.opened} onClick={() => { const value = window.prompt('Междустрочный интервал', '1.5'); if (value) run('line-spacing', { value: Number(value) }); }} />
          </div>
          <div className="ofc-row">
            <RibbonButton icon="align-left" label="Выровнять по левому краю" disabled={!props.opened} onClick={() => run('align', { value: 'left' })} />
            <RibbonButton icon="align-center" label="Выровнять по центру" disabled={!props.opened} onClick={() => run('align', { value: 'center' })} />
            <RibbonButton icon="align-right" label="Выровнять по правому краю" disabled={!props.opened} onClick={() => run('align', { value: 'right' })} />
            <RibbonButton icon="align-justify" label="Выровнять по ширине" disabled={!props.opened} onClick={() => run('align', { value: 'justify' })} />
            <RibbonButton icon="indent-decrease" label="Уменьшить отступ" disabled={!props.opened} onClick={() => run('indent', { value: 'left', delta: -360 })} />
            <RibbonButton icon="indent-increase" label="Увеличить отступ" disabled={!props.opened} onClick={() => run('indent', { value: 'left', delta: 360 })} />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Стили">
        <RibbonTile title="Обычный" sample="Обычный" className="ofc-tile--normal ofc-tile--narrow" disabled={!props.opened} onClick={() => run('style', { value: 'Normal' })} />
        <RibbonTile title="Заголовок 1" sample="Заголовок" className="ofc-tile--title1 ofc-tile--narrow" disabled={!props.opened} onClick={() => run('style', { value: 'Heading1' })} />
      </RibbonGroup>
      <RibbonGroup label="Редактирование">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="find" label="Найти" disabled={!props.opened} onClick={() => run('find')} />
            <RibbonButton icon="replace" label="Заменить" disabled={!props.opened} onClick={() => run('replace')} />
            <RibbonButton icon="select" label="Выделить" disabled={!props.opened} onClick={() => run('select-all')} />
          </div>
          {/* Правка текста: клик по тексту документа заполняет поле адресом
              run'а, Enter применяет правку и перераскладывает документ. */}
          <label
            className="ofc-edit-cell"
            title="Текст run'а под курсором; Enter применяет правку"
          >
            править текст
            <input
              id="edit-run"
              type="text"
              placeholder={props.editTarget ?? 'текст'}
              value={props.editText}
              disabled={!props.opened}
              style={{ width: 130 }}
              onChange={(event) => {
                props.onEditText(event.target.value);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  props.onEditRun();
                }
              }}
            />
          </label>
        </div>
      </RibbonGroup>
      {/* Экспорт — первое, что делает вьюер: без открытого документа
          экспортировать нечего, поэтому кнопки ждут его. */}
      <RibbonGroup label="Экспорт">
        <RibbonButton
          icon="pdf"
          label="PDF"
          large
          id="export-pdf"
          disabled={props.exportDisabled('pdf')}
          onClick={() => {
            props.onExport('pdf');
          }}
        />
        <RibbonButton
          icon="markdown"
          label="Markdown"
          large
          id="export-md"
          disabled={props.exportDisabled('markdown')}
          onClick={() => {
            props.onExport('markdown');
          }}
        />
        <RibbonButton
          icon="html"
          label="HTML"
          large
          id="export-html"
          disabled={props.exportDisabled('html')}
          onClick={() => {
            props.onExport('html');
          }}
        />
        <RibbonButton
          icon="text"
          label="Текст"
          large
          id="export-txt"
          disabled={props.exportDisabled('text')}
          onClick={() => {
            props.onExport('text');
          }}
        />
      </RibbonGroup>
      {/* Навигация — переход по страницам; сам счётчик «страница 1 / 2» живёт
          в строке состояния, здесь остаются кнопки и общее число страниц. */}
      <RibbonGroup label="Навигация">
        <div className="ofc-row">
          <button
            id="page-prev"
            type="button"
            className="ofc-navbtn"
            title="Предыдущая страница"
            disabled={!props.canPrev}
            onClick={() => {
              props.onStepPage(-1);
            }}
          >
            ←
          </button>
          <span className="ofc-navlabel">
            страниц: <b id="pages">{props.pageCount}</b>
          </span>
          <button
            id="page-next"
            type="button"
            className="ofc-navbtn"
            title="Следующая страница"
            disabled={!props.canNext}
            onClick={() => {
              props.onStepPage(1);
            }}
          >
            →
          </button>
        </div>
      </RibbonGroup>
    </div>
  );
}

/** Страница «Слово». */
export default function DocxPage(): ReactElement {
  // Вкладка ленты — вид страницы, а не документ: её место здесь, рядом с
  // разметкой ленты.
  const [tab, setTab] = useState('home');
  const page = useDocxPage();
  const groups = OTHER_RIBBONS[tab];

  return (
    <div className={`ofc-app${page.reading ? ' ofc-app--reading' : ''}${page.showParagraphMarks ? ' ofc-app--paragraph-marks' : ''}`}>
      <TitleBar
        logo={<LogoWord size={20} />}
        app="Слово"
        documentName={page.label ?? 'Документ1'}
        onSearch={() => page.onCommand('find')}
        onUndo={page.onUndo}
        onRedo={page.onRedo}
        onSave={page.onSave}
        canUndo={page.canUndo}
        canRedo={page.canRedo}
        saveDisabled={!page.opened}
      />
      <div className="ofc-ribbon">
        <RibbonTabs tabs={TABS} activeId={tab} onSelect={setTab} />
        {tab === 'home' ? (
          <HomeRibbon
            ready={page.ready}
            fixtures={page.fixtures}
            fixture={page.fixture}
            onFileChange={page.onFileChange}
            onFixtureChange={page.onFixtureChange}
            canPrev={page.canPrev}
            canNext={page.canNext}
            pageCount={page.pageCount}
            onStepPage={page.stepPage}
            onExport={page.exportDocument}
            exportDisabled={page.exportDisabled}
            opened={page.opened}
            editText={page.editText}
            editTarget={page.editTarget}
            onEditText={page.onEditText}
            onEditRun={page.onEditRun}
            onCommand={page.onCommand}
          />
        ) : (
          <div className="ofc-ribbon-body">
            {groups?.map((group) => (
              <RibbonGroup key={group.label} label={group.label}>
                {group.buttons.map((button) => (
                  <RibbonButton
                    key={button.label}
                    icon={button.icon}
                    label={button.label}
                    disabled={!page.opened && button.command !== 'create' && button.command !== 'help' && button.command !== 'feedback'}
                    active={button.command === 'ruler' ? page.showRuler : button.command === 'grid' ? page.showGrid : button.command === 'paragraph-marks' ? page.showParagraphMarks : button.command === 'reading' ? page.reading : button.command === 'page-layout' ? !page.reading : false}
                    onClick={() => page.onCommand(button.command)}
                  />
                ))}
              </RibbonGroup>
            ))}
          </div>
        )}
      </div>
      {page.showRuler ? <Ruler zoom={page.zoom} /> : null}
      {/* Вьюер добавляет свой scroller рядом с необязательной сеткой; собственные
          узлы внутри scroller React не трогает. */}
      <main
        id="viewer"
        ref={page.hostRef}
        onDragOver={page.onDragOver}
        onDragLeave={page.onDragLeave}
        onDrop={page.onDrop}
      >{page.showGrid ? <div className="ofc-view-grid" aria-hidden="true" /> : null}</main>
      <StatusBar
        left={
          <>
            <span className="ofc-statusbar__page">
              страница <b id="page-info">{page.pageInfo}</b>
            </span>
            <span id="hint">перетащите .docx в окно</span>
          </>
        }
        right={
          <>
            <span id="status">{page.status}</span>
            <span className="ofc-statusbar__sep" />
            <StatusZoom
              id="zoom"
              options={ZOOM_OPTIONS}
              value={zoomLabel(page.zoom)}
              disabled={!page.ready}
              onChange={(label) => {
                page.setZoom(zoomByLabel(label));
              }}
            />
          </>
        }
      >
        <FrameMeters handle={page.viewer} />
        {/* Кэш документов: метрика подвала, меняется редко — состоянием React. */}
        <span id="cache">{page.cacheLabel}</span>
      </StatusBar>
      {page.dialog === 'help' ? <HelpDialog onClose={page.closeDialog} /> : null}
      {page.dialog === 'pen' || page.dialog === 'marker' || page.dialog === 'eraser' ? (
        <DrawingPad mode={page.dialog} onClose={page.closeDialog} onInsert={page.insertDrawing} />
      ) : null}
      {page.dialog === 'spellcheck' ? (
        <SpellcheckDialog text={page.editText} onClose={page.closeDialog} onApply={page.applySpellcheck} />
      ) : null}
      {page.busyLabel !== null ? <LoadingOverlay label={page.busyLabel} /> : null}
    </div>
  );
}
