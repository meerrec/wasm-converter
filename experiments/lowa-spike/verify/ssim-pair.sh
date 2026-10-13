#!/usr/bin/env bash
#
# Постраничный SSIM двух произвольных PDF — эталонная сторона гипотезы H6
# спайка LOWA («PDF от LOWA vs нативный soffice, SSIM ≥ 0,99»).
#
# Методика намеренно повторяет scripts/oracle-ssim.sh: тот же растеризатор,
# тот же фильтр и тот же разбор строки метрики. Число, снятое другой
# методикой, нельзя поставить рядом с оракульными — сравнение было бы между
# методиками, а не между PDF.
#
#   bash ssim-pair.sh <pdf-a> <pdf-b> [workdir]
#
# Код возврата:
#   0 — сравнение состоялось (низкий SSIM — результат, а не сбой скрипта);
#   1 — сбой сравнения (растеризация, ffmpeg, разбор метрики);
#   2 — неверные аргументы;
#   3 — нет системных утилит.
set -euo pipefail
# Маска без совпадений даёт пустой список, а не слово со звёздочкой.
shopt -s nullglob

usage="usage: ssim-pair.sh <pdf-a> <pdf-b> [workdir]"

if [ "$#" -lt 2 ]; then
  echo "$usage" >&2
  exit 2
fi

pdf_a=$1
pdf_b=$2
workdir=${3:-}

for pdf in "$pdf_a" "$pdf_b"; do
  if [ ! -f "$pdf" ]; then
    echo "нет файла: $pdf" >&2
    exit 1
  fi
done

# --- системные утилиты ------------------------------------------------------

# ffmpeg — единственный источник SSIM (у ImageMagick 6 метрики нет вовсе, а
# `-metric SSIM` седьмой печатает distortion, из которого SSIM выходит как
# 1 − 2·distortion, — в отчёт попало бы не то число). ffprobe нужен для
# размеров страниц и берётся из того же пакета, отдельной утилитой не является.
missing=()
for tool in pdftoppm ffmpeg ffprobe; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -ne 0 ]; then
  echo "нет утилит: ${missing[*]}" >&2
  echo "Ubuntu/Debian: sudo apt-get install -y --no-install-recommends poppler-utils ffmpeg" >&2
  echo "macOS:         brew install poppler ffmpeg" >&2
  exit 3
fi

if [ -z "$workdir" ]; then
  workdir=$(mktemp -d "${TMPDIR:-/tmp}/ssim-pair.XXXXXX")
else
  mkdir -p "$workdir"
fi
mkdir -p "$workdir/pages/a" "$workdir/pages/b" "$workdir/crop"
rm -f "$workdir"/pages/a/*.png "$workdir"/pages/b/*.png

echo "A: $pdf_a"
echo "B: $pdf_b"
echo "workdir: $workdir"

# --- растеризация -----------------------------------------------------------

# `-gray`: сравнение в оттенках серого. Цветовые расхождения так не ловятся —
# это осознанная цена: оракульный скрипт сравнивает серые страницы, и цветной
# прогон дал бы числа, несопоставимые с ним. `-r 150` — та же плотность, что
# у оракула: SSIM зависит от разрешения, и другая плотность сдвинула бы числа.
px_a=$workdir/pages/a/page
px_b=$workdir/pages/b/page
if ! pdftoppm -png -gray -r 150 "$pdf_a" "$px_a" >"$workdir/pdftoppm-a.log" 2>&1; then
  echo "pdftoppm не растеризовал A: $pdf_a" >&2
  exit 1
fi
if ! pdftoppm -png -gray -r 150 "$pdf_b" "$px_b" >"$workdir/pdftoppm-b.log" 2>&1; then
  echo "pdftoppm не растеризовал B: $pdf_b" >&2
  exit 1
fi

# pdftoppm печатает номер страницы с дополнением нулями до ширины общего числа
# страниц, поэтому лексикографический порядок имён совпадает с порядком
# страниц — отдельный числовой ключ сортировки не нужен.
imgs_a=("$px_a"-*.png)
imgs_b=("$px_b"-*.png)
pages_a=${#imgs_a[@]}
pages_b=${#imgs_b[@]}

if [ "$pages_a" -eq 0 ] || [ "$pages_b" -eq 0 ]; then
  echo "растеризация не дала страниц (A: $pages_a, B: $pages_b)" >&2
  exit 1
fi

echo "страниц у A: $pages_a"
echo "страниц у B: $pages_b"

shared=$pages_a
[ "$pages_b" -lt "$shared" ] && shared=$pages_b

# Разное число страниц — самостоятельный результат измерения, а не деталь
# реализации: молчаливое усечение до общей длины выдало бы «страницы совпали»
# за документы, у которых разошёлся даже объём.
if [ "$pages_a" -ne "$pages_b" ]; then
  echo "ВНИМАНИЕ: число страниц разное — у A $pages_a, у B $pages_b; сравнены первые $shared"
fi

# --- попиксельное сравнение -------------------------------------------------

ssim_file=$workdir/ssim.txt
: >"$ssim_file"
skipped=0

for ((i = 0; i < shared; i++)); do
  a=${imgs_a[$i]}
  b=${imgs_b[$i]}
  page=$((i + 1))

  size_a=$(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0:s=x "$a" 2>/dev/null) || size_a=""
  size_b=$(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0:s=x "$b" 2>/dev/null) || size_b=""
  if [ -z "$size_a" ] || [ -z "$size_b" ]; then
    echo "страница $page: не определил размеры страниц — SSIM не считан"
    skipped=$((skipped + 1))
    continue
  fi

  if [ "$size_a" != "$size_b" ]; then
    # Пиксельный размер poppler считает как ceil(точек * dpi / 72), и на одной
    # и той же бумаге он выходит разным: A4 записанный как 595x842 pt даёт
    # 1240x1755, A4 в 595.28x841.89 pt — 1241x1754. Пара пикселей — округление
    # сетки растеризации, и такие страницы обрезаются до общего размера:
    # обрезка, в отличие от ресайза, не размывает края и не занижает метрику.
    # Больше двух пикселей — это разная бумага или масштаб, и SSIM по ним
    # ничего не значил бы, поэтому страница пропускается явной строкой.
    w_a=${size_a%x*} h_a=${size_a#*x}
    w_b=${size_b%x*} h_b=${size_b#*x}
    dw=$((w_a - w_b))
    dh=$((h_a - h_b))
    dw=$((dw < 0 ? -dw : dw))
    dh=$((dh < 0 ? -dh : dh))
    if [ "$dw" -gt 2 ] || [ "$dh" -gt 2 ]; then
      echo "страница $page: размеры $size_a и $size_b — разные бумага или масштаб, SSIM не считан"
      skipped=$((skipped + 1))
      continue
    fi
    w=$((w_a < w_b ? w_a : w_b))
    h=$((h_a < h_b ? h_a : h_b))
    crop_a=$workdir/crop/a-$page.png
    crop_b=$workdir/crop/b-$page.png
    # Обрезка ffmpeg'ом, а не ImageMagick: ffmpeg уже обязателен для SSIM, и
    # лишняя зависимость в скрипте сравнения не нужна.
    ffmpeg -hide_banner -loglevel error -i "$a" -vf "crop=$w:$h" -y "$crop_a" 2>/dev/null || true
    ffmpeg -hide_banner -loglevel error -i "$b" -vf "crop=$w:$h" -y "$crop_b" 2>/dev/null || true
    [ -s "$crop_a" ] && a=$crop_a
    [ -s "$crop_b" ] && b=$crop_b
    note=" (размеры $size_a и $size_b — расхождение ≤2 px, сравнено по фрагменту)"
  else
    note=""
  fi

  # Метрика печатается в stderr, поэтому stdout уводится в /dev/null, а stderr
  # попадает в подстановку. У ffmpeg ненулевой код — всегда сбой (в отличие от
  # `compare`, где код 1 означал «картинки различаются»).
  status=0
  out=$(ffmpeg -hide_banner -loglevel info -i "$a" -i "$b" -lavfi ssim -f null - 2>&1 >/dev/null) || status=$?
  if [ "$status" -ne 0 ]; then
    echo "страница $page: ffmpeg упал (код $status) — SSIM не считан"
    skipped=$((skipped + 1))
    continue
  fi

  # Строка фильтра — `[Parsed_ssim_0 @ 0x…] SSIM Y:0.983609 (17.86) All:0.983609 (17.86)`;
  # берётся `All`. Для серых страниц (`-gray` у pdftoppm) Y и All совпадают, но
  # канонично именно общее число. awk, а не grep|head: на пустой находке
  # конвейер под pipefail уронил бы скрипт из-за «нет совпадений» — это не
  # ошибка растеризации. Запятая в регулярке — на случай LC_NUMERIC с запятой.
  val=$(printf '%s\n' "$out" | awk 'match($0, /All:-?[0-9]+([.,][0-9]+)?/) { print substr($0, RSTART + 4, RLENGTH - 4); exit }')
  val=${val//,/.}
  if [ -z "$val" ]; then
    echo "страница $page: ffmpeg не напечатал SSIM — метрика не считана"
    skipped=$((skipped + 1))
    continue
  fi

  printf '%s\n' "$val" >>"$ssim_file"
  printf '| %d | %s |%s\n' "$page" "$val" "$note"
done

# --- итог -------------------------------------------------------------------

echo
if [ ! -s "$ssim_file" ]; then
  echo "SSIM не снят ни с одной страницы — сравнивать нечего"
  exit 1
fi

# min ловит полностью разъехавшуюся страницу, которую среднее скрыло бы; mean
# показывает, насколько близки страницы в целом. Локаль прижата к C: и
# сравнение, и printf зависят от LC_NUMERIC (в ru_RU разделитель — запятая).
read -r min_ssim mean_ssim < <(
  LC_ALL=C awk '{ if (NR == 1 || $1 < min) min = $1; sum += $1 } END { printf "%.6f %.6f\n", min, sum / NR }' "$ssim_file"
)
count=$(wc -l <"$ssim_file" | tr -d ' ')

echo "страниц у A: $pages_a, у B: $pages_b; сравнено $shared, SSIM снят с $count, пропущено $skipped"
echo "min:  $min_ssim"
echo "mean: $mean_ssim"
