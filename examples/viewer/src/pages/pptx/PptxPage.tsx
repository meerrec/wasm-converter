import {
  useCallback,
  useEffect,
  useState,
  type ChangeEvent,
  type ReactElement,
} from 'react';
import { createPptxViewer } from '@meerrec/core';
import {
  Icon,
  LogoPowerPoint,
  RibbonButton,
  RibbonGroup,
  RibbonSelect,
  RibbonTabs,
  RibbonToggle,
  StatusBar,
  StatusZoom,
  TitleBar,
  type IconName,
  type RibbonTab,
  type StatusZoomOption,
} from '../../components/office/index.js';
import { useEditor } from '../../lib/use_editor.js';
import { clearViewer, publishViewer } from '../../lib/bridge.js';
import { downloadBlob, exportFileName } from '../../lib/download.js';
import { fetchFixture, listFixtures } from '../../lib/fixtures.js';
import { useViewer } from '../../lib/use_viewer.js';
import {
  pptxWorkerUrl,
  resolvePptxPdfWasm,
  resolvePptxWasm,
} from '../../lib/viewer_urls.js';
import type { EditorArgs, EditorResult } from '../../editor/types.js';

/**
 * Страница PPTX («PowerPoint»): каркас Office вокруг вьюера.
 *
 * Работает то, что делает вьюер: открытие презентации (файлом или фикстурой),
 * навигация по слайдам, зум и три экспорта. Дефолтная фикстура открывается
 * сама при монтировании — детерминированный путь для e2e. Контракт id:
 * `#viewer`, `#slide-info`, `#slide-prev`, `#slide-next`, `#zoom`,
 * `#export-pdf`, `#export-png`, `#export-text`, `#status`.
 *
 * Правки ленты уходят в OOXML editor-worker: меняются только затронутые
 * XML-part’ы исходного пакета, а «Сохранить» отдаёт настоящий PPTX.
 */

/** Фикстура, которую страница открывает сама при монтировании. */
const DEFAULT_FIXTURE = 'simple/two_slides.pptx';

/** Вкладки ленты — как в PowerPoint: «Файл», «Главная» и вставка. */
const TABS: RibbonTab[] = [
  { id: 'file', label: 'Файл' },
  { id: 'home', label: 'Главная' },
  { id: 'insert', label: 'Вставка' },
  { id: 'view', label: 'Вид' },
];

/** Масштабы зума: значения — строки контракта `#zoom`, подписи — проценты. */
const ZOOM: readonly StatusZoomOption[] = [
  { value: '0.5', label: '50 %' },
  { value: '0.75', label: '75 %' },
  { value: '1', label: '100 %' },
  { value: '1.5', label: '150 %' },
  { value: '2', label: '200 %' },
];

/** Гарнитуры и размеры — те же, что в PowerPoint по умолчанию. */
const FONTS = ['Calibri', 'Cambria', 'Times New Roman', 'Arial'];
const SIZES = ['8', '9', '10', '11', '12', '14', '16', '18', '24'];

/** Ответ на `prompt`: пустая строка и отмена — «ничего не выбрано». */
function ask(message: string, initial = ''): string | null {
  const value = window.prompt(message, initial);
  if (value === null) return null;
  const trimmed = value.trim();
  return trimmed === '' ? null : trimmed;
}

/** Число из `prompt`; NaN и отмена — «ничего не выбрано». */
function askNumber(message: string, initial: string): number | null {
  const value = ask(message, initial);
  if (value === null) return null;
  const parsed = Number(value.replace(',', '.'));
  return Number.isFinite(parsed) ? parsed : null;
}

/** Текст ошибки: у исключения он есть, у остальных значений — запись строкой. */
function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Состояние страницы PPTX: вьюер, открытие презентации, навигация по слайдам,
 * зум, экспорт и команды OOXML-редактора.
 */
export function usePptxPage(): {
  hostRef: React.RefObject<HTMLElement | null>;
  ready: boolean;
  error: string;
  documentName: string;
  slideIndex: number;
  slideCount: number;
  zoom: number;
  busy: 'pdf' | 'png' | 'text' | null;
  fixtures: string[];
  fixture: string;
  onFileChange(event: ChangeEvent<HTMLInputElement>): void;
  onFixtureChange(event: ChangeEvent<HTMLSelectElement>): void;
  showPrev(): void;
  showNext(): void;
  setZoom(zoom: number): void;
  exportPdf(): Promise<void>;
  exportPng(): Promise<void>;
  exportText(): Promise<void>;
  runCommand(command: string, args?: EditorArgs): Promise<EditorResult>;
  save(): Promise<void>;
  undo(): Promise<void>;
  redo(): Promise<void>;
  copySlideText(): Promise<void>;
  cutSlideText(): Promise<void>;
  pasteSlideText(): Promise<void>;
  copyFormat(): Promise<void>;
  pasteFormat(): Promise<void>;
  report(message: string): void;
  canUndo: boolean;
  canRedo: boolean;
} {
  const [ready, setReady] = useState(false);
  /** Сообщение последнего действия: отказ открытия, экспорта и правки. */
  const [error, setError] = useState('');
  /** Нейтральный отчёт: что именно сделала команда. */
  const [notice, setNotice] = useState('');
  const [documentName, setDocumentName] = useState('Презентация1');
  const [slideIndex, setSlideIndex] = useState(0);
  const [slideCount, setSlideCount] = useState(0);
  const [zoom, setZoomState] = useState(1);
  const [busy, setBusy] = useState<'pdf' | 'png' | 'text' | null>(null);
  const [fixtures, setFixtures] = useState<string[]>([]);
  const [fixture, setFixture] = useState(DEFAULT_FIXTURE);

  // Редактор — синглтон на страницу: он держит разобранный пакет презентации
  // и историю правок, пока страница жива.
  const editor = useEditor();

  // Вьюер поднимает общий хук: он же откладывает создание макротактом и
  // гасит повторы в dev StrictMode. Со своим эффектом первый проход создавал
  // второй `Worker` на том же URL (ручку ещё нечем было уничтожить), а WebKit
  // такой повторный скрипт воркера может отвергнуть COEP-проверкой — и мост
  // не появлялся вовсе.
  const { hostRef, viewer, error: bootError } = useViewer(
    async (host) =>
      createPptxViewer(host, {
        workerUrl: pptxWorkerUrl,
        pptxWasm: await resolvePptxWasm(),
        pptxPdfWasm: await resolvePptxPdfWasm(),
      }),
    (handle) => {
      publishViewer({ kind: 'pptx', viewer: handle });
    },
  );

  // Список фикстур — удобство разработки: без него презентацию открывают
  // файлом, поэтому недоступный список — не ошибка страницы.
  useEffect(() => {
    let alive = true;
    void listFixtures('pptx').then((names) => {
      if (alive) setFixtures(names);
    });
    return () => {
      alive = false;
    };
  }, []);

  // Мост сквозных тестов живёт ровно столько, сколько страница: иначе после
  // перехода на другую страницу тест увидел бы мёртвый вьюер.
  useEffect(() => clearViewer, []);

  /**
   * Открыть презентацию байтами: общий путь файла, фикстуры и повторного
   * показа после правки. `fromEditor` не переоткрывает редактор, а
   * `keepIndex` сохраняет текущий слайд (или уводит на последний созданный).
   */
  const openPresentation = useCallback(
    async (
      bytes: ArrayBuffer,
      name: string,
      options: { fromEditor?: boolean; keepIndex?: number | 'last' } = {},
    ): Promise<void> => {
      if (!viewer) return;
      setError('');
      try {
        // Редактор получает копию: вьюер вправе забрать буфер переносом.
        if (options.fromEditor !== true) await editor.open('pptx', bytes.slice(0), name);
        const slides = await viewer.open(bytes);
        const desired = options.keepIndex === 'last' ? slides.length - 1 : options.keepIndex ?? 0;
        const index = Math.max(0, Math.min(desired, slides.length - 1));
        viewer.showSlide(index);
        setSlideCount(slides.length);
        setSlideIndex(viewer.slideIndex);
        setDocumentName(name);
        setReady(true);
      } catch (e) {
        // Презентация не открылась — прежняя остаётся в вьюере, а причина
        // уходит в строку состояния.
        setError(`не открылось: ${errorText(e)}`);
      }
    },
    [viewer, editor.open],
  );

  // Фикстура открывается сама, как только вьюер поднялся: детерминированный
  // путь для сквозных тестов. Причина отказа уходит в строку состояния.
  useEffect(() => {
    if (viewer === null) return;
    let alive = true;
    void fetchFixture('pptx', DEFAULT_FIXTURE)
      .then((bytes) => {
        if (alive) void openPresentation(bytes, DEFAULT_FIXTURE);
      })
      .catch((e: unknown) => {
        if (alive) setError(`не открылось: ${errorText(e)}`);
      });
    return () => {
      alive = false;
    };
  }, [viewer, openPresentation]);

  const onFileChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const file = event.currentTarget.files?.[0];
      if (!file) return;
      void (async () => {
        try {
          await openPresentation(await file.arrayBuffer(), file.name);
        } catch (e) {
          // Чтение файла тоже может сорваться (файл исчез с диска).
          setError(`не открылось: ${errorText(e)}`);
        }
      })();
    },
    [openPresentation],
  );

  const onFixtureChange = useCallback(
    (event: ChangeEvent<HTMLSelectElement>) => {
      const name = event.currentTarget.value;
      setFixture(name);
      if (!name) return;
      void (async () => {
        try {
          await openPresentation(await fetchFixture('pptx', name), name);
        } catch (e) {
          setError(`не открылось: ${errorText(e)}`);
        }
      })();
    },
    [openPresentation],
  );

  const show = useCallback(
    (index: number) => {
      if (!viewer) return;
      viewer.showSlide(index);
      setSlideIndex(viewer.slideIndex);
    },
    [viewer],
  );

  const setZoom = useCallback(
    (next: number) => {
      viewer?.setZoom(next);
      setZoomState(viewer?.zoom ?? next);
    },
    [viewer],
  );

  /**
   * Команда OOXML-редактора: правка живёт в editor-worker, поэтому вьюеру
   * отдаются изменившиеся байты заново — подпись презентации не меняется.
   */
  const runCommand = useCallback(
    async (command: string, args: EditorArgs = {}): Promise<EditorResult> => {
      try {
        const result = await editor.execute(command, { slide: slideIndex, ...args });
        if (result.changed && result.bytes) {
          const keepIndex = command === 'create-slide' || command === 'duplicate-slide'
            ? 'last' as const
            : slideIndex;
          await openPresentation(result.bytes, documentName, { fromEditor: true, keepIndex });
        }
        return result;
      } catch (e) {
        // Отказ команды — тоже состояние страницы: лента остаётся живой, а
        // причина видна в строке состояния, а не только в консоли.
        setError(`команда «${command}» не выполнена: ${errorText(e)}`);
        throw e;
      }
    },
    [editor.execute, slideIndex, documentName, openPresentation],
  );

  /** Отмена правки: редактор отдаёт прежние байты, вьюер показывает их снова. */
  const undo = useCallback(async (): Promise<void> => {
    const result = await editor.undo();
    if (result.bytes) {
      await openPresentation(result.bytes, documentName, { fromEditor: true, keepIndex: slideIndex });
    }
  }, [editor.undo, documentName, openPresentation, slideIndex]);

  const redo = useCallback(async (): Promise<void> => {
    const result = await editor.redo();
    if (result.bytes) {
      await openPresentation(result.bytes, documentName, { fromEditor: true, keepIndex: slideIndex });
    }
  }, [editor.redo, documentName, openPresentation, slideIndex]);

  /** Сохранение презентации: байты собирает редактор, скачивает страница. */
  const save = useCallback(async (): Promise<void> => {
    const result = await editor.save();
    if (!result.bytes) return;
    const value = (result.value ?? {}) as { name?: string };
    const name = exportFileName(value.name ?? documentName, 'pptx');
    downloadBlob(
      new Uint8Array(result.bytes),
      name,
      'application/vnd.openxmlformats-officedocument.presentationml.presentation',
    );
    setNotice(`PPTX сохранён: ${name}`);
  }, [editor.save, documentName]);

  /** Копировать текст текущего слайда в системный буфер. */
  const copySlideText = useCallback(async (): Promise<void> => {
    try {
      const result = await runCommand('get-text');
      const text = typeof result.value === 'string' ? result.value : '';
      await navigator.clipboard.writeText(text);
      setNotice(text ? 'текст слайда скопирован' : 'на слайде нет текста');
    } catch (e) {
      setError(`копирование не удалось: ${errorText(e)}`);
    }
  }, [runCommand]);

  /** Вырезать текст слайда: редактор отдаёт его и очищает объекты, копия — в буфер. */
  const cutSlideText = useCallback(async (): Promise<void> => {
    try {
      const result = await runCommand('cut-text');
      const text = typeof result.value === 'string' ? result.value : '';
      await navigator.clipboard.writeText(text);
      setNotice('текст слайда вырезан');
    } catch (e) {
      setError(`вырезание не удалось: ${errorText(e)}`);
    }
  }, [runCommand]);

  /** Вставить текст из буфера обмена в текущий слайд. */
  const pasteSlideText = useCallback(async (): Promise<void> => {
    try {
      const text = await navigator.clipboard.readText();
      await runCommand('paste-text', text ? { text } : {});
      setNotice('текст вставлен из буфера обмена');
    } catch (e) {
      setError(`вставка не удалась: ${errorText(e)}`);
    }
  }, [runCommand]);

  /** Формат по образцу: снимок `rPr` первого объекта и перенос на объект. */
  const copyFormat = useCallback(async (): Promise<void> => {
    try {
      await runCommand('copy-format');
      setNotice('формат скопирован');
    } catch (e) {
      setError(`формат не скопирован: ${errorText(e)}`);
    }
  }, [runCommand]);

  const pasteFormat = useCallback(async (): Promise<void> => {
    try {
      await runCommand('paste-format');
      setNotice('формат применён');
    } catch (e) {
      setError(`формат не применён: ${errorText(e)}`);
    }
  }, [runCommand]);

  const exportPdf = useCallback(async () => {
    if (!viewer) return;
    setBusy('pdf');
    try {
      const bytes = await viewer.exportPdf();
      downloadBlob(
        new Uint8Array(bytes) as unknown as BlobPart,
        exportFileName('pptx', 'pdf'),
        'application/pdf',
      );
    } finally {
      setBusy(null);
    }
  }, [viewer]);

  const exportPng = useCallback(async () => {
    if (!viewer) return;
    setBusy('png');
    try {
      const bytes = await viewer.exportPng();
      downloadBlob(bytes as unknown as BlobPart, exportFileName('pptx', 'png'), 'image/png');
    } finally {
      setBusy(null);
    }
  }, [viewer]);

  const exportText = useCallback(async () => {
    if (!viewer) return;
    setBusy('text');
    try {
      const text = await viewer.exportMarkdown();
      downloadBlob(
        new TextEncoder().encode(text) as unknown as BlobPart,
        exportFileName('pptx', 'md'),
        'text/markdown',
      );
    } finally {
      setBusy(null);
    }
  }, [viewer]);

  return {
    hostRef,
    ready,
    // Отказ создания вьюера важнее последнего сообщения: без вьюера не
    // работает ничего, и строка состояния обязана это назвать.
    error: bootError ?? (error || notice),
    documentName,
    slideIndex,
    slideCount,
    zoom,
    busy,
    fixtures,
    fixture,
    onFileChange,
    onFixtureChange,
    showPrev: () => show(slideIndex - 1),
    showNext: () => show(slideIndex + 1),
    setZoom,
    exportPdf,
    exportPng,
    exportText,
    runCommand,
    save,
    undo,
    redo,
    copySlideText,
    cutSlideText,
    pasteSlideText,
    copyFormat,
    pasteFormat,
    report: setNotice,
    canUndo: editor.canUndo,
    canRedo: editor.canRedo,
  };
}

/** Пропсы вкладки: всё состояние и команды страницы. */
interface RibbonProps {
  page: ReturnType<typeof usePptxPage>;
}

/** Пропсы ленты «Главная». */
interface HomeRibbonProps extends RibbonProps {
  onExportPdf(): void;
  onExportPng(): void;
  onExportText(): void;
}

/** Лента «Главная»: буфер, слайды, шрифт, открытие, экспорт и навигация. */
function HomeRibbon(props: HomeRibbonProps): ReactElement {
  const { page } = props;
  const [fontFamily, setFontFamily] = useState('Calibri');
  const [fontSize, setFontSize] = useState('18');
  const [bold, setBold] = useState(false);
  const [italic, setItalic] = useState(false);
  const [underline, setUnderline] = useState(false);
  const editing = page.ready;
  const exportDisabled = page.busy !== null || !page.ready;

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
      <RibbonGroup label="Буфер обмена">
        <RibbonButton
          icon="paste"
          label="Вставить"
          large
          disabled={!editing}
          onClick={() => void page.pasteSlideText()}
        />
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton
              icon="cut"
              label="Вырезать"
              disabled={!editing}
              onClick={() => void page.cutSlideText()}
            />
            <RibbonButton
              icon="copy"
              label="Копировать"
              disabled={!editing}
              onClick={() => void page.copySlideText()}
            />
          </div>
          <div className="ofc-row">
            <RibbonButton
              icon="format-painter"
              label="Формат по образцу"
              title="Снять формат с текста слайда"
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
      <RibbonGroup label="Слайды">
        <div className="ofc-row">
          <RibbonButton
            icon="file"
            label="Создать слайд"
            disabled={!editing}
            onClick={() => run('create-slide')}
          />
          <RibbonButton
            icon="sheet"
            label="Макет"
            title="Сменить макет текущего слайда"
            disabled={!editing}
            onClick={() => {
              const layout = askNumber('Индекс макета (0 — первый в презентации)', '0');
              if (layout !== null) run('slide-layout', { layout });
            }}
          />
          <RibbonButton
            icon="view"
            label="Сбросить"
            title="Вернуть расположение объектов макета"
            disabled={!editing}
            onClick={() => run('reset-slide')}
          />
        </div>
        <div className="ofc-row">
          <RibbonButton
            icon="copy"
            label="Дублировать"
            disabled={!editing}
            onClick={() => run('duplicate-slide')}
          />
          <RibbonButton
            icon="clear"
            label="Удалить"
            disabled={!editing || page.slideCount <= 1}
            onClick={() => run('delete-slide')}
          />
          <RibbonButton
            icon="sort"
            label="Порядок"
            title="Переставить слайды: список новых номеров"
            disabled={!editing}
            onClick={() => {
              const order = ask('Новый порядок слайдов через запятую (0 — первый)', '0');
              if (order === null) return;
              const parsed = order
                .split(',')
                .map((part) => Number(part.trim()))
                .filter((value) => Number.isInteger(value) && value >= 0);
              if (parsed.length > 0) run('sort-slides', { order: parsed });
            }}
          />
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
                run('font-family', { family: value });
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
                run('font-size', { size: Number(value) });
              }}
            />
            <RibbonButton
              icon="text-color"
              label="Цвет текста"
              disabled={!editing}
              onClick={() => {
                const value = ask('Цвет текста RRGGBB', '000000');
                if (value) run('text-color', { color: value });
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
        </div>
      </RibbonGroup>
      <RibbonGroup label="Открыть">
        <div className="ofc-open">
          <label className="ofc-file" title="Открыть презентацию с диска">
            <input
              id="file"
              type="file"
              accept=".pptx"
              disabled={!page.ready}
              onChange={page.onFileChange}
            />
            <Icon name="file" size={14} />
            Открыть
          </label>
          <select
            id="fixture"
            title="Фикстура"
            value={page.fixture}
            disabled={!page.ready}
            onChange={page.onFixtureChange}
          >
            <option value="">— фикстура —</option>
            {page.fixtures.map((name) => (
              <option key={name} value={name}>
                {name.replace(/\.pptx$/, '')}
              </option>
            ))}
          </select>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Экспорт">
        <RibbonButton
          icon="pdf"
          label="PDF"
          large
          id="export-pdf"
          disabled={exportDisabled}
          onClick={props.onExportPdf}
        />
        <RibbonButton
          icon="picture"
          label="PNG"
          large
          id="export-png"
          disabled={exportDisabled}
          onClick={props.onExportPng}
        />
        <RibbonButton
          icon="text"
          label="Текст"
          large
          id="export-text"
          disabled={exportDisabled}
          onClick={props.onExportText}
        />
      </RibbonGroup>
      <RibbonGroup label="Навигация">
        <div className="ofc-row">
          <button
            id="slide-prev"
            type="button"
            className="ofc-navbtn"
            title="Предыдущий слайд"
            disabled={page.slideIndex <= 0}
            onClick={page.showPrev}
          >
            ←
          </button>
          <span className="ofc-navlabel">
            слайд{' '}
            <b>
              {page.slideCount === 0 ? '—' : `${page.slideIndex + 1} / ${page.slideCount}`}
            </b>
          </span>
          <button
            id="slide-next"
            type="button"
            className="ofc-navbtn"
            title="Следующий слайд"
            disabled={page.slideIndex + 1 >= page.slideCount}
            onClick={page.showNext}
          >
            →
          </button>
        </div>
      </RibbonGroup>
    </div>
  );
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
          event.currentTarget.value = '';
        }}
      />
      <Icon name={props.icon} size={14} />
      {props.label}
    </label>
  );
}

/** Вкладка «Вставка»: слайд, таблица, рисунок, диаграмма и надпись. */
function InsertRibbon({ page }: RibbonProps): ReactElement {
  const editing = page.ready;
  const run = (command: string, args?: Record<string, unknown>): void => {
    void page.runCommand(command, args).catch(() => {});
  };
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Слайды">
        <RibbonButton
          icon="file"
          label="Создать слайд"
          large
          disabled={!editing}
          onClick={() => run('create-slide')}
        />
      </RibbonGroup>
      <RibbonGroup label="Таблицы">
        <RibbonButton
          icon="table"
          label="Таблица"
          disabled={!editing}
          onClick={() => {
            const rows = askNumber('Строк в таблице', '3');
            if (rows === null) return;
            const cols = askNumber('Столбцов в таблице', '3');
            if (cols !== null) run('insert-table', { rows, cols });
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
            const title = ask('Заголовок диаграммы', 'Диаграмма');
            if (title) run('insert-chart', { title });
          }}
        />
      </RibbonGroup>
      <RibbonGroup label="Текст">
        <RibbonButton
          icon="text"
          label="Надпись"
          disabled={!editing}
          onClick={() => {
            const text = ask('Текст надписи');
            if (text) run('insert-text', { text });
          }}
        />
      </RibbonGroup>
    </div>
  );
}

/** Вкладка «Файл»: сохранение, печать, ссылка и история правок. */
function FileRibbon({ page }: RibbonProps): ReactElement {
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Презентация">
        <RibbonButton
          icon="save"
          label="Сохранить"
          large
          id="save"
          title="Скачать презентацию с правками в PPTX"
          disabled={!page.ready}
          onClick={() => void page.save()}
        />
        <RibbonButton
          icon="print"
          label="Печать"
          large
          disabled={!page.ready}
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
        <RibbonButton
          icon="file"
          label="Создать"
          title="Пустая презентация создаётся открытием пустого файла"
          disabled
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

/** Вкладка «Вид»: вьюер показывает слайд только в обычном режиме. */
function ViewRibbon(): ReactElement {
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Режимы просмотра">
        <RibbonButton
          icon="view"
          label="Обычный"
          title="Вьюер показывает презентацию только в обычном режиме"
          disabled
        />
        <RibbonButton
          icon="page-setup"
          label="Сортировщик"
          title="Режим сортировщика не поддерживается движком"
          disabled
        />
      </RibbonGroup>
      <RibbonGroup label="Отображение">
        <RibbonButton icon="ruler" label="Линейка" title="Линейки в вьюере нет" disabled />
        <RibbonButton icon="grid" label="Сетка" title="Сетки в вьюере нет" disabled />
      </RibbonGroup>
    </div>
  );
}

/**
 * Страница PPTX («PowerPoint»): каркас Office — титульная строка, вкладки,
 * лента, холст вьюера и строка состояния.
 */
export default function PptxPage(): ReactElement {
  const [tab, setTab] = useState('home');
  const page = usePptxPage();

  return (
    <div className="ofc-app">
      <TitleBar
        logo={<LogoPowerPoint size={20} />}
        app="PowerPoint"
        documentName={page.documentName}
      />
      <div className="ofc-ribbon">
        <RibbonTabs tabs={TABS} activeId={tab} onSelect={setTab} />
        {tab === 'home' ? (
          <HomeRibbon
            page={page}
            onExportPdf={() => void page.exportPdf()}
            onExportPng={() => void page.exportPng()}
            onExportText={() => void page.exportText()}
          />
        ) : tab === 'file' ? (
          <FileRibbon page={page} />
        ) : tab === 'insert' ? (
          <InsertRibbon page={page} />
        ) : (
          <ViewRibbon />
        )}
      </div>
      {/* Хост вьюера пуст: разметку внутри него рисует вьюер, а React в неё
          не вмешивается — иначе он снёс бы её при любом своём обновлении. */}
      <main id="viewer" ref={page.hostRef} />
      <StatusBar
        left={
          <span className="ofc-statusbar__page">
            Слайд{' '}
            <b id="slide-info">
              {page.slideCount === 0 ? '—' : `${page.slideIndex + 1} / ${page.slideCount}`}
            </b>
          </span>
        }
        right={
          <>
            <span id="status">{page.error}</span>
            <span className="ofc-statusbar__sep" />
            <StatusZoom
              id="zoom"
              options={ZOOM}
              value={String(page.zoom)}
              disabled={!page.ready}
              onChange={(value) => page.setZoom(Number(value))}
            />
          </>
        }
      />
    </div>
  );
}
