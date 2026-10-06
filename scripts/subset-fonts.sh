#!/usr/bin/env bash
# Подрезка Carlito (четыре начертания) до набора символов, нужного просмотрщику
# и PDF-экспортёру. Результат — crates/render/src/fonts/carlito*-subset.ttf.
#
# Источник: https://github.com/googlefonts/carlito, каталог fonts/ttf/, версия
# шрифта 1.104. Коммит пришпилен: по ветке main файлы могут переехать, а
# подрезанные шрифты обязаны воспроизводиться байт в байт.
# Лицензия — SIL OFL 1.1; рядом с подрезанными файлами обязан лежать OFL.txt
# (требование лицензии, он в репозитории).
#
# Запуск: bash scripts/subset-fonts.sh [куда-положить]
# По умолчанию результат кладётся в crates/render/src/fonts; другой каталог
# нужен, чтобы проверить результат до замены файлов в репозитории.
#
# Требуется uv (https://docs.astral.sh/uv/): pyftsubset ставится через
# uvx --from fonttools==<версия> (версия пришпилена — от неё зависит байтовый
# результат подрезки).
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUT_DIR=${1:-"$REPO_ROOT/crates/render/src/fonts"}

FONTTOOLS_VERSION=4.66.1
CARLITO_COMMIT=3a810cab78ebd6e2e4eed42af9e8453c4f9b850a
BASE_URL="https://raw.githubusercontent.com/googlefonts/carlito/$CARLITO_COMMIT/fonts/ttf"

FACES="Regular Bold Italic BoldItalic"

# Набор символов. Диапазоны сняты с прежнего carlito-subset.ttf (464 кодпоинта)
# плюс U+2116 (№), которого в нём не было.
#
# ВНИМАНИЕ: у pyftsubset второй конец диапазона пишется БЕЗ префикса U+
# (U+0400-0475); форма U+0400-U+0475 падает с ValueError.
UNICODES="U+0020-007E"    # ASCII целиком: буквы, цифры, знаки, пробел
UNICODES+=",U+00A0-00AC"  # NBSP..¬: неразрывный пробел, ¡ ¢ £ ¤ ¥ ¦ § ¨ © ª « ¬
UNICODES+=",U+00AE-00FF"  # ®..ÿ: ® ° ± ² ³ ´ µ ¶ · ¸ ¹ º » ¼ ½ ¾ ¿ и Latin-1 буквы
UNICODES+=",U+0400-0475"  # кириллица Ѐ..ѵ
UNICODES+=",U+0477-0486"  # кириллица ѷ..҆ (U+0476 и U+0487 в Carlito отсутствуют)
UNICODES+=",U+0488-04FF"  # кириллица ҈..ӿ — блок U+0400–U+04FF закрыт целиком (254 кодпоинта)
UNICODES+=",U+2012-2022"  # тире, кавычки-«лапки», † ‡ •
UNICODES+=",U+2026"       # … многоточие
UNICODES+=",U+20AC"       # € евро
UNICODES+=",U+2116"       # № знак номера (нужен фикстурам с редкими символами)
UNICODES+=",U+2122"       # ™

# Контрольные суммы полных шрифтов: страховка от оборванной загрузки (curl -f
# ловит код ответа, но не усечение файла).
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@"
  else
    shasum -a 256 "$@"  # macOS
  fi
}

src_hash() {
  case "$1" in
    Regular) printf '%s' f6418f708baede9789daef5d458c0f53d2a888af9820e8062934e504fedc6595 ;;
    Bold) printf '%s' bb5d20f79b82599ec72983597437373a80f2d2085fa91fc144fd74e876a594db ;;
    Italic) printf '%s' 0b019225e58d702bfedcbd35c21696769f8ee115cb6343f84c2f240312450d1c ;;
    BoldItalic) printf '%s' b32928186c119599e03ca6a1ffc680fdcb7fac95772f4b95d989cf6cd3861517 ;;
  esac
}

out_name() {
  case "$1" in
    Regular) printf 'carlito-subset.ttf' ;;
    Bold) printf 'carlito-bold-subset.ttf' ;;
    Italic) printf 'carlito-italic-subset.ttf' ;;
    BoldItalic) printf 'carlito-bolditalic-subset.ttf' ;;
  esac
}

command -v uvx >/dev/null 2>&1 || {
  echo "нужен uvx (uv): https://docs.astral.sh/uv/getting-started/installation/" >&2
  exit 1
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/src" "$TMP/subset"

# --no-hinting: полные Carlito несут hinting (cvt/fpgm/prep и инструкции в glyf),
# на этом наборе он даёт 140 864 Б вместо 88 624 Б (≈+59%). Подсказки адресованы
# растровому хинтингу на экране; в PDF они не встраиваются, а canvas-рендер их
# игнорирует. Прежний carlito-subset.ttf собран так же — без хинтинга.
# --name-IDs='*': сохраняет имена 0–14, включая copyright и текст лицензии.
for face in $FACES; do
  src="$TMP/src/Carlito-$face.ttf"
  curl -fsSL --retry 3 -o "$src" "$BASE_URL/Carlito-$face.ttf"
  got=$(sha256 "$src" | awk '{print $1}')
  want=$(src_hash "$face")
  [ "$got" = "$want" ] || {
    echo "контрольная сумма Carlito-$face.ttf не совпала: ожидали $want, получили $got" >&2
    exit 1
  }

  uvx --from "fonttools==$FONTTOOLS_VERSION" pyftsubset "$src" \
    --output-file="$TMP/subset/$(out_name "$face")" \
    --unicodes="$UNICODES" \
    --no-hinting \
    --name-IDs='*'
done

# Проверка, что подрезка дала ровно запрошенный набор (ни пропусков, ни лишнего).
uvx --from "fonttools==$FONTTOOLS_VERSION" python3 - "$UNICODES" "$TMP/subset" <<'PY'
import pathlib, sys
from fontTools.ttLib import TTFont
from fontTools.subset import parse_unicodes

expected = set(parse_unicodes(sys.argv[1]))
failed = False
for path in sorted(pathlib.Path(sys.argv[2]).glob('*.ttf')):
    cmap = set(TTFont(path).getBestCmap())
    missing = sorted(expected - cmap)
    extra = sorted(cmap - expected)
    if missing or extra:
        failed = True
        print(f'{path.name}: расхождение с набором: нет {len(missing)}, лишних {len(extra)}', file=sys.stderr)
    else:
        print(f'{path.name}: {len(cmap)} кодпоинтов — набор совпал')
sys.exit(1 if failed else 0)
PY

mkdir -p "$OUT_DIR"
cp "$TMP/subset/"*.ttf "$OUT_DIR/"
echo "готово: $OUT_DIR"
ls -l "$OUT_DIR"/*subset.ttf
