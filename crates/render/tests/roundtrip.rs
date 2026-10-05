use doc_converter_render::display_list::*;

#[test]
fn build_serialize_decode_1000_rects() {
    let mut dl = DisplayList::with_capacity(1001);
    dl.push(DrawCommand::Clear);
    for i in 0..1000 {
        dl.push(DrawCommand::Rect {
            x: i as f32,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            fill: Color::rgba(0, 0, 0, 255),
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        });
    }
    let bytes = dl.to_bytes();
    let rdr = DisplayList::from_bytes(&bytes).unwrap();
    assert_eq!(rdr.cmd_count(), 1001);
    assert_eq!(rdr.iter().count(), 1001);
}

#[test]
fn roundtrip_line_styles_and_underline() {
    let styles = [
        LineStyle::Solid,
        LineStyle::Dashed,
        LineStyle::Dotted,
        LineStyle::Double,
    ];
    let mut dl = DisplayList::new();
    for style in styles {
        dl.push(DrawCommand::Line {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 10.0,
            stroke: Color::BLACK,
            stroke_w: 2.0,
            style,
        });
    }
    let text = dl.intern("link");
    let font = dl.intern("Calibri");
    dl.push(DrawCommand::Text {
        x: 0.0,
        y: 0.0,
        text,
        font,
        size: 12.0,
        color: Color::BLACK,
        align: TextAlign::Left,
        baseline: TextBaseline::Alphabetic,
        bold: false,
        italic: false,
        underline: true,
    });

    let bytes = dl.to_bytes();
    let rdr = DisplayList::from_bytes(&bytes).unwrap();
    let cmds: Vec<_> = rdr.iter().map(Result::unwrap).collect();
    assert_eq!(cmds.len(), 5);
    for (cmd, style) in cmds.iter().zip(styles) {
        match cmd {
            DrawCommand::Line { style: got, .. } => assert_eq!(*got, style),
            _ => panic!("expected Line"),
        }
    }
    match &cmds[4] {
        DrawCommand::Text { underline, .. } => assert!(*underline),
        _ => panic!("expected Text"),
    }
}

#[test]
fn native_ring_fifo_cycles() {
    use doc_converter_render::sab::reader::NativeRingReader;
    let mut r = NativeRingReader::new();
    for _ in 0..1000 {
        assert!(r.push(b"frame"));
        assert_eq!(r.pop(), Some(&b"frame"[..]));
    }
}
