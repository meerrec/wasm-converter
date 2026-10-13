/**
 * Воркер вьюера документа — тот же, что у примера-просмотрщика
 * (`examples/viewer/src/workers/docx_worker.ts`): ядро держит его код в отдельной точке входа,
 * а страница подключает её суффиксом `?worker&url` — только так Vite собирает воркер отдельным
 * чанком (см. src/viewer.ts).
 *
 * Импорт относительный, а не `@doc-converter/core/worker`: харнесс вне pnpm-workspace, и имя
 * пакета в нём не резолвится. Голый импорт `@doc-converter/wasm` внутри этой сборки закрывает
 * алиас из vite.config.ts.
 */
import '../../../packages/core/dist/worker.js';
