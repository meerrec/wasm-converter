#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack not found. Install: cargo install wasm-pack" >&2
  exit 1
fi

# Два модуля: основной (просмотр) и ленивый экспорт в PDF. Второй тянет
# printpdf, поэтому в первый не входит: воркер грузит его по клику.
# --out-dir резолвится относительно каталога крейта, а не CWD, поэтому пути абсолютные.
wasm-pack build crates/wasm \
  --target web \
  --out-dir "$ROOT/packages/wasm/pkg" \
  --out-name doc_converter_wasm \
  --release

wasm-pack build crates/pdf-wasm \
  --target web \
  --out-dir "$ROOT/packages/wasm-pdf/pkg" \
  --out-name doc_converter_pdf_wasm \
  --release

if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -O4 packages/wasm/pkg/doc_converter_wasm_bg.wasm \
    -o packages/wasm/pkg/doc_converter_wasm_bg.wasm
  wasm-opt -O4 packages/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm \
    -o packages/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm
fi

echo "OK: WASM built -> packages/wasm/pkg/, packages/wasm-pdf/pkg/"
