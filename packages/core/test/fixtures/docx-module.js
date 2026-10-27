// Заглушка ленивого DOCX-модуля: воркер грузит её тем же динамическим импортом
// по URL из `init`, что и настоящий `@meerrec/wasm-docx`, — подмена через
// `vi.mock` не проверила бы саму ленивость. Из воркера модуль не виден, поэтому
// отметки о загрузке и вызовах он оставляет в `globalThis`; тест сбрасывает их
// перед каждым сценарием.

/** Отметки заглушки; тест заводит объект заново перед каждым сценарием. */
const marks = () =>
  (globalThis.__docxStub ??= {
    loads: 0,
    initCalls: 0,
    pages: [
      { width: 595, height: 842 },
      { width: 595, height: 842 },
    ],
    calls: {},
  });

/** Записать вызов: аргументы те же, какими их получила бы настоящая функция. */
const record = (name, args) => {
  const m = marks();
  (m.calls[name] ??= []).push(args);
};

marks().loads++;

export default async (...args) => {
  marks().initCalls++;
  record('default', args);
  // Ворота «модуль ещё не загружен»: тест кладёт сюда `{ promise }` и открывает
  // их сам, чтобы проверить догон накопленных чанков. Без ворот `init`
  // резолвится сразу, как у настоящего модуля.
  const gate = globalThis.__docxStubGate;
  if (gate) await gate.promise;
};

// Потоковое открытие (ADR-0031/0049): воркер зовёт `stream_new` после загрузки
// модуля, `stream_append` на каждый чанк и `stream_open_docx` в конце.
let nextStreamId = 1;

export function stream_new(...args) {
  record('stream_new', args);
  return nextStreamId++;
}

export function stream_append(id, bytes, ...args) {
  // Длина, а не сам буфер: тест сверяет порядок и поштучность, а не содержимое.
  record('stream_append', [id, bytes.byteLength, ...args]);
}

export function stream_free(id, ...args) {
  record('stream_free', [id, ...args]);
}

export function stream_open_docx(id, ...args) {
  record('stream_open_docx', [id, ...args]);
  return marks().pages;
}

export function docx_open_bytes(...args) {
  record('docx_open_bytes', args);
  return marks().pages;
}

export function docx_open(...args) {
  record('docx_open', args);
  return marks().pages;
}

export function docx_close(...args) {
  record('docx_close', args);
}

export function docx_is_open(...args) {
  record('docx_is_open', args);
  return true;
}

// Сборщики кадра стопки: окно (от верха стопки) и зазор между страницами —
// те же аргументы, что у настоящего модуля. Параметры названы, чтобы порядок
// аргументов было видно из заглушки: расходятся они — расходится и запись.

export function docx_build_display_list_sab(sab, slotCapacity, viewport, gap) {
  record('docx_build_display_list_sab', [sab, slotCapacity, viewport, gap]);
  return { written: true, cmds: 1, buildMs: 0 };
}

export function docx_build_display_list_into(out, viewport, gap) {
  record('docx_build_display_list_into', [out, viewport, gap]);
  return { written: true, cmds: 1, bytes: 0, buildMs: 0 };
}

// Картинки документа: список и «битые» id тест подменяет глобалами, потому что
// достучаться до заглушки иначе нечем — воркер грузит её по URL из `init`.
// По умолчанию список пуст: картинки нужны только сценарию о них. Байты
// кодируют id первым элементом, чтобы тест видел, какая картинка дошла до
// декодирования; `__docxImagesBroken` — id, на которых `docx_image_bytes`
// падает, как испорченная часть пакета.

export function docx_images(...args) {
  record('docx_images', args);
  return globalThis.__docxImages ?? [];
}

export function docx_image_bytes(id, ...args) {
  record('docx_image_bytes', [id, ...args]);
  if (globalThis.__docxImagesBroken?.includes(id)) {
    throw new Error(`docx stub: image ${id} is broken`);
  }
  return new Uint8Array([id]);
}

// Текстовые выгрузки: у каждой свой узнаваемый ответ, чтобы тест видел, какая
// именно функция ответила на запрос формата.

export function docx_export_markdown(...args) {
  record('docx_export_markdown', args);
  return '# markdown stub';
}

export function docx_export_html(...args) {
  record('docx_export_html', args);
  return '<!DOCTYPE html><html><body>html stub</body></html>';
}

export function docx_export_text(...args) {
  record('docx_export_text', args);
  return 'text stub';
}
