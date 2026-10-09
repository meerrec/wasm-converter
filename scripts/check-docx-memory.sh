#!/usr/bin/env bash
# Гейт DoD 6 спринта 8: пиковая память разбора 50 МБ DOCX с изображениями
# ≤ 200 МиБ (ROADMAP.md, спринт 8, «Бюджеты памяти»; общие бюджеты — §9
# «Бенчмарки и бюджеты»). Механизм повторяет гейт DoD 8 спринта 7 —
# scripts/check-pdf-memory.sh: тот же time(1), тот же разбор, те же коды.
#
# Метрика — ru_maxrss процесса mem_probe_docx, снятый системным time(1): у mem_probe_docx
# нет своего счётчика RSS, а пик нужен за весь разбор, а не в точке замера.
# Формат у time(1) платформенный: BSD на macOS печатает байты по `-l`, GNU на
# Linux — килобайты по `-v`; разбор обоих живёт здесь, чтобы число в CI означало
# то же, что число в отчёте спринта.
#
# Usage:
#   bash scripts/check-docx-memory.sh <mem_probe_docx> <fixture.docx> [предел МиБ]
#
# Тяжёлую фикстуру в git не кладут: её пишет
# `npx tsx scripts/generate_docx_fixtures.ts --large` в target/fixtures/docx-large
# (каталог `target/` под .gitignore, ~11 с на все большие фикстуры), оттуда её
# берут mem_probe_docx, бенч и этот гейт.
set -euo pipefail

probe=${1:?usage: check-docx-memory.sh <mem_probe_docx> <fixture.docx> [limit_mib]}
fixture=${2:?usage: check-docx-memory.sh <mem_probe_docx> <fixture.docx> [limit_mib]}
limit_mib=${3:-200}

# Критерии фикстуры: замер имеет смысл, только если файл — тот самый тяжёлый
# DOCX с картинками. Пол 50 МиБ взят из формулировки бюджета, а не из размера
# конкретного файла: генератор целится в 57 МиБ (75 шумовых PNG по ~770 КиБ
# дают ~56 МиБ, пол профиля — 50 МиБ), и вход ниже пола означал бы, что взят
# не тот файл. Абзацы и картинки ничего не измеряют — они лишь отсекают замер
# не на той фикстуре (та же роль, что min_pages=480 в PDF-гейте): 5000 абзацев
# заметно ниже 6000 у фикстуры, а картинка нужна хотя бы одна, потому что
# бюджет назван «DOCX с изображениями»: без них пик набирает разбор XML,
# а не распаковка картинок.
min_input_mib=50
min_paragraphs=5000
min_images=1

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

# stdout mem_probe_docx и stderr time(1) сливаются: в лог уходит и вывод прогона
# (вход, абзацы, изображения, время разбора), и метрики time. LC_ALL=C — у GNU
# time есть переводы (в том числе русский), и разбор строки метрики не должен
# зависеть от локали раннера.
if ! run=$(LC_ALL=C "$time_bin" "${time_args[@]}" "$probe" --open-only "$fixture" 2>&1); then
  printf '%s\n' "$run"
  echo "mem_probe_docx завершился с ошибкой" >&2
  exit 1
fi
printf '%s\n' "$run"

# На строке метрики в обоих форматах значение — первый чисто числовой токен
# (после него в macOS идут слова, в GNU — уже ничего).
rss=$(printf '%s\n' "$run" | awk -v re="$rss_re" \
  '$0 ~ re { for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+$/) { print $i; exit } }')
if [ -z "$rss" ]; then
  # Фигурные скобки обязательны: следом стоит закрывающая кавычка, и bash 3.2
  # втягивает её байты в имя переменной — на этом пути (метрики нет) скрипт
  # падал бы с unbound variable вместо кода 2.
  echo "не нашёл строку «${rss_re}» в выводе time(1)" >&2
  exit 2
fi
peak_bytes=$((rss * rss_scale))
limit_bytes=$((limit_mib * 1024 * 1024))

# Числа фикстуры — из единственной стабильной строки mem_probe_docx; LC_ALL=C и
# здесь: шаблоны с кириллицей должны совпасть одинаково на любой локали раннера.
stats() { printf '%s\n' "$run" | LC_ALL=C sed -nE "$1" | tail -1; }
input_mib=$(stats 's/.*вход ([0-9]+(\.[0-9]+)?) МиБ.*/\1/p')
paragraphs=$(stats 's/.*абзацев ([0-9]+).*/\1/p')
images=$(stats 's/.*изображений ([0-9]+).*/\1/p')

check_min() { # <значение> <минимум> <подпись>
  if awk -v v="$1" -v m="$2" 'BEGIN { exit !(v >= m) }'; then
    return 0
  fi
  echo "фикстура не та: $3 = $1, нужно не меньше $2 — замер не о 50 МБ DOCX с изображениями" >&2
  return 1
}

if [ -z "$input_mib" ] || [ -z "$paragraphs" ] || [ -z "$images" ]; then
  echo "mem_probe_docx не напечатал вход/абзацы/изображения — фикстура не проверена" >&2
  exit 1
fi

bad=0
check_min "$input_mib" "$min_input_mib" "вход (МиБ)" || bad=1
check_min "$paragraphs" "$min_paragraphs" "абзацев" || bad=1
check_min "$images" "$min_images" "изображений" || bad=1
if [ "$bad" -ne 0 ]; then
  exit 1
fi

peak_mib=$(awk -v b="$peak_bytes" 'BEGIN { printf "%.2f", b / 1048576 }')
echo "DoD: пик RSS $peak_mib МиБ на $(basename "$fixture") (вход $input_mib МиБ, $paragraphs абз., $images изобр.); предел $limit_mib МиБ"

if [ "$peak_bytes" -gt "$limit_bytes" ]; then
  echo "ПРЕДЕЛ: пик $peak_mib МиБ > $limit_mib МиБ — бюджет «пиковая память на 50 МБ DOCX с изображениями» (ROADMAP.md, спринт 8)" >&2
  exit 1
fi
