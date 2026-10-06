//! Спайк B1: где живёт память при записи PDF и что даёт потоковая запись.
//!
//! Режимы:
//!   baseline <N> <file>   — сегодняшний путь: страницы копятся в PdfDocument + lopdf
//!   baseline-bytes <N>    — то же + Vec<u8> всего файла (как `PdfDocument::save`)
//!   stream <N> <file>     — потоковая запись готового документа (освобождает lopdf-объекты)
//!   lazy <N> <file>       — страница строится, пишется и сразу освобождается (пик O(1))
//!   validate <file>       — перечитать файл lopdf-ом: страницы, /Annots, /Outlines
//!
//! Пик RSS снимает внешний `/usr/bin/time -l`.

use std::collections::BTreeSet;
use std::io::{BufWriter, Write};
use std::time::Instant;

use printpdf::streaming::{serialize_pdf_streaming, StreamSession};
use printpdf::{
    Actions, BuiltinFont, Line, LinePoint, LinkAnnotation, Mm, Op, PdfDocument, PdfPage,
    PdfSaveOptions, PdfWarnMsg, Point, Pt, Rect, TextItem,
};

/// Одна страница ≈ `cells` текстовых элементов + линии, как таблица с ячейками.
fn build_page(p: usize, cells: usize) -> PdfPage {
    let mut ops: Vec<Op> = Vec::with_capacity(cells + 4);
    ops.push(Op::StartTextSection);
    ops.push(Op::SetFontSizeBuiltinFont {
        size: Pt(10.0),
        font: BuiltinFont::Helvetica,
    });
    ops.push(Op::SetTextCursor {
        pos: Point::new(Mm(10.0), Mm(280.0)),
    });
    for i in 0..cells {
        ops.push(Op::WriteTextBuiltinFont {
            items: vec![
                TextItem::Text(format!("cell {i} value {}", (i * 7 + p) % 97)),
                TextItem::Offset(-20.0),
            ],
            font: BuiltinFont::Helvetica,
        });
        if i % 20 == 0 {
            ops.push(Op::DrawLine {
                line: Line {
                    points: vec![
                        LinePoint {
                            p: Point::new(Mm(10.0), Mm(200.0)),
                            bezier: false,
                        },
                        LinePoint {
                            p: Point::new(Mm(200.0), Mm(200.0)),
                            bezier: false,
                        },
                    ],
                    is_closed: false,
                },
            });
        }
    }
    ops.push(Op::EndTextSection);
    if p % 50 == 0 {
        ops.push(Op::LinkAnnotation {
            link: LinkAnnotation::new(
                Rect {
                    x: Pt(10.0),
                    y: Pt(10.0),
                    width: Pt(100.0),
                    height: Pt(20.0),
                },
                Actions::Uri("https://example.com".into()),
                None,
                None,
                None,
            ),
        });
    }
    PdfPage::new(Mm(210.0), Mm(297.0), ops)
}

fn meta(n_pages: usize) -> PdfDocument {
    let mut doc = PdfDocument::new("spike");
    for p in 1..=n_pages {
        if p % 100 == 1 {
            doc.add_bookmark(&format!("Page {p}"), p);
        }
    }
    doc
}

fn build_all(n_pages: usize, cells: usize) -> PdfDocument {
    let mut doc = meta(n_pages);
    for p in 0..n_pages {
        doc.with_pages(vec![build_page(p, cells)]);
    }
    doc
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("help");

    if mode == "validate" {
        let bytes = std::fs::read(&args[1]).unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        let pages = doc.get_pages();
        let mut links = 0usize;
        for (_, page_id) in &pages {
            let page = doc.get_object(*page_id).unwrap().as_dict().unwrap();
            if let Ok(res) = page.get(b"Resources").and_then(|r| r.as_reference()) {
                let res = doc.get_object(res).unwrap().as_dict().unwrap();
                if let Ok(a) = res.get(b"Annots") {
                    links += a.as_array().map(Vec::len).unwrap_or(0);
                }
            }
        }
        let catalog = doc.catalog().unwrap();
        let n_outlines = catalog
            .get(b"Outlines")
            .ok()
            .and_then(|o| o.as_reference().ok())
            .and_then(|id| doc.get_object(id).ok())
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"Count").ok())
            .and_then(|c| c.as_i64().ok())
            .unwrap_or(0);
        println!("страниц: {}, ссылок: {links}, Outlines: {n_outlines}", pages.len());
        return;
    }

    if mode == "compare" {
        let a = lopdf::Document::load_mem(&std::fs::read(&args[1]).unwrap()).unwrap();
        let b = lopdf::Document::load_mem(&std::fs::read(&args[2]).unwrap()).unwrap();
        let pa = a.get_pages();
        let pb = b.get_pages();
        let mut diffs = 0usize;
        for ((n, ia), (_, ib)) in pa.iter().zip(pb.iter()) {
            let ca = a.get_page_content(*ia).unwrap();
            let cb = b.get_page_content(*ib).unwrap();
            if ca != cb {
                diffs += 1;
                if diffs <= 3 {
                    println!("страница {n}: контент различается ({} vs {} Б)", ca.len(), cb.len());
                }
            }
        }
        println!("страниц: {} vs {}, с разным контентом: {diffs}", pa.len(), pb.len());
        return;
    }

    let n_pages: usize = args[1].parse().unwrap();
    let cells = std::env::var("CELLS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000usize);

    println!(
        "size_of::<Op>() = {} Б, страниц {n_pages}, ячеек/стр {cells}",
        std::mem::size_of::<Op>()
    );

    let opts = PdfSaveOptions::default();
    let mut warnings: Vec<PdfWarnMsg> = Vec::new();

    match mode {
        "baseline" => {
            let t0 = Instant::now();
            let doc = build_all(n_pages, cells);
            println!("сборка документа: {:.1} мс", t0.elapsed().as_secs_f64() * 1000.0);
            let file = std::fs::File::create(&args[2]).unwrap();
            let mut w = BufWriter::new(file);
            let t1 = Instant::now();
            doc.save_writer(&mut w, &opts, &mut warnings);
            w.flush().unwrap();
            let len = std::fs::metadata(&args[2]).unwrap().len();
            println!(
                "baseline(writer) -> {len} Б, запись {:.1} мс",
                t1.elapsed().as_secs_f64() * 1000.0
            );
        }
        "baseline-bytes" => {
            let t0 = Instant::now();
            let doc = build_all(n_pages, cells);
            println!("сборка документа: {:.1} мс", t0.elapsed().as_secs_f64() * 1000.0);
            let t1 = Instant::now();
            let bytes = doc.save(&opts, &mut warnings);
            println!(
                "baseline(bytes) -> {} Б, запись {:.1} мс",
                bytes.len(),
                t1.elapsed().as_secs_f64() * 1000.0
            );
        }
        "stream" => {
            let t0 = Instant::now();
            let mut doc = build_all(n_pages, cells);
            println!("сборка документа: {:.1} мс", t0.elapsed().as_secs_f64() * 1000.0);
            let file = std::fs::File::create(&args[2]).unwrap();
            let mut w = BufWriter::new(file);
            let t1 = Instant::now();
            serialize_pdf_streaming(&mut doc, &opts, &mut w, &mut warnings).unwrap();
            w.flush().unwrap();
            let len = std::fs::metadata(&args[2]).unwrap().len();
            println!(
                "stream(file) -> {len} Б, запись {:.1} мс",
                t1.elapsed().as_secs_f64() * 1000.0
            );
        }
        "lazy" => {
            let meta_doc = meta(n_pages);
            let fonts = BTreeSet::from([BuiltinFont::Helvetica]);
            let file = std::fs::File::create(&args[2]).unwrap();
            let mut w = BufWriter::new(file);
            let t1 = Instant::now();
            let mut session =
                StreamSession::begin(&meta_doc, fonts, n_pages, &opts, &mut w, &mut warnings)
                    .unwrap();
            for p in 0..n_pages {
                let page = build_page(p, cells);
                session.write_page(&page, &mut warnings).unwrap();
            }
            session.finish().unwrap();
            w.flush().unwrap();
            let len = std::fs::metadata(&args[2]).unwrap().len();
            println!(
                "lazy(file) -> {len} Б, сборка+запись {:.1} мс",
                t1.elapsed().as_secs_f64() * 1000.0
            );
        }
        "lazy-bytes" => {
            let meta_doc = meta(n_pages);
            let fonts = BTreeSet::from([BuiltinFont::Helvetica]);
            let t1 = Instant::now();
            let mut out: Vec<u8> = Vec::new();
            let mut session =
                StreamSession::begin(&meta_doc, fonts, n_pages, &opts, &mut out, &mut warnings)
                    .unwrap();
            for p in 0..n_pages {
                let page = build_page(p, cells);
                session.write_page(&page, &mut warnings).unwrap();
            }
            session.finish().unwrap();
            println!(
                "lazy(bytes) -> {} Б, сборка+запись {:.1} мс",
                out.len(),
                t1.elapsed().as_secs_f64() * 1000.0
            );
        }
        other => {
            eprintln!("неизвестный режим: {other}");
            std::process::exit(2);
        }
    }
}
