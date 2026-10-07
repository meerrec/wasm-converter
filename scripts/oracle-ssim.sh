#!/usr/bin/env bash
# Эталонное сравнение нашего PDF-экспорта XLSX с PDF от LibreOffice: постраничный
# SSIM. Числа — материал для разбора расхождений, а не гейт: скрипт печатает
# таблицу и падает только тогда, когда сравнить не удалось ничего.
#
# Ни растеризация, ни SSIM здесь не пишутся своим кодом намеренно. `soffice`,
# `pdftoppm` и ImageMagick `compare` — реализация, не зависящая от нашего
# рендера; свой SSIM мерил бы согласие нашего кода с самим собой и был бы
# снисходителен ровно к тем ошибкам, которые ищем.
#
# Usage:
#   bash scripts/oracle-ssim.sh <mem_probe> <каталог-фикстур> <workdir> [фикстура...]
#
# Коды выхода:
#   0 — прогон состоялся (низкий SSIM — результат, а не ошибка скрипта);
#   1 — ни одну фикстуру сравнить не удалось: сломан харнесс или окружение;
#   2 — неверное употребление: нет mem_probe или каталога фикстур;
#   3 — нет системных утилит (soffice/pdftoppm/unzip/ImageMagick).
#
# Только однолистовые книги. Наш экспорт рисует лист 0, а `soffice` печатает всю
# книгу: на второй странице эталона окажется второй лист, у нас — вторая
# страница первого, и постраничный SSIM мерил бы совпадение разных листов.
# Отбор идёт по числу `<sheet ` в xl/workbook.xml, многолистовые книги
# пропускаются с примечанием. Следующий шаг к ним — экспорт по индексу листа
# (`PdfOptions::sheet_index` уже есть): тогда пары «лист книги ↔ страницы
# эталона» станут осмысленными.
#
# Бумага: у фикстур не задан `paperSize`, поэтому эталон печатает дефолт
# LibreOffice, а тот берётся из локали его профиля (см. заведение профиля
# ниже), а не из LANG и не из /etc/papersize — проверено прогонами 37538750822
# и 37560908632. Профиль прописывает метрическую локаль, чтобы дефолт стал A4,
# как у нашего экспорта. Размеры страниц всё равно сверяются до SSIM:
# расхождение означает разные бумагу или масштаб, и число по таким страницам
# ничего не значило бы.
#
# Внешние утилиты есть в CI (см. .github/workflows/oracle.yml); там же прогон и
# живёт. Локально без них скрипт штатно скипается кодом 3.
set -euo pipefail

usage="usage: oracle-ssim.sh <mem_probe> <каталог-фикстур> <workdir> [фикстура...]"

probe=${1:?$usage}
fixtures_dir=${2:?$usage}
workdir=${3:?$usage}
shift 3

# Умолчание — фикстуры, выбранные под разные части рендера: простые значения,
# числовые форматы, границы, заливки, шрифты, объединённые ячейки, условное
# форматирование, картинки, диаграммы, гиперссылки, кириллица с переносом и
# многопстраничная книга (единственная здесь проверяет пагинацию).
#
# Комментариев в списке нет: единственная книга с примечаниями
# (comments-legacy.xlsx) — двухлистовая, и на ней постраничное сравнение как раз
# и ломается; она вернётся в список вместе с экспортом по индексу листа.
default_fixtures=(
  values-numbers.xlsx
  values-strings.xlsx
  formats-date-ru.xlsx
  styles-border-styles.xlsx
  styles-font-names.xlsx
  layout-merged.xlsx
  cf-color-scale.xlsx
  images-png.xlsx
  charts-five-kinds.xlsx
  layout-links.xlsx
  text-cyrillic-wrap.xlsx
  scale-ten-pages.xlsx
)

if [ "$#" -gt 0 ]; then
  fixtures=("$@")
else
  fixtures=("${default_fixtures[@]}")
fi

# --- системные утилиты ------------------------------------------------------

missing=()
for tool in soffice pdftoppm unzip; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done

# ImageMagick 7 прячет `compare` за мультитулом `magick`; в 6 (Ubuntu-пакет
# imagemagick) это отдельный бинарник. Ставим массив, а не строку: путь может
# содержать пробелы, и разбор по словам был бы ошибкой.
if command -v magick >/dev/null 2>&1; then
  im_compare=(magick compare)
  im_identify=(magick identify)
  im_convert=(magick convert)
elif command -v compare >/dev/null 2>&1 && command -v convert >/dev/null 2>&1; then
  im_compare=(compare)
  im_identify=(identify)
  im_convert=(convert)
else
  missing+=("imagemagick")
fi

if [ "${#missing[@]}" -ne 0 ]; then
  echo "нет утилит: ${missing[*]}" >&2
  echo "Ubuntu/Debian: sudo apt-get install -y --no-install-recommends libreoffice-calc poppler-utils imagemagick unzip" >&2
  echo "macOS:         brew install --cask libreoffice && brew install poppler imagemagick (unzip в системе)" >&2
  exit 3
fi

if [ ! -x "$probe" ]; then
  echo "нет исполняемого $probe — соберите: cargo build --release -p doc-converter-pdf --example mem_probe" >&2
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

# LibreOffice берёт бумагу документа, у которого не задан `paperSize`, из локали
# своего профиля: у свежего headless-профиля `ooSetupSystemLocale` пуст, и он
# уходит на en-US → Letter. Ни `LANG`/`LC_ALL` (прогон 37538750822), ни
# `/etc/papersize` (37560908632) на это не влияют. Заводим профиль заранее и
# прописываем метрическую локаль — дефолт становится A4 и совпадает с нашим
# экспортом; LibreOffice этот файл дополняет, а не перезаписывает.
lo_profile=$workdir/louser
mkdir -p "$lo_profile/user"
cat >"$lo_profile/user/registrymodifications.xcu" <<'XCU'
<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
 <item oor:path="/org.openoffice.Setup/L10N"><prop oor:name="ooSetupSystemLocale" oor:op="fuse"><value>ru_RU</value></prop></item>
</oor:items>
XCU

# Прогон без ограничения по времени рискует зависнуть до лимита джобы, поэтому
# soffice идёт под timeout(1). В macOS его нет без coreutils — тогда запускаем
# как есть и говорим об этом: молча терять ограничение хуже, чем предупредить.
if command -v timeout >/dev/null 2>&1; then
  lo_timeout() { timeout 120 "$@"; }
elif command -v gtimeout >/dev/null 2>&1; then
  lo_timeout() { gtimeout 120 "$@"; }
else
  echo "предупреждение: timeout(1) нет — soffice запускается без ограничения в 120 с" >&2
  lo_timeout() { "$@"; }
fi

# --- прогон -----------------------------------------------------------------

# Результат одной фикстуры — в этих переменных: их читает цикл.
lo_pages="-"
our_pages="-"
min_ssim="-"
mean_ssim="-"
note=""
outcome=""

compare_fixture() {
  local name=$1
  local file=$fixtures_dir/$name
  lo_pages="-"
  our_pages="-"
  min_ssim="-"
  mean_ssim="-"
  note=""
  outcome=""

  if [ ! -f "$file" ]; then
    note="нет файла"
    outcome=skip
    return 0
  fi

  # `<sheets>` не совпадёт с шаблоном `<sheet ` (пробел в конце) — считаются
  # только сами листы.
  local sheets
  sheets=$(unzip -p "$file" xl/workbook.xml 2>/dev/null | grep -o '<sheet ' | wc -l | tr -d ' ') || true
  if [ -z "$sheets" ] || [ "$sheets" -eq 0 ]; then
    note="не разобрал xl/workbook.xml"
    outcome=skip
    return 0
  fi
  if [ "$sheets" -ne 1 ]; then
    note="пропущена: в книге $sheets листа — наш экспорт берёт лист 0, эталон печатает всю книгу"
    outcome=skip
    return 0
  fi

  local stem=${name%.xlsx}
  local lo_pdf=$workdir/ref/$stem.pdf
  local our_pdf=$workdir/our/$stem.pdf
  local log=$workdir/log/$stem.log
  local status=0

  # Старые файлы этого же имени могли остаться от прошлого прогона и выдали бы
  # себя за результат текущего.
  rm -f "$lo_pdf" "$our_pdf"

  lo_timeout soffice --headless --norestore \
    -env:UserInstallation=file://"$workdir"/louser \
    --convert-to pdf --outdir "$workdir/ref" "$file" >"$log" 2>&1 || status=$?
  if [ "$status" -ne 0 ]; then
    if [ "$status" -eq 124 ]; then
      note="soffice: таймаут 120 с"
    else
      note="soffice: код $status (лог $log)"
    fi
    outcome=fail
    return 0
  fi
  # LibreOffice умеет вернуть 0 и не написать файл, поэтому мало кода выхода.
  if [ ! -s "$lo_pdf" ]; then
    note="LibreOffice не создал $stem.pdf (лог $log)"
    outcome=fail
    return 0
  fi

  status=0
  "$probe" "$file" --out "$our_pdf" >"$log.our" 2>&1 || status=$?
  if [ "$status" -ne 0 ] || [ ! -s "$our_pdf" ]; then
    note="mem_probe: код $status, PDF не создан (лог $log.our)"
    outcome=fail
    return 0
  fi

  local lo_px=$workdir/pages/lo/$stem
  local our_px=$workdir/pages/our/$stem
  rm -f "$lo_px"-*.png "$our_px"-*.png
  if ! pdftoppm -png -gray -r 150 "$lo_pdf" "$lo_px" >"$log.lo.ppm" 2>&1; then
    note="pdftoppm не растеризовал PDF LibreOffice"
    outcome=fail
    return 0
  fi
  if ! pdftoppm -png -gray -r 150 "$our_pdf" "$our_px" >"$log.our.ppm" 2>&1; then
    note="pdftoppm не растеризовал наш PDF"
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
    note="растеризация не дала страниц"
    outcome=fail
    return 0
  fi

  lo_pages=${#lo_imgs[@]}
  our_pages=${#our_imgs[@]}
  local shared=$lo_pages
  if [ "$our_pages" -lt "$shared" ]; then shared=$our_pages; fi
  if [ "$lo_pages" -ne "$our_pages" ]; then
    note="страниц у LO $lo_pages, у нас $our_pages — сравнены первые $shared"
  fi

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
      continue
    fi
    if [ "$size_a" != "$size_b" ]; then
      # Пиксельный размер страницы poppler считает как ceil(точек * dpi / 72), и
      # на одной и той же бумаге он выходит разным: наш A4 записан как 595x842 pt
      # и даёт 1240x1755, A4 LibreOffice (595.28x841.89 pt) — 1241x1754. Пара
      # пикселей — это округление сетки растеризации, а не разные бумага или
      # масштаб, поэтому такие страницы обрезаются до общего размера: обрезка, в
      # отличие от ресайза, не размывает края и не занижает метрику. Больше двух
      # пикселей — это Letter против A4 или другой масштаб, и SSIM по ним ничего
      # не значил бы.
      local w_a=${size_a%x*} h_a=${size_a#*x} w_b=${size_b%x*} h_b=${size_b#*x}
      local dw=$((w_a - w_b)) dh=$((h_a - h_b))
      [ "$dw" -lt 0 ] && dw=$((-dw))
      [ "$dh" -lt 0 ] && dh=$((-dh))
      if [ "$dw" -gt 2 ] || [ "$dh" -gt 2 ]; then
        note+="${note:+; }страница $((i + 1)): размеры $size_a и $size_b — разные бумага или масштаб, SSIM не считан"
        continue
      fi
      local w=$((w_a < w_b ? w_a : w_b)) h=$((h_a < h_b ? h_a : h_b))
      local crop_lo=$workdir/crop/lo-$stem-$((i + 1)).png
      local crop_our=$workdir/crop/our-$stem-$((i + 1)).png
      if ! "${im_convert[@]}" "$a" -crop "${w}x${h}+0+0" +repage "$crop_lo" 2>/dev/null ||
        ! "${im_convert[@]}" "$b" -crop "${w}x${h}+0+0" +repage "$crop_our" 2>/dev/null; then
        note+="${note:+; }страница $((i + 1)): не обрезал страницы до общего размера"
        continue
      fi
      note+="${note:+; }страница $((i + 1)): размеры $size_a и $size_b — округление сетки растеризации, сравнено по фрагменту ${w}x${h}"
      a=$crop_lo
      b=$crop_our
    fi

    # Метрика печатается в stderr, поэтому stdout уводится в /dev/null, а stderr
    # попадает в подстановку. Код 1 у compare — не сбой, а «картинки
    # различаются»: это и есть измеряемый случай. Сбой — код 2, его и ловим.
    status=0
    val=$("${im_compare[@]}" -metric SSIM "$a" "$b" "$workdir/diff/$stem-$((i + 1)).png" 2>&1 >/dev/null) || status=$?
    if [ "$status" -ge 2 ]; then
      note+="${note:+; }страница $((i + 1)): compare упал (код $status)"
      continue
    fi

    # IM6 печатает «0.987654», IM7 — «0.987654 (0.987654)»; первое число — сам
    # SSIM. awk, а не grep|head: на пустой находке конвейер под pipefail уронил
    # бы скрипт из-за «нет совпадений» — это не ошибка растеризации.
    val=$(printf '%s\n' "$val" | awk 'match($0, /-?[0-9]+([.][0-9]+)?([eE][-+]?[0-9]+)?/) { print substr($0, RSTART, RLENGTH); exit }')
    if [ -z "$val" ]; then
      note+="${note:+; }страница $((i + 1)): compare не напечатал SSIM"
      continue
    fi
    printf '%s\n' "$val" >>"$ssim_file"
  done

  if [ -s "$ssim_file" ]; then
    # min ловит полностью разъехавшуюся страницу, которую среднее скрыло бы;
    # mean показывает, насколько близки страницы в целом.
    read -r min_ssim mean_ssim < <(
      awk '{ if (NR == 1 || $1 < min) min = $1; sum += $1 } END { printf "%.4f %.4f\n", min, sum / NR }' "$ssim_file"
    )
    outcome=ok
  else
    note+="${note:+; }SSIM не снят ни с одной страницы"
    outcome=fail
  fi

  return 0
}

rows=()
ok=0
skipped=0
failed=0

for name in "${fixtures[@]}"; do
  compare_fixture "$name"
  case "$outcome" in
    ok) ok=$((ok + 1)) ;;
    skip) skipped=$((skipped + 1)) ;;
    *) failed=$((failed + 1)) ;;
  esac
  note=${note:-—}
  rows+=("| $name | $lo_pages | $our_pages | $min_ssim | $mean_ssim | $note |")
done

# --- отчёт ------------------------------------------------------------------

report=$workdir/report.md
# Версия и локаль эталона — часть результата: по ним видно, каким LibreOffice и
# с какой бумагой по умолчанию получены числа (см. шапку скрипта). `--version`
# печатает несколько строк, версия — первая.
soffice_version=$(soffice --version 2>/dev/null | head -1) || true
[ -n "$soffice_version" ] || soffice_version="неизвестна"
{
  echo "# XLSX: наш PDF против LibreOffice"
  echo
  echo "Постраничный SSIM: ImageMagick \`compare -metric SSIM\`, 1.0 — страницы"
  echo "совпали. Растеризация \`pdftoppm -png -gray -r 150\`; режим \`-gray\`"
  echo "намеренно слеп к цвету — метрика ловит геометрию, шрифт и пагинацию, а"
  echo "совпадение заливок и цветов текста ею не проверяется."
  echo
  echo "Прогон: $(date -u '+%Y-%m-%d %H:%M UTC')"
  echo "Эталон: $soffice_version; локаль LANG=${LANG:-(не задана)} LC_ALL=${LC_ALL:-(не задана)}"
  echo
  echo "| фикстура | страниц у LO | страниц у нас | min SSIM | mean SSIM | примечание |"
  echo "| --- | --- | --- | --- | --- | --- |"
  printf '%s\n' "${rows[@]}"
  echo
  echo "Сравнено: $ok, пропущено: $skipped, с ошибкой: $failed."
} | tee "$report"

# Ноль сравнений при нуле пропусков означает, что не сработал конвейер целиком
# (нет soffice нужной версии, сломан экспорт) — это состояние харнесса, и о нём
# CI должен узнать падением. Пропуски же — нормальный исход отбора.
if [ "$ok" -eq 0 ] && [ "$skipped" -eq 0 ]; then
  echo "ни одну фикстуру сравнить не удалось — смотрите логи в $workdir/log" >&2
  exit 1
fi
