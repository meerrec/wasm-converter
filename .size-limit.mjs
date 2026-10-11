// size-limit меряет JS-бандлы через esbuild, а `.wasm` для него — не модуль.
// Поэтому конфиг не JSON: обеим `.wasm`-строкам нужен `modifyEsbuildConfig`
// с loader'ом `copy` — он кладёт файл в каталог сборки как есть, и
// @size-limit/file считает его gzip. PDF-модуль бюджета не имеет: он грузится
// по требованию и на старт просмотрщика не влияет. У DOCX-модуля бюджет есть,
// хотя он тоже ленивый: DOCX — второй просмотрщик, и по решению спринта
// (ROADMAP §9) его строка стережёт и рост модуля, и возврат его кода в
// основной бандл.
export default [
  { path: 'packages/core/dist/worker.js', limit: '550 KB', gzip: true },
  { path: 'packages/core/dist/index.js',  limit: '200 KB', gzip: true },
  {
    name: 'wasm (viewer)',
    path: 'packages/wasm/pkg/*.wasm',
    limit: '300 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // 380,6 kB по факту: бюджет поднят с 380 kB в Спринте 11 — пришли
    // запланированные слайсы (рисунки с медиачастью, колонтитулы и поля
    // PAGE/NUMPAGES, многоколоночность, границы и заливка из стиля таблицы).
    // Запас ~5 % — как у строки просмотрщика; удвоение или возврат ещё одного
    // формата в модуль бюджет покраснеет.
    name: 'wasm (docx)',
    path: 'packages/wasm-docx/pkg/*.wasm',
    limit: '400 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
];
