import { useEffect, useState, type ReactElement } from 'react';
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
  TitleBar,
  type IconName,
  type RibbonTab,
} from '../../components/office/index.js';
import { listFixtures } from '../../lib/fixtures.js';

/**
 * Страница DOCX («Слово»): каркас Office вокруг вьюера.
 *
 * Слайс рисует только каркас: команды ленты недоступны, работают контролы
 * документа (файл, фикстура, зум) и экспорт — его обработчики появятся вместе
 * с вьюером. Места, куда он встанет, помечены id: `#viewer`, `#page-info`,
 * `#status` и счётчики кадров.
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

/** Лента «Главная»: та же, что в Word, и так же разложена по группам. */
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
        <div className="ofc-row">
          <RibbonButton icon="find" label="Найти" disabled />
          <RibbonButton icon="replace" label="Заменить" disabled />
          <RibbonButton icon="select" label="Выделить" disabled />
        </div>
      </RibbonGroup>
      {/* Экспорт — единственная работающая группа: она уводит документ из
          вьюера в файл и вьюера не требует. */}
      <RibbonGroup label="Экспорт">
        <RibbonButton icon="pdf" label="PDF" large id="export-pdf" />
        <RibbonButton icon="markdown" label="Markdown" large id="export-md" />
        <RibbonButton icon="html" label="HTML" large id="export-html" />
        <RibbonButton icon="text" label="Текст" large id="export-txt" />
      </RibbonGroup>
      {/* Масштаб — тоже команда вьюера, а не каркаса: она переживает отказ
          документа и остаётся доступной, когда остальная лента пуста. */}
      <RibbonGroup label="Масштаб">
        <RibbonSelect
          id="zoom"
          options={ZOOM_LABELS}
          value={zoomLabel(props.zoom)}
          ariaLabel="Масштаб"
          title="Масштаб документа"
          width={88}
          onChange={(label) => {
            props.onZoom(zoomByLabel(label));
          }}
        />
      </RibbonGroup>
    </div>
  );
}

/** Страница «Слово». */
export default function DocxPage(): ReactElement {
  const [tab, setTab] = useState('home');
  const [zoom, setZoom] = useState('1');
  const [fixture, setFixture] = useState('');
  const [fixtures, setFixtures] = useState<string[]>([]);

  // Список фикстур — удобство разработки: без него файл выбирают и перетаскивают.
  useEffect(() => {
    let alive = true;
    void listFixtures('docx').then((names) => {
      if (alive) setFixtures(names);
    });
    return () => {
      alive = false;
    };
  }, []);

  const groups = OTHER_RIBBONS[tab];
  return (
    <div className="ofc-app">
      <TitleBar logo={<LogoWord size={20} />} app="Слово" documentName="Документ1">
        <label className="ofc-file" title="Открыть документ с диска">
          <input id="file" type="file" accept=".docx" />
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
      <Ruler zoom={Number(zoom)} />
      <main id="viewer" />
      <StatusBar
        left={
          <>
            <span>
              страница <b id="page-info">—</b>
            </span>
            <span id="hint">перетащите .docx в окно</span>
          </>
        }
        right={
          <>
            <span id="status" />
            <span className="ofc-statusbar__sep" />
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
