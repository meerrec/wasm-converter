#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack not found. Install: cargo install wasm-pack" >&2
  exit 1
fi

wasm-pack build crates/wasm \
  --target web \
  --out-dir packages/wasm/pkg \
  --out-name doc_converter_wasm \
  --release

if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -O4 packages/wasm/pkg/doc_converter_wasm_bg.wasm \
    -o packages/wasm/pkg/doc_converter_wasm_bg.wasm
fi

echo "OK: WASM built -> packages/wasm/pkg/"
