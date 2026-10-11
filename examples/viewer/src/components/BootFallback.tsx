import type { ReactElement } from 'react';

/** Заглушка `Suspense`: страница ещё едет чанком, показывать нечего. */
export function BootFallback(): ReactElement {
  return <div className="boot-fallback">загружаю…</div>;
}
