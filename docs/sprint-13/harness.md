# Харнесс LOWA: свой запуск конвертации DOCX/XLSX в PDF

Слайс B2 спайка 13. Артефакт DoD: `experiments/lowa-spike/` — минимальная страница и воркер,
которые поднимают LOWA (LibreOffice WebAssembly, публичная сборка ZetaOffice) и по выбору файла
отдают PDF. Рядом — `tools/harness-check.mjs`, который прогоняет это в настоящем Chromium и
сверяет результат с эталоном нативного LibreOffice.

## Как запускать

```bash
cd /Users/irek/GitHub/wasm-converter/doc-converter
pnpm -C experiments/lowa-spike install --ignore-workspace   # один раз, ставит только vite
pnpm -C experiments/lowa-spike dev                          # http://localhost:5199/
```

`--ignore-workspace` нужен потому, что `experiments/` не входит в `pnpm-workspace.yaml`: без флага
pnpm пытается трактовать пакет как часть корневого workspace.

### Артефакты LOWA

Артефакты сборки ZetaOffice лежат в репозитории — `experiments/lowa-spike/public/lowa/` — и
хранятся через Git LFS (`.gitattributes`): это те же байты, что отдаёт CDN, то есть сжатые
brotli, ~50 МБ вместо ~250 МБ распакованных. После клона нужен `git lfs` в `PATH` и один раз

```bash
git lfs pull   # иначе на месте артефактов останутся ~130-байтовые указатели
```

Зеркало `/lowa/*` отдаёт браузеру эти файлы. Каталог `/tmp/lowa-harness-cache`
(`LOWA_CACHE_DIR`) остался резервом: если копия в репозитории не приехала целиком (нет
`git lfs pull`), зеркало пишет об этом в лог и берёт те же байты с CDN.

Дальше в браузере: страница сама поднимает офис (~3 с при прогретом зеркале, ~22 с на холодную),
затем «Выбрать файл» → DOCX или XLSX → появляются ссылка «Скачать PDF» и предпросмотр в iframe.
Кнопка «Перечислить фильтры» дёргает UNO-перечисление фильтров и печатает счёт в лог.

Автоматическая проверка (сама поднимает свой dev-сервер, гоняет Playwright, сверяет страницы):

```bash
node experiments/lowa-spike/tools/harness-check.mjs
```

Ручки на окружении: `LOWA_BASE=https://cdn.zetaoffice.net/zetaoffice_latest/` (взять артефакты
прямо с CDN, без зеркала), `LOWA_SKIP_XLSX=1`, `LOWA_OUT` (куда складывать результат,
по умолчанию `/tmp/lowa-measure`), `LOWA_CACHE_DIR` (резервный кэш зеркала, по умолчанию
`/tmp/lowa-harness-cache`), `LOWA_PORT` (по умолчанию 5199).

## Из чего состоит

| Файл | Роль |
|---|---|
| `experiments/lowa-spike/vite.config.ts` | COOP/COEP на всех ответах + зеркало артефактов на `/lowa/*` из `public/lowa/` |
| `experiments/lowa-spike/index.html` | Минимальная страница: выбор файла, лог, ссылка и iframe |
| `experiments/lowa-spike/src/main.js` | Главный поток: собирает `Module`, получает порт к офису, возит байты |
| `experiments/lowa-spike/public/office-thread.js` | Сторона офиса: UNO, фильтры, конвертация (приезжает через `uno_scripts`) |
| `experiments/lowa-spike/tools/harness-check.mjs` | Приёмка: dev-сервер + Playwright + `pdfinfo` |

Поток данных: страница кладёт байты в memfs (`FS.writeFile`) → порт → офис вызывает
`loadComponentFromURL('file:///tmp/input.docx', '_default', 0, [Hidden, ReadOnly])` и
`storeToURL('file:///tmp/output.pdf', [FilterName, Overwrite])` → страница читает `/tmp/output.pdf`
из memfs и отдаёт Blob. Порт — тот самый MessageChannel из runtime-contract §1:
`Module.uno_main` резолвится на странице, `zetajs.mainPort` (= `Module.uno_mainPort`) — в офисе.

Протокол свой, с обеих сторон одна страница кода: запрос `{id, cmd}` → ответ `{id, ok, …}` либо
`{id, ok:false, error:{message, type, stack}}`. Таймауты поставлены на странице (5 мин на подъём,
5 мин на команду), потому что при внутренней ошибке офис молчит и не отвечает (известное
поведение примера ZetaOffice, `office_thread.js:69-72`). `FilterData` заполняется целиком
(`FilterName` + `Overwrite`), как и требует контракт §5.

## Что проверено фактом

Прогон `node experiments/lowa-spike/tools/harness-check.mjs`, Chromium 153 (полная сборка,
`channel: chromium`), фикстура `test-fixtures/СУС УТП_v2.0_31.10.25.docx`:

| Проверка | Результат |
|---|---|
| cross-origin isolation | `crossOriginIsolated === true` |
| Подъём офиса | 2.5 с при прогретом зеркале, 21.6 с при загрузке прямо с CDN |
| Перечисление фильтров | 237 имён через `FilterFactory` |
| `writer_pdf_Export` | найден, `DocumentService = com.sun.star.text.TextDocument` |
| `calc_pdf_Export` | найден, `DocumentService = com.sun.star.sheet.SpreadsheetDocument` |
| DOCX → PDF | **161 страница**, 4 791 564 Б, 4.5 с (из них UNO 4.6 с) |
| Эталон нативного LO | 161 страница (`/tmp/lowa-measure-native/sus-utp-native.pdf`) — совпадает |
| XLSX → PDF (Calc) | 120 страниц, 301 246 Б, 0.5 с |
| Ссылка «Скачать PDF» на странице | есть, `blob:`-URL с атрибутом `download` |
| Ошибка вместо зависания | файл вне memfs → `com.sun.star.lang.IllegalArgumentException: Unsupported URL <file:///tmp/нет-такого.docx>: "type detection failed"`, ответ за доли секунды |
| Мусор под именем `.docx` | ошибки нет: LO открывает его как текст и отдаёт PDF в 1 страницу |

Результаты: `/tmp/lowa-measure/harness.pdf`, `/tmp/lowa-measure/harness-xlsx.pdf`,
`/tmp/lowa-measure/harness.png`, машинный протокол — `/tmp/lowa-measure/harness-check.json`,
перечисление фильтров — `/tmp/lowa-measure/filters.json`.

Совпадение по числу страниц с нативным LibreOffice и по размеру PDF с публичным демо ZetaOffice
(тот же файл, 4 791 564 Б — байт в байт) — главное свидетельство, что харнесс конвертирует именно
той же сборкой и тем же фильтром, а не «чем-то похожим».

### Про перечисление фильтров

Контракт §5 требовал не доверять памяти, а получить имена из `FilterFactory`. Итог: из прокси-дерева
`zetajs.uno.com.sun.star` синглтон **не достаётся** — `css.document.FilterFactory` даёт `undefined`,
`context.getByName('/singletons/com.sun.star.document.FilterFactory')` тоже. Работает третья дорога:

```js
context.getServiceManager()
  .createInstanceWithContext('com.sun.star.document.FilterFactory', context)
  .getElementNames();   // 237 имён, среди них writer_pdf_Export и calc_pdf_Export
```

Свойства каждой pdf-записи (`getByName(name)` → `sequence<PropertyValue>`) читаются без ошибок:
именно из них взяты `DocumentService` для Writer и Calc. Оба запасных маршрута остались в коде
(`locateFilterFactory`) и пишут в диагностику, чем закончились, — если сборка сменится, это будет
видно в `filters.json`.

## Грабли, на которые харнесс уже наступил

- **`locateFile(path, prefix)` зовут с пустым префиксом.** XHR за `soffice.data.js.metadata` идёт
  через `Module.locateFile(name, "")`, поэтому `prefix ?? base` оставляет пустую строку и запрос
  уезжает в 404, а рантайм навсегда залипает в «still waiting on run dependencies: dependency:
  soffice.data.js.metadata». Лечится `prefix || base` — как в `zetaHelper.js:38`.
- **У прокси zeta.js методы интерфейсов уже на объекте.** `css.container.XNameAccess.query(obj)`
  не существует: `getElementNames()` вызывается прямо у результата. `queryInterface` нужен только
  там же, где в примере, — для проверки `XCloseable` перед закрытием модели.
- **`zeta.js` на CDN нет.** `https://cdn.zetaoffice.net/zetaoffice_latest/zeta.js` отдаёт 404:
  сборка везёт только `soffice.*`, а zeta.js демо раздаёт со своего сайта. Поэтому харнесс всегда
  берёт `zeta.js` с зеркала, а `?base=` меняет только источник `soffice.*`.
- **COOP/COEP обязателен, и это не формальность.** Без них страница не изолирована,
  `SharedArrayBuffer` недоступен, pthreads не поднимаются. Харнесс проверяет `crossOriginIsolated`
  и падает с внятным сообщением, а не зависает.
- **Артефакты чужие и большие.** Оригиналы: `soffice.js` 858 124 Б (br: 157 580), `soffice.wasm`
  161 667 499 Б (br: 36 279 250), `soffice.data` 99 520 604 Б (br: 15 891 013),
  `soffice.data.js.metadata` 215 180 Б, `zeta.js` 42 946 Б. Зеркало (`vite.config.ts`) отдаёт
  браузеру эти байты с тем же `content-encoding`, так что распаковывает их браузер, а не Node;
  страница при этом грузит всё со своего origin и CORS/CORP не участвуют. Байты берутся из
  репозиторной копии (`public/lowa/`, Git LFS), а не с CDN: кэш `LOWA_CACHE_DIR` — только резерв.
- **Сборка с UI.** CDN-сборка — Qt-вариант: в консоли видны `QObject::connect(QWindow, QtFrame):
  invalid nullptr parameter` и `QRect(0,0 0x0)`. На конвертацию не влияет, но canvas в разметке
  обязан быть (в `zetaHelper.js:20` он ищется без проверки на null), поэтому в `index.html` он есть,
  скрытый.
- **LibreOffice всеяден.** Битый файл с расширением `.docx` ошибки не даёт: тип определяется по
  содержимому, текст открывается как документ Writer, на выходе PDF в одну страницу. Отрицательную
  проверку протокола поэтому делают не «мусорным файлом», а путём вне memfs.

## Ограничения харнесса

Это спайк, а не продукт, и он намеренно стоит на чужих внешних артефактах:

- **Зависимость от внешнего CDN** (`cdn.zetaoffice.net/zetaoffice_latest/`, сборка ZetaOffice 24.2,
  `Producer: ZetaOffice 24.2` в готовом PDF) и от сайта демо для `zeta.js` (MIT). Своей сборки LOWA
  у спайка нет — это слайс B1 (`build/`), и подмена источника на свою сборку делается через
  `?base=` без правок кода.
- **Одна сессия офиса, один документ за раз.** Модель предыдущей конвертации закрывается перед
  следующей; параллельные конвертации не поддержаны (они и не могут идти параллельно — UNO-вызов
  блокирует pthread офиса). Изоляции сессий нет.
- **Нет прогресса и отмены.** Про долгую конвертацию страница знает только «идёт»; прервать её
  нельзя, можно лишь дождаться таймаута.
- **Память и производительность не измеряются.** Этому посвящён отдельный слайс
  (`tools/measure-envelope.mjs`); харнесс отвечает только за «PDF получен».
- **Проверка качества PDF — не здесь.** Сверяется число страниц (`pdfinfo`) и размер; попиксельное
  сравнение с эталоном ведёт слайс сравнения (`verify/`).
- **`node_modules` в `experiments/lowa-spike/`** — локальная установка vite (в workspace
  эксперимент не входит). Артефакты LOWA лежат в репозитории через Git LFS, поэтому клону нужен
  `git lfs pull`: без него зеркало уходит в резервный кэш и тянет те же файлы с CDN (~50 МБ).

## Отклонения от постановки

- Добавлена проверка XLSX-пути (`calc_pdf_Export`): приёмка требовала только DOCX, но харнесс по
  заданию обязан уметь оба формата, а доказательство «Calc работает» стоит один лишний прогон.
- Добавлено зеркало артефактов в `vite.config.ts`: без него каждый прогон тянет ~50 МБ с CDN
  (~22 с только на сеть), что делает итерации и приёмку дорогими. Источник артефактов от этого не
  меняется — те же байты CDN, только один раз. Позже байты переехали в репозиторий (Git LFS), и
  зеркало берёт их оттуда; скачивание осталось только резервом.
- Скриншот снимается в полной сборке Chromium (`channel: chromium`): у headless shell нет
  встроенного просмотрщика PDF, и iframe на снимке остаётся пустым.
