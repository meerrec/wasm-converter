#!/usr/bin/env bash
# Захват эталонов strict-viewer локальным Microsoft Word/Excel для macOS.
#
# Порт scripts/strict-viewer-references.ps1. COM на macOS нет, поэтому с Office
# говорит AppleScript (`osascript`): `open` + `save as … format PDF`. Это тот же
# локальный Microsoft Office, только другой IPC, поэтому manifest честно пишет
# `provider: office-desktop`, `application: word-desktop`/`excel-desktop`.
#
# Ловушки, проверенные на Word 16.115 и записанные в код:
#
#   * `active document` после `open` остаётся `missing value`, а имена всех
#     документов — «Document1»: брать документ можно только по индексу, и
#     надёжнее по индексу, который создал сам `open` (последний).
#   * `save as` принимает direct-параметр `document`, а не `active document`;
#     `file name` должен быть `POSIX file`, а не строка и не HFS-путь.
#   * холодный старт Word на macOS — 28 с, поэтому ожидание документа
#     параметризовано (`WAIT_TIMEOUT`), а не фиксировано.
#   * Word можно закрыть, даже если он был открыт с документами пользователя:
#     скрипт закрывает только то, что открыл сам (счётчик документов до/после),
#     и не завершает приложение, если пользовательские документы остались.
#
# Наш движок эталоном не является никогда: кадры берёт локальный Office.
# Reference отсюда — кандидат, а не accepted baseline (baseline-policy §3 п.5, §5).
#
# Использование:
#   scripts/strict-viewer-references-macos.sh --browser-version 131.0.6778.86
#   scripts/strict-viewer-references-macos.sh --filter 'docx-*' --keep-pdf
#
# Коды выхода: 0 — хотя бы один drop импортирован; 1 — импортировано ноль;
# 2 — предпосылка не выполнена (нет Word/Excel/корпуса/инструментов).

set -euo pipefail

FIXTURES='test-fixtures'
CORPUS='test-fixtures/strict-viewer/corpus.json'
OUT='strict-viewer-refs-out'
DPI=144
BROWSER_VERSION=''
FONT_PACKAGE='carlito-subset'
FONT_VERSION='1.104'
OPERATOR="${USER:-$(hostname -s)}"
FILTER=''
KEEP_PDF=0
INCLUDE_WEB_MODES=0
WAIT_TIMEOUT=120

MAX_PNG_BYTES=$((4 * 1024 * 1024))
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORD_VERSION='не использовался'
EXCEL_VERSION='не использовался'
PDFTOPPM_VERSION=''
CORPUS_VERSION=''
CORPUS_MATRIX_VERSION=''
OS_NAME=''
OS_VERSION=''
OS_ARCH=''

die() {
    printf 'ОШИБКА: %s\n' "$1" >&2
    exit 2
}

usage() {
    sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    cat <<'EOF'
Флаги:
  --fixtures DIR          корень фикстур (по умолчанию test-fixtures)
  --corpus FILE           corpus index (по умолчанию test-fixtures/strict-viewer/corpus.json)
  --out DIR               каталог drop-ов и отчёта (по умолчанию strict-viewer-refs-out)
  --dpi N                 DPI растеризации; схема фиксирует 144
  --browser-version V     version для environment.browser (обязателен, если есть экспорт)
  --font-package NAME     environment.fonts.package (carlito-subset)
  --font-version V        environment.fonts.version (1.104)
  --operator NAME         operator в capture.json (по умолчанию $USER)
  --filter GLOB           только фикстуры, чей id подходит под glob
  --keep-pdf              не удалять промежуточные PDF/PNG
  --include-web-modes     пометить print-layout/normal как «требует ручного захвата»
  --wait-timeout S        предел ожидания документа в Word/Excel (120 с)
  -h, --help              эта справка
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --fixtures) FIXTURES="$2"; shift 2 ;;
        --corpus) CORPUS="$2"; shift 2 ;;
        --out) OUT="$2"; shift 2 ;;
        --dpi) DPI="$2"; shift 2 ;;
        --browser-version) BROWSER_VERSION="$2"; shift 2 ;;
        --font-package) FONT_PACKAGE="$2"; shift 2 ;;
        --font-version) FONT_VERSION="$2"; shift 2 ;;
        --operator) OPERATOR="$2"; shift 2 ;;
        --filter) FILTER="$2"; shift 2 ;;
        --keep-pdf) KEEP_PDF=1; shift ;;
        --include-web-modes) INCLUDE_WEB_MODES=1; shift ;;
        --wait-timeout) WAIT_TIMEOUT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "неизвестный флаг: $1 (см. --help)" ;;
    esac
done

step() { printf '==> %s\n' "$1"; }

# Утилиты -------------------------------------------------------------------

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "не найдена команда $1. $2"
}

app_path() {
    # Имя .app — единственный признак установленного Office: AppleScript
    # поднимает приложение по имени, а не по пути.
    local name="$1"
    if [ -d "/Applications/$name.app" ] || [ -d "$HOME/Applications/$name.app" ]; then
        return 0
    fi
    return 1
}

app_version() {
    /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
        "/Applications/$1.app/Contents/Info.plist" 2>/dev/null ||
        echo 'unknown'
}

resolve_repo_path() {
    # Относительный путь ищется от текущего каталога, затем от корня репозитория —
    # так же, как в .ps1: запуск из подкаталога не должен ломать корпус.
    local path="$1"
    if [ "${path#/}" != "$path" ]; then
        [ -e "$path" ] && realpath "$path" || return 1
    fi
    if [ -e "$PWD/$path" ]; then
        realpath "$PWD/$path"
    elif [ -e "$REPO_ROOT/$path" ]; then
        realpath "$REPO_ROOT/$path"
    else
        return 1
    fi
}

sha256_hex() {
    shasum -a 256 "$1" | awk '{print $1}'
}

json_escape() {
    # Строки в capture.json — пути и версии; контрольные символы и кавычки
    # экранируются, чтобы JSON-структура не зависела от содержимого.
    local s="$1"
    s="${s//\\/\\\\}"
    s="${s//\"/\\\"}"
    s="${s//$'\n'/\\n}"
    s="${s//$'\r'/}"
    s="${s//$'\t'/\\t}"
    s="${s//$'\x1f'/}"
    s="${s//$'\b'/\\b}"
    s="${s//$'\f'/\\f}"
    printf '%s' "$s"
}

# PNG ------------------------------------------------------------------------

# Проверка заголовка: сигнатура, IHDR, 8 бит, RGB8 (2) или RGBA8 (6), ≤ 4 МиБ.
# Печатает "<width> <height>" и падает, если PNG не годен в эталон.
png_header() {
    local path="$1"
    python3 - "$path" "$MAX_PNG_BYTES" <<'PY'
import struct
import sys

path, limit = sys.argv[1], int(sys.argv[2])
data = open(path, 'rb').read()
if len(data) < 33:
    sys.exit(f'PNG слишком короткий: {path}')
if data[:8] != b'\x89PNG\r\n\x1a\n':
    sys.exit(f'PNG не имеет валидной сигнатуры: {path}')
length = struct.unpack('>I', data[8:12])[0]
if length != 13 or data[12:16] != b'IHDR':
    sys.exit(f'PNG не имеет валидного IHDR: {path}')
width, height, depth, color = struct.unpack('>IIBB', data[16:26])
if depth != 8:
    sys.exit(f'PNG должен быть 8-битным, получено bit depth {depth}: {path}')
if color not in (2, 6):
    sys.exit(f'PNG должен быть RGB8 (colorType 2) или RGBA8 (colorType 6), '
             f'получено colorType {color}: {path}')
if len(data) > limit:
    sys.exit(f'PNG больше 4 МиБ ({len(data)} байт): {path}')
print(width, height)
PY
}

# AppleScript-экспорт --------------------------------------------------------

# Первый аргумент — приложение ("Microsoft Word"/"Microsoft Excel"), второй —
# исходник, третий — PDF. Инструкция лежит в одном heredoc: отдельные файлы
# рядом со скриптом добавляли бы зависимость от пути, которая не нужна.
export_pdf() {
    local input="$1" output="$2"
    local script
    script="$(mktemp "${TMPDIR:-/tmp}/strict-viewer-export.XXXXXX.applescript")"
    # shellcheck disable=SC2016
    cat >"$script" <<'APPLESCRIPT'
on run argv
    set inPath to item 1 of argv
    set outPath to item 2 of argv
    set waitSeconds to (item 3 of argv) as integer

    -- Документ адресуется по индексу, который создал сам `open`: у Word на
    -- macOS `active document` после `open` остаётся missing value.
    -- Приложение — литерал, а не переменная: с `tell application appName`
    -- osascript не подхватывает словарь Word, и `save as` не компилируется
    -- («Expected end of line … found class name», -2741).
    tell application "Microsoft Word"
        activate
        set beforeCount to count of documents
        open POSIX file inPath
    end tell

    set deadline to (current date) + waitSeconds
    repeat
        tell application "Microsoft Word"
            set nowCount to count of documents
        end tell
        if nowCount > beforeCount then exit repeat
        if (current date) > deadline then error "документ не открылся за " & waitSeconds & " с"
        delay 0.5
    end repeat

    tell application "Microsoft Word"
        set theDoc to document nowCount
        -- `save as` принимает direct-параметр document и POSIX file;
        -- строка и HFS-путь дают -1708/-1728.
        save as theDoc file name (POSIX file outPath) file format format PDF
        close theDoc saving no
    end tell
    return "ok"
end run
APPLESCRIPT
    local result
    if ! result="$(osascript "$script" "$input" "$output" "$WAIT_TIMEOUT" 2>&1)"; then
        rm -f "$script"
        printf '%s\n' "$result" >&2
        return 1
    fi
    rm -f "$script"
    [ -f "$output" ] || { echo "Office не создал PDF: $output" >&2; return 1; }
}

# Первый лист книги сохранён как PDF: Excel на macOS не отдаёт AppleScript
# выбор листа, поэтому берётся тот, который Office считает активным при
# открытии (первый видимый). Скрытый активный лист — ошибка фикстуры.
export_sheet_pdf() {
    local input="$1" output="$2"
    local script
    script="$(mktemp "${TMPDIR:-/tmp}/strict-viewer-export.XXXXXX.applescript")"
    # shellcheck disable=SC2016
    cat >"$script" <<'APPLESCRIPT'
on run argv
    set inPath to item 1 of argv
    set outPath to item 2 of argv
    set waitSeconds to (item 3 of argv) as integer

    -- Приложение — литерал: у `tell application appName` osascript не
    -- подхватывает словарь и `save as` не компилируется (-2741).
    tell application "Microsoft Excel"
        activate
        set beforeCount to count of documents
        open POSIX file inPath
    end tell

    set deadline to (current date) + waitSeconds
    repeat
        tell application "Microsoft Excel"
            set nowCount to count of documents
        end tell
        if nowCount > beforeCount then exit repeat
        if (current date) > deadline then error "книга не открылась за " & waitSeconds & " с"
        delay 0.5
    end repeat

    tell application "Microsoft Excel"
        set theBook to document nowCount
        save as theBook file name (POSIX file outPath) file format format PDF
        close theBook saving no
    end tell
    return "ok"
end run
APPLESCRIPT
    if ! osascript "$script" "$input" "$output" "$WAIT_TIMEOUT" >/dev/null 2>&1; then
        rm -f "$script"
        return 1
    fi
    rm -f "$script"
    [ -f "$output" ] || { echo "Office не создал PDF: $output" >&2; return 1; }
}

# Печатает capture.json. JSON собирает jq: ручная конкатенация скобок уже
# давала невалидный файл, а импорт от этого падает с кодом 3.
# Изображения передаются парами «uri|файл»; в JSON идёт только uri.
capture_json() {
    local format="$1" mode="$2" source_rel="$3" application="$4"
    local application_version="$5" shape="$6" font_assets="$7"
    shift 7
    local uris=()
    for pair in "$@"; do
        uris+=("${pair%|*}")
    done

    local mode_args=()
    if [ "$mode" = 'pdf' ]; then
        mode_args+=(--arg rasterizer_version "$PDFTOPPM_VERSION" --argjson dpi "$DPI")
    fi

    jq -n \
        --arg corpus_version "$CORPUS_VERSION" \
        --arg matrix_version "$CORPUS_MATRIX_VERSION" \
        --arg source_path "$source_rel" \
        --arg application "$application" \
        --arg application_version "$application_version" \
        --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        --arg operator "$OPERATOR" \
        --arg os_name "$OS_NAME" \
        --arg os_version "$OS_VERSION" \
        --arg os_arch "$OS_ARCH" \
        --arg browser_version "$BROWSER_VERSION" \
        --arg font_package "$FONT_PACKAGE" \
        --arg font_version "$FONT_VERSION" \
        --argjson font_assets "$font_assets" \
        --arg format "$format" \
        --arg mode "$mode" \
        --arg shape "$shape" \
        --argjson uris "$(printf '%s\n' "${uris[@]}" | jq -R . | jq -s .)" \
        "${mode_args[@]}" \
        '
        def environment:
          {
            os: { name: $os_name, version: $os_version, architecture: $os_arch },
            browser: { name: "chromium", version: $browser_version },
            applicationZoomPercent: 100,
            browserZoomPercent: 100,
            locale: "ru-RU",
            timezone: "UTC",
            fonts: { package: $font_package, version: $font_version, assets: $font_assets }
          }
          + (if $mode == "pdf"
             then { rasterizer: { name: "pdftoppm", version: $rasterizer_version, dpi: $dpi } }
             else { viewportCss: { width: 1440, height: 900 }, deviceScaleFactor: 1, colorScheme: "light" }
             end);
        def view:
          if $format == "docx"
          then { format: "docx", mode: $mode, revisions: "accepted", comments: "hidden" }
          else {
            format: "xlsx", mode: $mode, activeSheet: { index: 0, name: $shape },
            hiddenSheets: "exclude", showGridLines: false, showHeadings: false
          }
          end;
        def captures:
          [ $uris | to_entries[] | . as $entry | ($entry.key + 1) as $page |
            ({ id: "page-\($page)", pageNumber: $page, image: $entry.value }
             + (if $format == "docx"
                then { kind: "docx-page" }
                else { kind: "xlsx-print-page", sheet: { index: 0, name: $shape } }
                end)) ];
        {
          corpus: { version: $corpus_version, matrixVersion: $matrix_version },
          source: { path: $source_path },
          reference: {
            provider: "office-desktop",
            application: $application,
            applicationVersion: $application_version,
            capturedAt: $captured_at,
            captureMethod: "automated",
            operator: $operator,
            exportedPdf: { uri: "reference/export.pdf" }
          },
          environment: environment,
          view: view,
          dynamicData: {
            dates: "fixed-value",
            fields: "cached-result",
            formulaCache: {
              policy: "cached-values-only", dirty: false,
              fullCalcOnLoad: false, forceFullCalc: false
            },
            externalLinks: "cached-values-no-network"
          },
          captures: captures
        }'
}

# Список шрифтовых ассетов для environment.fonts.assets (ADR-0051).
font_assets_json() {
    local dir="$REPO_ROOT/crates/render/src/fonts"
    local out='' sep=''
    local entry file style weight
    [ -d "$dir" ] || die "не найден шрифтовой набор: $dir (нужен для environment.fonts.assets)"
    while IFS='|' read -r file style weight; do
        [ -f "$dir/$file" ] || die "не найден файл шрифта: $dir/$file"
        out+="$sep"'{"family":"Carlito","style":"'"$style"'","weight":'"$weight"',"sha256":"'"$(sha256_hex "$dir/$file")"'"}'
        sep=','
    done <<'EOF'
carlito-subset.ttf|normal|400
carlito-bold-subset.ttf|bold|700
carlito-italic-subset.ttf|italic|400
carlito-bolditalic-subset.ttf|italic700|700
EOF
    # Четвёртый файл — bold-italic: вес 700, стиль italic.
    out="${out/italic700/italic}"
    printf '[%s]' "$out"
}

# Предпосылки -----------------------------------------------------------------

[ "$(uname -s)" = 'Darwin' ] ||
    die 'скрипт рассчитан на macOS: Word/Excel через AppleScript. На Windows — scripts/strict-viewer-references.ps1.'

[ "$DPI" -gt 0 ] 2>/dev/null || die "DPI должен быть больше 0 (получено $DPI)"

corpus_path="$(resolve_repo_path "$CORPUS")" || die "не найден corpus index: $CORPUS"
fixtures_root="$(resolve_repo_path "$FIXTURES")" || die "не найден каталог фикстур: $FIXTURES"

CORPUS_VERSION="$(jq -r '.corpus.version // empty' "$corpus_path")"
CORPUS_MATRIX_VERSION="$(jq -r '.corpus.matrixVersion // empty' "$corpus_path")"
[ -n "$CORPUS_VERSION" ] && [ -n "$CORPUS_MATRIX_VERSION" ] ||
    die "corpus index $corpus_path не содержит corpus.version/corpus.matrixVersion"

require_command jq 'Установите jq (brew install jq).'
require_command python3 'Python 3 нужен для проверки заголовка PNG.'
require_command osascript 'osascript входит в macOS.'
require_command pdftoppm 'Установите Poppler (brew install poppler) и добавьте его в PATH.'
require_command pdfinfo 'Установите Poppler (brew install poppler).'
require_command pnpm 'Установите pnpm 9 и добавьте его в PATH.'
require_command node 'Установите Node.js 20+ и добавьте его в PATH.'

PDFTOPPM_VERSION="$(pdftoppm -v 2>&1 | head -1 | sed -nE 's/.*version[[:space:]]+([0-9][0-9A-Za-z.-]*).*/\1/p')"
[ -n "$PDFTOPPM_VERSION" ] || die 'не удалось определить версию pdftoppm (pdftoppm -v).'

OS_NAME="$(sw_vers -productName 2>/dev/null || echo macOS)"
OS_VERSION="$(sw_vers -productVersion 2>/dev/null || echo unknown)"
OS_ARCH="$(uname -m)"

# --- отбор фикстур и режимов ------------------------------------------------

plans_file="$(mktemp "${TMPDIR:-/tmp}/strict-viewer-plans.XXXXXX")"
trap 'rm -f "$plans_file"' EXIT

jq -r --arg filter "$FILTER" '
  .fixtures[]
  | select($filter == "" or (.id | test($filter | gsub("\\*"; ".*"))))
  | . as $f
  | $f.modes[]
  | [$f.format, $f.id, ., $f.source.path] | @tsv
' "$corpus_path" >"$plans_file"

[ -s "$plans_file" ] || {
    if [ -n "$FILTER" ]; then
        die "фильтр --filter '$FILTER' не совпал ни с одной фикстурой корпуса."
    fi
    die "в корпусе $corpus_path нет фикстур."
}

# Гарнитура, которой Office реально нарисовал PDF.
#
# Эталон обязан быть снят тем лицом, которое объявляет manifest (ADR-0058):
# манифест с `carlito-subset` и PDF со встроенным Calibri — ложь о среде,
# и компаратор покажет «та же вёрстка, другая гарнитура» вместо регрессии.
# Word молча подменяет неизвестное семейство (TimesNewRoman), поэтому проверка
# идёт по содержимому PDF, а не по установленным шрифтам.
pdf_fonts() {
    local pdf="$1"
    # `pdffonts` печатает шапку из двух строк; имена — третий столбец.
    pdffonts "$pdf" 2>/dev/null | tail -n +3 | awk '{print $1}' | sed 's/^[A-Z]*+//'
}

# Проверить, что все гарнитуры эталона — либо объявленный `$expected`, либо
# служебные `*Fallback`/`Type3`, которые Office пишет для знаков вне шрифта.
check_reference_fonts() {
    local pdf="$1" expected="$2" fixture="$3"
    local names unexpected
    names="$(pdf_fonts "$pdf")"
    if [ -z "$names" ]; then
        echo "не удалось прочитать гарнитуры эталона (pdffonts): $pdf"
        return 1
    fi
    unexpected="$(printf '%s\n' "$names" | grep -v -e "^$expected" -e 'Fallback' -e '^Type3' || true)"
    if [ -n "$unexpected" ]; then
        echo "эталон нарисован не $expected ($(printf '%s' "$unexpected" | tr '\n' ' ' | sed 's/ *$//')): фикстура $fixture должна объявлять шрифт $expected (ADR-0058)"
        return 1
    fi
    return 0
}

# Действие по паре format/mode; правила те же, что в .ps1.
action_of() {
    case "$1/$2" in
        docx/pdf | xlsx/print-preview) echo 'export' ;;
        docx/print-layout | xlsx/normal) echo 'web' ;;
        *) echo 'unknown' ;;
    esac
}

need_word=0
need_excel=0
export_count=0
while IFS=$'\t' read -r format fixture mode source_rel; do
    action="$(action_of "$format" "$mode")"
    if [ "$action" = 'export' ]; then
        export_count=$((export_count + 1))
        [ "$format" = 'docx' ] && need_word=1
        [ "$format" = 'xlsx' ] && need_excel=1
    fi
done <"$plans_file"

if [ "$export_count" -gt 0 ] && [ -z "$BROWSER_VERSION" ]; then
    modes="$(while IFS=$'\t' read -r format _ mode _; do
        printf '%s.%s\n' "$format" "$mode"
    done <"$plans_file" | sort -u | paste -sd, - | sed 's/,/, /g')"
    die "не задан --browser-version: схема manifest требует environment.browser.version для режимов $modes."
fi

if [ "$need_word" -eq 1 ]; then
    app_path 'Microsoft Word' || die 'не найден Microsoft Word в /Applications (нужна AppleScript-автоматизация).'
    WORD_VERSION="$(app_version 'Microsoft Word')"
fi
if [ "$need_excel" -eq 1 ]; then
    app_path 'Microsoft Excel' || die 'не найден Microsoft Excel в /Applications (нужна AppleScript-автоматизация).'
    EXCEL_VERSION="$(app_version 'Microsoft Excel')"
fi

font_assets="$(font_assets_json)"

# --- каталоги ----------------------------------------------------------------

case "$OUT" in
    /*) ;;
    *) OUT="$PWD/$OUT" ;;
esac
drop_root="$OUT/drop"
work_root="$OUT/work"
log_dir="$OUT/log"
mkdir -p "$drop_root" "$work_root" "$log_dir"

if [ "$DPI" -ne 144 ]; then
    printf 'ПРЕДУПРЕЖДЕНИЕ: схема manifest фиксирует dpi = 144; значение %s не пройдёт валидацию.\n' "$DPI" >&2
fi

# --- прогон ------------------------------------------------------------------

results_file="$(mktemp "${TMPDIR:-/tmp}/strict-viewer-results.XXXXXX")"
trap 'rm -f "$plans_file" "$results_file"' EXIT

printf 'format\tfixture\tmode\tpages\tstatus\treason\n' >"$results_file"

record() {
    # Причина не должна ломать таблицу отчёта: переводы строк и | убираются.
    local reason
    reason="$(printf '%s' "$6" | tr '\n\r' '  ' | sed 's/|/\\|/g')"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" "$reason" >>"$results_file"
}

# Компонент пути из корпуса: slug без разделителей и «..». Эти имена идут в
# `rm -rf`, поэтому `../../x` или пустое значение не должны выйти из $OUT.
is_safe_component() {
    [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] && [[ "$1" != *..* ]]
}

# Относительный путь исходника внутри каталога фикстур: без ведущего «/»,
# без сегментов «..» и «.».
is_safe_relative_path() {
    local rel="$1" part parts
    [ -n "$rel" ] && [ "${rel#/}" = "$rel" ] && [ "${rel%/}" = "$rel" ] || return 1
    IFS='/' read -ra parts <<<"$rel"
    for part in "${parts[@]}"; do
        case "$part" in '' | . | ..) return 1 ;; esac
    done
}

# Каталог, который скрипт собирается удалить, обязан лежать строго внутри корня.
assert_child_dir() {
    local root="$1" dir="$2"
    case "$dir" in
        "$root"/?*) ;;
        *) die "путь «$dir» вне корня «$root» — отказ удалять." ;;
    esac
}

while IFS=$'\t' read -r format fixture mode source_rel; do
    label="$format/$fixture/$mode"
    case "$format" in
        docx | xlsx) ;;
        *) die "фикстура «$fixture»: неизвестный формат «$format»." ;;
    esac
    is_safe_component "$fixture" ||
        die "id фикстуры «$fixture» не годится как имя каталога (ожидается slug из букв, цифр, '.', '_', '-')."
    is_safe_component "$mode" ||
        die "фикстура «$fixture»: режим «$mode» не годится как имя каталога."
    is_safe_relative_path "$source_rel" ||
        die "фикстура «$fixture»: source.path «$source_rel» должен быть относительным путём внутри $fixtures_root."
    action="$(action_of "$format" "$mode")"

    if [ "$action" = 'web' ]; then
        reason='режим снимается с экрана веб-версии Word/Excel Online; PDF-экспорт его не даёт'
        if [ "$INCLUDE_WEB_MODES" -eq 1 ]; then
            reason='требует ручного захвата в веб-версии; скрипт этот режим не реализует'
        fi
        step "$label — Skip ($reason)"
        record "$format" "$fixture" "$mode" 0 skipped "$reason"
        continue
    fi
    if [ "$action" = 'unknown' ]; then
        reason="неизвестный режим «$mode» для формата $format"
        step "$label — Skip ($reason)"
        record "$format" "$fixture" "$mode" 0 skipped "$reason"
        continue
    fi

    step "$label"
    work_dir="$work_root/$format/$fixture/$mode"
    drop_dir="$drop_root/$format/$fixture/$mode"
    ref_dir="$drop_dir/reference"
    source_path="$fixtures_root/$source_rel"

    # Повторный прогон не должен оставлять страницы от прошлого захвата.
    assert_child_dir "$work_root" "$work_dir"
    assert_child_dir "$drop_root" "$drop_dir"
    rm -rf "$work_dir" "$drop_dir"
    # pages/ нужен до pdftoppm: он не создаёт подкаталог префикса.
    mkdir -p "$work_dir/pages" "$ref_dir"

    if [ ! -f "$source_path" ]; then
        reason="исходный документ не найден: $source_path"
        printf '    failed: %s\n' "$reason"
        record "$format" "$fixture" "$mode" 0 failed "$reason"
        continue
    fi

    pdf_path="$work_dir/export.pdf"
    if [ "$format" = 'docx' ]; then
        if ! export_pdf "$source_path" "$pdf_path" 2>"$work_dir/export.log"; then
            reason="AppleScript-экспорт Word: $(tail -1 "$work_dir/export.log")"
            printf '    failed: %s\n' "$reason"
            record "$format" "$fixture" "$mode" 0 failed "$reason"
            continue
        fi
    else
        if ! export_sheet_pdf "$source_path" "$pdf_path" 2>"$work_dir/export.log"; then
            reason="AppleScript-экспорт Excel: $(tail -1 "$work_dir/export.log")"
            printf '    failed: %s\n' "$reason"
            record "$format" "$fixture" "$mode" 0 failed "$reason"
            continue
        fi
    fi

    # Абсолютный путь: pdftoppm пишет «<prefix>-N.png» рядом с префиксом.
    if ! pdftoppm -png -r "$DPI" "$pdf_path" "$work_dir/pages/page" \
        >"$log_dir/$fixture-$mode-pdftoppm.log" 2>&1; then
        reason="pdftoppm завершился с ошибкой: $(tail -1 "$log_dir/$fixture-$mode-pdftoppm.log")"
        printf '    failed: %s\n' "$reason"
        record "$format" "$fixture" "$mode" 0 failed "$reason"
        continue
    fi

    shopt -s nullglob
    page_files=("$work_dir"/pages/page-*.png)
    shopt -u nullglob
    if [ "${#page_files[@]}" -eq 0 ]; then
        reason='pdftoppm не создал ни одной страницы'
        printf '    failed: %s\n' "$reason"
        record "$format" "$fixture" "$mode" 0 failed "$reason"
        continue
    fi

    image_pairs=()
    page=0
    failed_reason=''
    for file in "${page_files[@]}"; do
        page=$((page + 1))
        if ! png_header "$file" >/dev/null; then
            failed_reason="$(png_header "$file" 2>&1 || true)"
            break
        fi
        if [ "$mode" = 'pdf' ]; then
            name="page-$page.png"
        else
            name="print-page-$page.png"
        fi
        cp -f "$file" "$ref_dir/$name"
        image_pairs+=("reference/$name|$file")
    done
    if [ -n "$failed_reason" ]; then
        printf '    failed: %s\n' "$failed_reason"
        record "$format" "$fixture" "$mode" 0 failed "$failed_reason"
        continue
    fi

    # Шрифт эталона обязан совпасть с объявленным в manifest (ADR-0058):
    # PDF со встроенным Calibri при `carlito-subset` — дефект захвата.
    if ! fonts_report="$(check_reference_fonts "$pdf_path" 'Carlito' "$fixture" 2>&1)"; then
        reason="$fonts_report"
        printf '    failed: %s\n' "$reason"
        record "$format" "$fixture" "$mode" 0 failed "$reason"
        continue
    fi

    cp -f "$pdf_path" "$ref_dir/export.pdf"

    if [ "$format" = 'docx' ]; then
        application='word-desktop'
        application_version="$WORD_VERSION"
    else
        application='excel-desktop'
        application_version="$EXCEL_VERSION"
    fi

    # Имя листа для xlsx-снимков: первый лист книги, как его видит корпус.
    sheet_name=''
    if [ "$format" = 'xlsx' ]; then
        sheet_name="$(python3 - "$source_path" <<'PY'
import re
import sys
import zipfile

path = sys.argv[1]
with zipfile.ZipFile(path) as archive:
    names = archive.namelist()
    pattern = re.compile(r'^xl/worksheets/sheet\d+\.xml$')
    sheets = sorted(name for name in names if pattern.match(name))
    if not sheets:
        print('')
        sys.exit(0)
    xml = archive.read('xl/workbook.xml').decode('utf-8', 'replace')
    match = re.search(r'<sheet\b[^>]*\bname="([^"]*)"', xml)
    print(match.group(1) if match else '')
PY
)"
        [ -n "$sheet_name" ] || sheet_name='Sheet1'
    fi

    capture_json "$format" "$mode" "$source_rel" "$application" "$application_version" \
        "$sheet_name" "$font_assets" "${image_pairs[@]}" >"$drop_dir/capture.json"

    if ! (cd "$REPO_ROOT" && pnpm visual-oracle reference import \
        --from "$drop_dir" \
        --corpus "$corpus_path" \
        --references "$drop_root" \
        --fixtures "$fixtures_root") >"$log_dir/$fixture-$mode-import.log" 2>&1; then
        reason="импорт завершился с ошибкой: $(tail -1 "$log_dir/$fixture-$mode-import.log")"
        printf '    failed: %s\n' "$reason"
        record "$format" "$fixture" "$mode" 0 failed "$reason"
        continue
    fi

    record "$format" "$fixture" "$mode" "$page" imported ''
    printf '    imported: %s стр., manifest %s\n' "$page" "$drop_dir/manifest.json"
done <"$plans_file"

# --- отчёт -------------------------------------------------------------------

imported=$(awk -F'\t' 'NR > 1 && $5 == "imported"' "$results_file" | wc -l | tr -d ' ')
skipped=$(awk -F'\t' 'NR > 1 && $5 == "skipped"' "$results_file" | wc -l | tr -d ' ')
failed=$(awk -F'\t' 'NR > 1 && $5 == "failed"' "$results_file" | wc -l | tr -d ' ')
total_pages=$(awk -F'\t' 'NR > 1 && $5 == "imported" { sum += $4 } END { print sum + 0 }' "$results_file")

report="$OUT/report.md"
{
    echo '# Эталоны strict-viewer из локального Microsoft Office (macOS)'
    echo
    echo '> **Оговорка о политике.** Скрипт снимает эталон локальным Microsoft Word/Excel через'
    echo '> AppleScript — тот же путь захвата, что COM на Windows (`docs/adr/0056-capture-automation-com.md`,'
    echo '> `baseline-policy.md` §1 и §3 п.5). Reference отсюда — **кандидат, а не accepted baseline**:'
    echo '> принятие по-прежнему требует baseline-only PR и двух approvals (§5).'
    echo
    echo "- Дата: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "- Машина: $(hostname -s)"
    echo "- Оператор: $OPERATOR"
    echo "- ОС: $OS_NAME $OS_VERSION ($OS_ARCH)"
    echo "- Microsoft Word: $WORD_VERSION"
    echo "- Microsoft Excel: $EXCEL_VERSION"
    echo "- Poppler: $PDFTOPPM_VERSION"
    echo "- DPI: $DPI, растеризация \`pdftoppm -png -r $DPI\` (без \`-gray\`)"
    echo "- Корпус: \`$corpus_path\` (version $CORPUS_VERSION, matrixVersion $CORPUS_MATRIX_VERSION)"
    echo "- Drop: \`$drop_root\`"
    echo "- Шрифтовой набор: $FONT_PACKAGE $FONT_VERSION (SHA-256 в capture.json)"
    [ -n "$FILTER" ] && echo "- Фильтр: \`$FILTER\`"
    [ "$INCLUDE_WEB_MODES" -eq 1 ] && echo '- Веб-режимы помечены через `--include-web-modes`'
    echo
    echo '| Формат | Фикстура | Режим | Страниц | Статус | Причина |'
    echo '| --- | --- | --- | ---: | --- | --- |'
    tail -n +2 "$results_file" | awk -F'\t' '{ printf "| %s | %s | %s | %s | %s | %s |\n", $1, $2, $3, $4, $5, $6 }'
    echo
    echo '## Итоги'
    echo
    echo "Импортировано: $imported, пропущено: $skipped, с ошибкой: $failed."
    echo "Страниц в импортированных drop-ах: $total_pages."
    echo
    echo '## Ограничения'
    echo
    echo '- `docx.print-layout` и `xlsx.normal` снимаются с экрана веб-версии Word/Excel Online и из PDF-экспорта не получаются. Скрипт их пропускает; эквивалент не выдумывается.'
    echo '- Эталон снимает локальный Word/Excel через AppleScript — разрешённый способ захвата (`accept-capture-automation`, `baseline-policy.md` §1).'
    echo '- `revisions: accepted`, `comments: hidden` достигаются сохранением в PDF без разметки правок: правки не «принимаются» физически, а не попадают в PDF.'
    echo '- В `environment.fonts` записан версионированный набор Carlito из репозитория; фактический рендер Word/Excel использует системные шрифты. Расхождение фиксируется как ограничение кандидата.'
    echo '- XLSX снимается только с листа, который Excel открывает активным (первый видимый); скрытый активный лист — ошибка фикстуры.'
    echo '- `showGridLines: false`, `showHeadings: false` объявлены для print-preview; если в книге включена печать сетки или заголовков, объявление разойдётся с изображением.'
    echo '- Импорт вызывается с `--references <Out>/drop`, поэтому manifest пишется рядом с `capture.json`, а drop самодостаточен. Перенос drop в `test-fixtures/strict-viewer` — отдельный baseline-only PR.'
    echo '- Reference — кандидат: принятие требует baseline-only PR и двух approvals (baseline-policy §5).'
    echo
    echo '## Артефакты'
    echo
    echo "- \`$drop_root\` — drop-ы \`<format>/<fixture-id>/<mode>/\` с \`capture.json\`, \`manifest.json\` и \`reference/\`."
    echo "- \`$log_dir\` — stdout/stderr pdftoppm и импорта."
    [ "$KEEP_PDF" -eq 1 ] && echo "- \`$work_root\` — промежуточные PDF и PNG (\`--keep-pdf\`)."
} >"$report"

if [ "$KEEP_PDF" -eq 0 ]; then
    rm -rf "$work_root"
fi

printf '\n'
cat "$report"
printf '\nОтчёт: %s\n' "$report"

if [ "$imported" -eq 0 ]; then
    printf 'ПРЕДУПРЕЖДЕНИЕ: ни одного drop не импортировано; смотрите %s\n' "$report" >&2
    exit 1
fi
exit 0
