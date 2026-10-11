import type { DocxViewerHandle, XlsxViewerHandle } from '@doc-converter/core';

/** Мост к активному вьюеру: контракт сквозных тестов Playwright. */
export type ViewerBridge =
  | { kind: 'docx'; viewer: DocxViewerHandle }
  | { kind: 'xlsx'; viewer: XlsxViewerHandle };

declare global {
  interface Window {
    /**
     * Имя не `viewer` намеренно: под ним уже живёт элемент с этим id —
     * браузер делает id глобальной переменной, и второе значение на том же
     * имени молча перекрыло бы первое.
     */
    docConverter?: ViewerBridge;
  }
}

/** Опубликовать вьюер страницы для сквозных тестов. */
export function publishViewer(bridge: ViewerBridge): void {
  window.docConverter = bridge;
}

/** Убрать мост при размонтировании страницы: тест не должен видеть мёртвый вьюер. */
export function clearViewer(): void {
  delete window.docConverter;
}
