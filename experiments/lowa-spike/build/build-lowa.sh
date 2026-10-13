#!/usr/bin/env bash
#
# Сборка headless LibreOffice core (LOWA) в WASM, writer-only, на Linux-VM.
#
# Почему скрипт, а не ручные шаги: арендованная VM живёт часами, и повторный
# прогон (проверка воспроизводимости, слайс B1 плана спринта 13) должен быть
# механическим. Всё, что скрипт качает и собирает, лежит в $BUILD_ROOT; в
# репозиторий не попадает ничего, кроме этого файла.
#
# Источник фактов о сборке — LibreOffice/core, static/README.wasm.md, раздел
# "Building headless LibreOffice as WASM for use in another product", плюс
# configure.ac и solenv/gbuild/platform/EMSCRIPTEN_INTEL_GCC.mk пинованного
# тега. Ссылки на строки — в комментариях ниже и в docs/sprint-13/build-log.md.
#
# Запуск: см. README.md рядом с этим файлом.

set -euo pipefail

# ---------------------------------------------------------------------------
# Пины
# ---------------------------------------------------------------------------

# emsdk. configure.ac пинованного тега требует минимум 3.1.46
# (EMSCRIPTEN_MIN_MAJOR/MINOR/TINY), поэтому рекомендованный в README для
# headless «3.1.30» уже не проходит configure — ниже 3.1.46 не опускаться.
# 4.0.10 — версия, которую README объявляет текущей рабочей ("We're using
# Qt 5.15.2 with Emscripten 4.0.10"); README же признаёт, что emsdk
# нестабилен между версиями, поэтому пин обязателен.
# Тег 4.0.10 → commit 62a853cd3b3134398ce85cde8bb5cbb2ef0194cb.
EMSDK_VERSION="${EMSDK_VERSION:-4.0.10}"
EMSDK_COMMIT="${EMSDK_COMMIT:-62a853cd3b3134398ce85cde8bb5cbb2ef0194cb}"
EMSDK_GIT_URL="${EMSDK_GIT_URL:-https://github.com/emscripten-core/emsdk.git}"

# LibreOffice core: ближайший стабильный тег, а не master — иначе прогон
# повторить нельзя (master уезжает). Открытые ветки 25.8/26.8 тянутся из
# одного репозитория, поэтому фиксируем и тег, и sha: тег читаем человеку,
# sha проверяем после клона (тег могут переставить).
# Тег libreoffice-26.8.1.1 → commit f3b61cf45f5198919f352f5163cd1dc2c000ba28.
LO_TAG="${LO_TAG:-libreoffice-26.8.1.1}"
LO_COMMIT="${LO_COMMIT:-f3b61cf45f5198919f352f5163cd1dc2c000ba28}"
LO_GIT_URL="${LO_GIT_URL:-https://github.com/LibreOffice/core.git}"

# ---------------------------------------------------------------------------
# Что собираем
# ---------------------------------------------------------------------------

# README (headless-раздел) называет writer минимумом для конвертера в PDF.
WASM_MODULE="${WASM_MODULE:-writer}"

# Развилка PROXY_TO_PTHREAD. На master/proxy включён по умолчанию
# (configure.ac: -sPROXY_TO_PTHREAD=1 уходит на линк soffice_bin), а выключение
# меняет поведение рантайма: вместо проксированного главного потока
# подставляется -sPTHREAD_POOL_SIZE=7
# (solenv/gbuild/platform/EMSCRIPTEN_INTEL_GCC.mk, ветка
# `ifeq ($(ENABLE_EMSCRIPTEN_PROXY_TO_PTHREAD),)`). Это предмет замера H2/H3,
# поэтому переключатель, а не константа: 1 = дефолт апстрима, 0 = отключить.
PROXY_TO_PTHREAD="${PROXY_TO_PTHREAD:-1}"

# Экспериментальный JSPI (configure.ac --enable-emscripten-jspi). Включение
# добавляет в линк -sJSPI и -sJSPI_EXPORTS со списком символов, который
# упоминает qstdweb::EventListener — то есть Qt-шный. Есть риск, что для
# headless-сборки без Qt экспорт не разрешится; проверить первым делом.
JSPI="${JSPI:-0}"

# Дополнительные флаги линковки. По умолчанию пусто, и это осознанно: совет
# README «добавить -s WASM_BIGINT=1, ASSERTIONS=1, -g3, чтобы не переписывать
# wasm на emscripten-finalize» уже выполнен в самом LO — и то, и другое стоит в
# gb_EMSCRIPTEN_LDFLAGS (EMSCRIPTEN_INTEL_GCC.mk), и там же комментарий, что
# именно так гасится перезапись. Переменная нужна для замеров, а не для сборки
# «по умолчанию».
EXTRA_LDFLAGS="${EXTRA_LDFLAGS:-}"

# LTO у LO включён для оптимизированных сборок и сильно поднимает пик памяти на
# линковке (в EMSCRIPTEN_INTEL_GCC.mk прямо написано: без элиминации кода линк
# падает по OOM даже с 64 ГБ). DISABLE_LTO=1 — аварийный рычаг на случай OOM,
# ценой размера wasm; для чистого прогона держать 0.
DISABLE_LTO="${DISABLE_LTO:-0}"

# ccache полезен, если прогонов несколько на одной VM.
ENABLE_CCACHE="${ENABLE_CCACHE:-0}"

# Прочие флаги.configure — для разовых экспериментов, в манифест попадут.
EXTRA_CONFIGURE_FLAGS="${EXTRA_CONFIGURE_FLAGS:-}"

# Параллелизм. Значение по умолчанию подставляется позже, после проверки
# инструментов: вызов nproc до неё уронил бы скрипт без внятного сообщения.
JOBS="${JOBS:-}"

# ---------------------------------------------------------------------------
# Предусловия к машине
# ---------------------------------------------------------------------------

# README: «the LO WASM possibly needs 64GB RAM» — это про линковку, а не про
# компиляцию. Меньше 64 ГБ пробовать можно (ALLOW_UNDERSPEC=1), но результат
# неизвестен, и по умолчанию скрипт отказывается: дешевле арендовать больше,
# чем потерять час сборки на OOM.
MIN_RAM_GB="${MIN_RAM_GB:-64}"
# Диска в README не нормировано; оценка снизу — из практики сборки LO
# (исходники + workdir + instdir + tarballs). Уточняется первым прогоном.
MIN_DISK_GB="${MIN_DISK_GB:-120}"
ALLOW_UNDERSPEC="${ALLOW_UNDERSPEC:-0}"

# ---------------------------------------------------------------------------
# Пути
# ---------------------------------------------------------------------------

BUILD_ROOT="${BUILD_ROOT:-$HOME/lowa-build}"
EMSDK_DIR="$BUILD_ROOT/emsdk"
LO_SRC="$BUILD_ROOT/core"
# Tarballs держим вне workdir и передаём через --with-external-tar: повторный
# прогон на той же VM не должен заново тянуть сотни мегабайт с
# dev-www.libreoffice.org.
TARBALLS="$BUILD_ROOT/tarballs"
ARTIFACTS="$BUILD_ROOT/artifacts"
BUILD_LOG="$BUILD_ROOT/build-$(date -u +%Y%m%dT%H%M%SZ).log"

# ---------------------------------------------------------------------------
# Утилиты вывода
# ---------------------------------------------------------------------------

say()  { printf '\n=== %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf 'ПРЕДУПРЕЖДЕНИЕ: %s\n' "$*" >&2; }
die()  { printf 'ОШИБКА: %s\n' "$*" >&2; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------------------
# Шаг 0. Предусловия машины
# ---------------------------------------------------------------------------

[ "$(uname -s)" = "Linux" ] || die "скрипт рассчитан на Linux (обнаружено: $(uname -s))."

mkdir -p "$BUILD_ROOT" "$TARBALLS" "$ARTIFACTS"

# Лог пишем с самого начала: прогон на чужой VM должен оставлять след, иначе
# отчёт о сборке будет пересказом памяти.
exec > >(tee -a "$BUILD_LOG") 2>&1
trap 'code=$?; if [ "$code" -ne 0 ]; then printf "\nПрогон упал (код %s). Полный лог: %s\n" "$code" "$BUILD_LOG" >&2; fi' EXIT

say "Шаг 0. Проверка предусловий"
info "BUILD_ROOT=$BUILD_ROOT"
info "лог: $BUILD_LOG"

# Минимальный набор инструментов хоста. README: «the cross-toolset just depends
# on a minimal toolset (gcc, libc-dev, flex, bison); all else is built from
# source». Остальное в списке — то, без чего не отработает autogen.sh
# (autoconf/automake/perl/python3/pkg-config) или сам скрипт (curl, sha256sum).
REQUIRED_TOOLS=(git make gcc g++ flex bison autoconf automake pkg-config python3 perl curl sha256sum nproc unzip zip patch)
MISSING=()
for tool in "${REQUIRED_TOOLS[@]}"; do
  have "$tool" || MISSING+=("$tool")
done
if [ "${#MISSING[@]}" -ne 0 ]; then
  printf 'ОШИБКА: не хватает инструментов: %s\n' "${MISSING[*]}" >&2
  cat >&2 <<'EOF'
Установите их средствами дистрибутива, например (Debian/Ubuntu):
  apt-get update && apt-get install -y build-essential git curl unzip zip patch \
      flex bison autoconf automake pkg-config python3 perl
EOF
  exit 1
fi
info "инструменты: на месте (${#REQUIRED_TOOLS[@]} шт.)"

[ -n "$JOBS" ] || JOBS="$(nproc)"

mem_kb="$(awk '/^MemTotal:/ {print $2}' /proc/meminfo 2>/dev/null || true)"
if [ -n "${mem_kb:-}" ]; then
  mem_gb=$(( mem_kb / 1024 / 1024 ))
  info "RAM: ${mem_gb} ГБ, vCPU: $JOBS"
  if [ "$mem_gb" -lt "$MIN_RAM_GB" ]; then
    if [ "$ALLOW_UNDERSPEC" = "1" ]; then
      warn "RAM ${mem_gb} ГБ < ${MIN_RAM_GB} ГБ — продолжаем по ALLOW_UNDERSPEC=1."
    else
      die "RAM ${mem_gb} ГБ < ${MIN_RAM_GB} ГБ. Линковка оптимизированной сборки LO может упасть по OOM.
      Возьмите машину крупнее либо перезапустите с ALLOW_UNDERSPEC=1 (на свой риск)."
    fi
  fi
else
  warn "/proc/meminfo недоступен — проверка RAM пропущена."
fi

# LO сам отказывается собираться под root (Makefile.in: цель check-if-root,
# «Building LibreOffice as root is a very bad idea»); исключение — контейнер,
# который она распознаёт через systemd-detect-virt. Арендованные VM обычно
# выдают root, поэтому предупреждаем здесь, а не через час на make.
if [ "$(id -u)" = "0" ]; then
  warn "скрипт запущен от root: LO откажется собираться (кроме распознаваемого контейнера).
      Заведите обычного пользователя и запустите ещё раз: adduser --disabled-password builder && su - builder"
fi

disk_gb="$(df -Pk "$BUILD_ROOT" | awk 'NR==2 {printf "%d", $4/1024/1024}')"
[ -n "$disk_gb" ] || die "не удалось определить свободное место на $BUILD_ROOT (df ничего не вернул)."
info "свободно на $BUILD_ROOT: ${disk_gb} ГБ"
if [ "$disk_gb" -lt "$MIN_DISK_GB" ]; then
  if [ "$ALLOW_UNDERSPEC" = "1" ]; then
    warn "диск ${disk_gb} ГБ < ${MIN_DISK_GB} ГБ — продолжаем по ALLOW_UNDERSPEC=1."
  else
    die "свободно ${disk_gb} ГБ < ${MIN_DISK_GB} ГБ. Исходники, workdir и tarballs LO занимают десятки гигабайт;
      место кончится посреди сборки. Перезапустите с ALLOW_UNDERSPEC=1, если уверены в оценке."
  fi
fi

# ---------------------------------------------------------------------------
# Шаг 1. emsdk
# ---------------------------------------------------------------------------

say "Шаг 1. emsdk $EMSDK_VERSION"

if [ -d "$EMSDK_DIR/.git" ]; then
  current_commit="$(git -C "$EMSDK_DIR" rev-parse HEAD)"
  if [ "$current_commit" = "$EMSDK_COMMIT" ]; then
    info "emsdk уже на нужном коммите — установка/активация по необходимости"
  else
    die "в $EMSDK_DIR другой emsdk ($current_commit, ожидался $EMSDK_COMMIT).
      Удалите каталог и перезапустите, либо задайте другой EMSDK_DIR через BUILD_ROOT."
  fi
else
  info "клонирую emsdk (shallow, тег $EMSDK_VERSION)"
  git clone --depth 1 --branch "$EMSDK_VERSION" "$EMSDK_GIT_URL" "$EMSDK_DIR"
  current_commit="$(git -C "$EMSDK_DIR" rev-parse HEAD)"
  [ "$current_commit" = "$EMSDK_COMMIT" ] || die "клонированный emsdk не совпал с пином:
      получено $current_commit, ожидалось $EMSDK_COMMIT (тег переставили?)."
fi

# emsdk install качает SDK (~1-2 ГБ) и не сообщает «уже установлено» надёжно,
# поэтому отметка о своей работе — свой stamp.
stamp="$EMSDK_DIR/.lowa-installed-$EMSDK_VERSION"
if [ -f "$stamp" ]; then
  info "emsdk $EMSDK_VERSION уже устанавливался — пропускаю"
else
  "$EMSDK_DIR/emsdk" install "$EMSDK_VERSION"
  touch "$stamp"
fi

"$EMSDK_DIR/emsdk" activate "$EMSDK_VERSION"

# emsdk_env.sh написан без расчёта на set -u (обращается к непроверенным
# переменным), поэтому на время source ослабляем режим.
set +u
# shellcheck disable=SC1091
source "$EMSDK_DIR/emsdk_env.sh" >/dev/null
set -u

command -v emcc >/dev/null 2>&1 || die "emcc не появился в PATH после source emsdk_env.sh."
info "emcc: $(emcc --version | head -n 1)"
info "node: $(node --version 2>/dev/null || echo 'нет (emsdk приносит свой)')"

# ---------------------------------------------------------------------------
# Шаг 2. Исходники LibreOffice
# ---------------------------------------------------------------------------

say "Шаг 2. LibreOffice core $LO_TAG"

if [ -d "$LO_SRC/.git" ]; then
  current_commit="$(git -C "$LO_SRC" rev-parse HEAD)"
  if [ "$current_commit" = "$LO_COMMIT" ]; then
    info "исходники уже на нужном коммите"
  else
    die "$LO_SRC на коммите $current_commit, ожидался $LO_COMMIT.
      Для чистого прогона удалите каталог целиком: инкрементальная сборка с другого пина
      даст артефакт неизвестного происхождения."
  fi
else
  info "клонирую LO core (shallow, тег $LO_TAG) — несколько минут"
  git clone --depth 1 --branch "$LO_TAG" "$LO_GIT_URL" "$LO_SRC"
  current_commit="$(git -C "$LO_SRC" rev-parse HEAD)"
  [ "$current_commit" = "$LO_COMMIT" ] || die "клонированный LO не совпал с пином:
      получено $current_commit, ожидалось $LO_COMMIT (тег переставили?)."
fi

# ---------------------------------------------------------------------------
# Шаг 3. autogen
# ---------------------------------------------------------------------------

say "Шаг 3. autogen.sh"

# Флаги — из headless-раздела README.wasm.md (он же оговаривает, что Qt не
# нужен) плюс наши переключатели. Distro-конфиг LibreOfficeWASM32 для headless
# НЕ подходит: в нём --enable-qt5, то есть GUI-путь.
# --enable-wasm-strip явно не нужен: он включается самим хостом emscripten
# (configure.ac, case emscripten: enable_wasm_strip=yes).
CONFIGURE_FLAGS=(
  "--disable-debug"
  "--enable-sal-log"
  "--disable-crashdump"
  "--host=wasm32-local-emscripten"
  "--disable-gui"
  "--with-wasm-module=$WASM_MODULE"
  "--with-package-format=emscripten"
  "--with-external-tar=$TARBALLS"
)
# Ветвления через if, а не через `[ ... ] && ...`: под set -e ложное условие в
# такой конструкции возвращает ненулевой код всему списку и роняет скрипт —
# на этом прогон обрывался бы ровно в самом начале, без сборки.
if [ "$PROXY_TO_PTHREAD" != "1" ]; then
  CONFIGURE_FLAGS+=("--disable-emscripten-proxy-to-pthread")
fi
# JSPI по умолчанию выключен на стороне configure, поэтому флаг только на включение.
if [ "$JSPI" = "1" ]; then
  CONFIGURE_FLAGS+=("--enable-emscripten-jspi")
fi
if [ "$DISABLE_LTO" = "1" ]; then
  CONFIGURE_FLAGS+=("--disable-lto")
fi
if [ "$ENABLE_CCACHE" = "1" ]; then
  if have ccache; then
    CONFIGURE_FLAGS+=("--enable-ccache")
  else
    warn "ENABLE_CCACHE=1, но ccache не найден — собираю без него."
  fi
fi
# Словоразбиение здесь намеренное: EXTRA_CONFIGURE_FLAGS — строка флагов.
# shellcheck disable=SC2206
[ -z "$EXTRA_CONFIGURE_FLAGS" ] || CONFIGURE_FLAGS+=($EXTRA_CONFIGURE_FLAGS)

# autogen.input — документированный способ передать опции; имена опций
# остаются в файле, то есть в логе и в манифесте видно, чем собирали.
# Сборка идёт in-tree ($LO_SRC/workdir): вариант srcdir != builddir LO
# поддерживает, но добавляет ещё один путь, который придётся держать одинаковым
# между прогонами — иначе в артефакт (в отладочную информацию и в метаданные ФС)
# попадут разные строки, и сверка sha256 будет врать.
printf '%s\n' "${CONFIGURE_FLAGS[@]}" > "$LO_SRC/autogen.input"
info "autogen.input:"
sed 's/^/      /' "$LO_SRC/autogen.input"

# Повторный прогон с теми же флагами пропускается: autogen.sh + configure — это
# минуты, а инкрементальная сборка и так не переделывает готовое.
flags_hash="$(printf '%s\n' "${CONFIGURE_FLAGS[@]}" | sha256sum | awk '{print $1}')"
stamp_hash_file="$LO_SRC/.lowa-autogen-flags"
if [ -f "$LO_SRC/config_host.mk" ] && [ -f "$stamp_hash_file" ] && [ "$(cat "$stamp_hash_file")" = "$flags_hash" ]; then
  info "configure уже выполнялся с этими же флагами — пропускаю autogen.sh"
else
  info "запускаю autogen.sh (emconfigure выставляет EMMAKEN_JUST_CONFIGURE;"
  info "configure без него ругается предупреждением — это ожидаемо)"
  ( cd "$LO_SRC" && ./autogen.sh )
  printf '%s\n' "$flags_hash" > "$stamp_hash_file"
fi

# ---------------------------------------------------------------------------
# Шаг 4. Загрузка внешних tarballs
# ---------------------------------------------------------------------------

say "Шаг 4. make fetch (сеть)"

# Отдельным шагом, потому что падение сети должно случаться до многочасовой
# компиляции, а не посреди неё: fetch повторяем, он идемпотентен.
# bootstrap — потому что цель fetch сама по себе не собирает gbuild-мейкфайлы
# (у LO на них завязан download).
( cd "$LO_SRC" && make bootstrap fetch )

# ---------------------------------------------------------------------------
# Шаг 5. Флаги линковки (только если задан EXTRA_LDFLAGS)
# ---------------------------------------------------------------------------

say "Шаг 5. Дополнительные флаги линковки"

# configure у LO не умеет extra-ldflags: есть --with-extra-cc-flags и
# --with-extra-cxx-flags, но они дописываются к CC/CXX, а линк-флаги
# emscripten-сборки живут в gb_EMSCRIPTEN_LDFLAGS (solenv/gbuild/platform/
# EMSCRIPTEN_INTEL_GCC.mk). Чтобы поменять их, файл надо дописать — отсюда
# патч с маркером: он идемпотентен и снимается сам, когда EXTRA_LDFLAGS пуст,
# иначе дерево исходников осталось бы грязным и следующий прогон собирался бы
# не из того, что записано в пинах.
PLATFORM_MK="$LO_SRC/solenv/gbuild/platform/EMSCRIPTEN_INTEL_GCC.mk"
LDFLAGS_MARKER="lowa-extra-ldflags"

[ -f "$PLATFORM_MK" ] || die "не найден $PLATFORM_MK — раскладка LO изменилась, патч флагов не применить."
if grep -q ">>> $LDFLAGS_MARKER" "$PLATFORM_MK"; then
  awk -v m="$LDFLAGS_MARKER" '
    $0 ~ ">>> " m { skip = 1 }
    skip && $0 ~ "<<< " m { skip = 0; next }
    !skip { print }
  ' "$PLATFORM_MK" > "$PLATFORM_MK.lowa.tmp"
  mv "$PLATFORM_MK.lowa.tmp" "$PLATFORM_MK"
  info "прежний блок флагов снят"
fi
if [ -n "$EXTRA_LDFLAGS" ]; then
  # Никаких пустых строк вокруг блока: файл апстрима заканчивается переводом
  # строки, и при снятии блока дерево должно стать побайтово равным чекауту.
  {
    printf '# >>> %s (добавлено build-lowa.sh; снимается запуском без EXTRA_LDFLAGS)\n' "$LDFLAGS_MARKER"
    printf 'gb_EMSCRIPTEN_LDFLAGS += %s\n' "$EXTRA_LDFLAGS"
    printf '# <<< %s\n' "$LDFLAGS_MARKER"
  } >> "$PLATFORM_MK"
  info "к линку добавлено: $EXTRA_LDFLAGS"
else
  info "не задано — собираю с флагами апстрима"
fi

# ---------------------------------------------------------------------------
# Шаг 6. Сборка
# ---------------------------------------------------------------------------

say "Шаг 6. make -j$JOBS"

# Цели уже: LO собирает soffice.js и его .wasm/.data как один линк-таргет;
# урезать make до подцели — TODO (см. build-log.md, раздел «Осталось
# непроверенным»): в README такой цели нет, а угаданная цель рискует дать
# неполный instdir.
started=$SECONDS
( cd "$LO_SRC" && make -j"$JOBS" )
info "сборка заняла $(( SECONDS - started )) с"

# ---------------------------------------------------------------------------
# Шаг 7. Артефакты
# ---------------------------------------------------------------------------

say "Шаг 7. Сбор артефактов"

# Три файла, а не один: soffice.wasm — код, soffice.js — загрузчик (в него
# вшиты pre-js апстрима, включая static/emscripten/environment.js), а
# soffice.data — in-memory ФС со шрифтами и ресурсами, которую README называет
# обязательной для работы ядра. Каталог, куда их кладёт make, между версиями
# гуляет (instdir/program против workdir/installation/LibreOffice/emscripten),
# поэтому ищем по имени в известных местах и берём первое найденное.
CANDIDATE_DIRS=(
  "$LO_SRC/instdir/program"
  "$LO_SRC/workdir/installation/LibreOffice/emscripten"
  "$LO_SRC/workdir/CustomTarget/static/emscripten_fs_image"
)
SRC_DIRS_USED=()
for name in soffice.js soffice.wasm soffice.data; do
  src=""
  for dir in "${CANDIDATE_DIRS[@]}"; do
    if [ -f "$dir/$name" ]; then
      src="$dir/$name"
      break
    fi
  done
  [ -n "$src" ] || die "не найден $name ни в одном из каталогов:
      ${CANDIDATE_DIRS[*]}
      Сборка не дошла до линковки soffice_bin либо артефакты лежат иначе — см. лог $BUILD_LOG."
  cp -f "$src" "$ARTIFACTS/$name"
  info "$name ← $src"
  SRC_DIRS_USED+=("$(dirname "$src")")
done

# Комплект должен быть из одного каталога: .wasm, .js и .data — результат одной
# и той же линковки. Разные каталоги означают, что часть файлов осталась от
# прошлого прогона, и артефакт собран из несовместимых кусков.
if [ "$(printf '%s\n' "${SRC_DIRS_USED[@]}" | sort -u | wc -l)" -ne 1 ]; then
  warn "файлы взяты из разных каталогов: ${SRC_DIRS_USED[*]}
      Это признак смеси из разных сборок. Проверьте перед отправкой артефакта."
fi

# ---------------------------------------------------------------------------
# Шаг 8. Пост-проверки и манифест
# ---------------------------------------------------------------------------

say "Шаг 8. Пост-проверки"

for name in soffice.js soffice.wasm soffice.data; do
  bytes="$(wc -c < "$ARTIFACTS/$name")"
  [ "$bytes" -gt 0 ] || die "$name пустой."
  # Порог грубый и нужен против одного конкретного исхода: линковка прошла, но
  # без модулей writer (например, опечатка в --with-wasm-module) — тогда wasm
  # в разы меньше ожидаемого.
  if [ "$name" = "soffice.wasm" ] && [ "$bytes" -lt 5000000 ]; then
    warn "soffice.wasm всего $bytes Б — похоже, собрался не тот набор модулей."
  fi
  printf '    %-14s %10s Б  %6.1f МБ  sha256 %s\n' \
    "$name" "$bytes" "$(awk -v b="$bytes" 'BEGIN {print b/1048576}')" \
    "$(sha256sum "$ARTIFACTS/$name" | awk '{print $1}')"
done

MANIFEST="$ARTIFACTS/MANIFEST.txt"
{
  echo "lowa build manifest"
  echo "дата (UTC):     $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "хост:           $(uname -srm)"
  echo "vCPU:           $JOBS"
  echo "RAM, ГБ:        ${mem_gb:-неизвестно}"
  echo "emsdk:          $EMSDK_VERSION ($EMSDK_COMMIT)"
  echo "LO:             $LO_TAG ($LO_COMMIT)"
  echo "wasm-модуль:    $WASM_MODULE"
  echo "PROXY_TO_PTHREAD: $PROXY_TO_PTHREAD"
  echo "JSPI:           $JSPI"
  echo "DISABLE_LTO:    $DISABLE_LTO"
  echo "ccache:         $ENABLE_CCACHE"
  echo "EXTRA_LDFLAGS:  ${EXTRA_LDFLAGS:-<пусто>}"
  echo "флаги configure:"
  sed 's/^/  /' "$LO_SRC/autogen.input"
  echo "git HEAD собранного дерева: $(git -C "$LO_SRC" rev-parse HEAD)"
  echo "изменения в дереве исходников (ожидается только solenv/gbuild/platform/EMSCRIPTEN_INTEL_GCC.mk,"
  echo "и только если задан EXTRA_LDFLAGS — плюс локальный autogen.input):"
  git -C "$LO_SRC" status --porcelain | sed 's/^/  /'
  echo "артефакты:"
  for name in soffice.js soffice.wasm soffice.data; do
    echo "  $name $(wc -c < "$ARTIFACTS/$name") $(sha256sum "$ARTIFACTS/$name" | awk '{print $1}')"
  done
} > "$MANIFEST"

# ---------------------------------------------------------------------------
# Итог
# ---------------------------------------------------------------------------

say "Готово"
info "артефакты ($(du -sh "$ARTIFACTS" | awk '{print $1}') всего):"
ls -l "$ARTIFACTS"
info "манифест: $MANIFEST"
info "лог:      $BUILD_LOG"
cat <<'EOF'

Дальше:
  1. Забрать три файла из каталога artifacts (путь напечатан выше) и MANIFEST.txt
     рядом с ними: это release asset, *.wasm в git не кладём.
  2. Прогнать второй раз на чистой VM с теми же BUILD_ROOT и LO_SRC —
     иначе в артефакт попадут другие пути, и sha256 разойдётся сам по себе.
  3. Записать результат в docs/sprint-13/build-log.md.
EOF
