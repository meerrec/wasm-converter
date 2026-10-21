# Рецепты: просмотр XLSX

Просмотрщик собирает разметку сам: внутри переданного хоста появляется
прокручиваемая область, распорка по размеру листа и холст, приклеенный к
видимой части. Байты книги передаются воркеру переводом владения.

---

### <a id="xlsx-01"></a>Рецепт 1. Минимальное открытие книги

**Задача.** Открыть `.xlsx` из `<input type="file">` и показать первый лист.

```ts
// xlsx_worker.ts — отдельный бандл воркера, одна строка
import '@meerrec/core/xlsx-worker';

// main
import { createXlsxViewer, type SheetInfo } from '@meerrec/core';
import workerUrl from './xlsx_worker.ts?worker&url'; // Vite

const viewer = await createXlsxViewer(document.getElementById('viewer')!, {
  workerUrl,
});

const file = await input.files![0]!.arrayBuffer();
const sheets: SheetInfo[] = await viewer.open(file);
console.log(sheets.map((s) => s.name));

viewer.destroy(); // гасит воркер и убирает разметку
```

**Под капотом.** `open` отдаёт байты воркеру переводом владения (transfer),
поэтому после вызова основной поток буфером пользоваться не должен — книга
живёт в `thread_local!` воркера, и кадр собирается сразу в слот ring'а.

---

### <a id="xlsx-02"></a>Рецепт 2. Листы книги: переключение и размеры

**Задача.** Показать список листов, переключать их и знать размеры.

```ts
import { createXlsxViewer, type SheetInfo } from '@meerrec/core';

const viewer = await createXlsxViewer(host, { workerUrl });
const sheets = await viewer.open(bytes);

const select = document.createElement('select');
for (const s of sheets) {
  const option = document.createElement('option');
  option.value = String(s.index);
  option.textContent = s.hidden ? `${s.name} (скрыт)` : s.name;
  select.append(option);
}
select.addEventListener('change', () => viewer.showSheet(Number(select.value)));
host.parentElement!.prepend(select);

// Размеры листа — в пикселях раскладки, до зума; нужны, например, для
// собственных индикаторов прокрутки.
console.log(sheets[viewer.sheetIndex]!.width, sheets[viewer.sheetIndex]!.height);
```

**Под капотом.** `SheetInfo` приходит из Rust: `name`, `index`, `hidden`,
`width`/`height` и закреплённые строки/колонки (`frozenRows`/`frozenCols` и
их размеры) — скрытые листы в списке есть и помечены флагом `hidden`.

---

### <a id="xlsx-03"></a>Рецепт 3. Зум и прокрутка

**Задача.** Кнопки зума «+»/«−» и прыжок к ячейке.

```ts
viewer.setZoom(viewer.zoom * 1.1); // зум: 1 — сто процентов
viewer.setZoom(Math.max(0.5, viewer.zoom / 1.1));

// Прокрутить так, чтобы точка раскладки оказалась в левом верхнем углу.
// Координаты — в пикселях раскладки, то есть до зума.
viewer.scrollTo(0, 0);          // в начало листа
viewer.scrollTo(500, 300);      // на 500 px правее и 300 px ниже начала

// Координаты конкретных ячеек в пикселях раскладки в API просмотрщика не
// выставляются — их знает Rust-раскладка; для «прыжка к ячейке» используйте
// свои известные координаты или результат hitTest'а вкупе с размерами листа
// (см. рецепт 2).
```

**Под капотом.** Единицы в запросе кадра разные и это важно: `viewport.x/y`
— пиксели раскладки, `w/h` — физические пиксели холста, `scale` — зум,
умноженный на DPR; сам зум хранится в `viewer.zoom` и обновляется
`setZoom`, который перерисовывает лист.

---

### <a id="xlsx-04"></a>Рецепт 4. hitTest: ячейка под курсором

**Задача.** Показывать адрес ячейки под курсором, как строка формул Excel.

```ts
/** Имена колонок как в Excel: 0 → A, 26 → AA. */
function columnName(col: number): string {
  let name = '';
  let n = col + 1;
  while (n > 0) {
    const rem = (n - 1) % 26;
    name = String.fromCharCode(65 + rem) + name;
    n = Math.floor((n - 1) / 26);
  }
  return name;
}

host.addEventListener('pointermove', async (event) => {
  const hit = await viewer.hitTest(event.clientX, event.clientY);
  if (hit === null) {
    cellLabel.textContent = '';
    return;
  }
  const [row, col] = hit; // ноль-базированные
  cellLabel.textContent = `${columnName(col)}${row + 1}`;
});
```

**Под капотом.** `hitTest` принимает координаты события мыши в координатах
страницы и возвращает `[row, col]` (или `null` вне листа) — вопрос уходит
в воркер, где лежит разобранная книга, поэтому на каждый `mousemove`
ответ дёшев, но не бесплатен: подписка живёт на `pointermove`, а не на
каждый кадр.

---

### <a id="xlsx-05"></a>Рецепт 5. Гиперссылки: наведение и клик

**Задача.** Подсветить ссылку под курсором и открыть её по клику.

```ts
import { createXlsxViewer, type HyperlinkInfo } from '@meerrec/core';

const viewer = await createXlsxViewer(host, {
  workerUrl,
  // Колбэк получает саму ссылку; клик по ячейке без ссылки его не трогает.
  onHyperlink: (link) => {
    console.log(`click: ${link.target} (${link.display ?? 'без подписи'})`);
    window.open(link.target, '_blank', 'noopener');
  },
});

await viewer.open(bytes);

host.addEventListener('pointermove', async (event) => {
  const link: HyperlinkInfo | null = await viewer.hyperlinkAt(
    event.clientX,
    event.clientY,
  );
  host.style.cursor = link === null ? 'default' : 'pointer';
});
```

**Под капотом.** `HyperlinkInfo` — `{ target, display, tooltip }`, где
`display`/`tooltip` равны `null`, если файл их не объявил; ассоциация
«ячейка → ссылка» живёт в раскладке листа на Rust-стороне, поэтому
`hyperlinkAt` отвечает по точке, а не по подписи ячейки.

---

### <a id="xlsx-06"></a>Рецепт 6. Сетка, заголовки и тёмная тема

**Задача.** Переключатели ленты: сетка, заголовки строк/колонок, тёмная тема.

```ts
const viewer = await createXlsxViewer(host, {
  workerUrl,
  theme: 'dark',      // начальная тема; по умолчанию светлая, как в Excel
  showGrid: true,
  showHeaders: true,
});

await viewer.open(bytes);

// В любой момент после открытия:
viewer.setView({ showGrid: false });
viewer.setView({ showHeaders: false });
viewer.setView({ theme: document.documentElement.classList.contains('dark') ? 'dark' : 'light' });
```

**Под капотом.** В тёмной теме цвета темы книги и контраст текста
пересчитываются под тёмный фон на стороне Rust — это не CSS-инверсия
картинки, а другая раскраска того же DisplayList.

---

### <a id="xlsx-07"></a>Рецепт 7. Метрики кадров через onTick

**Задача.** Показывать FPS и число команд кадра без перерисовки React на
каждый тик.

```ts
import type { PaintStats } from '@meerrec/core';

const meter = document.getElementById('frame-meter')!;
const unsubscribe = viewer.onTick((stats: PaintStats) => {
  // Пишем в DOM напрямую: кадры идут десятки раз в секунду, и setState на
  // каждом из них перерисовывал бы страницу целиком.
  meter.textContent =
    `${stats.cmds} команд, ${stats.paintMs.toFixed(1)} мс` +
    (stats.dropped ? ', кадр пропущен' : '');
});

// Подписку можно снять, не гася вьюер:
// unsubscribe();
viewer.destroy();
```

**Под капотом.** `tick` — статистика painter'а (`PaintStats`), которую воркер
шлёт после каждого кадра; для листов `buildMs` отсутствует, потому что
сборка DisplayList для книги идёт без пагинации и дешевле, чем у документов.

---

### <a id="xlsx-08"></a>Рецепт 8. DPR и ретина: что считается автоматически

**Задача.** Разобраться, что надо (и не надо) делать для экранов с
`devicePixelRatio = 2`.

```ts
// Никаких опций DPR у вьюера нет — и это правильно: зум задаёт пользователь
// (viewer.setZoom), а DPR вычисляет ResizeObserver внутри.
const viewer = await createXlsxViewer(host, { workerUrl });
await viewer.open(bytes);

// devicePixelRatio нужен только для интерфейса, не для кадра:
console.log(`DPR экрана: ${window.devicePixelRatio}`);
console.log(`Зум: ${viewer.zoom}, эффективный масштаб кадра: ${viewer.zoom * window.devicePixelRatio}`);
```

**Под капотом.** `ResizeObserver` (debounce 100 мс) шлёт воркеру `resize` с
DPR, клампированным лимитом 16M пикселей холста (`computeDpr`), а запрос
кадра несёт `scale = zoom × DPR` — поэтому ручной учёт DPR в приложении
только сломает картинку.

---

Ссылки: контракт DisplayList — [ADR-0003](../adr/0003-render-boundary.md),
раскладка и единицы запроса кадра — `packages/core/src/protocol.ts`.
