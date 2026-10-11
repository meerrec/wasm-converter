/**
 * Имя файла для скачивания: подпись документа без каталога и расширения плюс
 * своё расширение; без подписи — запасное имя `document.<ext>`.
 *
 * @param label — подпись открытого документа: имя файла или путь фикстуры.
 * @param extension — расширение целевого формата, без точки.
 */
export function exportFileName(label: string, extension: string): string {
  const trimmed = label.trim();
  if (!trimmed) return `document.${extension}`;
  // Имя фикстуры приходит путём с категорией (`simple/one_paragraph.docx`),
  // а скачивается файл с плоским именем: каталог срезаем. Расширение у
  // выбранного файла может быть любым, поэтому срезается последнее.
  const name = trimmed.replace(/^.*[/\\]/, '');
  const base = name.replace(/\.[^.]*$/, '');
  return `${base || name}.${extension}`;
}

/** Отдать данные файлом: временная ссылка на blob, клик и отзыв ссылки. */
export function downloadBlob(data: BlobPart, name: string, mime: string): void {
  const url = URL.createObjectURL(new Blob([data], { type: mime }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  // Отзываем не сразу: браузер начинает скачивание асинхронно, и немедленный
  // revoke успевает отменить его (в WebKit — стабильно).
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}
