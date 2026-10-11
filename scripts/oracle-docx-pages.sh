#!/usr/bin/env bash
# Эталонное сравнение нашего PDF-экспорта DOCX с PDF от LibreOffice: число
# страниц и постраничный SSIM. Числа — материал для разбора расхождений, а не
# гейт: скрипт печатает таблицу и падает только тогда, когда сравнить не
# удалось ничего.
#
# Ни растеризация, ни SSIM здесь не пишутся своим кодом намеренно. `soffice`,
# `pdftoppm` и фильтр `ssim` в ffmpeg — реализация, не зависящая от нашего
# рендера; свой SSIM мерил бы согласие нашего кода с самим собой и был бы
# снисходителен ровно к тем ошибкам, которые ищем. Считать метрику в
# ImageMagick нельзя: в 6-й версии (Ubuntu 24.04, то есть CI) типа метрики SSIM
# нет вовсе, а `-metric SSIM` седьмой печатает distortion (SSIM = 1 − 2·distortion),
# и в отчёт попадало бы не то число. ImageMagick остаётся для размеров страниц
# (`identify`) и обрезки (`convert`).
#
# Отличие от XLSX-оракула (scripts/oracle-ssim.sh), при общей методике:
#   * эталон для DOCX — LibreOffice Writer, а не Calc (пакет в CI другой);
#   * фикстуры перебираются деревом (`test-fixtures/docx/**`), а не списком:
#     их 97 в 18 каталогах, и список пришлось бы править на каждой новой
#     фикстуре;
#   * нет отбора «однолистовых»: страница PDF DOCX — страница документа, а не
#     лист книги, и сравнивать нечего отбрасывать;
#   * фикстуры, объявленные фатальными в своём `.json` (`"fatal": "Zip"`),
#     пропускаются: наш разбор на них обязан упасть, эталон сравнивать не с чем
#     (флаг `--include-broken` возвращает их в прогон ради разбора сбоев);
#   * сравнивается ещё и число страниц — критерий DoD спринта 9
#     (ROADMAP.md:868-872): ±1 страница против эталона.
#
# Эталон — LibreOffice, а не MS Word: отклонение зафиксировано в
# docs/sprint-9/report.md («Отклонения»).
#
# Usage:
#   bash scripts/oracle-docx-pages.sh <docx_export> <каталог-фикстур> <workdir> [флаги] [фикстура...]
#
# Флаги:
#   --limit <N>      — сравнить N фикстур, равномерно разбросанных по дереву
#                      (шаг по отсортированному списку, а не первые N): быстрый
#                      прогон вместо полного и без перекоса в начало дерева;
#   --include-broken — не пропускать фикстуры с объявленной фатальностью.
#
# Фикстуры (после флагов) — пути относительно каталога фикстур
# (`tables/simple_2x2.docx`); без них берутся все `.docx` дерева.
#
# Коды выхода:
#   0 — прогон состоялся (низкий SSIM и расхождение страниц — результат, а не
#       ошибка скрипта);
#   1 — ни одну фикстуру сравнить не удалось: сломан харнесс или окружение;
#   2 — неверное употребление: меньше трёх аргументов, нет docx_export, под
#       именем не пример docx_export или нет каталога фикстур;
#   3 — нет системных утилит (soffice/pdftoppm/pdfinfo/ffmpeg/ImageMagick).
#
# Внешние утилиты есть в CI (см. .github/workflows/oracle.yml); там же прогон и
# живёт. Локально без них скрипт штатно скипается кодом 3.
set -euo pipefail
# Маска без совпадений даёт пустой список, а не слово со звёздочкой.
shopt -s nullglob

usage="usage: oracle-docx-pages.sh <docx_export> <каталог-фикстур> <workdir> [--limit N] [--include-broken] [фикстура...]"

if [ "$#" -lt 3 ]; then
  echo "$usage" >&2
  exit 2
fi

probe=$1
fixtures_dir=$2
workdir=$3
shift 3

limit=0
include_broken=0
fixtures=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --limit)
      if [ "$#" -lt 2 ] || ! [[ "$2" =~ ^[0-9]+$ ]]; then
        echo "--limit ждёт число: $usage" >&2
        exit 2
      fi
      limit=$2
      shift 2
      ;;
    --include-broken)
      include_broken=1
      shift
      ;;
    --*)
      echo "неизвестный флаг $1: $usage" >&2
      exit 2
      ;;
    *)
      fixtures+=("$1")
      shift
      ;;
  esac
done

# --- системные утилиты ------------------------------------------------------

missing=()
# ffmpeg — единственный источник SSIM, см. шапку: у ImageMagick метрики либо нет
# (6), либо она печатает distortion (7). pdfinfo — число страниц эталона и
# размеры страниц: pdftoppm их даёт лишь косвенно (числом файлов).
for tool in soffice pdftoppm pdfinfo ffmpeg; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done

# ImageMagick 7 прячет `identify` и `convert` за мультитулом `magick`; в 6
# (Ubuntu-пакет imagemagick) это отдельные бинарники. Ставим массив, а не
# строку: путь может содержать пробелы, и разбор по словам был бы ошибкой.
if command -v magick >/dev/null 2>&1; then
  im_identify=(magick identify)
  im_convert=(magick convert)
elif command -v convert >/dev/null 2>&1 && command -v identify >/dev/null 2>&1; then
  im_identify=(identify)
  im_convert=(convert)
else
  missing+=("imagemagick")
fi

if [ "${#missing[@]}" -ne 0 ]; then
  echo "нет утилит: ${missing[*]}" >&2
  echo "Ubuntu/Debian: sudo apt-get install -y --no-install-recommends libreoffice-writer poppler-utils ffmpeg imagemagick" >&2
  echo "macOS:         brew install --cask libreoffice && brew install poppler ffmpeg imagemagick" >&2
  exit 3
fi

if [ ! -f "$probe" ] || [ ! -x "$probe" ]; then
  echo "нет исполняемого $probe — соберите: cargo build --release -p doc-converter-pdf --example docx_export" >&2
  exit 2
fi

# `-x` не отличает наш пример от чужого бинарника под тем же именем: дальше
# скрипт зовёт `--out` и умирает уже после прогона эталонов. Признак свой —
# `--out` в usage; без аргументов пример печатает usage и выходит кодом 2.
probe_usage=$("$probe" 2>&1 || true)
if [[ "$probe_usage" != *"--out"* ]]; then
  echo "под именем $probe не пример docx_export — соберите: cargo build --release -p doc-converter-pdf --example docx_export" >&2
  exit 2
fi

if [ ! -d "$fixtures_dir" ]; then
  echo "нет каталога фикстур $fixtures_dir" >&2
  exit 2
fi

# soffice ждёт в `-env:UserInstallation` URL, то есть абсолютный путь, а каталоги
# страниц и диффов должны быть одним деревом на весь прогон.
mkdir -p "$workdir"
workdir=$(cd "$workdir" && pwd)
fixtures_dir=$(cd "$fixtures_dir" && pwd)
mkdir -p "$workdir/ref" "$workdir/our" "$workdir/pages/lo" "$workdir/pages/our" \
  "$workdir/diff" "$workdir/crop" "$workdir/log" "$workdir/ssim"

# Прогон без ограничения по времени рискует зависнуть до лимита джобы, поэтому
# soffice и наш пример идут под timeout(1). В macOS его нет без coreutils —
# тогда запускаем как есть и говорим об этом: молча терять ограничение хуже,
# чем предупредить.
timeout_available=1
if command -v timeout >/dev/null 2>&1; then
  run_timeout() { timeout "$@"; }
elif command -v gtimeout >/dev/null 2>&1; then
  run_timeout() { gtimeout "$@"; }
else
  timeout_available=0
  run_timeout() { shift; "$@"; }
fi
LO_TIMEOUT=120
# Наш экспорт в разы быстрее эталона (миллисекунды на фикстуру): 60 с — уже
# признак зацикливания раскладки, а не медленной машины.
OUR_TIMEOUT=60

# --- отбор фикстур ----------------------------------------------------------

if [ "${#fixtures[@]}" -eq 0 ]; then
  # Новых строк в именах фикстур не бывает, а `-print0` с ручным разбором
  # усложнил бы чтение сильнее, чем экономит.
  while IFS= read -r name; do
    [ -n "$name" ] && fixtures+=("$name")
  done < <(find "$fixtures_dir" -type f -name '*.docx' | sed "s|^$fixtures_dir/||" | LC_ALL=C sort)
fi

if [ "$limit" -gt 0 ] && [ "${#fixtures[@]}" -gt "$limit" ]; then
  stride=$(((${#fixtures[@]} + limit - 1) / limit))
  sampled=()
  for ((i = 0; i < ${#fixtures[@]} && ${#sampled[@]} < limit; i += stride)); do
    sampled+=("${fixtures[$i]}")
  done
  fixtures=("${sampled[@]}")
fi

if [ "${#fixtures[@]}" -eq 0 ]; then
  echo "в $fixtures_dir не нашлось ни одного .docx" >&2
  exit 2
fi

# --- прогон -----------------------------------------------------------------

# Результат одной фикстуры — в этих переменных: их читает цикл.
lo_pages="-"
our_pages="-"
page_delta="-"
first_ssim="-"
min_ssim="-"
mean_ssim="-"
layout_ms="-"
note=""
outcome=""
fail_rank=0
fail_status=""

# Статус сбоя не понижается: у фикстуры бывает и сбой сравнения, и сбой
# окружения — в отчёте важнее причина (окружение), а не следствие.
mark_fail() {
  if [ "$1" -gt "$fail_rank" ]; then
    fail_rank=$1
    fail_status=$2
  fi
}

# Формат подписи для отчёта, а не парсируемый диапазон: перечисление страниц
# примечанием к таблице. Фигурные скобки у имён обязательны: в bash 3.2 (macOS)
# тире сразу после `$first` съедает последний байт значения и старший байт тире.
format_page_ranges() {
  local out="" first="" last="" p
  for p in "$@"; do
    if [ -z "$last" ]; then
      first=$p
    elif [ "$p" -ne "$((last + 1))" ]; then
      if [ "$first" -eq "$last" ]; then
        out+="${out:+, }$first"
      else
        out+="${out:+, }${first}–${last}"
      fi
      first=$p
    fi
    last=$p
  done
  if [ -n "$last" ]; then
    if [ "$first" -eq "$last" ]; then
      out+="${out:+, }$first"
    else
      out+="${out:+, }${first}–${last}"
    fi
  fi
  printf '%s\n' "$out"
}

# Фикстура объявлена фатальной, если её `.json` несёт `"fatal"`: по этому
# признаку гейт парсера знает, что документ обязан не открыться. Сравнивать
# такой документ с эталоном не с чем — у нас нет ни раскладки, ни PDF.
declared_fatal() {
  local meta=$fixtures_dir/${1%.docx}.json
  [ -f "$meta" ] || return 1
  grep -o '"fatal"[[:space:]]*:[[:space:]]*"[^"]*"' "$meta" | head -1 | sed 's/.*"\([^"]*\)"$/\1/'
}

compare_fixture() {
  local name=$1
  local file=$fixtures_dir/$name
  lo_pages="-"
  our_pages="-"
  page_delta="-"
  first_ssim="-"
  min_ssim="-"
  mean_ssim="-"
  layout_ms="-"
  note=""
  outcome=""
  fail_rank=0
  fail_status=""
  local -a paper_pages=()
  # Размеры страниц запоминаем один раз на фикстуру, а страницы копим списком:
  # примечание на каждой странице давало бы у многостраничной фикстуры десяток
  # одинаковых строк об одном и том же расхождении.
  local paper_sizes="" crop_sizes=""
  local -a crop_pages=()

  if [ ! -f "$file" ]; then
    note="нет файла"
    outcome=skip
    return 0
  fi

  local fatal
  fatal=$(declared_fatal "$name") || fatal=""
  if [ -n "$fatal" ] && [ "$include_broken" -eq 0 ]; then
    note="пропущена: объявлена фатальной (fatal: $fatal) — эталону нечего сравнивать"
    outcome=skip
    return 0
  fi

  # Имя плоское: фикстуры лежат в подкаталогах, а файлы прогона — в одном
  # каталоге на все. Слэш в имени файла создал бы подкаталог, которого нет.
  local stem=${name%.docx}
  stem=${stem//\//_}
  local lo_pdf=$workdir/ref/$stem.pdf
  local our_pdf=$workdir/our/$stem.pdf
  local log=$workdir/log/$stem.log
  # soffice называет результат по имени входного файла без каталога, а прогон
  # ведётся по дереву: `broken/x.docx` и `basic/x.docx` дали бы один и тот же
  # `x.pdf` на две фикстуры. Поэтому файл эталона сразу переезжает под плоское
  # имя со каталогом (`broken_x.pdf`) — оно уникально по построению.
  local lo_raw=$workdir/ref/$(basename "${name%.docx}").pdf
  local status=0

  # Старые файлы этого же имени могли остаться от прошлого прогона и выдали бы
  # себя за результат текущего.
  rm -f "$lo_pdf" "$our_pdf" "$lo_raw"

  run_timeout "$LO_TIMEOUT" soffice --headless --norestore \
    -env:UserInstallation=file://"$workdir"/louser \
    --convert-to pdf --outdir "$workdir/ref" "$file" >"$log" 2>&1 || status=$?
  if [ "$status" -ne 0 ]; then
    if [ "$status" -eq 124 ]; then
      # Таймаут — про машину или зависший конвертер, а не про документ.
      note="soffice: таймаут $LO_TIMEOUT с (лог $log)"
      mark_fail 3 "сбой окружения"
    else
      # Ненулевой код soffice — про документ: он либо не открылся эталоном
      # (битые фикстуры `broken/**`), либо уронил конвертер (`cyclic_based_on`
      # роняет Writer в SIGSEGV). Отдельный статус, а не «сбой окружения»:
      # сравнивать тут не с чем, и в счётчике ошибок окружения такие фикстуры
      # читались бы поломкой харнесса.
      local how
      if [ "$status" -gt 128 ]; then
        # Смерть по сигналу: хвост лога тут — сообщение самого bash о падении
        # («Segmentation fault: 11»), а не диагностика конвертера.
        how="сигнал $((status - 128)) ($(kill -l $((status - 128)) 2>/dev/null || echo "?"))"
      else
        how="код $status, $(tail -1 "$log" 2>/dev/null)"
      fi
      note="эталон не открыл документ: soffice $how (лог $log)"
      mark_fail 3 "эталон не открыл"
    fi
    outcome=fail
    return 0
  fi
  # LibreOffice умеет вернуть 0 и не написать файл, поэтому мало кода выхода.
  if [ ! -s "$lo_raw" ]; then
    note="эталон не открыл документ: LibreOffice не создал $(basename "$lo_raw") (лог $log)"
    mark_fail 3 "эталон не открыл"
    outcome=fail
    return 0
  fi
  mv -f "$lo_raw" "$lo_pdf"

  status=0
  run_timeout "$OUR_TIMEOUT" "$probe" "$file" --out "$our_pdf" >"$log.our" 2>&1 || status=$?
  if [ "$status" -ne 0 ] || [ ! -s "$our_pdf" ]; then
    local reason
    reason=$(tail -1 "$log.our" 2>/dev/null) || reason=""
    note="docx_export: код $status${reason:+ — $reason}"
    mark_fail 2 "сбой экспорта"
    outcome=fail
    return 0
  fi

  # Строка контракта примера:
  # `<путь>: страниц <N>, раскладка <X.XX> мс, PDF <Y.YY> мс`. Нет её — пример
  # сменил формат, и молча сверять нули хуже, чем упасть.
  our_pages=$(grep -o 'страниц [0-9][0-9]*' "$log.our" | tail -1 | grep -o '[0-9][0-9]*') || our_pages=""
  layout_ms=$(grep -o 'раскладка [0-9][0-9.]*' "$log.our" | tail -1 | grep -o '[0-9][0-9.]*') || layout_ms=""
  if [ -z "$our_pages" ]; then
    note="docx_export: строка контракта не разобрана (лог $log.our)"
    mark_fail 1 "сбой сравнения"
    outcome=fail
    return 0
  fi

  # `LC_ALL=C` — чтобы «Pages:» не перевела локаль: ключи pdfinfo английские, но
  # раскладка зависит от LC_MESSAGES.
  lo_pages=$(LC_ALL=C pdfinfo "$lo_pdf" | awk '/^Pages:/ { print $2; exit }') || lo_pages=""
  if [ -z "$lo_pages" ]; then
    note="pdfinfo не разобрал $stem.pdf"
    mark_fail 3 "сбой окружения"
    outcome=fail
    return 0
  fi

  page_delta=$((our_pages - lo_pages))
  # Порог ±1 — критерий DoD (ROADMAP.md:868-872): страница туда-обратно на
  # разрыве абзаца не расхождение. Больше — расхождение, но не сбой скрипта:
  # это измеренная величина, и она уходит в отчёт отдельным исходом.
  if [ "$page_delta" -ge -1 ] && [ "$page_delta" -le 1 ]; then
    [ "$page_delta" -eq 0 ] || note="страниц у LO $lo_pages, у нас $our_pages — в допуске ±1"
  else
    note="страниц у LO $lo_pages, у нас $our_pages (Δ $page_delta) — расхождение больше допуска ±1"
    outcome=pages
  fi

  local lo_px=$workdir/pages/lo/$stem
  local our_px=$workdir/pages/our/$stem
  rm -f "$lo_px"-*.png "$our_px"-*.png
  if ! pdftoppm -png -gray -r 150 "$lo_pdf" "$lo_px" >"$log.lo.ppm" 2>&1; then
    note+="${note:+; }pdftoppm не растеризовал PDF LibreOffice"
    mark_fail 3 "сбой окружения"
    outcome=fail
    return 0
  fi
  if ! pdftoppm -png -gray -r 150 "$our_pdf" "$our_px" >"$log.our.ppm" 2>&1; then
    note+="${note:+; }pdftoppm не растеризовал наш PDF"
    mark_fail 2 "сбой экспорта"
    outcome=fail
    return 0
  fi

  # pdftoppm печатает номер страницы с дополнением нулями до ширины общего
  # числа страниц, поэтому лексикографический порядок имён совпадает с
  # порядком страниц — отдельный числовой ключ сортировки не нужен.
  local -a lo_imgs=()
  local -a our_imgs=()
  local f
  for f in "$lo_px"-*.png; do [ -e "$f" ] && lo_imgs+=("$f"); done
  for f in "$our_px"-*.png; do [ -e "$f" ] && our_imgs+=("$f"); done
  if [ "${#lo_imgs[@]}" -eq 0 ] || [ "${#our_imgs[@]}" -eq 0 ]; then
    note+="${note:+; }растеризация не дала страниц"
    mark_fail 1 "сбой сравнения"
    outcome=fail
    return 0
  fi

  local shared=$lo_pages
  if [ "$our_pages" -lt "$shared" ]; then shared=$our_pages; fi

  local ssim_file=$workdir/ssim/$stem.txt
  : >"$ssim_file"
  local i a b size_a size_b val
  for ((i = 0; i < shared; i++)); do
    a=${lo_imgs[$i]}
    b=${our_imgs[$i]}
    size_a=$("${im_identify[@]}" -format '%wx%h' "$a" 2>/dev/null) || size_a=""
    size_b=$("${im_identify[@]}" -format '%wx%h' "$b" 2>/dev/null) || size_b=""
    if [ -z "$size_a" ] || [ -z "$size_b" ]; then
      note+="${note:+; }страница $((i + 1)): не определил размеры страниц"
      mark_fail 1 "сбой сравнения"
      continue
    fi
    if [ "$size_a" != "$size_b" ]; then
      # Пиксельный размер страницы poppler считает как ceil(точек * dpi / 72), и
      # на одной и той же бумаге он выходит разным: наш A4 записан как 595x842 pt
      # и даёт 1240x1755, A4 LibreOffice (595.28x841.89 pt) — 1241x1754. Пара
      # пикселей — это округление сетки растеризации, а не разные бумага или
      # масштаб, поэтому такие страницы обрезаются до общего размера: обрезка, в
      # отличие от ресайза, не размывает края и не занижает метрику. Больше двух
      # пикселей — это Letter против A4 или другой масштаб (у DOCX бумага идёт из
      # `w:sectPr`, то есть из документа), и SSIM по ним ничего не значил бы.
      local w_a=${size_a%x*} h_a=${size_a#*x} w_b=${size_b%x*} h_b=${size_b#*x}
      local dw=$((w_a - w_b)) dh=$((h_a - h_b))
      dw=$((dw < 0 ? -dw : dw))
      dh=$((dh < 0 ? -dh : dh))
      if [ "$dw" -gt 2 ] || [ "$dh" -gt 2 ]; then
        paper_pages+=("$((i + 1))")
        [ -n "$paper_sizes" ] || paper_sizes="$size_a и $size_b"
        continue
      fi
      local w=$((w_a < w_b ? w_a : w_b)) h=$((h_a < h_b ? h_a : h_b))
      local crop_lo=$workdir/crop/lo-$stem-$((i + 1)).png
      local crop_our=$workdir/crop/our-$stem-$((i + 1)).png
      if ! "${im_convert[@]}" "$a" -crop "${w}x${h}+0+0" +repage "$crop_lo" 2>/dev/null ||
        ! "${im_convert[@]}" "$b" -crop "${w}x${h}+0+0" +repage "$crop_our" 2>/dev/null; then
        note+="${note:+; }страница $((i + 1)): не обрезал страницы до общего размера"
        mark_fail 1 "сбой сравнения"
        continue
      fi
      crop_pages+=("$((i + 1))")
      [ -n "$crop_sizes" ] || crop_sizes="$size_a и $size_b"
      a=$crop_lo
      b=$crop_our
    fi

    # Карта различий — артефакт для разбора, а не часть метрики: compare раньше
    # писал её третьим аргументом, теперь её собирает отдельный convert. Не
    # записалась — прогон это не валит: SSIM от неё не зависит.
    "${im_convert[@]}" "$a" "$b" -compose difference -composite \
      "$workdir/diff/$stem-$((i + 1)).png" 2>/dev/null || true

    # SSIM считает ffmpeg, а не ImageMagick: в 6-й версии (Ubuntu 24.04, CI) этого
    # типа метрики нет, а `-metric SSIM` седьмой печатает distortion, из которого
    # SSIM получается как 1 − 2·distortion, — в отчёт попало бы не то число.
    # `-lavfi ssim` даёт канонический SSIM в [−1, 1] одной реализацией в обеих
    # системах. Метрика печатается в stderr, поэтому stdout уводится в /dev/null,
    # а stderr попадает в подстановку. В отличие от compare, у которого код 1
    # означал «картинки различаются», у ffmpeg ненулевой код — всегда сбой.
    status=0
    val=$(ffmpeg -hide_banner -loglevel info -i "$a" -i "$b" -lavfi ssim -f null - 2>&1 >/dev/null) || status=$?
    if [ "$status" -ne 0 ]; then
      note+="${note:+; }страница $((i + 1)): ffmpeg упал (код $status)"
      mark_fail 1 "сбой сравнения"
      continue
    fi

    # Строка фильтра — `[Parsed_ssim_0 @ 0x…] SSIM Y:0.983609 (17.86) All:0.983609 (17.86)`;
    # нужен `All`. Для серых страниц (`-gray` у pdftoppm) Y и All совпадают, но
    # канонично именно общее число. awk, а не grep|head: на пустой находке
    # конвейер под pipefail уронил бы скрипт из-за «нет совпадений» — это не
    # ошибка растеризации. Запятая в регулярке — на случай LC_NUMERIC с запятой:
    # дальше значение уходит в файл SSIM, который читает awk уже с точкой.
    val=$(printf '%s\n' "$val" | awk 'match($0, /All:-?[0-9]+([.,][0-9]+)?/) { print substr($0, RSTART + 4, RLENGTH - 4); exit }')
    val=${val//,/.}
    if [ -z "$val" ]; then
      note+="${note:+; }страница $((i + 1)): ffmpeg не напечатал SSIM"
      mark_fail 1 "сбой сравнения"
      continue
    fi
    printf '%s\n' "$val" >>"$ssim_file"
  done

  if [ "${#paper_pages[@]}" -ne 0 ]; then
    note+="${note:+; }страницы $(format_page_ranges "${paper_pages[@]}"): размеры $paper_sizes — разные бумага или масштаб, SSIM не считан"
    mark_fail 3 "сбой окружения"
  fi
  # Обрезка — не сбой, а поправка на округление: без примечания низкий SSIM
  # такой страницы выглядел бы расхождением рендера, хотя это разница размеров.
  if [ "${#crop_pages[@]}" -ne 0 ]; then
    note+="${note:+; }страницы $(format_page_ranges "${crop_pages[@]}"): размеры $crop_sizes — расхождение ≤2 px (округление сетки растеризации), сравнено по фрагменту"
  fi

  if [ -s "$ssim_file" ]; then
    # min ловит полностью разъехавшуюся страницу, которую среднее скрыло бы;
    # mean показывает, насколько близки страницы в целом. Первая страница —
    # отдельно: критерий DoD нормирует именно её (ROADMAP.md:869). Локаль
    # прижата к C: и сравнение, и printf зависят от LC_NUMERIC (в ru_RU
    # разделитель — запятая).
    read -r first_ssim min_ssim mean_ssim < <(
      LC_ALL=C awk '{ if (NR == 1) first = $1; if (NR == 1 || $1 < min) min = $1; sum += $1 }
        END { printf "%.4f %.4f %.4f\n", first, min, sum / NR }' "$ssim_file"
    )
    [ -n "$first_ssim" ] && printf '%s\n' "$first_ssim $name" >>"$workdir/ssim-first.txt"
    # `pages` остаётся: расхождение страниц — результат измерения, а не сбой, и
    # SSIM по совпавшим страницам его не отменяет.
    [ "$outcome" = "pages" ] || outcome=ok
  else
    note+="${note:+; }SSIM не снят ни с одной страницы"
    mark_fail 1 "сбой сравнения"
    outcome=fail
  fi

  return 0
}

# Первые страницы всех фикстур — отдельным файлом: по нему считаются итоги DoD.
: >"$workdir/ssim-first.txt"
: >"$workdir/layout-ms.txt"

rows=()
ok=0
pages_failed=0
skipped=0
failed=0
ref_failed=0
env_failed=0
export_failed=0
cmp_failed=0

for name in "${fixtures[@]}"; do
  compare_fixture "$name"
  [ "$layout_ms" = "-" ] || printf '%s\n' "$layout_ms $name" >>"$workdir/layout-ms.txt"
  case "$outcome" in
    ok) ok=$((ok + 1)); row_status="сравнено" ;;
    pages) pages_failed=$((pages_failed + 1)); row_status="расхождение страниц" ;;
    skip) skipped=$((skipped + 1)); row_status="пропущено" ;;
    *) failed=$((failed + 1))
       row_status=$fail_status
       case "$fail_status" in
         "эталон не открыл") ref_failed=$((ref_failed + 1)) ;;
         "сбой окружения") env_failed=$((env_failed + 1)) ;;
         "сбой экспорта") export_failed=$((export_failed + 1)) ;;
         *) cmp_failed=$((cmp_failed + 1)) ;;
       esac ;;
  esac
  note=${note:-—}
  note=${note//|/\\|}
  rows+=("| $name | $lo_pages | $our_pages | $page_delta | $first_ssim | $min_ssim | $mean_ssim | $layout_ms | $row_status | $note |")
done

compared=$((ok + pages_failed))

# --- отчёт ------------------------------------------------------------------

report=$workdir/report.md
# Версия эталона — часть результата: по ней видно, каким LibreOffice получены
# числа (между версиями Writer менялась посадка строк). `--version` печатает
# несколько строк, версия — первая.
soffice_version=$(soffice --version 2>/dev/null | head -1) || true
[ -n "$soffice_version" ] || soffice_version="неизвестна"

summary=$(
  LC_ALL=C awk '
    { if (NR == 1 || $1 < min) min = $1; sum += $1; n++
      if ($1 >= 0.95) good++ }
    END {
      if (n == 0) { print "SSIM первой страницы: ни одной страницы не сравнено"; exit }
      printf "SSIM первой страницы: min %.4f, mean %.4f, ≥0.95 у %d из %d\n", min, sum / n, good, n
    }' "$workdir/ssim-first.txt"
)
layout_summary=$(
  LC_ALL=C awk '
    { if ($1 > max) { max = $1; worst = $2 } sum += $1; n++ }
    END {
      if (n == 0) { print "раскладка: ни одной фикстуры не разложено"; exit }
      printf "раскладка: max %.2f мс (%s), mean %.2f мс, замеров %d\n", max, worst, sum / n, n
    }' "$workdir/layout-ms.txt"
)
# Распределение Δ = наши страницы − страницы эталона: по нему видно, куда
# смещена пагинация, а «в допуске ±1» этого не показывает. Поле 5 строки
# таблицы (поля режет `-F'|'`: первое — пустое, до первой черты).
delta_line=$(printf '%s\n' "${rows[@]}" |
  LC_ALL=C awk -F'|' '{ d = $5; gsub(/ /, "", d); if (d != "-") print d }' |
  sort -n | uniq -c | LC_ALL=C awk '{ printf " %s→%s;", $2, $1 }')
[ -n "$delta_line" ] || delta_line=" пусто"

# Десять худших первых страниц: разбор расхождений начинается с них, а не с
# таблицы на сотню строк. Строки файла — «<ssim> <имя>», поэтому ключ первый.
worst=$(sort -n "$workdir/ssim-first.txt" | head -10 | LC_ALL=C awk '{ printf "| %s | %.4f |\n", $2, $1 }')

# «Эталон не открыл» — отдельным слагаемым: битые фикстуры `broken/**` до
# сравнения не доходят, и в «ошибке окружения» они выглядели бы поломкой
# харнесса, которой нет. В счётчик failed они входят: если эталон не открыл ни
# одного документа прогона, это уже про окружение.
ref_clause=""
if [ "$ref_failed" -gt 0 ]; then
  ref_clause=", эталон не открыл: $ref_failed"
fi

{
  echo "# DOCX: наш PDF против LibreOffice"
  echo
  echo "Число страниц и постраничный SSIM: ffmpeg \`-lavfi ssim\`, 1.0 — страницы"
  echo "совпали. Растеризация \`pdftoppm -png -gray -r 150\`; режим \`-gray\`"
  echo "намеренно слеп к цвету — метрика ловит геометрию, шрифт и пагинацию, а"
  echo "совпадение заливок и цветов текста ею не проверяется."
  echo
  echo "Эталон — LibreOffice Writer (не MS Word): отклонение зафиксировано в"
  echo "\`docs/sprint-9/report.md\`."
  echo
  echo "Прогон: $(date -u '+%Y-%m-%d %H:%M UTC')"
  echo "Эталон: $soffice_version; локаль LANG=${LANG:-(не задана)} LC_ALL=${LC_ALL:-(не задана)}"
  echo "Фикстур в прогоне: ${#fixtures[@]} из $(find "$fixtures_dir" -type f -name '*.docx' | wc -l | tr -d ' ')"
  if [ "$timeout_available" -eq 0 ]; then
    echo
    echo "**Внимание:** timeout(1) не найден — soffice и docx_export запускались без"
    echo "ограничения по времени (поставьте coreutils для \`timeout\`/\`gtimeout\`)."
  fi
  echo
  echo "| фикстура | страниц у LO | наших | Δ | SSIM 1-й | min | mean | раскладка, мс | статус | примечание |"
  echo "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
  printf '%s\n' "${rows[@]}"
  echo
  echo "## Итоги"
  echo
  echo "- Число страниц ±1 (DoD, ROADMAP.md:868): **$((ok)) из $compared** сравнённых фикстур; расхождений больше допуска — $pages_failed."
  echo "- $summary"
  echo "- $layout_summary"
  echo "- $delta_line"
  if [ -n "$worst" ]; then
    echo
    echo "## Худшие 10 (SSIM первой страницы)"
    echo
    echo "| фикстура | SSIM 1-й |"
    echo "| --- | --- |"
    printf '%s\n' "$worst"
  fi
  echo
  if [ "$failed" -gt 0 ]; then
    echo "Сравнено: $compared (в допуске: $ok, расхождение страниц: $pages_failed), пропущено: $skipped${ref_clause}, с ошибкой: $failed (окружение: $env_failed, экспорт: $export_failed, сравнение: $cmp_failed)."
  else
    echo "Сравнено: $compared (в допуске: $ok, расхождение страниц: $pages_failed), пропущено: $skipped${ref_clause}, с ошибкой: $failed."
  fi
  if [ "${#fixtures[@]}" -lt 50 ]; then
    echo
    echo "Фикстур меньше 50: выборка не закрывает DoD «±1 страница на 50 фикстурах» —"
    echo "это прогон по части дерева (\`--limit\` или явный список)."
  fi
} | tee "$report"

# Ноль сравнений при хотя бы одной упавшей фикстуре означает, что не сработал
# конвейер целиком (нет soffice нужной версии, сломан экспорт) — это состояние
# харнесса, и о нём CI должен узнать падением. Прогон, где всё пропущено, —
# нормальный исход отбора, он остаётся зелёным.
if [ "$compared" -eq 0 ] && [ "$failed" -gt 0 ]; then
  echo "ни одну фикстуру сравнить не удалось — смотрите логи в $workdir/log" >&2
  exit 1
fi
