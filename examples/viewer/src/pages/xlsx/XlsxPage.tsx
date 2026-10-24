import type { ChangeEvent, ReactElement } from 'react';
import { FrameMeters } from '../../components/FrameMeters.js';
import {
  FormulaBar,
  Icon,
  LogoExcel,
  LoadingOverlay,
  RibbonButton,
  RibbonGroup,
  RibbonSelect,
  RibbonTabs,
  RibbonTile,
  RibbonToggle,
  StatusBar,
  StatusZoom,
  TitleBar,
  type IconName,
  type RibbonTab,
  type StatusZoomOption,
} from '../../components/office/index.js';
import { useXlsxPage } from './use_xlsx_page.js';

/**
 * Страница XLSX («ДваИксель»): каркас Office вокруг вьюера книги.
 *
 * Разметка и лента — здесь, состояние и команды вьюера — в `useXlsxPage`:
 * вьюер императивный, и его вызовы разложены по обработчикам и эффектам
 * одного места, а не размазаны по вёрстке.
 */

/** Вкладки ленты — как в Excel. */
const TABS: RibbonTab[] = [
  { id: 'file', label: 'Файл' },
  { id: 'home', label: 'Главная' },
  { id: 'insert', label: 'Вставка' },
  { id: 'layout', label: 'Разметка страницы' },
  { id: 'formulas', label: 'Формулы' },
  { id: 'data', label: 'Данные' },
  { id: 'review', label: 'Рецензирование' },
  { id: 'view', label: 'Вид' },
  { id: 'help', label: 'Справка' },
];

/** Гарнитуры и размеры — те, что предлагает Excel по умолчанию. */
const FONTS = ['Calibri', 'Arial', 'Times New Roman'];
const SIZES = ['8', '9', '10', '11', '12', '14', '16', '18', '24'];

/** Масштабы зума: те же, что у вьюера, и в том же виде, что в Office. */
const ZOOM = [
  { value: '0.5', label: '50 %' },
  { value: '0.75', label: '75 %' },
  { value: '1', label: '100 %' },
  { value: '1.5', label: '150 %' },
  { value: '2', label: '200 %' },
];

/** Значения списка масштаба в подвале: подписи Office и есть значения `#zoom`. */
const ZOOM_OPTIONS: readonly StatusZoomOption[] = ZOOM.map((option) => ({
  value: option.label,
  label: option.label,
}));

/** Множитель по подписи из списка; отбор идёт по своему же списку. */
function zoomByLabel(label: string): string {
  const found = ZOOM.find((option) => option.label === label);
  return found === undefined ? '1' : found.value;
}

/** Подпись текущего масштаба — её показывают лента и строка состояния. */
function zoomLabel(value: string): string {
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
      label: 'Книга',
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
      label: 'Таблицы',
      buttons: [
        { icon: 'table', label: 'Сводная таблица' },
        { icon: 'cell', label: 'Таблица' },
      ],
    },
    {
      label: 'Иллюстрации',
      buttons: [
        { icon: 'picture', label: 'Рисунок' },
        { icon: 'chart', label: 'Диаграмма' },
      ],
    },
    { label: 'Ссылки', buttons: [{ icon: 'link', label: 'Ссылка' }] },
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
      label: 'Печать',
      buttons: [
        { icon: 'print', label: 'Область печати' },
        { icon: 'freeze', label: 'Закрепить области' },
      ],
    },
  ],
  formulas: [
    {
      label: 'Библиотека функций',
      buttons: [
        { icon: 'function', label: 'Вставить функцию' },
        { icon: 'sum', label: 'Автосумма' },
        { icon: 'formulas', label: 'Финансовые' },
      ],
    },
    {
      label: 'Зависимости',
      buttons: [
        { icon: 'link', label: 'Влияющие ячейки' },
        { icon: 'review', label: 'Вычислить формулу' },
      ],
    },
  ],
  data: [
    {
      label: 'Получение данных',
      buttons: [
        { icon: 'data', label: 'Из текста' },
        { icon: 'table', label: 'Из таблицы' },
      ],
    },
    {
      label: 'Сортировка и фильтр',
      buttons: [
        { icon: 'sort', label: 'Сортировка' },
        { icon: 'filter', label: 'Фильтр' },
        { icon: 'clear', label: 'Очистить' },
      ],
    },
  ],
  review: [
    {
      label: 'Примечания',
      buttons: [{ icon: 'comment', label: 'Создать примечание' }],
    },
    {
      label: 'Правописание',
      buttons: [
        { icon: 'spell-check', label: 'Орфография' },
        { icon: 'freeze', label: 'Закрепить' },
      ],
    },
  ],
  view: [
    {
      label: 'Режимы просмотра',
      buttons: [
        { icon: 'view', label: 'Обычный' },
        { icon: 'page-setup', label: 'Разметка страницы' },
      ],
    },
    {
      label: 'Отображение',
      buttons: [
        { icon: 'grid', label: 'Сетка' },
        { icon: 'headers', label: 'Заголовки' },
        { icon: 'formulas', label: 'Формулы' },
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

/** Пропсы ленты «Главная»: команды вьюера, которые лента показывает. */
interface HomeRibbonProps {
  /** Вьюер не поднялся — открывать и переключать вид нечем. */
  controlsDisabled: boolean;
  fixtures: string[];
  fixture: string;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onFixtureChange: (name: string) => void;
  /** Показ листа: сетка, заголовки и «вписать по ширине». */
  showGrid: boolean;
  showHeaders: boolean;
  onShowGrid: (checked: boolean) => void;
  onShowHeaders: (checked: boolean) => void;
  exportDisabled: boolean;
  /** Вписать столбцы по ширине страницы; строки при этом переносятся. */
  fitToWidth: boolean;
  onFitToWidth: (checked: boolean) => void;
  onExportSheet: () => void;
  onExportBook: () => void;
  /** Переход между листами в группе «Листы». */
  sheetIndex: number;
  sheetCount: number;
  onStepSheet: (delta: number) => void;
}

/** Лента «Главная»: та же, что в Excel, и так же разложена по группам. */
function HomeRibbon(props: HomeRibbonProps): ReactElement {
  return (
    <div className="ofc-ribbon-body">
      {/* Открыть — первое, что делает страница: книга с диска или фикстура из
          списка. Без вьюера открывать нечего, поэтому контролы ждут его. */}
      <RibbonGroup label="Открыть">
        <div className="ofc-open">
          <label className="ofc-file" title="Открыть книгу с диска">
            <input
              id="file"
              type="file"
              accept=".xlsx"
              disabled={props.controlsDisabled}
              onChange={props.onFileChange}
            />
            <Icon name="file" size={14} />
            Открыть
          </label>
          <select
            id="fixture"
            title="Фикстура"
            value={props.fixture}
            disabled={props.controlsDisabled}
            onChange={(event) => {
              props.onFixtureChange(event.target.value);
            }}
          >
            <option value="">— фикстура —</option>
            {props.fixtures.map((name) => (
              <option key={name} value={name}>
                {name.replace(/\.xlsx$/, '')}
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
            <RibbonButton icon="highlight" label="Цвет заливки" disabled />
          </div>
          <div className="ofc-row">
            <RibbonToggle icon="bold" label="Полужирный" disabled />
            <RibbonToggle icon="italic" label="Курсив" disabled />
            <RibbonToggle icon="underline" label="Подчёркнутый" disabled />
            <RibbonToggle icon="strikethrough" label="Зачёркнутый" disabled />
            <RibbonButton icon="borders" label="Границы" disabled />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Выравнивание">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="align-left" label="Выровнять по левому краю" disabled />
            <RibbonButton icon="align-center" label="Выровнять по центру" disabled />
            <RibbonButton icon="align-right" label="Выровнять по правому краю" disabled />
            <RibbonButton icon="align-justify" label="Выровнять по ширине" disabled />
          </div>
          <div className="ofc-row">
            <RibbonButton icon="wrap-text" label="Перенос текста" disabled />
            <RibbonButton icon="merge" label="Объединить ячейки" disabled />
            <RibbonButton icon="indent-decrease" label="Уменьшить отступ" disabled />
            <RibbonButton icon="indent-increase" label="Увеличить отступ" disabled />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Число">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="currency" label="Денежный формат" disabled />
            <RibbonButton icon="percent" label="Процентный формат" disabled />
            <RibbonButton icon="comma" label="Разделитель разрядов" disabled />
          </div>
          <div className="ofc-row">
            <RibbonButton icon="decimal-increase" label="Увеличить разрядность" disabled />
            <RibbonButton icon="decimal-decrease" label="Уменьшить разрядность" disabled />
            <RibbonButton icon="format" label="Формат ячеек" disabled />
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
          title="Заголовок"
          sample="Заголовок"
          className="ofc-tile--title1 ofc-tile--narrow"
          disabled
        />
      </RibbonGroup>
      <RibbonGroup label="Ячейки">
        <div className="ofc-row">
          <RibbonButton icon="cell" label="Вставить" disabled />
          <RibbonButton icon="sheet" label="Удалить" disabled />
          <RibbonButton icon="format" label="Формат" disabled />
        </div>
      </RibbonGroup>
      <RibbonGroup label="Редактирование">
        <div className="ofc-row">
          <RibbonButton icon="sum" label="Автосумма" disabled />
          <RibbonButton icon="sort" label="Сортировка" disabled />
          <RibbonButton icon="filter" label="Фильтр" disabled />
        </div>
      </RibbonGroup>
      {/* Экспорт — команда вьюера: PDF собирает воркер, страница только
          отдаёт файл. До открытия книги гасить нечего, поэтому она ждёт её. */}
      <RibbonGroup label="Экспорт">
        <RibbonButton
          icon="pdf"
          label="PDF"
          large
          id="export-pdf"
          title="Текущий лист в PDF"
          disabled={props.exportDisabled}
          onClick={props.onExportSheet}
        />
        <RibbonButton
          icon="export-book"
          label="Книга"
          large
          id="export-book-pdf"
          title="Вся книга одним PDF"
          disabled={props.exportDisabled}
          onClick={props.onExportBook}
        />
      </RibbonGroup>
      {/* Вид — то, как показан лист и как он ляжет на страницы PDF. */}
      <RibbonGroup label="Вид">
        <div className="ofc-stack">
          <label className="ofc-check">
            <input
              id="grid"
              type="checkbox"
              checked={props.showGrid}
              disabled={props.controlsDisabled}
              onChange={(event) => {
                props.onShowGrid(event.target.checked);
              }}
            />{' '}
            сетка
          </label>
          <label className="ofc-check">
            <input
              id="headers"
              type="checkbox"
              checked={props.showHeaders}
              disabled={props.controlsDisabled}
              onChange={(event) => {
                props.onShowHeaders(event.target.checked);
              }}
            />{' '}
            заголовки
          </label>
          <label
            className="ofc-check"
            title="Уместить столбцы по ширине страницы: широкие таблицы не рвутся по столбцам, строки переходят на следующие страницы"
          >
            <input
              id="fit-to-width"
              type="checkbox"
              checked={props.fitToWidth}
              disabled={props.exportDisabled}
              onChange={(event) => {
                props.onFitToWidth(event.target.checked);
              }}
            />{' '}
            вписать по ширине
          </label>
        </div>
      </RibbonGroup>
      {/* Листы — переход между листами книги; сам список листов живёт в
          строке состояния. */}
      <RibbonGroup label="Листы">
        <div className="ofc-row">
          <button
            type="button"
            className="ofc-navbtn"
            title="Предыдущий лист"
            disabled={props.sheetIndex <= 0}
            onClick={() => {
              props.onStepSheet(-1);
            }}
          >
            ←
          </button>
          <span className="ofc-navlabel">
            лист{' '}
            <b>
              {props.sheetCount === 0 ? '—' : `${props.sheetIndex + 1} / ${props.sheetCount}`}
            </b>
          </span>
          <button
            type="button"
            className="ofc-navbtn"
            title="Следующий лист"
            disabled={props.sheetIndex + 1 >= props.sheetCount}
            onClick={() => {
              props.onStepSheet(1);
            }}
          >
            →
          </button>
        </div>
      </RibbonGroup>
    </div>
  );
}

/** Страница «ДваИксель». */
export default function XlsxPage(): ReactElement {
  const page = useXlsxPage();
  const groups = OTHER_RIBBONS[page.tab];
  return (
    <div className="ofc-app">
      <TitleBar
        logo={<LogoExcel size={20} />}
        app="ДваИксель"
        documentName={page.documentName}
      />
      <div className="ofc-ribbon">
        <RibbonTabs tabs={TABS} activeId={page.tab} onSelect={page.onTabSelect} />
        {page.tab === 'home' ? (
          <HomeRibbon
            controlsDisabled={page.controlsDisabled}
            fixtures={page.fixtures}
            fixture={page.fixture}
            onFileChange={page.onFileChange}
            onFixtureChange={page.onFixtureChange}
            showGrid={page.showGrid}
            showHeaders={page.showHeaders}
            onShowGrid={page.onShowGrid}
            onShowHeaders={page.onShowHeaders}
            exportDisabled={page.exportDisabled}
            fitToWidth={page.fitToWidth}
            onFitToWidth={page.onFitToWidth}
            onExportSheet={page.onExportSheet}
            onExportBook={page.onExportBook}
            sheetIndex={page.sheetIndex}
            sheetCount={page.sheets.length}
            onStepSheet={(delta) => {
              page.onSheetChange(String(page.sheetIndex + delta));
            }}
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
      {/* Строка формул остаётся каркасом Excel: имя активной ячейки теперь
          живёт в строке состояния рядом с листом. */}
      <FormulaBar />
      {/* Хост вьюера пуст: разметку внутри собирает сам вьюер, и React в неё
          не вмешивается — иначе он сносил бы чужие узлы при перерисовке. */}
      <main
        id="viewer"
        ref={page.hostRef}
        onPointerMove={page.onPointerMove}
        onDragOver={page.onDragOver}
        onDragLeave={page.onDragLeave}
        onDrop={page.onDrop}
      />
      <StatusBar
        left={
          <>
            <label className="ofc-statusbar__sheet">
              лист
              {/* Список листов приходит от `open`: до него выбирать нечего, а
                  после неудачного открытия в нём остаётся прежняя книга. */}
              <select
                id="sheet"
                title="Лист"
                value={String(page.sheetIndex)}
                disabled={page.sheetDisabled}
                onChange={(event) => {
                  page.onSheetChange(event.target.value);
                }}
              >
                {page.sheets.length === 0 ? (
                  <option value="">—</option>
                ) : (
                  page.sheets.map((sheet) => (
                    <option key={sheet.index} value={String(sheet.index)}>
                      {sheet.hidden ? `${sheet.name} (скрыт)` : sheet.name}
                    </option>
                  ))
                )}
              </select>
            </label>
            <span className="ofc-statusbar__cell">
              ячейка <span id="cell" ref={page.cellRef}>—</span>
            </span>
            <span id="hint">перетащите .xlsx в окно</span>
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
              disabled={page.controlsDisabled}
              onChange={(label) => {
                page.onZoomChange(zoomByLabel(label));
              }}
            />
          </>
        }
      >
        <FrameMeters handle={page.viewer} />
        {/* Кэш документов: метрика подвала, меняется редко — состоянием React. */}
        <span id="cache">{page.cacheLabel}</span>
      </StatusBar>
      {page.busyLabel !== null ? <LoadingOverlay label={page.busyLabel} /> : null}
    </div>
  );
}
