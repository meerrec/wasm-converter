#!/usr/bin/env python3
"""Attach evidence-based Stage 1 failure categories to an offline corpus report."""
import argparse
from collections import Counter
import html
import json
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile

CATEGORIES = ("parse", "model", "layout", "font", "render", "harness", "reference")


def local(tag):
    return tag.rsplit("}", 1)[-1]


def analyze_source(path):
    result = {"path": str(path), "parts": {}, "features": {}}
    counts = Counter()
    text_lengths = []
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        result["features"]["drawingParts"] = sum(name.startswith(("word/drawings/", "xl/drawings/")) and name.endswith(".xml") for name in names)
        result["features"]["chartParts"] = sum(name.startswith("xl/charts/chart") and name.endswith(".xml") for name in names)
        result["features"]["mediaParts"] = sum(name.startswith(("word/media/", "xl/media/")) and not name.endswith("/") for name in names)
        xml_names = [name for name in names if name.endswith(".xml") and (
            name in ("word/document.xml", "word/numbering.xml", "word/styles.xml", "xl/workbook.xml", "xl/sharedStrings.xml")
            or name.startswith("word/header") or name.startswith("word/footer") or name.startswith("xl/worksheets/sheet")
            or name.startswith("xl/drawings/drawing"))]
        for name in xml_names:
            part_counts = Counter()
            try:
                root = ET.fromstring(archive.read(name))
            except ET.ParseError:
                result["parts"][name] = {"malformed": True}
                continue
            for element in root.iter():
                tag = local(element.tag)
                part_counts[tag] += 1
                counts[tag] += 1
                if tag in ("t", "v", "f") and element.text:
                    text_lengths.append(len(element.text))
                if tag in ("row", "col") and element.attrib.get("hidden") in ("1", "true"):
                    result["features"]["hiddenRows" if tag == "row" else "hiddenColumns"] = result["features"].get("hiddenRows" if tag == "row" else "hiddenColumns", 0) + 1
            result["parts"][name] = dict(sorted(part_counts.items()))
    wanted = ("tbl", "tr", "trHeight", "tblHeader", "gridSpan", "vMerge", "drawing", "anchor", "inline",
              "fldSimple", "instrText", "sectPr", "numPr", "basedOn", "row", "c", "f", "mergeCell",
              "pane", "dataValidation", "hyperlink", "oneCellAnchor", "twoCellAnchor", "absoluteAnchor")
    result["features"].update({key: counts[key] for key in wanted if counts[key]})
    result["features"]["maxTextLength"] = max(text_lengths, default=0)
    return result


def classify(result, manifest, analysis):
    if result["status"] == "match":
        return {"category": "match", "confidence": "observed", "rationale": "All unmasked pixels satisfy visual-v1."}
    source = manifest["source"]["path"]
    name = Path(source).stem
    features = analysis["features"]
    evidence = {key: features[key] for key in ("tbl", "trHeight", "tblHeader", "drawing", "anchor", "inline", "chartParts",
                                                   "oneCellAnchor", "twoCellAnchor", "f", "pane", "hiddenRows", "hiddenColumns",
                                                   "maxTextLength") if key in features}
    base = {"evidence": evidence, "confidence": "triage"}
    if result["status"] in ("capture-error", "harness-error"):
        return {**base, "category": "harness", "rationale": "Capture or comparator failed before a visual conclusion."}
    if result["status"] == "missing-reference":
        return {**base, "category": "reference", "rationale": "The immutable reference artifact is unavailable."}
    if result["status"] == "page-mismatch":
        if name == "charts-five-kinds":
            return {**base, "category": "reference", "rationale": "Office print preview omits chart objects present in OOXML; native print extent retains them."}
        if name == "images-over-data":
            return {**base, "category": "reference", "rationale": "Office paginates distant image anchors differently; native output retains the OOXML object extent."}
        return {**base, "category": "layout", "rationale": "Actual and reference pagination differ without a known Office limitation."}
    if result.get("reason") == "dimension-mismatch":
        return {**base, "category": "harness", "rationale": "Reference and actual use different page/canvas representations; no rescaling is permitted."}
    if manifest["source"]["format"] == "docx":
        if name.startswith("long_table"):
            return {**base, "category": "layout", "rationale": "Large table-wide differences remain around row geometry and page breaking."}
        if "tables/" in source:
            return {**base, "category": "layout", "rationale": "The differing region is a table with explicit grid/row/border XML; geometry is the first triage target."}
        if "images/" in source:
            return {**base, "category": "render", "rationale": "The source contains raster drawings and wrap/anchor geometry; image painting is the first triage target."}
        if name == "fld_simple":
            return {**base, "category": "model", "rationale": "The source contains fields and cached results; field result modeling is the first triage target."}
        if name in ("sections", "breaks_and_tabs", "default_header_footer", "even_odd"):
            return {**base, "category": "layout", "rationale": "The differing content is controlled by section, break, tab, or header/footer geometry."}
        return {**base, "category": "font", "rationale": "A small text-region residual remains; font metrics/rasterization is the first triage target."}
    if name in ("values-formulas", "values-errors", "formats-date-ru"):
        return {**base, "category": "model", "rationale": "Formula/error/date value semantics are present in the differing sheet."}
    if name in ("charts-five-kinds", "images-over-data"):
        return {**base, "category": "render", "rationale": "OOXML chart/image objects occupy the differing content."}
    if name in ("layout-frozen-columns", "columns-hidden", "rows-hidden", "values-long-text", "scale-ten-pages", "sheets-three"):
        return {**base, "category": "layout", "rationale": "Sheet extent, visibility, freezing, overflow, or scaling controls the differing content."}
    if name.startswith("styles-") or name == "content-rich-text":
        return {**base, "category": "render", "rationale": "Cell text/border/fill painting is the first triage target for this style fixture."}
    return {**base, "category": "font", "rationale": "The residual is primarily cell text; font metrics/rasterization is the first triage target."}


def write_html(path, report):
    rows = []
    for category in (*CATEGORIES, "match"):
        item = report["summary"].get(category, {"count": 0, "examples": []})
        examples = "".join(f"<li><code>{html.escape(example['id'])}</code>: {html.escape(example['rationale'])}</li>" for example in item["examples"])
        rows.append(f"<section><h2>{html.escape(category)} <small>{item['count']}</small></h2><ul>{examples}</ul></section>")
    worst = "".join(f"<tr><td><code>{html.escape(item['id'])}</code></td><td>{html.escape(item['classification']['category'])}</td><td>{item.get('mismatchRatio', '')}</td><td>{html.escape(item['classification']['rationale'])}</td></tr>" for item in report["worstCaptures"])
    sources = "".join(f"<tr><td>{html.escape(key)}</td><td><code>{html.escape(json.dumps(value['features'], ensure_ascii=False))}</code></td></tr>" for key, value in report["sourceAnalysis"].items())
    document = f"<!doctype html><meta charset='utf-8'><title>Stage 1 failure classification</title><style>body{{font:14px system-ui;max-width:1400px;margin:auto;padding:24px}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #ccc;padding:6px;text-align:left;vertical-align:top}}code{{white-space:pre-wrap}}small{{color:#666}}</style><h1>Stage 1 failure classification</h1><p>Categories are triage hypotheses backed by capture metrics and static OOXML evidence, not confirmed root causes.</p>{''.join(rows)}<h2>Worst captures</h2><table><tr><th>ID</th><th>Category</th><th>Mismatch ratio</th><th>Rationale</th></tr>{worst}</table><h2>Static OOXML analysis</h2><table><tr><th>Source</th><th>Features</th></tr>{sources}</table>"
    path.write_text(document, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text())
    report = json.loads(args.report.read_text())
    manifests = {}
    analyses = {}
    for document in plan["documents"]:
        manifest = document["manifest"]
        for capture in manifest["captures"]:
            manifests[capture["id"]] = manifest
        source = manifest["source"]["path"]
        if source not in analyses:
            analyses[source] = analyze_source(Path(source))
    classified = []
    for result in report["results"]:
        manifest = manifests.get(result["id"])
        if manifest is None:
            manifest = next((value["manifest"] for value in plan["documents"] if result["id"].startswith(value["manifest"]["source"]["path"])), None)
        if manifest is None:
            result["classification"] = {"category": "harness", "confidence": "observed", "rationale": "No source manifest can be associated with this result."}
        else:
            result["classification"] = classify(result, manifest, analyses[manifest["source"]["path"]])
        classified.append(result)
    grouped = {category: [] for category in (*CATEGORIES, "match")}
    for result in classified:
        grouped[result["classification"]["category"]].append(result)
    summary = {}
    for category, values in grouped.items():
        values.sort(key=lambda value: value.get("mismatchRatio", 1 if value["status"] != "match" else 0), reverse=True)
        summary[category] = {"count": len(values), "examples": [{"id": value["id"], "status": value["status"],
                             "mismatchRatio": value.get("mismatchRatio"), "rationale": value["classification"]["rationale"]} for value in values[:5]]}
    worst = sorted((value for value in classified if value["status"] != "match"),
                   key=lambda value: value.get("mismatchRatio", 1), reverse=True)[:30]
    output = {"schemaVersion": "1.0.0", "baselineId": report.get("baselineId"), "summary": summary,
              "worstCaptures": worst, "sourceAnalysis": analyses, "results": classified}
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "classification.json").write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n")
    write_html(args.output / "classification.html", output)
    print(json.dumps({key: value["count"] for key, value in summary.items()}, ensure_ascii=False))


if __name__ == "__main__":
    main()
