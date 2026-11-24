#!/usr/bin/env python3
"""Build immutable candidate reference manifests from saved Office Online captures.

Office Online is used only by the separate capture step. This command is offline:
it reads raw PNG/PDF artifacts, verifies source digests, rasterizes Word PDFs and
writes manifests accepted by manifest.schema.json. It never modifies references
in the repository unless --output explicitly names that directory.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess

import pymupdf
from PIL import Image


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def png_info(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise ValueError("Not a PNG with an IHDR header")
    width, height = struct.unpack(">II", data[16:24])
    if data[24] != 8 or data[25] != 6:
        raise ValueError(f"Expected RGBA8 PNG, got depth={data[24]} color={data[25]}")
    return {"sha256": digest(data), "width": width, "height": height,
            "bytes": len(data), "colorType": "RGBA8"}


def copy_png(source, target):
    data = source.read_bytes()
    if data[24] != 8 or data[25] != 6:
        converted = io.BytesIO()
        Image.open(io.BytesIO(data)).convert("RGBA").save(converted, format="PNG")
        data = converted.getvalue()
    info = png_info(data)
    target.write_bytes(data)
    return info


def pdf_fonts(path):
    doc = pymupdf.open(path)
    assets = {}
    for page in doc:
        for font in page.get_fonts(full=True):
            xref = font[0]
            if xref in assets:
                continue
            name, extension, _, data = doc.extract_font(xref)
            family = name.split("+")[-1]
            assets[xref] = {"family": family, "style": "normal", "weight": 400,
                            "sha256": digest(data),
                            "source": f"Embedded subset in reference.pdf ({extension})"}
    return sorted(assets.values(), key=lambda item: (item["family"], item["sha256"]))


def environment(browser, version, fonts, rasterizer=False):
    result = {
        "os": {"name": "Linux", "version": platform.release(), "architecture": platform.machine()},
        "browser": {"name": "chromium", "version": browser},
        "viewportCss": {"width": 1440, "height": 900},
        "deviceScaleFactor": 1,
        "applicationZoomPercent": 100,
        "browserZoomPercent": 100,
        "locale": "ru-RU",
        "timezone": "UTC",
        "colorScheme": "light",
        "fonts": {"package": "office-online-embedded-subsets", "version": version, "assets": fonts},
    }
    if rasterizer:
        version_text = subprocess.check_output(["pdftoppm", "-v"], stderr=subprocess.STDOUT, text=True)
        result["rasterizer"] = {"name": "pdftoppm", "version": version_text.split()[1], "dpi": 144}
    return result


def common(source, capture, baseline_id, browser, fonts):
    source_path = Path(source["path"])
    data = source_path.read_bytes()
    if digest(data) != source["sha256"]:
        raise ValueError(f"Source digest differs from corpus: {source_path}")
    return {
        "schemaVersion": "1.0.0",
        "corpus": {"id": "strict-viewer-stage1", "version": "1.0.0", "matrixVersion": "1.0.0",
                   "baselineId": baseline_id,
                   "storage": {"kind": "git-lfs", "rootUri": "test-fixtures/strict-viewer/references"}},
        "source": {"path": source["path"], "format": source["kind"], "sha256": digest(data), "bytes": len(data)},
        "reference": {"provider": "office-online",
                      "application": "word-online" if source["kind"] == "docx" else "excel-online",
                      "applicationVersion": capture["applicationVersion"],
                      "capturedAt": capture["capturedAt"], "captureMethod": "semi-automated",
                      "operator": "wasm-converter-stage1-capture"},
        "environment": environment(browser, capture["applicationVersion"], fonts),
        "dynamicData": {"dates": "fixed-value", "fields": "cached-result",
                        "formulaCache": {"policy": "cached-values-only", "dirty": False,
                                         "fullCalcOnLoad": False, "forceFullCalc": False},
                        "externalLinks": "cached-values-no-network"},
        "comparisonProfile": "visual-v1",
        "notes": ["Candidate reference: acceptance requires the independent baseline approval policy.",
                  "Captured on Linux; certification matrix 1.0.0 Windows 11 is not certified by this artifact."],
    }


def prepare_word(raw_root, output, corpus, baseline_id, browser):
    for source in corpus["documents"]:
        if source["kind"] != "docx":
            continue
        raw_dir = raw_root / source["id"]
        capture = read_json(raw_dir / "capture.json")
        if capture["source"]["sha256"] != source["sha256"]:
            raise ValueError(f"Raw capture source mismatch: {source['id']}")
        pdf = raw_dir / "reference.pdf"
        if digest(pdf.read_bytes()) != capture["pdfSha256"]:
            raise ValueError(f"PDF digest mismatch: {source['id']}")
        fonts = pdf_fonts(pdf)
        target_doc = output / "docx" / source["id"]
        target_doc.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(pdf, target_doc / "reference.pdf")
        page_count = len(capture["pages"])
        for mode in ("print-layout", "pdf"):
            target = target_doc / mode
            target.mkdir(exist_ok=True)
            manifest = common(source, capture, baseline_id, browser, fonts)
            manifest["view"] = {"format": "docx", "mode": mode, "revisions": "accepted", "comments": "hidden"}
            if mode == "pdf":
                manifest["reference"]["exportedPdf"] = {"uri": "../reference.pdf", "sha256": capture["pdfSha256"]}
                manifest["environment"] = environment(browser, capture["applicationVersion"], fonts, True)
                prefix = target / "raster"
                subprocess.run(["pdftoppm", "-r", "144", "-png", str(pdf), str(prefix)], check=True)
            captures = []
            for number in range(1, page_count + 1):
                name = f"page-{number}.png"
                if mode == "pdf":
                    raster = target / f"raster-{str(number).zfill(len(str(page_count)))}.png"
                    info = copy_png(raster, target / name)
                    raster.unlink()
                else:
                    info = copy_png(raw_dir / f"raw-page-{number}.png", target / name)
                capture_mode = "print" if mode == "print-layout" else "pdf"
                captures.append({"id": f"{source['id']}-{capture_mode}-page-{number}", "kind": "docx-page",
                                 "pageNumber": number, "image": {"uri": name, **info}})
            manifest["captures"] = captures
            write_json(target / "manifest.json", manifest)


def prepare_excel(raw_root, output, corpus, baseline_id, browser):
    for source in corpus["documents"]:
        if source["kind"] != "xlsx":
            continue
        raw_dir = raw_root / source["id"]
        capture = read_json(raw_dir / "capture.json")
        if capture["source"]["sha256"] != source["sha256"]:
            raise ValueError(f"Raw capture source mismatch: {source['id']}")
        for sheet_number, sheet in enumerate(capture["sheets"], 1):
            if sheet.get("hidden"):
                continue
            selector = {"index": sheet["index"], "name": sheet["name"]}
            for mode in ("normal", "print"):
                target = output / "xlsx" / source["id"] / f"sheet-{sheet_number}-{mode}"
                target.mkdir(parents=True, exist_ok=True)
                manifest = common(source, capture, baseline_id, browser, capture.get("fonts", []))
                manifest["view"] = {"format": "xlsx", "mode": "normal" if mode == "normal" else "print-preview",
                                    "activeSheet": selector, "hiddenSheets": "exclude",
                                    "showGridLines": True, "showHeadings": True}
                captures = []
                if mode == "normal":
                    info = copy_png(raw_dir / sheet["normalFile"], target / "actual.png")
                    captures.append({"id": f"{source['id']}-sheet-{sheet_number}-normal", "kind": "xlsx-screen",
                                     "sheet": selector, "visibleRange": "A1:Z30", "scroll": {"x": 0, "y": 0},
                                     "activeCell": "A1", "image": {"uri": "actual.png", **info},
                                     "masks": [{"id": "office-row-gutter-focus",
                                                "reason": "documented-nondeterminism",
                                                "rect": {"x": 23, "y": 35, "width": 15, "height": 550},
                                                "textRule": "Only the Office row-header gutter focus tint is excluded; all worksheet cells remain unmasked."}]})
                else:
                    for page in sheet["pages"]:
                        number = int(Path(page["file"]).stem.rsplit("-", 1)[1])
                        name = f"page-{number}.png"
                        info = copy_png(raw_dir / page["file"], target / name)
                        captures.append({"id": f"{source['id']}-sheet-{sheet_number}-print-page-{number}",
                                         "kind": "xlsx-print-page", "pageNumber": number, "sheet": selector,
                                         "image": {"uri": name, **info}})
                manifest["captures"] = captures
                write_json(target / "manifest.json", manifest)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--word", type=Path, required=True)
    parser.add_argument("--excel", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, default=Path("test-fixtures/strict-viewer/corpus.json"))
    parser.add_argument("--baseline-id", default="office-online-2026-11-22-candidate")
    parser.add_argument("--browser", help="Chromium version; detected from CHROMIUM_PATH or /usr/bin/chromium by default")
    args = parser.parse_args()
    browser = args.browser
    if not browser:
        import os
        executable = os.environ.get("CHROMIUM_PATH", "/usr/bin/chromium")
        version_text = subprocess.check_output([executable, "--version"], text=True)
        match = re.search(r"(\d+\.\d+\.\d+\.\d+)", version_text)
        if not match:
            raise ValueError(f"Cannot parse Chromium version: {version_text.strip()}")
        browser = match.group(1)
    corpus = read_json(args.corpus)
    args.output.mkdir(parents=True, exist_ok=True)
    prepare_word(args.word, args.output, corpus, args.baseline_id, browser)
    prepare_excel(args.excel, args.output, corpus, args.baseline_id, browser)
    manifests = list(args.output.glob("**/manifest.json"))
    captures = sum(len(read_json(path)["captures"]) for path in manifests)
    print(f"Prepared {len(manifests)} manifests and {captures} captures in {args.output}")


if __name__ == "__main__":
    main()
