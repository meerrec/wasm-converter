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
    // 330,6 kB по факту: запас ~15 % — чуть больше, чем у строки просмотрщика
    // (285,4 из 300, ~5 %). Модуль новый, и в нём ещё запланированы слайсы
    // (картинки DOCX, поиск по тексту); но удвоение или возврат ещё одного
    // формата в модуль бюджет покраснеет.
    name: 'wasm (docx)',
    path: 'packages/wasm-docx/pkg/*.wasm',
    limit: '380 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
];
