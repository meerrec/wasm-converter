# @meerrec/react

React-обёртки над просмотрщиками [`@meerrec/core`](../core/README.md): хуки
`useXlsxViewer`/`useDocxViewer` и компоненты `XlsxViewer`/`DocxViewer`. Обёртки
тонкие: разметку, стили, воркер и холст делает фабрика core, пакет только
держит жизненный цикл (создание в эффекте, `destroy()` в cleanup).

> **Отклонение от плана T4.1:** фабрики `createXlsxViewer`/`createDocxViewer`
> принимают хост-элемент (`HTMLElement`), в который сами монтируют прокрутку и
> холст. Поэтому хуки держатся за **host-div**, а не за `<canvas>`:
> `useXlsxViewer(hostRef, options?)`, где `hostRef` — `RefObject<HTMLElement | null>`.

## Установка

```bash
pnpm add @meerrec/core @meerrec/react
# npm i @meerrec/core @meerrec/react
```

`react` — peerDependency: `^18 || ^19`.

## Использование

```tsx
import { useRef } from 'react';
import { useXlsxViewer } from '@meerrec/react';

function Spreadsheet() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewer = useXlsxViewer(hostRef, { zoom: 1.2 });

  return (
    <div style={{ height: '80vh' }}>
      <div ref={hostRef} style={{ width: '100%', height: '100%' }} />
    </div>
  );
}
```

То же через компонент:

```tsx
import { XlsxViewer } from '@meerrec/react';

// host-div создаст сам компонент; hostRef — когда хост нужен снаружи.
<XlsxViewer options={{ zoom: 1.2 }} />
```

Хост обязан иметь размеры: фабрика кладёт в него прокрутку и холст на 100 %
ширины и высоты. При отсутствии `OffscreenCanvas` фабрика сама переходит в
main-режим (`renderMode: 'auto'` по умолчанию) — настройка не нужна.

## API

- `useXlsxViewer(hostRef, options?)` → `XlsxViewerHandle | null` — вьюер
  создаётся в `useEffect` (отложенно на макротак, чтобы dev-режим StrictMode
  не поднимал воркер дважды), `destroy()` вызывается в cleanup; вьюер
  пересоздаётся при смене идентичности объекта `options`; `null`, пока вьюер
  не создан.
- `useDocxViewer(hostRef, options?)` → `DocxViewerHandle | null` — то же для
  DOCX.
- `XlsxViewer({ hostRef?, options? })` / `DocxViewer({ hostRef?, options? })` —
  рендерят только host-div и подключают соответствующий хук.
- Типы `XlsxViewerOptions`/`XlsxViewerHandle`/`DocxViewerOptions`/
  `DocxViewerHandle` реэкспортируются из `@meerrec/core`.

## Демо

Страница демо отложена: пример React-страницы появится в
`examples/viewer` после выноса AGPL-демо в отдельный репозиторий (ADR-0038).
Покрытие сейчас — vitest + `tsc --noEmit`.

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
