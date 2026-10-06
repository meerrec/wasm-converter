#!/usr/bin/env bash
# Гейт DoD 8 спринта 7: пик RSS полного PDF-экспорта тяжёлой книги ≤ 40 МиБ.
#
# Метрика — ровно та, что в отчёте docs/sprint-7/streaming-report.md: ru_maxrss
# процесса mem_probe, снятый системным time(1) (в отчёте — `/usr/bin/time -l`
# на macOS). Формат у time(1) платформенный: BSD на macOS печатает байты по
# `-l`, GNU на Linux — килобайты по `-v`; разбор обоих живёт здесь, чтобы
# число в CI означало то же, что число в отчёте.
#
# Usage:
#   bash scripts/check-pdf-memory.sh <mem_probe> <fixture.xlsx> [предел МиБ]
#
# Тяжёлую фикстуру в git не кладут: её пишет `pnpm gen:fixtures` в
# target/fixtures, оттуда её берут mem_probe и бенч.
set -euo pipefail

probe=${1:?usage: check-pdf-memory.sh <mem_probe> <fixture.xlsx> [limit_mib]}
fixture=${2:?usage: check-pdf-memory.sh <mem_probe> <fixture.xlsx> [limit_mib]}
limit_mib=${3:-40}

# Ниже этого числа страниц замер перестаёт быть замером тяжёлой книги.
# Порог — консервативный: генератор целится в 500 страниц (24 000 строк), а
# исторически давал 480; здесь важно лишь отсечь замер не на той фикстуре,
# точное число страниц печатает сам mem_probe, а строгость к 500 держит бенч.
min_pages=480

time_bin=/usr/bin/time
if [ ! -x "$time_bin" ]; then
  echo "нет $time_bin — на Linux поставьте пакет time (apt-get install time)" >&2
  exit 2
fi

case "$(uname -s)" in
  Darwin)
    time_args=(-l)
    rss_re='maximum resident set size'
    rss_scale=1 # байты
    ;;
  Linux)
    time_args=(-v)
    rss_re='Maximum resident set size'
    rss_scale=1024 # килобайты
    ;;
  *)
    echo "неизвестная ОС $(uname -s): формат time(1) не разобран" >&2
    exit 2
    ;;
esac

# stdout mem_probe и stderr time(1) сливаются: в лог уходит и вывод прогона
# (страницы, время экспорта), и метрики time. LC_ALL=C — у GNU time есть
# переводы (в том числе русский), и разбор строки метрики не должен зависеть
# от локали раннера.
if ! run=$(LC_ALL=C "$time_bin" "${time_args[@]}" "$probe" "$fixture" 2>&1); then
  printf '%s\n' "$run"
  echo "mem_probe завершился с ошибкой" >&2
  exit 1
fi
printf '%s\n' "$run"

# На строке метрики в обоих форматах значение — первый чисто числовой токен
# (после него в macOS идут слова, в GNU — уже ничего).
rss=$(printf '%s\n' "$run" | awk -v re="$rss_re" \
  '$0 ~ re { for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+$/) { print $i; exit } }')
if [ -z "$rss" ]; then
  echo "не нашёл строку «$rss_re» в выводе time(1)" >&2
  exit 2
fi
peak_bytes=$((rss * rss_scale))
limit_bytes=$((limit_mib * 1024 * 1024))

pages=$(printf '%s\n' "$run" | sed -nE 's/.*страниц ([0-9]+).*/\1/p' | tail -1)
if [ -z "$pages" ]; then
  echo "предупреждение: mem_probe не напечатал число страниц — фикстура не проверена" >&2
elif [ "$pages" -lt "$min_pages" ]; then
  echo "фикстура дала $pages страниц — нужно не меньше $min_pages, иначе замер не о тяжёлой книге" >&2
  exit 1
fi

peak_mib=$(awk -v b="$peak_bytes" 'BEGIN { printf "%.2f", b / 1048576 }')
echo "DoD 8: пик RSS $peak_mib МиБ на $(basename "$fixture")${pages:+ ($pages стр.)}; предел $limit_mib МиБ"

if [ "$peak_bytes" -gt "$limit_bytes" ]; then
  echo "ПРЕВЫШЕНИЕ: пик $peak_mib МиБ > $limit_mib МиБ — DoD 8 (docs/sprint-7/streaming-report.md)" >&2
  exit 1
fi
