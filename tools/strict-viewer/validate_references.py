#!/usr/bin/env python3
"""Проверяет сохранённые Office references без изменения корпуса."""

import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]
REFERENCE_ROOT = ROOT / "test-fixtures/strict-viewer/references"
SCHEMA = json.loads((ROOT / "docs/strict-viewer/manifest.schema.json").read_text())
validator = Draft202012Validator(SCHEMA)
manifests = sorted(REFERENCE_ROOT.rglob("manifest.json"))
errors = []
captures = 0
pdfs = 0

for manifest_path in manifests:
    relative = manifest_path.relative_to(ROOT)
    manifest = json.loads(manifest_path.read_text())
    for error in validator.iter_errors(manifest):
        errors.append(f"{relative}: {'/'.join(map(str, error.path))}: {error.message}")
    source = ROOT / manifest["source"]["path"]
    if not source.is_file():
        errors.append(f"{relative}: missing source {source}")
    elif hashlib.sha256(source.read_bytes()).hexdigest() != manifest["source"]["sha256"]:
        errors.append(f"{relative}: source digest mismatch")
    for capture in manifest["captures"]:
        captures += 1
        image = manifest_path.parent / capture["image"]["uri"]
        if not image.is_file() or image.stat().st_size == 0:
            errors.append(f"{relative}: missing or empty capture {image}")
        elif image.stat().st_size > 4 * 1024 * 1024:
            errors.append(f"{relative}: capture exceeds 4 MiB: {image}")
    pdf = manifest_path.parent.parent / "reference.pdf"
    if pdf.exists() and manifest["view"]["mode"] == "print-layout":
        pdfs += 1
        if pdf.stat().st_size == 0:
            errors.append(f"{relative}: empty reference PDF")

repeatability_path = ROOT / "docs/strict-viewer/stage-1-reference-repeatability.json"
repeatability = json.loads(repeatability_path.read_text())
repeat_summary = repeatability.get("summary", {})
if not repeatability.get("repeatable"):
    errors.append("Saved Office reference repeatability did not pass")
for key, expected in {
    "captures": 1936,
    "identicalCaptures": 1936,
    "differentCaptures": 0,
    "manifestErrors": 0,
    "pdfs": 20,
}.items():
    if repeat_summary.get(key) != expected:
        errors.append(f"Reference repeatability {key}: expected {expected}, got {repeat_summary.get(key)}")

if len(manifests) != 84:
    errors.append(f"Expected 84 manifests, found {len(manifests)}")
if captures != 1936:
    errors.append(f"Expected 1936 captures, found {captures}")
if pdfs != 20:
    errors.append(f"Expected 20 reference PDFs, found {pdfs}")

if errors:
    for error in errors:
        print(f"ERROR: {error}")
    raise SystemExit(3)
print(f"PASS: {len(manifests)} manifests, {captures} PNG captures, {pdfs} PDFs; sources, schema and saved Office repeatability valid")
