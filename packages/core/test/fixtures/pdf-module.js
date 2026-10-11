// Заглушка ленивого PDF-модуля: воркер грузит её тем же динамическим импортом
// по URL из `init`, что и настоящий `@doc-converter/wasm-pdf`, — подмена через
// `vi.mock` не проверила бы саму ленивость. Из воркера модуль не виден, поэтому
// отметки о загрузке и вызовах он оставляет в `globalThis`; тест сбрасывает их
// перед каждым сценарием.

/** Отметки заглушки; тест заводит объект заново перед каждым сценарием. */
const marks = () =>
  (globalThis.__pdfStub ??= {
    loads: 0,
    initCalls: 0,
    calls: {},
  });

/** Записать вызов: аргументы те же, какими их получила бы настоящая функция. */
const record = (name, args) => {
  const m = marks();
  (m.calls[name] ??= []).push(args);
};

/** Узнаваемое начало PDF: по нему тест отличает ответ заглушки от чужих байтов. */
const PDF_BYTES = new TextEncoder().encode('%PDF-1.7 docx stub');

marks().loads++;

export default async (...args) => {
  marks().initCalls++;
  record('default', args);
};

export function export_docx_pdf(...args) {
  record('export_docx_pdf', args);
  // Копия константы, а не она сама: воркер переводит владение буфером при
  // postMessage, и в настоящем воркере буфер был бы отозван — общая константа
  // после первого экспорта осталась бы пустой.
  return new Uint8Array(PDF_BYTES);
}
