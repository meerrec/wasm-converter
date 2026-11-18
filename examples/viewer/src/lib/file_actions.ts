import { downloadBlob, exportFileName } from './download.js';

export const OOXML_MIME = {
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
} as const;

/** Записать текст в системный буфер, с fallback для страниц без Clipboard API. */
export async function copyText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const field = document.createElement('textarea');
  field.value = text;
  field.style.cssText = 'position:fixed;left:-10000px;top:0';
  document.body.append(field);
  field.select();
  const copied = document.execCommand('copy');
  field.remove();
  if (!copied) throw new Error('браузер запретил доступ к буферу обмена');
}

/** Прочитать текст системного буфера; явная ошибка вместо молчаливой пустой вставки. */
export async function readText(): Promise<string> {
  if (!navigator.clipboard?.readText) {
    throw new Error('чтение буфера обмена не поддерживается браузером');
  }
  return navigator.clipboard.readText();
}

/** Выбрать локальный файл без постоянного input в разметке. */
export function pickFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.style.display = 'none';
    document.body.append(input);
    const finish = (file: File | null): void => {
      input.remove();
      resolve(file);
    };
    input.addEventListener('change', () => finish(input.files?.[0] ?? null), { once: true });
    input.addEventListener('cancel', () => finish(null), { once: true });
    input.click();
  });
}

/** Открыть системный диалог печати готового PDF без новой вкладки. */
export function printPdf(bytes: Uint8Array): void {
  const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'application/pdf' }));
  const frame = document.createElement('iframe');
  frame.style.cssText = 'position:fixed;width:1px;height:1px;right:0;bottom:0;opacity:0';
  frame.src = url;
  const cleanup = (): void => {
    frame.remove();
    setTimeout(() => URL.revokeObjectURL(url), 30_000);
  };
  frame.onload = () => {
    const target = frame.contentWindow;
    if (!target) {
      cleanup();
      throw new Error('не удалось открыть предпросмотр печати');
    }
    target.addEventListener('afterprint', cleanup, { once: true });
    target.focus();
    target.print();
    // Некоторые WebKit не эмитят afterprint для PDF: DOM уже можно убрать,
    // а blob живёт достаточно долго для системного диалога.
    setTimeout(cleanup, 60_000);
  };
  document.body.append(frame);
}

/** Поделиться PDF через Web Share; где файловый sharing недоступен — скачать его. */
export async function sharePdf(bytes: Uint8Array, label: string): Promise<'shared' | 'downloaded'> {
  const name = exportFileName(label, 'pdf');
  const blob = new Blob([new Uint8Array(bytes)], { type: 'application/pdf' });
  const file = new File([blob], name, { type: 'application/pdf' });
  if (navigator.share && (!navigator.canShare || navigator.canShare({ files: [file] }))) {
    await navigator.share({ files: [file], title: name });
    return 'shared';
  }
  downloadBlob(blob, name, 'application/pdf');
  return 'downloaded';
}
