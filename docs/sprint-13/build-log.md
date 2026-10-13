# Сборка LOWA: процедура, пины, риски

Слайс B1 плана спринта 13 (`docs/sprint-13/plan.md`, §6). Результат слайса —
воспроизводимая процедура сборки headless writer-only LibreOffice core в WASM:
`experiments/lowa-spike/build/build-lowa.sh` и `experiments/lowa-spike/build/README.md`.

## 1. Статус прогона

**Не выполнялся** (нет ресурсов на рабочей машине; план — арендованная VM).

Причина та же, что записана в плане спринта, §5: 18 ГБ RAM против ≥ 64 ГБ,
которых требует линковка (цитата из README апстрима — в §2.2), 32 ГБ свободного
диска против десятков гигабайт промежуточных артефактов, Docker-демон не
запущен, emsdk и ccache отсутствуют. Никакие числа (время, размеры, sha256) в этом документе не
выдуманы: ниже — только факты из исходников апстрима со ссылками, оценки
помечены словом «оценка», а результаты прогона появляются здесь после аренды VM.

Что вместо прогона сделано и проверено:

- источники (`static/README.wasm.md`, `configure.ac`, `static/CustomTarget_emscripten_fs_image.mk`,
  `solenv/gbuild/platform/EMSCRIPTEN_INTEL_GCC.mk`, `desktop/Executable_soffice_bin.mk`,
  `distro-configs/LibreOfficeWASM32.conf`, `Makefile.in`, `Makefile.fetch`) прочитаны
  на **двух ревизиях** — master и пинованном теге `libreoffice-26.8.1.1`; расхождений
  по значимым пунктам нет (README в обеих ревизиях — 461 строка, ключевые места
  совпадают, у `EMSCRIPTEN_MIN_*` и дефолта PROXY_TO_PTHREAD — те же значения);
- скрипт проверен `bash -n`;
- проверена арифметика предусловий и сборки флагов: прогон с подменённым `uname`
  (даёт отказ «не хватает инструментов» списком, отказ по диску с понятным текстом,
  запись лога) и прогон блока сборки флагов на всех комбинациях
  `PROXY_TO_PTHREAD`/`JSPI`/`DISABLE_LTO`/`EXTRA_CONFIGURE_FLAGS`;
- на этом тесте нашлась и исправлена ошибка, которая обрушила бы прогон на VM:
  ветвление `[ … ] && FLAGS+=(…)` под `set -e` возвращает ненулевой код на ложном
  условии и завершает скрипт (шаг autogen падал бы всегда);
- проверен round-trip патча флагов линковки: файл апстрима после наложения и снятия
  блока побайтово равен чекауту.

## 2. Что известно до прогона

### 2.1. Тулчейн и целевые артефакты

- Сборка headless описана в `static/README.wasm.md`, раздел «Building headless
  LibreOffice as WASM for use in another product». Qt для неё не нужен:
  «No Qt needed. You don't need any dependencies other than those that normally
  are downloaded and compiled when building LibreOffice».
- Рекомендованный апстримом `autogen.input` (оттуда же): `--disable-debug`,
  `--enable-sal-log`, `--disable-crashdump`, `--host=wasm32-local-emscripten`,
  `--disable-gui`, `--with-wasm-module=writer`, `--with-package-format=emscripten`.
  Скрипт берёт ровно этот набор.
- Distro-конфиг `LibreOfficeWASM32` для headless **не подходит**: в
  `distro-configs/LibreOfficeWASM32.conf` лежит `--enable-qt5` (плюс `--disable-gen`,
  `--disable-scripting`, `--with-package-format=emscripten`) — это GUI-ветка. README
  про него пишет «it just provides --host=wasm32-local-emscripten», что неточно.
- Артефакт — три файла, а не один: `soffice.wasm`, `soffice.js` и `soffice.data`.
  README: `soffice.wasm` собирается «just because of how the makefilery has been set
  up», а нужен `soffice.data` — «the in-memory file system needed by the LibreOffice
  Technology core code during run-time»; он появляется как побочный продукт сборки
  `soffice.wasm`. `soffice.js` — загрузчик, в него вшиты pre-js апстрима
  (`desktop/Executable_soffice_bin.mk`: `static/emscripten/environment.js` и
  `soffice.data.js.link` попадают в линк как `--pre-js`).
- Для ориентации по величинам (не наши числа): в плане спринта, §4, собраны
  публичные сборки — GUI-демо TDF: `soffice.wasm` 155,9 МБ, `soffice.data` 86,7 МБ;
  оптимизированная сборка ZetaOffice: 36,3 МБ и 15,9 МБ brotli. Разброс семикратный,
  поэтому чужая сборка не заменяет свою; writer-only headless должен быть заметно
  меньше GUI-демо, но насколько — вопрос прогона.

### 2.2. RAM, линковка, `emscripten-finalize`

- README: «Linking takes quite a long time, because emscripten-finalize rewrites the
  whole WASM files with some options. This way the LO WASM possibly needs 64GB RAM».
  Там же рецепт ускорения — `-s WASM_BIGINT=1`, `ASSERTIONS=1` и `-g3`, чтобы
  `modify_wasm` не срабатывал, — и оговорка: «This is just needed for Qt examples,
  as LO already uses the correct flags!».
- Оговорка подтверждается кодом: в `solenv/gbuild/platform/EMSCRIPTEN_INTEL_GCC.mk`
  (тег 26.8.1.1) стоит `gb_EMSCRIPTEN_LDFLAGS += … -s WASM_BIGINT=1 … -s ASSERTIONS=1`
  с комментарием «To keep the link time (and memory) down, prevent all rewriting
  options from wasm-emscripten-finalize». То есть совет README по флагам для LO уже
  выполнен, и добавлять их вручную не нужно (в скрипте `EXTRA_LDFLAGS` по умолчанию пуст).
- Там же, строками ниже: «We need at least code elimination, otherwise linking OOMs
  even with 64GB» — то есть порог 64 ГБ относится к сборке с оптимизацией; `-O3` и
  `gb_COMPILEROPTFLAGS` включены в этой же платформе. Отсюда `DISABLE_LTO=1` в скрипте
  как аварийный рычаг, а не как режим по умолчанию.
- Начальный размер памяти в рантайме задаёт не сборщик, а апстрим: `-s TOTAL_MEMORY=1GB`,
  `-sSTACK_SIZE=131072`, `-sDEFAULT_PTHREAD_STACK_SIZE=65536` (там же).
- `static/emscripten/environment.js` (pre-js, попадает в `soffice.js`) выставляет
  `ENV.MAX_CONCURRENCY = '4'` и `Module.ignoreApplicationExit = true` — это и есть
  потолок параллелизма воркеров рантайма, не настройка сборки.

### 2.3. PROXY_TO_PTHREAD и JSPI — развилка, а не догма

- На master и на пинованном теге `PROXY_TO_PTHREAD` включён по умолчанию:
  `configure.ac`, `AC_ARG_ENABLE(emscripten-proxy-to-pthread, [--disable-emscripten-proxy-to-pthread], …, enable_emscripten_proxy_to_pthread=yes)`
  (master:2353–2360; тег 26.8.1.1:2339–2342). Сам флаг уходит на линк в
  `desktop/Executable_soffice_bin.mk`: `-sPROXY_TO_PTHREAD=1` при включённой опции.
- Выключение меняет не только рантайм, но и линк: в `EMSCRIPTEN_INTEL_GCC.mk` ветка
  `ifeq ($(ENABLE_EMSCRIPTEN_PROXY_TO_PTHREAD),)` добавляет `-sPTHREAD_POOL_SIZE=7`.
  То есть «выключено» — это не «как было раньше», а другой рантайм-контракт:
  пул из 7 предсозданных воркеров вместо проксирования главного потока.
- README описывает PROXY_TO_PTHREAD как решение внутри *Qt*-истории («-sPROXY_TO_PTHREAD
  would move the Qt5 event loop off the JS main thread… But Qt5 just doesn't appear to
  be prepared…»), и там же — целый раздел про голодание пула воркеров из-за
  вложенных циклов событий. Для headless-ядра, которое крутит вызывающий продукт
  (COWASM-подобный сценарий), это иная ситуация, поэтому выбор делается замером H2/H3,
  а не по умолчанию. В скрипте — переменная `PROXY_TO_PTHREAD` (1/0).
- `--enable-emscripten-jspi` — экспериментальная опция (`configure.ac`, «Experimentally
  enable use of the Emscripten -sJSPI feature»). В линк она добавляет `-sJSPI` и
  `-sJSPI_EXPORTS=_emscripten_check_mailbox,_ZN10emscripten8internal13MethodInvoker…qstdweb13EventListener…`
  — экспорт с символом Qt-шного `qstdweb::EventListener`. Для headless-сборки без Qt
  это подозрительно (символа может не быть, а `-sERROR_ON_UNDEFINED_SYMBOLS=1` в тех
  же LDFLAGS превращает это в ошибку линковки). Отсюда `JSPI=0` по умолчанию.
- `--enable-wasm-strip` в headless-наборе не нужен: он включается самим хостом
  (`configure.ac`, ветка `emscripten` case: `enable_wasm_strip=yes`), а `--with-wasm-module`
  дальше решает, какие модули оставить (при `writer` снимается `ENABLE_WASM_STRIP_WRITER`,
  ставятся `_CALC`, `_BASIC_DRAW_MATH_IMPRESS`, `_ACCESSIBILITY`; заодно отключаются
  импортёры Visio/CDR/Publisher и прочие, нужные draw/impress).

### 2.4. Что попадает в `soffice.data`

- Образ собирает `static/CustomTarget_emscripten_fs_image.mk` вызовом
  `$(EMSDK_FILE_PACKAGER) soffice.data --preload <список> --js-output=soffice.data.js
  --separate-metadata`, из `$(BUILDDIR)`; путь результата —
  `workdir/CustomTarget/static/emscripten_fs_image/`. Сам packager ищется как
  `$(em-config EMSCRIPTEN_ROOT)/tools/file_packager` — то есть зависит от активированного emsdk.
- В образ идут не только ресурсы writer: безусловными блоками перечислены
  `cui/ui/*.ui` (десятки файлов), `sfx/ui/*.ui`, `desktop/ui/*.ui`, `filter/ui/*.ui`,
  `services.rdb`, файлы `soffice.cfg` — то есть GUI-ресурсы даже в headless-сборке.
  Условные блоки (`ENABLE_WASM_STRIP_WRITER`, `_CALC`, `_BASIC_DRAW_MATH_IMPRESS`,
  `_CHART`) вырезают только ресурсы модулей; на `cui`/`sfx` они не влияют.
  Влияет это прямо на H1 (размер), и «лишние» ресурсы в образе — тема отдельного
  осознанного шага в B1/B3, а не побочный эффект опций.
- Шрифты: при `WITH_FONTS=TRUE` в образ добавляется цель `ooo_fonts`, плюс всегда
  идут списки `liblangtag_data` и `fontconfig_data`. Для конвертера это ядро
  функциональности (метрики текста), вырезать нельзя.

### 2.5. Требования к хосту и сети

- README: «The build itself is a cross build and the cross-toolset just depends on a
  minimal toolset (gcc, libc-dev, flex, bison); all else is built from source» — то
  есть тяжёлых системных зависимостей нет, но autogen/configure требуют обычного
  набора autotools и python3 (`configure.ac` проверяет, среди прочего, `emconfigure`
  в `PATH`, `EMMAKEN_JUST_CONFIGURE` и наличие `file_packager`).
- Внешние исходники LO качает сам: в `Makefile.in` цель `fetch: download` зависит от
  `Makefile.fetch`, а `build` — от `bootstrap fetch`; локация задаётся
  `--with-external-tar`. Поэтому в скрипте есть отдельный шаг `make bootstrap fetch`
  до многочасовой компиляции: сетевые отказы дешевле ловить там.
- README честно предупреждает про тулчейн: «emsdk install/activate is _not_ stable
  over time, as e.g. nodejs versions evolve» — отсюда пин версии и проверка sha
  после клона.

## 3. Выбранные пины и почему

| Что | Пин | Почему |
|---|---|---|
| emsdk | `4.0.10` (`62a853cd3b3134398ce85cde8bb5cbb2ef0194cb`) | README объявляет эту версию текущей рабочей («We're using Qt 5.15.2 with Emscripten 4.0.10») и предупреждает, что другая версия «often causes arbitrary problems»; сам emsdk нестабилен между версиями, поэтому тег, а не master |
| LibreOffice core | `libreoffice-26.8.1.1` (`f3b61cf45f5198919f352f5163cd1dc2c000ba28`) | ближайший стабильный тег вместо master: прогон должен повторяться через день, а master уезжает; тег и sha фиксируются оба — sha проверяется после клона (тег могут переставить) |
| Минимальная версия emscripten | ≥ `3.1.46` | `configure.ac`: `EMSCRIPTEN_MIN_MAJOR/MINOR/TINY = 3/1/46` (значения совпадают на master и на теге) |
| Набор флагов configure | из headless-раздела `README.wasm.md` | апстрим называет его рабочим именно для «use in another product»; distro-конфиг для этого не годится (в нём Qt5) |
| Способ сборки | in-tree (`$LO_SRC/workdir`) | srcdir ≠ builddir поддержан, но добавляет второй путь, который придётся держать одинаковым между прогонами ради сверки sha256 |

**Отдельно про версию emscripten.** README в headless-разделе говорит: «For building
LO core for use in COWASM, it is known to work to use Emscripten 3.1.30». Этот совет
устарел и противоречит `configure.ac` того же дерева: минимум — 3.1.46, то есть на
3.1.30 configure упадёт с `no, found 3.1.30`. Поэтому пин — 4.0.10 (текущая версия
README), а 3.1.30 годится только как нижняя граница для бисекции «где сломалось»,
и то начиная с 3.1.46.

**Как менять пин.** Для другого тега LO: `git ls-remote --tags --refs <url> '<тег>'`
даёт sha, его и надо положить в `LO_COMMIT` рядом с `LO_TAG` — скрипт сверяет их
после клона и отказывается собирать при расхождении. Если прогон покажет, что
комбинация не собирается именно из-за тулчейна, порядок действий — в разделе 4,
строка про несовпадение версии emscripten.

## 4. Что может сломаться

| Риск | Признак | Что делать |
|---|---|---|
| OOM на линковке | процесс убит (`Killed`), лог обрывается на линковке `soffice_bin`; в `dmesg` — oom-killer | уменьшить `JOBS` (компиляция вытесняет линковщик), затем `DISABLE_LTO=1`, в пределе — машина с большей RAM. По README линковке нужно «possibly 64GB» |
| Диск кончился посреди сборки | `No space left on device`, обычно на `workdir/` | ≥ 200 ГБ SSD на старте; при повторе — удалить `workdir/instdir/LinkTarget` неудачного прогона. Порог скрипта (120 ГБ) — оценка, уточняется прогоном |
| Версия emscripten ниже требуемой | `configure: error: … no, found 3.1.x` | поднять `EMSDK_VERSION` до ≥ 3.1.46; по умолчанию стоит 4.0.10 |
| Тулчейн не тот, что ждали (emsdk «нестабилен между версиями») | сборка падает в чужом месте (nodejs/llvm), либо второй прогон с того же пина даёт иной артефакт | проверить `EMSDK_COMMIT` в манифесте; при расхождении артефактов зафиксировать факт в этом файле и проверить соседнюю версию (шаг вверх/вниз по тегам emsdk), не меняя LO |
| Тег апстрима переставили | «клонированный LO не совпал с пином: получено …, ожидалось …» | снять актуальный sha через `git ls-remote`, осознанно обновить `LO_COMMIT`, записать в этот файл |
| Сеть до tarballs недоступна | падает `make bootstrap fetch` (dev-www.libreoffice.org, download.documentfoundation.org) | повторить: цель идемпотентна и не качает скачанное заново; при устойчивой блокировке — зеркало через `--with-external-tar` |
| `JSPI` на headless не линкуется | `undefined symbol` при `-sJSPI_EXPORTS` (в списке экспортов есть символ `qstdweb::EventListener`, а Qt в headless нет) | оставить `JSPI=0`; если включать — смотреть, ругается ли линковщик, и не тянуть Qt ради одного экспорта |
| PROXY_TO_PTHREAD даёт не тот рантайм-контракт | харнесс B2 висит или теряет кадры, H3 «нет» | прогнать оба варианта (`PROXY_TO_PTHREAD=1` и `0`) и сравнить; помнить, что выключение добавляет `-sPTHREAD_POOL_SIZE=7` и меняет схему воркеров, а не просто «убирает флаг» |
| Артефактов нет там, где ждём | `не найден soffice.* ни в одном из каталогов` | смотреть хвост лога: скорее всего сборка не дошла до линк-таргета; каталоги-кандидаты перечислены в сообщении |
| Часть артефактов от прошлого прогона | предупреждение «файлы взяты из разных каталогов» | удалить `artifacts/` и `instidir`/`instdir`, пересобрать |
| Грязное дерево исходников | в манифесте `git status --porcelain` показывает лишние файлы | ожидаемы только `autogen.input` и (при `EXTRA_LDFLAGS`) платформенный `.mk`; всё прочее — признак постороннего вмешательства, артефакт неизвестного происхождения |
| Сборка идёт от root | `Building LibreOffice as root is a very bad idea` (цель `check-if-root` в `Makefile.in`) | работать от обычного пользователя; исключение LO делает только для распознаваемого контейнера. Скрипт предупреждает об этом в предусловиях, не дожидаясь `make` |

## 5. Проверка воспроизводимости

Цель из плана (B1): «два прогона с одного пина дают тот же артефакт». Процедура —
механическая, опирается на `MANIFEST.txt`, который скрипт пишет рядом с артефактами.

1. **Прогон 1.** На VM A: `bash build-lowa.sh`. Сохранить `$BUILD_ROOT/artifacts`
   (три файла + `MANIFEST.txt`) и сам лог `build-*.log` вне VM.
2. **Прогон 2.** На **чистой** VM B (та же ОС и разрядность, столько же vCPU или
   больше), **с теми же `BUILD_ROOT` и `LO_SRC`**, теми же значениями переменных:
   тот же `bash build-lowa.sh`.
3. **Сверка.** Сравнить три sha256 из `MANIFEST.txt` (и `git HEAD`, и версию emsdk).
   Совпали — воспроизводимость по артефакту есть, это и есть критерий B1.
4. **Если не совпали** — разбираться по порядку, не перескакивая:
   - разные пути (`BUILD_ROOT`/`LO_SRC`) или разный `nproc` — перезапустить с
     одинаковыми (пути попадают в DWARF и в метаданные ФС);
   - разные версии emsdk/LO в манифесте — это не «нестабильность сборки», а другой
     вход; сверить пины;
   - дерево исходников грязное сверх ожидаемого (см. `git status --porcelain` в
     манифесте) — прогнать на чистом клоне;
   - если после этого sha расходятся, а размеры совпадают — зафиксировать в этом
     файле: «байтовой воспроизводимости нет, расхождение такое-то», и перейти к
     функциональному критерию: одинаковый результат конвертации фикстуры в харнессе B2
     (SSIM между двумя прогонами). Байтовое совпадение — цель, а не доказанный факт:
     на него влияет в том числе то, что emsdk тянет бинарь emscripten-releases из
     сети, и перевыпуск артефакта апстримом его ломает (это и есть риск из плана).

Стоимость поверки — второй прогон целиком, то есть ещё один «полдня аренды»; в плане
он заложен, и оба прогона должны идти подряд, пока VM и версия emsdk доступны.

## 6. Осталось непроверенным

Ничего из перечисленного не выдаётся за проверенное — это список вопросов, на
которые ответит прогон:

- **Собирается ли выбранная комбинация вообще**: emsdk 4.0.10 + тег
  `libreoffice-26.8.1.1` + `--with-wasm-module=writer` + `--disable-gui`. Статическая
  сверка по источникам противоречий не нашла, но факта сборки нет.
- **Где именно оказываются артефакты** после `--with-package-format=emscripten`.
  В README назван `workdir/installation/LibreOffice/emscripten` (там же — деплой
  `tar` из `instdir/program/soffice.*`), а `soffice.data` вообще рождается в
  `workdir/CustomTarget/static/emscripten_fs_image/`. Скрипт ищет по имени в трёх
  каталогах и предупреждает при смеси — но это обход, а не знание.
- **Можно ли сузить цель `make`** до линк-таргета `soffice_bin` (сэкономило бы часы).
  В README такой цели нет; угаданная цель рискует дать неполный `instdir`, поэтому
  скрипт собирает полным `make` — TODO на уточнение по факту.
- **Реальные числа**: время компиляции и линковки, пик RSS, размеры трёх файлов,
  достаточность 120/200 ГБ диска, реальный список файлов в `soffice.data`
  (ожидается по `.metadata` рядом с образом — сколько именно осталось GUI-ресурсов).
- **Хватает ли `-sPTHREAD_POOL_SIZE=7`** (автоматическая подстановка при выключенном
  PROXY_TO_PTHREAD) для writer-only сценария: README описывает голодание пула на
  вложенных циклах событий, и в headless это надо мерить, а не предполагать.
- **JSPI на headless**: подозрение на неразрешимый экспорт `qstdweb`-символа не
  проверено — опция по умолчанию выключена, включать её имеет смысл только если
  замеры H2/H3 упрутся в потоки.
- **Байтовая воспроизводимость** (раздел 5) — гипотеза, а не факт.
