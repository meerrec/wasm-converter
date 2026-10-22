// Индикатор занятости окна: разбор документа или сборка PDF.
//
// Не блокирует ввод модально: оверлей прозрачный, но перекрывает окно, чтобы
// повторный клик по экспорту/фикстуре не запускал вторую операцию. Текст
// озвучивается скринридером через `role=status`.

import type { ReactElement } from 'react';

/** `label` — что именно происходит: «Открываю документ…», «Экспортирую PDF…». */
export function LoadingOverlay({ label }: { label: string }): ReactElement {
  return (
    <div className="ofc-loading" role="status" aria-live="polite">
      <span className="ofc-loading__spinner" aria-hidden="true" />
      <span>{label}</span>
    </div>
  );
}
