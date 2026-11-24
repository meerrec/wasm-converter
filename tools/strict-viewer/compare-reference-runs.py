#!/usr/bin/env python3
"""Compare two prepared Office reference corpora without network access."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

from PIL import Image


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def comparable_pixels(image, masks):
    rgba = image.convert("RGBA")
    data = bytearray(rgba.tobytes())
    for mask in masks:
        rect = mask["rect"]
        x0, y0 = rect["x"], rect["y"]
        x1, y1 = x0 + rect["width"], y0 + rect["height"]
        if x0 < 0 or y0 < 0 or x1 > rgba.width or y1 > rgba.height:
            raise ValueError("Reference mask lies outside the image")
        for y in range(y0, y1):
            start = (y * rgba.width + x0) * 4
            data[start:start + rect["width"] * 4] = b"\0" * (rect["width"] * 4)
    return rgba.size, bytes(data)


def compare(first_root, second_root):
    first_manifests = {path.relative_to(first_root): path for path in first_root.glob("**/manifest.json")}
    second_manifests = {path.relative_to(second_root): path for path in second_root.glob("**/manifest.json")}
    results = []
    pdf_results = []
    for relative in sorted(first_manifests.keys() | second_manifests.keys()):
        first_path = first_manifests.get(relative)
        second_path = second_manifests.get(relative)
        if not first_path or not second_path:
            results.append({"manifest": str(relative), "status": "different", "reason": "manifest-missing"})
            continue
        first = json.loads(first_path.read_text(encoding="utf-8"))
        second = json.loads(second_path.read_text(encoding="utf-8"))
        for key in ("source", "view"):
            if first[key] != second[key]:
                results.append({"manifest": str(relative), "status": "different", "reason": f"{key}-changed"})
        if first["reference"]["applicationVersion"] != second["reference"]["applicationVersion"]:
            results.append({"manifest": str(relative), "status": "environment-different", "reason": "application-version-changed",
                            "first": first["reference"]["applicationVersion"], "second": second["reference"]["applicationVersion"]})
        if first["environment"]["browser"] != second["environment"]["browser"]:
            results.append({"manifest": str(relative), "status": "environment-different", "reason": "browser-version-changed"})
        first_captures = {capture["id"]: capture for capture in first["captures"]}
        second_captures = {capture["id"]: capture for capture in second["captures"]}
        for capture_id in sorted(first_captures.keys() | second_captures.keys()):
            left = first_captures.get(capture_id)
            right = second_captures.get(capture_id)
            if not left or not right:
                results.append({"id": capture_id, "status": "different", "reason": "capture-missing"})
                continue
            left_path = first_path.parent / left["image"]["uri"]
            right_path = second_path.parent / right["image"]["uri"]
            item = {"id": capture_id, "firstSha256": sha256(left_path), "secondSha256": sha256(right_path)}
            if left_path.read_bytes() == right_path.read_bytes():
                item.update(status="identical", reason="png-bytes")
            else:
                if left.get("masks", []) != right.get("masks", []):
                    item.update(status="different", reason="masks-changed")
                else:
                    masks = left.get("masks", [])
                    with Image.open(left_path) as left_image, Image.open(right_path) as right_image:
                        same = (left_image.size == right_image.size and
                                comparable_pixels(left_image, masks) == comparable_pixels(right_image, masks))
                    item.update(status="identical" if same else "different",
                                reason="masked-png-pixels" if same and masks else
                                       "png-pixels-different-encoding" if same else "png-pixels")
                    if masks:
                        item["masked"] = True
            results.append(item)
        for mode in ("pdf",):
            manifest_dir = first_path.parent
            if manifest_dir.name != mode:
                continue
            left_pdf = manifest_dir.parent / "reference.pdf"
            right_pdf = second_path.parent.parent / "reference.pdf"
            if not left_pdf.exists() or not right_pdf.exists():
                pdf_results.append({"manifest": str(relative), "status": "different", "reason": "pdf-missing"})
            else:
                pdf_results.append({"manifest": str(relative), "firstSha256": sha256(left_pdf), "secondSha256": sha256(right_pdf),
                                    "status": "identical" if left_pdf.read_bytes() == right_pdf.read_bytes() else "different"})
    return results, pdf_results


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("first", type=Path)
    parser.add_argument("second", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    results, pdfs = compare(args.first, args.second)
    summary = {
        "captures": sum("id" in result for result in results),
        "identicalCaptures": sum(result.get("status") == "identical" for result in results),
        "differentCaptures": sum("id" in result and result.get("status") != "identical" for result in results),
        "maskedIdenticalCaptures": sum(result.get("status") == "identical" and result.get("masked", False) for result in results),
        "manifestErrors": sum("id" not in result and result.get("status") == "different" for result in results),
        "environmentDifferences": sum(result.get("status") == "environment-different" for result in results),
        "pdfs": len(pdfs),
        "identicalPdfs": sum(result["status"] == "identical" for result in pdfs),
        "differentPdfs": sum(result["status"] != "identical" for result in pdfs),
    }
    repeatable = summary["differentCaptures"] == 0 and summary["manifestErrors"] == 0
    report = {"schemaVersion": "1.0.0", "repeatable": repeatable,
              "repeatabilityScope": "capture IDs, dimensions and decoded PNG pixels outside identical declared masks; PDF byte identity and Office build differences are reported separately",
              "pdfBytesIdentical": summary["differentPdfs"] == 0,
              "summary": summary, "results": results, "pdfs": pdfs}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary))
    return 0 if repeatable else 1


if __name__ == "__main__":
    sys.exit(main())
