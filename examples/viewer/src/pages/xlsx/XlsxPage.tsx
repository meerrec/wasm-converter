import { useEffect, useState, type ReactElement } from 'react';
import { FrameMeters } from '../../components/FrameMeters.js';
import {
  FormulaBar,
  Icon,
  LogoExcel,
  RibbonButton,
  RibbonGroup,
  RibbonSelect,
  RibbonTabs,
  RibbonTile,
  RibbonToggle,
  StatusBar,
  TitleBar,
  type IconName,
  type RibbonTab,
} from '../../components/office/index.js';
import { listFixtures } from '../../lib/fixtures.js';

/**
 * Страница XLSX («ДваИксель»): каркас Office вокруг вьюера книги.
 *
 * Как и на DOCX-странице, работает только то, что не зависит от вьюера:
 * открытие файла, выбор фикстуры, зум и экспорт. Команды ленты недоступны —
 * их обработчики появятся вместе с вьюером.
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

/** Подписи для списка «Масштаб»: в списке ленты живут они, а не множители. */
const ZOOM_LABELS = ZOOM.map((option) => option.label);

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

/** Лента «Главная»: та же, что в Excel, и так же разложена по группам. */
function HomeRibbon(props: { zoom: string; onZoom: (value: string) => void }): ReactElement {
  return (
    <div className="ofc-ribbon-body">
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
      {/* Экспорт — единственная работающая группа: она уводит книгу из вьюера
          в файл и вьюера не требует. */}
      <RibbonGroup label="Экспорт">
        <RibbonButton icon="pdf" label="PDF" large id="export-pdf" />
        <RibbonButton icon="export-book" label="Книга" large id="export-book-pdf" />
      </RibbonGroup>
      {/* Масштаб — тоже команда вьюера, а не каркаса: она переживает отказ
          книги и остаётся доступной, когда остальная лента пуста. */}
      <RibbonGroup label="Масштаб">
        <RibbonSelect
          id="zoom"
          options={ZOOM_LABELS}
          value={zoomLabel(props.zoom)}
          ariaLabel="Масштаб"
          title="Масштаб книги"
          width={88}
          onChange={(label) => {
            props.onZoom(zoomByLabel(label));
          }}
        />
      </RibbonGroup>
    </div>
  );
}

/** Страница «ДваИксель». */
export default function XlsxPage(): ReactElement {
  const [tab, setTab] = useState('home');
  const [zoom, setZoom] = useState('1');
  const [fixture, setFixture] = useState('');
  const [fixtures, setFixtures] = useState<string[]>([]);

  // Список фикстур — удобство разработки: без него книгу выбирают и перетаскивают.
  useEffect(() => {
    let alive = true;
    void listFixtures('xlsx').then((names) => {
      if (alive) setFixtures(names);
    });
    return () => {
      alive = false;
    };
  }, []);

  const groups = OTHER_RIBBONS[tab];
  return (
    <div className="ofc-app">
      <TitleBar logo={<LogoExcel size={20} />} app="ДваИксель" documentName="Книга1">
        <label className="ofc-file" title="Открыть книгу с диска">
          <input id="file" type="file" accept=".xlsx" />
          <Icon name="file" size={14} />
          Открыть
        </label>
        <select
          id="fixture"
          title="Фикстура"
          value={fixture}
          onChange={(event) => {
            setFixture(event.target.value);
          }}
        >
          <option value="">— фикстура —</option>
          {fixtures.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </TitleBar>
      <div className="ofc-ribbon">
        <RibbonTabs tabs={TABS} activeId={tab} onSelect={setTab} />
        {tab === 'home' ? (
          <HomeRibbon zoom={zoom} onZoom={setZoom} />
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
      <FormulaBar cellRef={<span id="cell">—</span>} />
      <main id="viewer" />
      <StatusBar
        left={
          <>
            <label className="ofc-statusbar__sheet">
              лист
              {/* Лист выбирают, когда книга открыта; до этого список пуст. */}
              <select id="sheet" defaultValue="" disabled title="Лист">
                <option value="">—</option>
              </select>
            </label>
            <span id="hint">перетащите .xlsx в окно</span>
          </>
        }
        right={
          <>
            <label className="ofc-statusbar__check">
              <input id="grid" type="checkbox" defaultChecked /> сетка
            </label>
            <label className="ofc-statusbar__check">
              <input id="headers" type="checkbox" defaultChecked /> заголовки
            </label>
            <span className="ofc-statusbar__sep" />
            <span id="status" />
            {/* Масштаб показывается, но не правится: ручка — в ленте. */}
            <span className="ofc-statusbar__zoom" title="Масштаб">
              <Icon name="zoom-out" size={14} />
              {zoomLabel(zoom)}
            </span>
          </>
        }
      >
        <FrameMeters handle={null} />
      </StatusBar>
    </div>
  );
}
