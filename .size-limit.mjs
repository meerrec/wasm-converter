// size-limit меряет JS-бандлы через esbuild, а `.wasm` для него — не модуль.
// Поэтому конфиг не JSON: трём `.wasm`-строкам нужен `modifyEsbuildConfig`
// с loader'ом `copy` — он кладёт файл в каталог сборки как есть, и
// @size-limit/file считает его gzip. PDF-модуль получил бюджет после миграции
// на собственный бэкенд (ADR-0022): строка фиксирует выигрыш −32 % gzip и не
// даёт вернуть PDF-библиотеку обратно. У DOCX-модуля бюджет есть,
// хотя он тоже ленивый: DOCX — второй просмотрщик, и по решению спринта
// (ROADMAP §9) его строка стережёт и рост модуля, и возврат его кода в
// основной бандл.
export default [
  { path: 'packages/core/dist/worker.js', limit: '550 KB', gzip: true },
  { path: 'packages/core/dist/index.js',  limit: '200 KB', gzip: true },
  {
    // Бюджет поднят 300 → 480 КБ в Спринте 12: коммит b43a2de вшил четыре
    // поднабора Carlito (~177 КБ gzip), чтобы превью рисовало той же
    // гарнитурой, что и PDF (иначе браузер подставляет Helvetica, ширина
    // строки уходит на ~14 %, превью расходится с экспортом). Вынос шрифтов
    // в ленивый ассет — задел после v1.0; строка по-прежнему стережёт
    // возврат кода в основной бандл.
    name: 'wasm (viewer)',
    path: 'packages/wasm/pkg/*.wasm',
    limit: '480 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    name: 'wasm (docx)',
    path: 'packages/wasm-docx/pkg/*.wasm',
    limit: '400 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // 991 kB по факту после миграции на собственный бэкенд (ADR-0022, было
    // ≈1,46 МБ с printpdf). Запас ~6 % — как у соседних строк; возврат
    // PDF-библиотеки или раздувание шрифтовой части бюджет покрасят.
    name: 'wasm (pdf)',
    path: 'packages/wasm-pdf/pkg/*.wasm',
    limit: '1050 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
];
