// Заглушка ленивого DOCX-модуля: воркер грузит её тем же динамическим импортом
// по URL из `init`, что и настоящий `@doc-converter/wasm-docx`, — подмена через
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
};

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

export function docx_build_display_list(...args) {
  record('docx_build_display_list', args);
  return new Uint8Array();
}

export function docx_build_display_list_sab(...args) {
  record('docx_build_display_list_sab', args);
  return { written: true, cmds: 1, buildMs: 0 };
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
