import { useState, type ChangeEvent, type ReactElement } from 'react';
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
import { useXlsxPage, type XlsxPageApi } from './use_xlsx_page.js';

/**
 * Страница XLSX («ДваИксель»): каркас Office вокруг вьюера книги.
 *
 * Разметка и лента — здесь, состояние и команды вьюера — в `useXlsxPage`:
 * вьюер императивный, и его вызовы разложены по обработчикам и эффектам
 * одного места, а не размазаны по вёрстке. Команды ленты правят OOXML в
 * editor-worker (`useEditor`), поэтому результат — настоящий XLSX, а не
 * только перерисованный кадр.
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

/** Ответ на `prompt`: пустая строка и отмена — «ничего не выбрано». */
function ask(message: string, initial = ''): string | null {
  const value = window.prompt(message, initial);
  if (value === null) return null;
  const trimmed = value.trim();
  return trimmed === '' ? null : trimmed;
}

/** Диапазон из `prompt`: команды над диапазоном без него не выполняются. */
function askRange(initial = 'A1'): string | null {
  return ask('Диапазон (например, A1:C10)', initial);
}

/** Число из `prompt`; NaN и отмена — «ничего не выбрано». */
function askNumber(message: string, initial: string): number | null {
  const value = ask(message, initial);
  if (value === null) return null;
  const parsed = Number(value.replace(',', '.'));
  return Number.isFinite(parsed) ? parsed : null;
}

/** Файл с диска — только локальный: сетевых источников лента не обещает. */
function FileButton(props: {
  id: string;
  accept: string;
  label: string;
  icon: IconName;
  disabled?: boolean;
  title?: string;
  onFile: (file: File) => void;
}): ReactElement {
  return (
    <label className="ofc-file" title={props.title ?? props.label}>
      <input
        id={props.id}
        type="file"
        accept={props.accept}
        disabled={props.disabled === true}
        onChange={(event) => {
          const file = event.currentTarget.files?.[0];
          if (file) props.onFile(file);
          // Один и тот же файл должен снова вызвать `change`.
          event.currentTarget.value = '';
        }}
      />
      <Icon name={props.icon} size={14} />
      {props.label}
    </label>
  );
}

/** Пропсы вкладки: всё состояние и команды страницы. */
interface RibbonProps {
  page: XlsxPageApi;
}

/**
 * Лента «Главная»: открытие, буфер, формат, редактирование, экспорт и вид.
 * Команды адресуются ячейке под курсором; диапазон спрашивается там, где он
 * нужен команде (объединение, сортировка, область печати).
 */
function HomeRibbon({ page }: RibbonProps): ReactElement {
  const [fontFamily, setFontFamily] = useState('Calibri');
  const [fontSize, setFontSize] = useState('11');
  const [bold, setBold] = useState(false);
  const [italic, setItalic] = useState(false);
  const [underline, setUnderline] = useState(false);
  const [strike, setStrike] = useState(false);
  const [wrap, setWrap] = useState(false);
  const editing = !page.exportDisabled;

  /** Команда ленты: отказ уже объяснён строкой состояния в хуке. */
  const run = (command: string, args?: Record<string, unknown>): void => {
    void page.runCommand(command, args).catch(() => {});
  };

  const toggle = (
    command: string,
    state: boolean,
    setState: (value: boolean) => void,
  ): void => {
    setState(!state);
    run(command);
  };

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
              disabled={page.controlsDisabled}
              onChange={page.onFileChange}
            />
            <Icon name="file" size={14} />
            Открыть
          </label>
          <select
            id="fixture"
            title="Фикстура"
            value={page.fixture}
            disabled={page.controlsDisabled}
            onChange={(event) => {
              page.onFixtureChange(event.target.value);
            }}
          >
            <option value="">— фикстура —</option>
            {page.fixtures.map((name) => (
              <option key={name} value={name}>
                {name.replace(/\.xlsx$/, '')}
              </option>
            ))}
          </select>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Буфер обмена">
        <RibbonButton
          icon="paste"
          label="Вставить"
          large
          disabled={!editing}
          onClick={() => void page.pasteCell()}
        />
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton
              icon="cut"
              label="Вырезать"
              disabled={!editing}
              onClick={() => void page.cutCell()}
            />
            <RibbonButton
              icon="copy"
              label="Копировать"
              disabled={!editing}
              onClick={() => void page.copyCell()}
            />
          </div>
          <div className="ofc-row">
            <RibbonButton
              icon="format-painter"
              label="Формат по образцу"
              title="Снять формат с ячейки"
              disabled={!editing}
              onClick={() => void page.copyFormat()}
            />
            <RibbonButton
              icon="format"
              label="Применить формат"
              disabled={!editing}
              onClick={() => void page.pasteFormat()}
            />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Шрифт">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonSelect
              options={FONTS}
              value={fontFamily}
              ariaLabel="Гарнитура"
              width={96}
              disabled={!editing}
              onChange={(value) => {
                setFontFamily(value);
                run('font-family', { value });
              }}
            />
            <RibbonSelect
              options={SIZES}
              value={fontSize}
              ariaLabel="Размер шрифта"
              width={44}
              disabled={!editing}
              onChange={(value) => {
                setFontSize(value);
                run('font-size', { value: Number(value) });
              }}
            />
            <RibbonButton
              icon="text-color"
              label="Цвет текста"
              disabled={!editing}
              onClick={() => {
                const value = ask('Цвет текста RRGGBB', '000000');
                if (value) run('text-color', { value });
              }}
            />
            <RibbonButton
              icon="highlight"
              label="Цвет заливки"
              disabled={!editing}
              onClick={() => {
                const value = ask('Цвет заливки RRGGBB', 'FFFF00');
                if (value) run('highlight', { value });
              }}
            />
          </div>
          <div className="ofc-row">
            <RibbonToggle
              icon="bold"
              label="Полужирный"
              active={bold}
              disabled={!editing}
              onClick={() => toggle('toggle-bold', bold, setBold)}
            />
            <RibbonToggle
              icon="italic"
              label="Курсив"
              active={italic}
              disabled={!editing}
              onClick={() => toggle('toggle-italic', italic, setItalic)}
            />
            <RibbonToggle
              icon="underline"
              label="Подчёркнутый"
              active={underline}
              disabled={!editing}
              onClick={() => toggle('toggle-underline', underline, setUnderline)}
            />
            <RibbonToggle
              icon="strikethrough"
              label="Зачёркнутый"
              active={strike}
              disabled={!editing}
              onClick={() => toggle('toggle-strikethrough', strike, setStrike)}
            />
            <RibbonButton
              icon="borders"
              label="Границы"
              disabled={!editing}
              onClick={() => {
                const value = ask('Границы: all, outline, inside или none', 'all');
                if (value) run('borders', { value });
              }}
            />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Выравнивание">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton
              icon="align-left"
              label="Выровнять по левому краю"
              disabled={!editing}
              onClick={() => run('align', { value: 'left' })}
            />
            <RibbonButton
              icon="align-center"
              label="Выровнять по центру"
              disabled={!editing}
              onClick={() => run('align', { value: 'center' })}
            />
            <RibbonButton
              icon="align-right"
              label="Выровнять по правому краю"
              disabled={!editing}
              onClick={() => run('align', { value: 'right' })}
            />
            <RibbonButton
              icon="align-justify"
              label="Выровнять по ширине"
              disabled={!editing}
              onClick={() => run('align', { value: 'justify' })}
            />
          </div>
          <div className="ofc-row">
            <RibbonToggle
              icon="wrap-text"
              label="Перенос текста"
              active={wrap}
              disabled={!editing}
              onClick={() => toggle('wrap-text', wrap, setWrap)}
            />
            <RibbonButton
              icon="merge"
              label="Объединить ячейки"
              disabled={!editing}
              onClick={() => {
                const range = askRange();
                if (range) run('merge', { range });
              }}
            />
            <RibbonButton
              icon="indent-decrease"
              label="Уменьшить отступ"
              disabled={!editing}
              onClick={() => run('indent', { delta: -1 })}
            />
            <RibbonButton
              icon="indent-increase"
              label="Увеличить отступ"
              disabled={!editing}
              onClick={() => run('indent', { delta: 1 })}
            />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Число">
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton
              icon="currency"
              label="Денежный формат"
              disabled={!editing}
              onClick={() => run('number-format', { value: 'currency' })}
            />
            <RibbonButton
              icon="percent"
              label="Процентный формат"
              disabled={!editing}
              onClick={() => run('number-format', { value: 'percent' })}
            />
            <RibbonButton
              icon="comma"
              label="Разделитель разрядов"
              disabled={!editing}
              onClick={() => run('number-format', { value: 'comma' })}
            />
          </div>
          <div className="ofc-row">
            <RibbonButton
              icon="decimal-increase"
              label="Увеличить разрядность"
              disabled={!editing}
              onClick={() => run('number-format', { value: 'decimal-increase' })}
            />
            <RibbonButton
              icon="decimal-decrease"
              label="Уменьшить разрядность"
              disabled={!editing}
              onClick={() => run('number-format', { value: 'decimal-decrease' })}
            />
            <RibbonButton
              icon="format"
              label="Формат ячеек"
              disabled={!editing}
              onClick={() => {
                const code = ask('Код формата числа', '0.00');
                if (code) run('number-format', { value: 'custom', code });
              }}
            />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Стили">
        <RibbonTile
          title="Обычный"
          sample="Обычный"
          className="ofc-tile--normal ofc-tile--narrow"
          disabled={!editing}
          onClick={() => run('style', { value: 'Normal' })}
        />
        <RibbonTile
          title="Заголовок"
          sample="Заголовок"
          className="ofc-tile--title1 ofc-tile--narrow"
          disabled={!editing}
          onClick={() => run('style', { value: 'Heading1' })}
        />
      </RibbonGroup>
      <RibbonGroup label="Ячейки">
        <div className="ofc-row">
          <RibbonButton
            icon="cell"
            label="Вставить"
            disabled={!editing}
            onClick={() => {
              const shift = ask('Сдвиг ячеек: down, right, up или left', 'down');
              if (shift) run('insert-cell', { shift });
            }}
          />
          <RibbonButton
            icon="sheet"
            label="Удалить"
            disabled={!editing}
            onClick={() => {
              const shift = ask('Сдвиг ячеек: down, right, up или left', 'up');
              if (shift) run('delete-cell', { shift });
            }}
          />
          <RibbonButton
            icon="format"
            label="Формат"
            disabled={!editing}
            onClick={() => {
              const code = ask('Код формата числа', '0.00');
              if (code) run('number-format', { value: 'custom', code });
            }}
          />
        </div>
      </RibbonGroup>
      <RibbonGroup label="Редактирование">
        <div className="ofc-row">
          <RibbonButton
            icon="sum"
            label="Автосумма"
            disabled={!editing}
            onClick={() => run('autosum')}
          />
          <RibbonButton
            icon="sort"
            label="Сортировка"
            disabled={!editing}
            onClick={() => {
              const range = askRange();
              if (!range) return;
              const column = askNumber('Столбец сортировки (0 — первый в диапазоне)', '0');
              if (column === null) return;
              run('sort', { range, column, descending: false });
            }}
          />
          <RibbonButton
            icon="filter"
            label="Фильтр"
            disabled={!editing}
            onClick={() => {
              const range = askRange();
              if (range) run('filter', { range });
            }}
          />
          <RibbonButton
            icon="clear"
            label="Очистить"
            disabled={!editing}
            onClick={() => {
              const range = askRange();
              if (range) run('clear', { range });
            }}
          />
        </div>
      </RibbonGroup>
      <RibbonGroup label="История">
        <div className="ofc-row">
          <RibbonButton
            icon="undo"
            label="Отменить"
            disabled={!page.canUndo}
            onClick={() => void page.undo()}
          />
          <RibbonButton
            icon="redo"
            label="Повторить"
            disabled={!page.canRedo}
            onClick={() => void page.redo()}
          />
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
          disabled={page.exportDisabled}
          onClick={page.onExportSheet}
        />
        <RibbonButton
          icon="export-book"
          label="Книга"
          large
          id="export-book-pdf"
          title="Вся книга одним PDF"
          disabled={page.exportDisabled}
          onClick={page.onExportBook}
        />
      </RibbonGroup>
      {/* Вид — то, как показан лист и как он ляжет на страницы PDF. */}
      <RibbonGroup label="Вид">
        <div className="ofc-stack">
          <label className="ofc-check">
            <input
              id="grid"
              type="checkbox"
              checked={page.showGrid}
              disabled={page.controlsDisabled}
              onChange={(event) => {
                page.onShowGrid(event.target.checked);
              }}
            />{' '}
            сетка
          </label>
          <label className="ofc-check">
            <input
              id="headers"
              type="checkbox"
              checked={page.showHeaders}
              disabled={page.controlsDisabled}
              onChange={(event) => {
                page.onShowHeaders(event.target.checked);
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
              checked={page.fitToWidth}
              disabled={page.exportDisabled}
              onChange={(event) => {
                page.onFitToWidth(event.target.checked);
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
            disabled={page.sheetIndex <= 0}
            onClick={() => {
              page.onSheetChange(String(page.sheetIndex - 1));
            }}
          >
            ←
          </button>
          <span className="ofc-navlabel">
            лист{' '}
            <b>
              {page.sheets.length === 0 ? '—' : `${page.sheetIndex + 1} / ${page.sheets.length}`}
            </b>
          </span>
          <button
            type="button"
            className="ofc-navbtn"
            title="Следующий лист"
            disabled={page.sheetIndex + 1 >= page.sheets.length}
            onClick={() => {
              page.onSheetChange(String(page.sheetIndex + 1));
            }}
          >
            →
          </button>
        </div>
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Файл»: сохранение, печать, ссылка и история правок. */
function FileRibbon({ page }: RibbonProps): ReactElement {
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Книга">
        <RibbonButton
          icon="save"
          label="Сохранить"
          large
          id="save"
          title="Скачать книгу с правками в XLSX"
          disabled={page.exportDisabled}
          onClick={() => void page.save()}
        />
        <RibbonButton
          icon="print"
          label="Печать"
          large
          disabled={page.exportDisabled}
          onClick={() => window.print()}
        />
        <RibbonButton
          icon="share"
          label="Поделиться"
          large
          title="Скопировать ссылку на страницу"
          onClick={() => {
            void navigator.clipboard
              .writeText(window.location.href)
              .then(() => page.report('ссылка скопирована'));
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="История">
        <RibbonButton
          icon="undo"
          label="Отменить"
          disabled={!page.canUndo}
          onClick={() => void page.undo()}
        />
        <RibbonButton
          icon="redo"
          label="Повторить"
          disabled={!page.canRedo}
          onClick={() => void page.redo()}
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Вставка»: таблицы, сводная, рисунок, диаграмма и ссылка. */
function InsertRibbon({ page }: RibbonProps): ReactElement {
  const editing = !page.exportDisabled;
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Таблицы">
        <RibbonButton
          icon="table"
          label="Таблица"
          large
          disabled={!editing}
          onClick={() => {
            const range = askRange();
            if (!range) return;
            const name = ask('Имя таблицы', 'Таблица1');
            if (name) void page.runCommand('create-table', { range, name }).catch(() => {});
          }}
        />
        <RibbonButton
          icon="sum"
          label="Сводная таблица"
          large
          title="Настоящий сводный лист с агрегатами по диапазону"
          disabled={!editing}
          onClick={() => {
            const range = askRange();
            if (!range) return;
            const name = ask('Имя сводного листа', 'Сводная');
            if (name) void page.runCommand('create-pivot', { range, name }).catch(() => {});
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="Иллюстрации">
        <FileButton
          id="insert-picture"
          accept="image/png,image/jpeg,image/gif,image/webp"
          label="Рисунок"
          icon="picture"
          disabled={!editing}
          title="Вставить изображение с диска"
          onFile={(file) => {
            void file
              .arrayBuffer()
              .then((bytes) =>
                page.runCommand('insert-picture', {
                  bytes,
                  mime: file.type || 'image/png',
                  width: 320,
                  height: 240,
                  name: file.name,
                }),
              )
              .catch(() => {});
          }}
        />
        <RibbonButton
          icon="chart"
          label="Диаграмма"
          disabled={!editing}
          onClick={() => {
            const range = askRange();
            if (!range) return;
            const title = ask('Заголовок диаграммы', 'Диаграмма');
            if (title) void page.runCommand('insert-chart', { range, title }).catch(() => {});
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="Ссылки">
        <RibbonButton
          icon="link"
          label="Ссылка"
          disabled={!editing}
          onClick={() => {
            const url = ask('Адрес ссылки', 'https://');
            if (url) void page.runCommand('insert-link', { url }).catch(() => {});
          }}
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Разметка страницы»: параметры листа для печати и PDF. */
function LayoutRibbon({ page }: RibbonProps): ReactElement {
  const editing = !page.exportDisabled;
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Параметры страницы">
        <RibbonButton
          icon="page-setup"
          label="Поля"
          disabled={!editing}
          onClick={() => {
            const margin = askNumber('Поля со всех сторон, дюймы', '0.7');
            if (margin === null) return;
            void page
              .runCommand('page-margins', { top: margin, right: margin, bottom: margin, left: margin })
              .catch(() => {});
          }}
        />
        <RibbonButton
          icon="sheet"
          label="Ориентация"
          disabled={!editing}
          onClick={() => {
            const value = ask('Ориентация: portrait или landscape', 'portrait');
            if (value) void page.runCommand('page-orientation', { value }).catch(() => {});
          }}
        />
        <RibbonButton
          icon="text"
          label="Размер"
          disabled={!editing}
          onClick={() => {
            const value = ask('Размер бумаги: A4, A3, Letter, Legal', 'A4');
            if (value) void page.runCommand('page-size', { value }).catch(() => {});
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="Печать">
        <RibbonButton
          icon="print"
          label="Область печати"
          disabled={!editing}
          onClick={() => {
            const range = askRange();
            if (range) void page.runCommand('print-area', { range }).catch(() => {});
          }}
        />
        <RibbonButton
          icon="freeze"
          label="Закрепить области"
          disabled={!editing}
          onClick={() => {
            const rows = askNumber('Заморозить строк сверху', '1');
            if (rows === null) return;
            const cols = askNumber('Заморозить столбцов слева', '0');
            if (cols === null) return;
            void page.runCommand('freeze', { row: rows, col: cols }).catch(() => {});
          }}
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Формулы»: вставка функций, автосумма и зависимости. */
function FormulasRibbon({ page }: RibbonProps): ReactElement {
  const editing = !page.exportDisabled;
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Библиотека функций">
        <RibbonButton
          icon="function"
          label="Вставить функцию"
          disabled={!editing}
          onClick={() => {
            const value = ask('Формула или имя функции (например, SUM(A1:A5))', 'SUM(');
            if (value) void page.runCommand('insert-function', { value }).catch(() => {});
          }}
        />
        <RibbonButton
          icon="sum"
          label="Автосумма"
          disabled={!editing}
          onClick={() => void page.runCommand('autosum').catch(() => {})}
        />
        <RibbonButton
          icon="formulas"
          label="Финансовые"
          disabled={!editing}
          onClick={() => {
            const value = ask('Финансовая функция: PMT, FV или PV', 'PMT');
            if (value) void page.runCommand('financial-function', { value }).catch(() => {});
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="Зависимости">
        <RibbonButton
          icon="link"
          label="Влияющие ячейки"
          disabled={!editing}
          onClick={() => {
            void page
              .runCommand('precedents')
              .then((result) => {
                const refs = Array.isArray(result.value) ? result.value.map(String) : [];
                page.report(refs.length === 0 ? 'влияющих ячеек нет' : `влияющие: ${refs.join(', ')}`);
              })
              .catch(() => {});
          }}
        />
        <RibbonButton
          icon="review"
          label="Вычислить формулу"
          disabled={!editing}
          onClick={() => {
            const value = ask('Формула для вычисления', '=1+1');
            if (!value) return;
            void page
              .runCommand('evaluate-formula', { value })
              .then((result) => page.report(`значение: ${String(result.value)}`))
              .catch(() => {});
          }}
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Данные»: импорт текста, сортировка, фильтр и очистка. */
function DataRibbon({ page }: RibbonProps): ReactElement {
  const editing = !page.exportDisabled;
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Получение данных">
        <FileButton
          id="import-text"
          accept=".csv,.tsv,.txt,text/csv,text/tab-separated-values,text/plain"
          label="Из текста"
          icon="data"
          disabled={!editing}
          title="Импортировать CSV/TSV в новый лист"
          onFile={(file) => {
            void file
              .text()
              .then((text) =>
                page.runCommand('import-text', {
                  text,
                  delimiter: file.name.toLowerCase().endsWith('.tsv') ? '\t' : ',',
                }),
              )
              .catch(() => {});
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="Сортировка и фильтр">
        <RibbonButton
          icon="sort"
          label="Сортировка"
          disabled={!editing}
          onClick={() => {
            const range = askRange();
            if (!range) return;
            const column = askNumber('Столбец сортировки (0 — первый в диапазоне)', '0');
            if (column === null) return;
            void page.runCommand('sort', { range, column, descending: false }).catch(() => {});
          }}
        />
        <RibbonButton
          icon="filter"
          label="Фильтр"
          disabled={!editing}
          onClick={() => {
            const range = askRange();
            if (range) void page.runCommand('filter', { range }).catch(() => {});
          }}
        />
        <RibbonButton
          icon="clear"
          label="Очистить"
          disabled={!editing}
          onClick={() => {
            const range = askRange();
            if (range) void page.runCommand('clear', { range }).catch(() => {});
          }}
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Рецензирование»: примечания к ячейке. */
function ReviewRibbon({ page }: RibbonProps): ReactElement {
  const editing = !page.exportDisabled;
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Примечания">
        <RibbonButton
          icon="comment"
          label="Создать примечание"
          disabled={!editing}
          onClick={() => {
            const text = ask('Текст примечания');
            if (text) void page.runCommand('add-comment', { text }).catch(() => {});
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="Правописание">
        <RibbonButton
          icon="spell-check"
          label="Орфография"
          title="Проверка орфографии не поддерживается движком"
          disabled
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Вид»: те же переключатели показа, что и на «Главной». */
function ViewRibbon({ page }: RibbonProps): ReactElement {
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Отображение">
        <div className="ofc-stack">
          <label className="ofc-check">
            <input
              type="checkbox"
              checked={page.showGrid}
              disabled={page.controlsDisabled}
              onChange={(event) => {
                page.onShowGrid(event.target.checked);
              }}
            />{' '}
            сетка
          </label>
          <label className="ofc-check">
            <input
              type="checkbox"
              checked={page.showHeaders}
              disabled={page.controlsDisabled}
              onChange={(event) => {
                page.onShowHeaders(event.target.checked);
              }}
            />{' '}
            заголовки
          </label>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Режимы просмотра">
        <RibbonButton
          icon="view"
          label="Обычный"
          title="Вьюер показывает книгу только в обычном режиме"
          disabled
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Справка»: движок не обещает интерактивной справки. */
function HelpRibbon(): ReactElement {
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Справка">
        <RibbonButton icon="help" label="Справка" disabled />
        <RibbonButton icon="comment" label="Отзыв" disabled />
      </RibbonGroup>
    </div>
  );
}

/** Тело ленты выбранной вкладки. */
function TabRibbon({ page }: RibbonProps): ReactElement | null {
  switch (page.tab) {
    case 'file':
      return <FileRibbon page={page} />;
    case 'home':
      return <HomeRibbon page={page} />;
    case 'insert':
      return <InsertRibbon page={page} />;
    case 'layout':
      return <LayoutRibbon page={page} />;
    case 'formulas':
      return <FormulasRibbon page={page} />;
    case 'data':
      return <DataRibbon page={page} />;
    case 'review':
      return <ReviewRibbon page={page} />;
    case 'view':
      return <ViewRibbon page={page} />;
    case 'help':
      return <HelpRibbon />;
    default:
      return null;
  }
}

/** Страница «ДваИксель». */
export default function XlsxPage(): ReactElement {
  const page = useXlsxPage();
  return (
    <div className="ofc-app">
      <TitleBar
        logo={<LogoExcel size={20} />}
        app="ДваИксель"
        documentName={page.documentName}
      />
      <div className="ofc-ribbon">
        <RibbonTabs tabs={TABS} activeId={page.tab} onSelect={page.onTabSelect} />
        <TabRibbon page={page} />
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
            {/* Правка ячейки под курсором: ввод + Enter ставит сырой текст на
                место показанного значения, карточка перерисовывается, PDF видит
                правку. Цель — ячейка последнего `hitTest`. */}
            <label className="ofc-edit-cell" title="Текст ячейки под курсором; Enter применяет правку">
              править ячейку
              <input
                id="edit-cell"
                type="text"
                placeholder="ячейка"
                disabled={page.editDisabled}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    page.onEditCell(event.currentTarget.value);
                    event.currentTarget.value = '';
                  }
                }}
              />
            </label>
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
