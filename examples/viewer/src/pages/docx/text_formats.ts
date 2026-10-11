import type { DocxViewerHandle } from '@doc-converter/core';

/**
 * Реестр текстовых форматов экспорта DOCX.
 *
 * Markdown, HTML и обычный текст отличаются только расширением, MIME-типом,
 * подписями и методом вьюера, поэтому логика экспорта у них общая, а различия
 * собраны в одной таблице: добавить формат — значит дописать строку сюда, а не
 * ветку в обработчике кнопки.
 */

/** Формат текстового экспорта: имя кнопки и ключ формата — одно и то же. */
export type TextFormat = 'markdown' | 'html' | 'text';

/** Всё, чем один текстовый формат отличается от другого. */
export interface TextFormatSpec {
  /** Расширение файла: приписывается к подписи документа вместо старого. */
  extension: string;
  /** MIME-тип скачиваемого файла. */
  mime: string;
  /** Имя формата в строке состояния: «Markdown готов: …». */
  title: string;
  /** Строка состояния, пока воркер собирает файл. */
  progress: string;
  /** Метод вьюера, отдающий содержимое. */
  run: (viewer: DocxViewerHandle) => Promise<string>;
}

/** Форматы отличаются только этими значениями, поэтому логика экспорта у них общая. */
export const textFormats: Record<TextFormat, TextFormatSpec> = {
  markdown: {
    extension: 'md',
    mime: 'text/markdown',
    title: 'Markdown',
    progress: 'экспортирую Markdown…',
    run: (viewer) => viewer.exportMarkdown(),
  },
  html: {
    extension: 'html',
    mime: 'text/html',
    title: 'HTML',
    progress: 'экспортирую HTML…',
    run: (viewer) => viewer.exportHtml(),
  },
  text: {
    extension: 'txt',
    mime: 'text/plain',
    title: 'Текст',
    progress: 'экспортирую текст…',
    run: (viewer) => viewer.exportText(),
  },
};
