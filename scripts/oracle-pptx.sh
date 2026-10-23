#!/usr/bin/env bash
# Эталонное сравнение нашего PDF-экспорта PPTX с PDF от LibreOffice Impress:
# постраничный SSIM слайдов. Числа — материал для разбора расхождений, а не
# гейт: скрипт печатает таблицу и падает только тогда, когда сравнить не
# удалось ничего.
#
# Растеризация и SSIM — системные утилиты (`soffice`, `pdftoppm`, `ssim`-фильтр
# ffmpeg), как у scripts/oracle-ssim.sh: свой SSIM мерил бы согласие нашего
# кода с самим собой.
#
# Usage:
#   bash scripts/oracle-pptx.sh <export-example> <каталог-фикстур> <workdir> [фикстура...]
#
# Коды выхода:
#   0 — прогон состоялся (низкий SSIM — результат, а не ошибка скрипта);
#   1 — ни одну фикстуру сравнить не удалось;
#   2 — неверное употребление;
#   3 — нет системных утилит (soffice/pdftoppm/ffmpeg/unzip).
set -euo pipefail
shopt -s nullglob

if [ "$#" -lt 3 ]; then
  echo "usage: oracle-pptx.sh <export-example> <каталог-фикстур> <workdir> [фикстура...]" >&2
  exit 2
fi

probe=$1
fixtures_dir=$2
workdir=$3
shift 3

default_fixtures=(
  simple/two_slides.pptx
  shapes/geometry.pptx
  images/picture.pptx
  charts/bar.pptx
  text/bullets.pptx
)

if [ "$#" -gt 0 ]; then
  fixtures=("$@")
else
  fixtures=("${default_fixtures[@]}")
fi

missing=()
for tool in soffice pdftoppm ffmpeg unzip; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -ne 0 ]; then
  echo "нет утилит: ${missing[*]}" >&2
  echo "Ubuntu/Debian: sudo apt-get install -y --no-install-recommends libreoffice-impress poppler-utils ffmpeg unzip" >&2
  exit 3
fi

mkdir -p "$workdir"
compared=0
echo "фикстура;страниц;средний SSIM"
for fixture in "${fixtures[@]}"; do
  src="$fixtures_dir/$fixture"
  [ -f "$src" ] || { echo "$fixture;0;нет фикстуры" >&2; continue; }

  # Эталон: LibreOffice Impress печатает всю презентацию в PDF.
  ref_pdf="$workdir/ref.pdf"
  rm -rf "$workdir/soffice" "$workdir/ref" "$workdir/ours"
  mkdir -p "$workdir/soffice" "$workdir/ref" "$workdir/ours"
  if ! soffice --headless --convert-to pdf --outdir "$workdir/soffice" "$src" >/dev/null 2>&1; then
    echo "$fixture;0;soffice не конвертировал" >&2
    continue
  fi
  ref=$(find "$workdir/soffice" -name '*.pdf' -print -quit)
  [ -n "$ref" ] || { echo "$fixture;0;нет эталонного PDF" >&2; continue; }

  # Наш экспорт — тот же путь, что у wasm: разбор → pptx-pdf.
  ours_pdf="$workdir/ours.pdf"
  if ! "$probe" "$src" "$ours_pdf" >/dev/null 2>&1; then
    echo "$fixture;0;наш экспорт упал" >&2
    continue
  fi

  # Постраничный SSIM: страницы у обоих PDF одни и те же (слайд = страница),
  # размер бумаги тоже задан документом (p:sldSz). Растеризация — в пиксельный
  # размер нашей страницы, а не по `-r`: LibreOffice пишет 960.009 pt там, где
  # у нас ровно 960, и при 150 dpi эталон выходил на пиксель шире. `ssim`
  # требует одинаковых размеров и на расхождении в пиксель падает.
  read -r page_w page_h < <(pdfinfo "$ours_pdf" | awk '/^Page size:/{printf "%d %d\n", $3, $5}')
  pdftoppm -png -scale-to-x "$page_w" -scale-to-y "$page_h" "$ref" "$workdir/ref/page" >/dev/null 2>&1
  pdftoppm -png -scale-to-x "$page_w" -scale-to-y "$page_h" "$ours_pdf" "$workdir/ours/page" >/dev/null 2>&1
  pages=0
  total=0
  for ref_page in "$workdir"/ref/page-*.png; do
    [ -f "$ref_page" ] || break
    number=${ref_page##*-}
    number=${number%.png}
    ours_page="$workdir/ours/page-$number.png"
    [ -f "$ours_page" ] || break
    value=$(ffmpeg -i "$ref_page" -i "$ours_page" -lavfi ssim -f null - 2>&1 |
      grep -o 'All:[0-9.]*' | tail -1 | cut -d: -f2)
    [ -n "$value" ] || break
    pages=$((pages + 1))
    total=$(awk "BEGIN { printf \"%.4f\", $total + $value }")
  done
  if [ "$pages" -eq 0 ]; then
    echo "$fixture;0;страницы не совпали" >&2
    continue
  fi
  mean=$(awk "BEGIN { printf \"%.4f\", $total / $pages }")
  echo "$fixture;$pages;$mean"
  compared=$((compared + 1))
done

if [ "$compared" -eq 0 ]; then
  echo "ни одну фикстуру сравнить не удалось" >&2
  exit 1
fi
