# Runtime-контракт LOWA: как JS поднимает LibreOffice и вызывает UNO

Слайс спайка 13. Контракт снят с исходников, а не из документации: `README.wasm.md` описывает
сборку, а не рантайм, и всё, что ниже, либо вычитано из кода, либо помечено как непроверенное.

**Источники** (номера строк — по этим копиям). Master: `static/emscripten/{uno,script,environment}.js`
(кроме них в каталоге только `favicon.ico`), `desktop/source/app/initjsunoscripting.cxx`
(далее `cxx:`), `static/README.wasm.md`, `configure.ac`, `comphelper/source/misc/threadpool.cxx`,
`include/comphelper/threadpool.hxx`, `solenv/gbuild/gbuild.mk`. zetajs main: `docs/start.md`,
`README.md`, `examples/convertpdf/{zetaHelper.js,office_thread.js,index.html,config.sample.js}`.
Артефакт сборки (не master, но именно его грузит браузер): `/tmp/lowa-probe/package/wasm/soffice.js`
и `soffice.worker.js`. Локальные копии — `/tmp/lowa-src-s2/` и `/tmp/lowa-probe/`.

## 1. Точка входа и порядок инициализации

Харнесс обязан выставить в `Module` до запуска `soffice.js`:

- `Module.uno_scripts` — массив строк-URL скриптов UNO-обвязки, грузятся **по порядку**; если
  какой-то из них использует zetajs, `zeta.js` обязан идти первым (`start.md:15`). Читает массив
  C++-сторона: `emscripten::val::module_property("uno_scripts")`, каждый URL разрешается
  относительно `location.href` через `rtl::Uri::convertRelToAbs` (`initjsunoscripting.cxx:32-44`).
  Загрузка — `importScripts.apply(self, urls)` при `PROXY_TO_PTHREAD` (`cxx:47-50`), иначе по одному
  `fetch`+`eval` на главном потоке (`cxx:52-68`).
- `Module.locateFile(path, prefix)` — в реализации `(prefix || soffice_base_url) + path`
  (`zetaHelper.js:38`); нужен, чтобы `soffice.wasm`/`soffice.data` брались с CDN, а не рядом с HTML.
- `Module.mainScriptUrlOrBlob` — из него Emscripten берёт скрипт для pthread-воркеров:
  `pthreadMainJs = Module["mainScriptUrlOrBlob"]`, не-строка превращается в `URL.createObjectURL`
  и уходит в `new Worker(pthreadMainJs, {workerData:"em-pthread", name:"em-pthread-"+PThread.nextWorkerID})`
  (`soffice.js` (артефакт), `PThread.allocateUnusedWorker`). Реализация кладёт туда Blob с
  `importScripts('<абс. URL soffice.js>')` (`zetaHelper.js:41`) — так воркер догружает саму LOWA.
- `Module.canvas` — для сборки с Qt обязан ссылаться на `<canvas id="qtcanvas">` (`start.md:17`);
  headless-сборка (`--disable-gui`) канвас не использует вовсе (`start.md:9`). Хелпер ищет
  `getElementById('qtcanvas')` без проверки на null (`zetaHelper.js:20`), а в демо-странице элемент
  есть, просто скрытый (`index.html:21`).

Порядок запуска (`zetaHelper.js`):

1. собрать объект `Module` и положить его в `window.Module` (`:35-41`, `:65`);
2. вставить `<script src="<base>soffice.js">`; `onload` срабатывает после исполнения скрипта
   (`:75-78`, `:106`);
3. в `onload` дождаться `Module.uno_main` — это MessagePort к офису (`:80-82`).

Промисы и словарь:

- `Module.uno_init` заводится в `uno.js:12-18`, резолвится `Module.uno_init$resolve`, который
  выполняет `Module.uno = init_unoembind_uno(Module, Module.unoTagSymbol)` (`uno.js:13-16`).
  Резолвит его сторона офиса: `setupMainChannel` (`cxx:71-82`) ловит `{cmd:"LOWA-channel"}` с
  переданным портом, сохраняет его в `Module.uno_mainPort` и зовёт `uno_init$resolve()`.
- `Module.uno_main` заводится в `uno.js:20-23`, резолвится `Module.uno_main$resolve(channel.port1)`
  из `resolveUnoMain` (`cxx:84-99`) — то есть на главном потоке страницы; `port2` уезжает в офис.
- Словарь — `Module.uno.com.sun.star...` (`uno.js:83`, `:87`, `:103`), плюс `Module.uno_Any`
  (`uno.js:46`), `Module.uno_Type` (`uno.js:52`, `:68`), `Module.unoObject` (`uno.js:37-115`),
  `Module.catchUnoException` (`uno.js:25-35`), `Module.getUnoComponentContext()`
  (`uno.js:97`, `README.wasm.md:215`). Embind-типы последовательностей —
  `Module.uno_Sequence_com$sun$star$beans$PropertyValue` с `Module.uno_Sequence.FromSize`
  (`README.wasm.md:212-214`).

Полная последовательность: главный поток грузит `soffice.js` → офис стартует в pthread-воркере,
`initJsUnoScripting()` (вызов — `appinit.cxx:88-90`, сразу после `setProcessServiceFactory`)
импортирует `uno_scripts`, ставит обработчик канала, асинхронно просит `resolveUnoMain` создать
MessageChannel (`cxx:103-110`) → главный поток получает `uno_main`, офис — `uno_mainPort`; дальше
весь трафик идёт по этому каналу.

## 2. Воркерность: где именно исполняются скрипты

- `script.js:10-12`: `if (typeof Module.script == 'undefined' && typeof document != 'undefined')
  Module.script = document.currentScript;` — проверка на `typeof document` существует ровно
  потому, что файл исполняется и там, где `document` нет, то есть в воркере. В `uno.js` обращений
  к `document` нет вообще (файл прочитан целиком) — тот же вывод с другой стороны.
- Поток — pthread-воркер Emscripten: `ENVIRONMENT_IS_PTHREAD = ENVIRONMENT_IS_WORKER &&
  self.name?.startsWith("em-pthread")` (`soffice.js` (артефакт)), имя `em-pthread-<N>` ставится в
  `allocateUnusedWorker`. В `soffice.worker.js` тот же признак виден полем
  `Module["ENVIRONMENT_IS_PTHREAD"]=true` и загрузкой главного скрипта через
  `importScripts(e.data.urlOrBlob)`.
- Дебажить надо именно его: `office_thread.js:5-6` — «переключите воркер в отладчике; это
  `em-pthread` с наибольшим потреблением памяти, где определён `zetajs`».
- `PROXY_TO_PTHREAD` включён по умолчанию: `configure.ac:2355-2358` объявляет
  `--disable-emscripten-proxy-to-pthread` с формулировкой «Experimentally disable use of the
  Emscripten -sPROXY_TO_PTHREAD feature» и дефолтом `enable_emscripten_proxy_to_pthread=yes`.
  Отсюда главный поток LOWA — воркер (`README.wasm.md:263`: «the first web worker thread, which is
  the LO main thread since we use -sPROXY_TO_PTHREAD»), поэтому и UNO-примеры из README
  предназначены для консоли воркера, а не страницы.
- Зачем это: эмуляция pthreads требует, чтобы событийный цикл главного JS-потока вовремя
  обслуживал spawn/join; если цикл занят (в Qt-сборке там крутится цикл VCL), спавны упираются в
  пул (`README.wasm.md:399-416`). Вынос LOWA в воркер освобождает главный поток. У Qt-сборки так
  сделать не удалось по другой причине — Qt5 обращается к `window` (`README.wasm.md:419-422`).
- `--enable-emscripten-jspi` (`configure.ac:2347-2352`, «Experimentally enable use of the Emscripten
  -sJSPI feature») — альтернатива, тоже экспериментальная. В `initjsunoscripting.cxx` ветвление
  идёт только по `HAVE_EMSCRIPTEN_PROXY_TO_PTHREAD` (`cxx:46`, `:85`), ветки по JSPI нет; в
  `README.wasm.md` слово JSPI не встречается, а `configure.ac:4417-4423` лишь заводит define.
  Что JSPI меняет в этом контракте — в исходниках не найдено.

## 3. Настройка конкурентности и выхода

`environment.js:1-5` целиком: `Module.preRun.push(() => { ENV.MAX_CONCURRENCY = '4'; })` и
`Module.ignoreApplicationExit = true`.

**Уточнение по источникам:** `MAX_CONCURRENCY` — это НЕ размер пула Emscripten-pthreads, а потолок
собственного пула LibreOffice: `comphelper::ThreadPool::getPreferredConcurrency()` читает
`getenv("MAX_CONCURRENCY")` и берёт `min(hardware_concurrency(), env)`
(`comphelper/source/misc/threadpool.cxx:137-153`), доккомментарий — «MAX_CONCURRENCY env. var.
controls the cap» (`threadpool.hxx:55-59`); те же 4 подставляются тестам через
`gb_TEST_ENV_VARS := MAX_CONCURRENCY=4` (`gbuild.mk:253`).

Размер пула pthreads задаётся при сборке, `-sPTHREAD_POOL_SIZE` (`README.wasm.md:407`). В
собранном артефакте ZetaOffice он равен четырём: `initMainThread(){var pthreadPoolSize=4;
while(pthreadPoolSize--){PThread.allocateUnusedWorker()}` (`soffice.js` (артефакт)), а
`pthreadPoolReady` до старта рантайма ждёт загрузки модуля во все воркеры
(`PThread.loadWasmModuleToAllWorkers`). Совпадение с `MAX_CONCURRENCY` — совпадение, а не связь.

Риск исчерпания пула описан в `README.wasm.md:409-417`: при спавне/join/re-spawn внутри одной
VCL-задачи спавны «eventually run out of -sPTHREAD_POOL_SIZE pre-spawned JS Workers»; рекомендованное
лечение — не join'ить такие потоки, а держать их запущенными. Что именно видит вызывающий при
исчерпании (ошибка, деградация, зависание) — в исходниках не найдено.

`Module.ignoreApplicationExit = true` нужен headless-сценарию: приложение не должно завершать
процесс после закрытия последнего документа. Где флаг читается, в исходниках master не найдено:
поиск по репозиторию даёт только эту строку, в артефактных `soffice.js` и `soffice.worker.js` её
нет вовсе. Известны имя и место установки (в `preRun`-окружении), потребитель — неизвестен; это
пункт проверки в спайке.

## 4. Требования среды

- Cross-origin isolation: `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy:
  require-corp` — требование к серверу, выписанное дважды: `README.wasm.md:150-153` и
  `zetajs/examples/convertpdf/README.md:25-29`. Без них браузер не отдаёт SharedArrayBuffer, на
  котором стоят pthreads Emscripten (`README.wasm.md:369` ссылается на emscripten pthreads; в
  склейке поток получает общий `wasmMemory` — `soffice.worker.js:1`).
- Артефакты с чужого origin: нужны CORS и CORP — zetajs перечисляет ровно
  `Cross-Origin-Resource-Policy: cross-origin`, `Access-Control-Allow-Origin: *`,
  `Access-Control-Allow-Methods: *` (`config.sample.js:14-18`), README zetajs упоминает CORS при
  хостинге на другом origin (`zetajs README.md:74`). Проверено запросом к CDN:
  `curl -I https://cdn.zetaoffice.net/zetaoffice_latest/soffice.js` отдаёт
  `access-control-allow-origin: *` и `cross-origin-resource-policy: cross-origin`.
- Отдавать нужно четыре файла: `soffice.{data,data.js.metadata,js,wasm}` (`config.sample.js:20`);
  `soffice.data` — образ in-memory FS, в артефакте ZetaOffice он ~99.7 МБ.
- Потолок памяти — wasm32: 4 ГБ адресного пространства (`README.wasm.md:326-328`, «You can actually
  allocate 4GB» со ссылкой на баг Mozilla); в Qt-части ставится TOTAL_MEMORY 1 ГБ
  (`README.wasm.md:323-325`).
- Потоки эмуляционные, JS-воркерами: «The Emscripten emulation of pthreads…»
  (`README.wasm.md:401`); динамические модули с потоками несовместимы — «you can't use modules and
  threads» (`README.wasm.md:288-293`).

## 5. Сценарий конвертации через UNO

Оговорка, обязательная для спайка. Имена фильтров ниже взяты из примера, а не из спецификации, и
их нужно получить перечислением: сервис `com.sun.star.document.FilterFactory`
(`offapi/com/sun/star/document/FilterFactory.idl:38`) экспортирует `XNameAccess` — «provides read
access to the complete set of configuration data» (там же `:183-184`) с `getElementNames()`, и
`XContainerQuery` (`:196`) для выборок по свойствам. Точная форма получения сервиса в zetajs
(функция-синглтон или `.create(ctx)`, `start.md:99-101`) в исходниках не найдена — проверять в
спайке. `FilterData` обязана заполняться явно: незаданные опции наследуют последние пользовательские
настройки экспорта — в исходниках не найдено, известная ловушка UNO, проверять в спайке.

Минимальный рабочий фрагмент (по `office_thread.js` и `index.html`):

```js
const css = zHT.css;                    // office_thread.js:16
const beanHidden    = new css.beans.PropertyValue({ Name: 'Hidden',    Value: true });
const beanOverwrite = new css.beans.PropertyValue({ Name: 'Overwrite', Value: true });
const beanFilter    = new css.beans.PropertyValue({ Name: 'FilterName', Value: 'writer_pdf_Export' });

// from/to — пути в memfs, вида /tmp/input.docx; расширение сохраняем: по нему LO определяет
// тип документа (index.html:40-49).
const xModel = zHT.desktop.loadComponentFromURL('file://' + from, '_blank', 0, [beanHidden]);
xModel.storeToURL('file://' + to, [beanOverwrite, beanFilter]);
```

- Прошлый документ закрывается до загрузки нового:
  `xModel.queryInterface(zetajs.type.interface(css.util.XCloseable))` → `xModel.close(false)`
  (`office_thread.js:55-58`).
- `_blank` — в примере (`office_thread.js:61`), `_default` — в примере README
  (`README.wasm.md:212-218`); разница в поведении в исходниках не описана.
- Ошибки: `catch { const exc = zetajs.catchUnoException(e); … exc.Message }`
  (`office_thread.js:70-72`); `catchUnoException` перебрасывает не-UNO исключения
  (`uno.js:25-30`), `zetajs.getAnyType` достаёт тип из ANY (`start.md:77`).
- Результат: файл появляется в memfs, читает его главный поток из `FS` (в хелпере это `window.FS`,
  `zetaHelper.js:82`), затем `new Blob([data], {type:'application/pdf'})` → `URL.createObjectURL`
  → iframe/ссылка (`index.html:61-64`). Запись входа — `FS.writeFile(from, new Uint8Array(data))`
  (`index.html:51`).
- При ошибке пример **не отвечает** отправителю (только логирует, `office_thread.js:69-72`), поэтому
  протокол ошибок и таймаут — забота харнесса.

## 6. Различия main ↔ thread

ZetaOffice делит обязанности жёстко:

- `ZetaHelperMain` (главный поток) владеет `window.Module` (`zetaHelper.js:65`), `thrPort` из
  `Module.uno_main` (`:81`), `FS` (`:82`), канвасом (`:20`). UNO здесь недоступен.
- `ZetaHelperThread` (воркер) владеет `zetajs`, `zJsModule`, `thrPort = zetajs.mainPort`
  (`:167-169`), `css`, `context`, `desktop`, `config` (`:170-173`). Только здесь есть UNO.
- Порты — две стороны одного MessageChannel: главному потоку его отдаёт `Module.uno_main`
  (`start.md:117`), воркеру — `zetajs.mainPort` (`start.md:119`), а на C++-стороне он лежит в
  `Module.uno_mainPort` (`initjsunoscripting.cxx:76`).
- Причина разделения прямо названа: код в воркере не имеет доступа к `document` (`start.md:115`).
- Прикладной протокол — свой: `{cmd:'convert', from, to}` → `{cmd:'converted', name, from, to}`
  (`office_thread.js:52`, `:63-68`); у хелпера служебные `ZetaHelper::thr_started` и
  `ZetaHelper::run_thr_script` (`zetaHelper.js:85-95`, `:130-149`) плюс обратный `thr_started`
  (`:156-158`). Формат целиком на совести приложения (`start.md:115`).

## 7. Чего контракт не обещает

- Имена фильтров не проверены: `writer_pdf_Export`, `calc_pdf_Export` — из примеров; обязательное
  перечисление через `FilterFactory` (§5).
- `FilterData` и наследование пользовательских настроек экспорта — в исходниках не найдено.
- `uno_scripts`, чтение через `module_property`, `importScripts` — деталь реализации master
  (`initjsunoscripting.cxx`). zetajs формулирует требование к массиву (`start.md:15`), но механизм
  не обещает; в allotropia-дистрибутиве он может отличаться.
- Повторная инициализация и второй документ в той же сессии не описаны: пример держит документ
  открытым «for debugging» и закрывает предыдущий лишь при следующем `convert`
  (`office_thread.js:54-58`) — это привычка примера, не контракт. Как изолировать сессии (отдельный
  воркер/страница) — в исходниках не найдено.
- Гонка в ветке без `PROXY_TO_PTHREAD`: там `runUnoScriptUrls` асинхронный
  (`fetch().then(…eval…)`, `cxx:52-68`), а `setupMainChannel` вызывается сразу (`cxx:108`), и
  окончания загрузки скриптов никто не ждёт. С `PROXY_TO_PTHREAD` всё синхронно (`importScripts`),
  так что ветка не боевая, но помнить о ней стоит.
- Исчерпание pthread-пула: README говорит «run out» (`README.wasm.md:415-417`), но что при этом
  наблюдает вызывающий — не найдено.
- `Module.ignoreApplicationExit`: потребитель флага не найден (§3).
- JSPI: за пределами configure-флага не описан (§2).
