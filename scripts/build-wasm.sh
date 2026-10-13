#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack not found. Install: cargo install wasm-pack" >&2
  exit 1
fi

# Три модуля: основной (просмотр), ленивый экспорт в PDF и ленивый
# просмотр DOCX. Второй тянет экспортёр PDF, третий — docx-парсер, поэтому
# в первый они не входят: воркер грузит их по клику.
# --out-dir резолвится относительно каталога крейта, а не CWD, поэтому пути абсолютные.
wasm-pack build crates/wasm \
  --target web \
  --out-dir "$ROOT/packages/wasm/pkg" \
  --out-name doc_converter_wasm \
  --release

# wasm-bindgen собирает #[wasm_bindgen]-элементы из всего графа зависимостей;
# своей PDF-библиотеки больше нет (ADR-0022), посторонних экспортов быть не
# должно. CI-шаг «check wasm exports» стережёт это в .d.ts модуля.
wasm-pack build crates/pdf-wasm \
  --target web \
  --out-dir "$ROOT/packages/wasm-pdf/pkg" \
  --out-name doc_converter_pdf_wasm \
  --release

wasm-pack build crates/docx-wasm \
  --target web \
  --out-dir "$ROOT/packages/wasm-docx/pkg" \
  --out-name doc_converter_docx_wasm \
  --release

if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -O4 packages/wasm/pkg/doc_converter_wasm_bg.wasm \
    -o packages/wasm/pkg/doc_converter_wasm_bg.wasm
  wasm-opt -O4 packages/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm \
    -o packages/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm
  wasm-opt -O4 packages/wasm-docx/pkg/doc_converter_docx_wasm_bg.wasm \
    -o packages/wasm-docx/pkg/doc_converter_docx_wasm_bg.wasm
fi

echo "OK: WASM built -> packages/wasm/pkg/, packages/wasm-pdf/pkg/, packages/wasm-docx/pkg/"
