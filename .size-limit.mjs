// size-limit меряет JS-бандлы через esbuild, а `.wasm` для него — не модуль.
// Поэтому конфиг не JSON: единственной не-JS строке нужен `modifyEsbuildConfig`
// с loader'ом `copy` — он кладёт файл в каталог сборки как есть, и
// @size-limit/file считает его gzip. PDF-модуль бюджета не имеет: он грузится
// по требованию и на старт просмотрщика не влияет.
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
];
