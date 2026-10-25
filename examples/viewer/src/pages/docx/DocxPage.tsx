import { useState, type ChangeEvent, type ReactElement } from 'react';
import { FrameMeters } from '../../components/FrameMeters.js';
import {
  Icon,
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
  buttons: { icon: IconName; label: string }[];
}

/**
 * Команды остальных вкладок: набор правдоподобный, но всё недоступно —
 * вкладка переключается, лента не пустует, а делать ничего не обещает.
 */
const OTHER_RIBBONS: Record<string, RibbonSpec[]> = {
  file: [
    {
      label: 'Документ',
      buttons: [
        { icon: 'file', label: 'Создать' },
        { icon: 'save', label: 'Сохранить' },
        { icon: 'print', label: 'Печать' },
        { icon: 'share', label: 'Поделиться' },
      ],
    },
  ],
  insert: [
    {
      label: 'Страницы',
      buttons: [
        { icon: 'file', label: 'Титульная страница' },
        { icon: 'page-setup', label: 'Разрыв' },
      ],
    },
    { label: 'Таблицы', buttons: [{ icon: 'table', label: 'Таблица' }] },
    {
      label: 'Иллюстрации',
      buttons: [
        { icon: 'picture', label: 'Рисунок' },
        { icon: 'chart', label: 'Диаграмма' },
      ],
    },
  ],
  draw: [
    {
      label: 'Перо',
      buttons: [
        { icon: 'format-painter', label: 'Перо' },
        { icon: 'highlight', label: 'Маркер' },
        { icon: 'clear', label: 'Ластик' },
      ],
    },
  ],
  design: [
    {
      label: 'Стили документа',
      buttons: [
        { icon: 'styles', label: 'Темы' },
        { icon: 'format', label: 'Набор стилей' },
      ],
    },
  ],
  layout: [
    {
      label: 'Параметры страницы',
      buttons: [
        { icon: 'page-setup', label: 'Поля' },
        { icon: 'sheet', label: 'Ориентация' },
        { icon: 'text', label: 'Размер' },
      ],
    },
    {
      label: 'Абзац',
      buttons: [
        { icon: 'indent-decrease', label: 'Отступ слева' },
        { icon: 'indent-increase', label: 'Отступ справа' },
      ],
    },
  ],
  refs: [
    {
      label: 'Оглавление',
      buttons: [
        { icon: 'multilevel-list', label: 'Оглавление' },
        { icon: 'link', label: 'Ссылка' },
      ],
    },
    {
      label: 'Названия',
      buttons: [
        { icon: 'cell', label: 'Вставить название' },
        { icon: 'text', label: 'Перекрёстная ссылка' },
      ],
    },
  ],
  mail: [
    {
      label: 'Начало слияния',
      buttons: [
        { icon: 'file', label: 'Начать слияние' },
        { icon: 'select', label: 'Выбрать получателей' },
      ],
    },
  ],
  review: [
    {
      label: 'Рецензирование',
      buttons: [
        { icon: 'comment', label: 'Создать примечание' },
        { icon: 'review', label: 'Принять' },
        { icon: 'cancel', label: 'Отклонить' },
      ],
    },
    {
      label: 'Правописание',
      buttons: [
        { icon: 'spell-check', label: 'Орфография' },
        { icon: 'find', label: 'Область проверки' },
      ],
    },
  ],
  view: [
    {
      label: 'Режимы просмотра',
      buttons: [
        { icon: 'view', label: 'Режим чтения' },
        { icon: 'cell', label: 'Разметка страницы' },
      ],
    },
    {
      label: 'Отображение',
      buttons: [
        { icon: 'ruler', label: 'Линейка' },
        { icon: 'grid', label: 'Сетка' },
        { icon: 'paragraph-mark', label: 'Знаки абзацев' },
      ],
    },
  ],
  help: [
    {
      label: 'Справка',
      buttons: [
        { icon: 'help', label: 'Справка' },
        { icon: 'comment', label: 'Отзыв' },
      ],
    },
  ],
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
}

/** Лента «Главная»: та же, что в Word, и так же разложена по группам. */
function HomeRibbon(props: HomeRibbonProps): ReactElement {
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
        <RibbonButton icon="paste" label="Вставить" large disabled />
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="cut" label="Вырезать" disabled />
            <RibbonButton icon="copy" label="Копировать" disabled />
          </div>
          <div className="ofc-row">
            <RibbonButton icon="format-painter" label="Формат по образцу" disabled />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Шрифт">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonSelect options={FONTS} value="Calibri" ariaLabel="Гарнитура" width={96} disabled />
            <RibbonSelect options={SIZES} value="11" ariaLabel="Размер шрифта" width={44} disabled />
            <RibbonButton icon="text-color" label="Цвет текста" disabled />
            <RibbonButton icon="highlight" label="Выделение цветом" disabled />
          </div>
          <div className="ofc-row">
            <RibbonToggle icon="bold" label="Полужирный" disabled />
            <RibbonToggle icon="italic" label="Курсив" disabled />
            <RibbonToggle icon="underline" label="Подчёркнутый" disabled />
            <RibbonToggle icon="strikethrough" label="Зачёркнутый" disabled />
            <RibbonToggle icon="subscript" label="Подстрочный знак" disabled />
            <RibbonToggle icon="superscript" label="Надстрочный знак" disabled />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Абзац">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="bullets" label="Маркированный список" disabled />
            <RibbonButton icon="numbering" label="Нумерованный список" disabled />
            <RibbonButton icon="multilevel-list" label="Многоуровневый список" disabled />
            <RibbonButton icon="line-spacing" label="Междустрочный интервал" disabled />
          </div>
          <div className="ofc-row">
            <RibbonButton icon="align-left" label="Выровнять по левому краю" disabled />
            <RibbonButton icon="align-center" label="Выровнять по центру" disabled />
            <RibbonButton icon="align-right" label="Выровнять по правому краю" disabled />
            <RibbonButton icon="align-justify" label="Выровнять по ширине" disabled />
            <RibbonButton icon="indent-decrease" label="Уменьшить отступ" disabled />
            <RibbonButton icon="indent-increase" label="Увеличить отступ" disabled />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Стили">
        <RibbonTile
          title="Обычный"
          sample="Обычный"
          className="ofc-tile--normal ofc-tile--narrow"
          disabled
        />
        <RibbonTile
          title="Заголовок 1"
          sample="Заголовок"
          className="ofc-tile--title1 ofc-tile--narrow"
          disabled
        />
      </RibbonGroup>
      <RibbonGroup label="Редактирование">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="find" label="Найти" disabled />
            <RibbonButton icon="replace" label="Заменить" disabled />
            <RibbonButton icon="select" label="Выделить" disabled />
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
    <div className="ofc-app">
      <TitleBar
        logo={<LogoWord size={20} />}
        app="Слово"
        documentName={page.label ?? 'Документ1'}
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
          />
        ) : (
          <div className="ofc-ribbon-body">
            {groups?.map((group) => (
              <RibbonGroup key={group.label} label={group.label}>
                {group.buttons.map((button) => (
                  <RibbonButton key={button.label} icon={button.icon} label={button.label} disabled />
                ))}
              </RibbonGroup>
            ))}
          </div>
        )}
      </div>
      <Ruler zoom={page.zoom} />
      {/* Хост вьюера пуст: разметку внутри него рисует вьюер, а React в неё не
          вмешивается — иначе он снёс бы её при любом своём обновлении. */}
      <main
        id="viewer"
        ref={page.hostRef}
        onDragOver={page.onDragOver}
        onDragLeave={page.onDragLeave}
        onDrop={page.onDrop}
      />
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
    </div>
  );
}
