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
#   2 — неверное употребление: меньше трёх аргументов, нет mem_probe, под
#       именем не PDF-пример mem_probe или нет каталога фикстур;
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
# Формат бумаги LibreOffice ищет в /etc/papersize (paperconf), а затем уже в
# умолчаниях локали (Letter в C.UTF-8 и en_US, A4 в ru_RU и de_DE); в CI
# workflow пишет `a4` в /etc/papersize, и файл перекрывает локаль — это и есть
# фактический рычаг бумаги. Наш экспорт всегда A4, поэтому размеры страниц
# сверяются до SSIM. Локаль эталона (ru_RU.UTF-8) задаёт workflow: вдобавок к
# papersize она управляет локализованным форматированием дат, а скрипт её не
# переключает — иначе числа зависели бы от машины, а не от рендера.
#
# Внешние утилиты есть в CI (см. .github/workflows/oracle.yml); там же прогон и
# живёт. Локально без них скрипт штатно скипается кодом 3.
set -euo pipefail
# Маска без совпадений даёт пустой список, а не слово со звёздочкой.
shopt -s nullglob

usage="usage: oracle-ssim.sh <mem_probe> <каталог-фикстур> <workdir> [фикстура...]"

if [ "$#" -lt 3 ]; then
  echo "$usage" >&2
  exit 2
fi

probe=$1
fixtures_dir=$2
workdir=$3
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
elif command -v compare >/dev/null 2>&1 && command -v convert >/dev/null 2>&1 \
  && command -v identify >/dev/null 2>&1; then
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

if [ ! -f "$probe" ] || [ ! -x "$probe" ]; then
  echo "нет исполняемого $probe — соберите: cargo build --release -p doc-converter-pdf --example mem_probe" >&2
  exit 2
fi

# `-x` не отличает PDF-пример от DOCX-ного: до 09.10.2026 у обоих было одно имя
# (mem_probe), и под ним мог лежать DOCX-бинарник — дальше скрипт зовёт `--out`
# и умирает уже после прогона эталонов. `--out` есть в usage только у PDF-ного
# примера; без аргументов оба печатают usage и выходят кодом 2.
probe_usage=$("$probe" 2>&1 || true)
if [[ "$probe_usage" != *"--out"* ]]; then
  echo "под именем $probe не PDF-пример mem_probe — соберите: cargo build --release -p doc-converter-pdf --example mem_probe" >&2
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

compare_fixture() {
  local name=$1
  local file=$fixtures_dir/$name
  lo_pages="-"
  our_pages="-"
  min_ssim="-"
  mean_ssim="-"
  note=""
  outcome=""
  fail_rank=0
  fail_status=""
  local -a paper_pages=()

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
    mark_fail 3 "сбой окружения"
    outcome=fail
    return 0
  fi
  # LibreOffice умеет вернуть 0 и не написать файл, поэтому мало кода выхода.
  if [ ! -s "$lo_pdf" ]; then
    note="LibreOffice не создал $stem.pdf (лог $log)"
    mark_fail 3 "сбой окружения"
    outcome=fail
    return 0
  fi

  status=0
  "$probe" "$file" --out "$our_pdf" >"$log.our" 2>&1 || status=$?
  if [ "$status" -ne 0 ] || [ ! -s "$our_pdf" ]; then
    note="mem_probe: код $status, PDF не создан (лог $log.our)"
    mark_fail 2 "сбой экспорта"
    outcome=fail
    return 0
  fi

  local lo_px=$workdir/pages/lo/$stem
  local our_px=$workdir/pages/our/$stem
  rm -f "$lo_px"-*.png "$our_px"-*.png
  if ! pdftoppm -png -gray -r 150 "$lo_pdf" "$lo_px" >"$log.lo.ppm" 2>&1; then
    note="pdftoppm не растеризовал PDF LibreOffice"
    mark_fail 3 "сбой окружения"
    outcome=fail
    return 0
  fi
  if ! pdftoppm -png -gray -r 150 "$our_pdf" "$our_px" >"$log.our.ppm" 2>&1; then
    note="pdftoppm не растеризовал наш PDF"
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
    note="растеризация не дала страниц"
    mark_fail 1 "сбой сравнения"
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
      # пикселей — это Letter против A4 или другой масштаб, и SSIM по ним ничего
      # не значил бы.
      local w_a=${size_a%x*} h_a=${size_a#*x} w_b=${size_b%x*} h_b=${size_b#*x}
      local dw=$((w_a - w_b)) dh=$((h_a - h_b))
      dw=$((dw < 0 ? -dw : dw))
      dh=$((dh < 0 ? -dh : dh))
      if [ "$dw" -gt 2 ] || [ "$dh" -gt 2 ]; then
        paper_pages+=("$((i + 1))")
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
      mark_fail 1 "сбой сравнения"
      continue
    fi

    # IM6 печатает «0.987654», IM7 — «0.987654 (0.987654)»; первое число — сам
    # SSIM. awk, а не grep|head: на пустой находке конвейер под pipefail уронил
    # бы скрипт из-за «нет совпадений» — это не ошибка растеризации. Запятая в
    # регулярке — на случай LC_NUMERIC с запятой: дальше значение уходит в файл
    # SSIM, который читает awk уже с точкой.
    val=$(printf '%s\n' "$val" | awk 'match($0, /-?[0-9]+([.,][0-9]+)?([eE][-+]?[0-9]+)?/) { print substr($0, RSTART, RLENGTH); exit }')
    val=${val//,/.}
    if [ -z "$val" ]; then
      note+="${note:+; }страница $((i + 1)): compare не напечатал SSIM"
      mark_fail 1 "сбой сравнения"
      continue
    fi
    printf '%s\n' "$val" >>"$ssim_file"
  done

  if [ "${#paper_pages[@]}" -ne 0 ]; then
    note+="${note:+; }страницы $(format_page_ranges "${paper_pages[@]}"): разные бумага или масштаб, SSIM не считан"
    mark_fail 3 "сбой окружения"
  fi

  if [ -s "$ssim_file" ]; then
    # min ловит полностью разъехавшуюся страницу, которую среднее скрыло бы;
    # mean показывает, насколько близки страницы в целом. Локаль прижата к C:
    # и сравнение, и printf зависят от LC_NUMERIC (в ru_RU разделитель — запятая).
    read -r min_ssim mean_ssim < <(
      LC_ALL=C awk '{ if (NR == 1 || $1 < min) min = $1; sum += $1 } END { printf "%.4f %.4f\n", min, sum / NR }' "$ssim_file"
    )
    outcome=ok
  else
    note+="${note:+; }SSIM не снят ни с одной страницы"
    mark_fail 1 "сбой сравнения"
    outcome=fail
  fi

  return 0
}

rows=()
ok=0
skipped=0
failed=0
env_failed=0
export_failed=0
cmp_failed=0

for name in "${fixtures[@]}"; do
  compare_fixture "$name"
  case "$outcome" in
    ok) ok=$((ok + 1)); row_status="сравнено" ;;
    skip) skipped=$((skipped + 1)); row_status="пропущено" ;;
    *) failed=$((failed + 1))
       row_status=$fail_status
       case "$fail_rank" in
         3) env_failed=$((env_failed + 1)) ;;
         2) export_failed=$((export_failed + 1)) ;;
         *) cmp_failed=$((cmp_failed + 1)) ;;
       esac ;;
  esac
  note=${note:-—}
  note=${note//|/\\|}
  rows+=("| $name | $lo_pages | $our_pages | $min_ssim | $mean_ssim | $note | $row_status |")
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
  echo "| фикстура | страниц у LO | страниц у нас | min SSIM | mean SSIM | примечание | статус |"
  echo "| --- | --- | --- | --- | --- | --- | --- |"
  printf '%s\n' "${rows[@]}"
  echo
  if [ "$failed" -gt 0 ]; then
    echo "Сравнено: $ok, пропущено: $skipped, с ошибкой: $failed (окружение: $env_failed, экспорт: $export_failed, сравнение: $cmp_failed)."
  else
    echo "Сравнено: $ok, пропущено: $skipped, с ошибкой: $failed."
  fi
} | tee "$report"

# Ноль сравнений при хотя бы одной упавшей фикстуре означает, что не сработал
# конвейер целиком (нет soffice нужной версии, сломан экспорт) — это состояние
# харнесса, и о нём CI должен узнать падением. Прогон, где всё пропущено, —
# нормальный исход отбора, он остаётся зелёным.
if [ "$ok" -eq 0 ] && [ "$failed" -gt 0 ]; then
  echo "ни одну фикстуру сравнить не удалось — смотрите логи в $workdir/log" >&2
  exit 1
fi
