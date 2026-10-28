// size-limit меряет JS-бандлы через esbuild, а `.wasm` для него — не модуль.
// Поэтому конфиг не JSON: трём `.wasm`-строкам нужен `modifyEsbuildConfig`
// с loader'ом `copy` — он кладёт файл в каталог сборки как есть, и
// @size-limit/file считает его gzip. PDF-модуль получил бюджет после миграции
// на собственный бэкенд (ADR-0022): строка фиксирует выигрыш −32 % gzip и не
// даёт вернуть PDF-библиотеку обратно. У DOCX-модуля бюджет есть,
// хотя он тоже ленивый: DOCX — второй просмотрщик, и по решению спринта
// (ROADMAP §9) его строка стережёт и рост модуля, и возврат его кода в
// основной бандл.
// Обёртки фреймворков (T4.1/T4.2): бюджет считает только код обёртки —
// @meerrec/core, react, vue и svelte — зависимости потребителя, у каждой из
// них своя строка бюджета или чужой пакет. `ignore` отдаёт их esbuild как
// external.
//
// @meerrec/svelte публикует исходник `.svelte` (peer svelte ^4 || ^5 —
// потребитель компилирует своей версией), поэтому esbuild мерит его через
// onLoad-хук: он компилирует компонент компилятором svelte из node_modules
// пакета, а рантайм svelte и esm-env помечает external.
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';

const svelteRequire = createRequire(
  new URL('./packages/svelte/package.json', import.meta.url),
);

const svelteMeasure = (config) => ({
  ...config,
  plugins: [
    ...(config.plugins ?? []),
    {
      name: 'svelte-measure',
      setup(build) {
        build.onResolve({ filter: /^(svelte|esm-env)(\/|$)/ }, (args) => ({
          path: args.path,
          external: true,
        }));
        build.onLoad({ filter: /\.svelte$/ }, async (args) => {
          // svelte/compiler — UMD-бандл: require отдаёт экспорты напрямую.
          const { compile } = svelteRequire('svelte/compiler');
          const source = await readFile(args.path, 'utf8');
          const { js } = compile(source, {
            filename: args.path,
            generate: 'client',
            runes: false,
          });
          return { contents: js.code, loader: 'js' };
        });
      },
    },
  ],
});

export default [
  {
    // Воркеры XLSX и DOCX разделены (направление «воркер на формат»,
    // ADR-0030 §6): в каждом та же painter/SAB-обвязка и кадровый цикл,
    // поэтому бюджет прежний — общий диспетчер по `DocFormat` больше не нужен.
    name: 'core (xlsx-worker)',
    path: 'packages/core/dist/xlsx_worker.js',
    limit: '550 KB',
    gzip: true,
  },
  {
    name: 'core (docx-worker)',
    path: 'packages/core/dist/docx_worker.js',
    limit: '550 KB',
    gzip: true,
  },
  {
    // Воркер PPTX — отдельный бандл (направление «воркер на формат»,
    // ADR-0030 §6): тот же бюджет, что у общего воркера — в нём те же
    // painter/SAB-обвязка и кадровый цикл.
    name: 'core (pptx-worker)',
    path: 'packages/core/dist/pptx_worker.js',
    limit: '550 KB',
    gzip: true,
  },
  { path: 'packages/core/dist/index.js',  limit: '200 KB', gzip: true },
  {
    // Бюджет поднят 300 → 480 КБ в Спринте 12: коммит b43a2de вшил четыре
    // поднабора Carlito (~177 КБ gzip), чтобы превью рисовало той же
    // гарнитурой, что и PDF (иначе браузер подставляет Helvetica, ширина
    // строки уходит на ~14 %, превью расходится с экспортом). Вынос шрифтов
    // в ленивый ассет — задел после v1.0; строка по-прежнему стережёт
    // возврат кода в основной бандл.
    //
    // 480 → 490 КБ в 2.0.2: редактирование ячеек (xlsx_set_cell_text,
    // снимок правок для lazy pdf-wasm) — +2,6 КБ gzip.
    name: 'wasm (viewer)',
    path: 'packages/wasm/pkg/*.wasm',
    limit: '490 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // Бюджет поднят 400 → 410 КБ в T4 (эпик «Индустриализация»): Strict OOXML
    // (ADR-0034) добавил детект варианта и таблицы namespace'ов — на CI модуль
    // вырос до 400,57 КБ gzip (+567 Б). Запас ~2,4 % — как у соседних строк.
    //
    // 410 → 420 КБ в Спринте 2: UAX #9 и TextLayer (ADR-0040/0041). Полная
    // UCD-таблица unicode-bidi не влезала вовсе (+13,2 КБ); с компактным
    // классификатором направлений модуль — 417,07 КБ (+7,05 КБ, 1,7 %).
    // Отказ от unicode-bidi сломал бы смешанные RTL-абзацы, поэтому бюджет
    // поднят осознанно; trade-off зафиксирован в отчёте спринта.
    // 420 → 425 КБ: подсветка знака (`w:highlight` и `w:rPr/w:shd`) и
    // подъём полосы набора над высоким подвалом добавили 1,33 КБ.
    // 425 → 435 КБ: правка текста run в рантайме (ADR-0043) — слой правок
    // в модели DOCX и его подстановка в раскладку. Замер 427,77 КБ gzip.
    name: 'wasm (docx)',
    path: 'packages/wasm-docx/pkg/*.wasm',
    limit: '435 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // Просмотрщик PPTX — второй ленивый воркер-модуль: бюджет как у docx
    // (480 КБ — решение T3.1, ADR-0033). PDF-экспорта в модуле нет —
    // `export_pptx_pdf` живёт в @meerrec/wasm-pdf, поэтому строка стережёт
    // возврат PDF-кода в просмотрщик.
    name: 'wasm (pptx)',
    path: 'packages/wasm-pptx/pkg/*.wasm',
    limit: '480 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // 991 kB по факту после миграции на собственный бэкенд (ADR-0022, было
    // ≈1,46 МБ с printpdf). Запас ~6 % — как у соседних строк; возврат
    // PDF-библиотеки или раздувание шрифтовой части бюджет покрасят.
    // 1050 → 1120 КБ: экспорт XLSB/ODS/ODT в PDF (ADR-0046/0048) — модуль
    // потянул `doc-converter-odf` (ods+odt) и `doc-converter-xlsb`, потому что
    // эти форматы доводятся до моделей XLSX/DOCX уже здесь. Замер 1,11 МБ.
    name: 'wasm (pdf)',
    path: 'packages/wasm-pdf/pkg/*.wasm',
    limit: '1120 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // 25 КБ по плану T4.1 (ROADMAP §9 задаёт бюджет react; единый лимит для
    // тонких обёрток — решение T4.2).
    name: '@meerrec/react',
    path: 'packages/react/dist/index.js',
    limit: '25 KB',
    gzip: true,
    ignore: ['react', 'react-dom', 'react/jsx-runtime', '@meerrec/core'],
  },
  {
    name: '@meerrec/vue',
    path: 'packages/vue/dist/index.js',
    limit: '25 KB',
    gzip: true,
    ignore: ['vue', '@meerrec/core'],
  },
  {
    name: '@meerrec/svelte',
    path: 'packages/svelte/dist/index.js',
    limit: '25 KB',
    gzip: true,
    ignore: ['@meerrec/core'],
    modifyEsbuildConfig: svelteMeasure,
  },
  {
    // Экспортёр PPTX в PDF — отдельный ленивый модуль (ADR-0033: просмотрщик
    // и экспортёр разнесены, как у DOCX): полный стек PPTX+PDF (разбор,
    // раскладка, шрифты, постраничный бэкенд) стоит ~782 КБ gzip. Лимит
    // 800 КБ — решение владельца T3.1; строка стережёт возврат экспортёра в
    // модуль просмотра и рост за счёт фич без пересмотра.
    // 800 → 830 КБ: экспорт ODP (ADR-0047) приносит `doc-converter-odf` с
    // разбором ODP, чтобы довести его до модели PPTX. Замер 808,59 КБ gzip.
    name: 'wasm (pptx-pdf)',
    path: 'packages/wasm-pptx-pdf/pkg/*.wasm',
    limit: '830 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // Ленивый модуль XLSB (ADR-0048): 242 КБ gzip по факту. Бюджет стережёт
    // возврат парсера BIFF12 в основной бандл и рост за счёт фич.
    name: 'wasm (xlsb)',
    path: 'packages/wasm-xlsb/pkg/*.wasm',
    limit: '260 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // Ленивый модуль ODS (ADR-0047): 259 КБ gzip по факту.
    name: 'wasm (ods)',
    path: 'packages/wasm-ods/pkg/*.wasm',
    limit: '275 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // Ленивый модуль ODT (ADR-0047): 308 КБ gzip по факту.
    name: 'wasm (odt)',
    path: 'packages/wasm-odt/pkg/*.wasm',
    limit: '330 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
  {
    // Ленивый модуль ODP (ADR-0047): 219 КБ gzip по факту.
    name: 'wasm (odp)',
    path: 'packages/wasm-odp/pkg/*.wasm',
    limit: '235 KB',
    gzip: true,
    modifyEsbuildConfig: (config) => ({ ...config, loader: { '.wasm': 'copy' } }),
  },
];
