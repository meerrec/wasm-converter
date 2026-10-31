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
  RibbonTabs,
  StatusBar,
  StatusZoom,
  TitleBar,
  type IconName,
  type RibbonTab,
  type StatusZoomOption,
} from '../../components/office/index.js';
import { clearViewer, publishViewer } from '../../lib/bridge.js';
import { downloadBlob, exportFileName } from '../../lib/download.js';
import { fetchFixture, listFixtures } from '../../lib/fixtures.js';
import { useViewer } from '../../lib/use_viewer.js';
import {
  pptxWorkerUrl,
  resolvePptxPdfWasm,
  resolvePptxWasm,
} from '../../lib/viewer_urls.js';

/**
 * Страница PPTX («PowerPoint»): каркас Office вокруг вьюера.
 *
 * Работает то, что делает вьюер: открытие презентации (файлом или фикстурой),
 * навигация по слайдам, зум и три экспорта. Дефолтная фикстура открывается
 * сама при монтировании — детерминированный путь для e2e. Контракт id:
 * `#viewer`, `#slide-info`, `#slide-prev`, `#slide-next`, `#zoom`,
 * `#export-pdf`, `#export-png`, `#export-text`, `#status`.
 */

/** Фикстура, которую страница открывает сама при монтировании. */
const DEFAULT_FIXTURE = 'simple/two_slides.pptx';

/** Вкладки ленты — как в PowerPoint: «Файл», «Главная» и декоративные. */
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

/** Группа команд вкладки-заглушки. */
interface RibbonSpec {
  label: string;
  buttons: { icon: IconName; label: string }[];
}

/**
 * Команды декоративных вкладок: набор правдоподобный, но всё недоступно —
 * вкладка переключается, лента не пустует, а делать ничего не обещает.
 */
const OTHER_RIBBONS: Record<string, RibbonSpec[]> = {
  file: [
    {
      label: 'Презентация',
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
      label: 'Слайды',
      buttons: [
        { icon: 'file', label: 'Создать слайд' },
        { icon: 'table', label: 'Таблица' },
      ],
    },
    {
      label: 'Иллюстрации',
      buttons: [
        { icon: 'picture', label: 'Рисунок' },
        { icon: 'chart', label: 'Диаграмма' },
      ],
    },
  ],
  view: [
    {
      label: 'Режимы просмотра',
      buttons: [
        { icon: 'view', label: 'Обычный' },
        { icon: 'page-setup', label: 'Сортировщик' },
      ],
    },
    {
      label: 'Отображение',
      buttons: [
        { icon: 'ruler', label: 'Линейка' },
        { icon: 'grid', label: 'Сетка' },
      ],
    },
  ],
};

/** Текст ошибки: у исключения он есть, у остальных значений — запись строкой. */
function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Состояние страницы PPTX: вьюер, открытие презентации, навигация по слайдам,
 * зум и экспорт.
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
} {
  const [ready, setReady] = useState(false);
  /** Сообщение последнего действия: отказ открытия, экспорта и правки. */
  const [error, setError] = useState('');
  const [documentName, setDocumentName] = useState('Презентация1');
  const [slideIndex, setSlideIndex] = useState(0);
  const [slideCount, setSlideCount] = useState(0);
  const [zoom, setZoomState] = useState(1);
  const [busy, setBusy] = useState<'pdf' | 'png' | 'text' | null>(null);
  const [fixtures, setFixtures] = useState<string[]>([]);
  const [fixture, setFixture] = useState(DEFAULT_FIXTURE);

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

  // Фикстура открывается сама, как только вьюер поднялся: детерминированный
  // путь для сквозных тестов. Причина отказа уходит в строку состояния.
  useEffect(() => {
    if (viewer === null) return;
    let alive = true;
    void (async () => {
      try {
        const slides = await viewer.open(await fetchFixture('pptx', DEFAULT_FIXTURE));
        if (!alive) return;
        setSlideCount(slides.length);
        setSlideIndex(0);
        setReady(true);
      } catch (e) {
        if (alive) setError(`не открылось: ${errorText(e)}`);
      }
    })();
    return () => {
      alive = false;
    };
  }, [viewer]);

  /** Открыть презентацию байтами: общий путь файла и фикстуры. */
  const openPresentation = useCallback(async (bytes: ArrayBuffer, name: string) => {
    if (!viewer) return;
    setError('');
    try {
      const slides = await viewer.open(bytes);
      setSlideCount(slides.length);
      setSlideIndex(0);
      setDocumentName(name);
    } catch (e) {
      // Презентация не открылась — прежняя остаётся в вьюере, а причина уходит
      // в строку состояния.
      setError(`не открылось: ${errorText(e)}`);
    }
  }, [viewer]);

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
    error: bootError ?? error,
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
  };
}

/** Пропсы ленты «Главная»: всё, что доступно от вьюера и страницы. */
interface HomeRibbonProps {
  ready: boolean;
  busy: 'pdf' | 'png' | 'text' | null;
  fixtures: string[];
  fixture: string;
  slideIndex: number;
  slideCount: number;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onFixtureChange: (event: ChangeEvent<HTMLSelectElement>) => void;
  onPrev(): void;
  onNext(): void;
  onExportPdf(): void;
  onExportPng(): void;
  onExportText(): void;
}

/** Лента «Главная»: открытие, экспорт и навигация по слайдам. */
function HomeRibbon(props: HomeRibbonProps): ReactElement {
  const exportDisabled = props.busy !== null || !props.ready;
  return (
    <div className="ofc-ribbon-body">
      <RibbonGroup label="Буфер обмена">
        <RibbonButton icon="paste" label="Вставить" large disabled />
        <div className="ofc-stack">
          <div className="ofc-row">
            <RibbonButton icon="cut" label="Вырезать" disabled />
            <RibbonButton icon="copy" label="Копировать" disabled />
          </div>
        </div>
      </RibbonGroup>
      <RibbonGroup label="Слайды">
        <div className="ofc-row">
          <RibbonButton icon="file" label="Создать слайд" disabled />
          <RibbonButton icon="sheet" label="Макет" disabled />
          <RibbonButton icon="view" label="Сбросить" disabled />
        </div>
      </RibbonGroup>
      <RibbonGroup label="Открыть">
        <div className="ofc-open">
          <label className="ofc-file" title="Открыть презентацию с диска">
            <input
              id="file"
              type="file"
              accept=".pptx"
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
            disabled={props.slideIndex <= 0}
            onClick={props.onPrev}
          >
            ←
          </button>
          <span className="ofc-navlabel">
            слайд{' '}
            <b>
              {props.slideCount === 0 ? '—' : `${props.slideIndex + 1} / ${props.slideCount}`}
            </b>
          </span>
          <button
            id="slide-next"
            type="button"
            className="ofc-navbtn"
            title="Следующий слайд"
            disabled={props.slideIndex + 1 >= props.slideCount}
            onClick={props.onNext}
          >
            →
          </button>
        </div>
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
  const groups = OTHER_RIBBONS[tab];

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
            ready={page.ready}
            busy={page.busy}
            fixtures={page.fixtures}
            fixture={page.fixture}
            slideIndex={page.slideIndex}
            slideCount={page.slideCount}
            onFileChange={page.onFileChange}
            onFixtureChange={page.onFixtureChange}
            onPrev={page.showPrev}
            onNext={page.showNext}
            onExportPdf={() => void page.exportPdf()}
            onExportPng={() => void page.exportPng()}
            onExportText={() => void page.exportText()}
          />
        ) : (
          <div className="ofc-ribbon-body">
            {(groups ?? []).map((group) => (
              <RibbonGroup key={group.label} label={group.label}>
                {group.buttons.map((button) => (
                  <RibbonButton key={button.label} icon={button.icon} label={button.label} disabled />
                ))}
              </RibbonGroup>
            ))}
          </div>
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
