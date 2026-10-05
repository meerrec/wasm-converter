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

#[test]
fn chart_command_carries_its_blob() {
    use doc_converter_render::chart::{ChartData, ChartKind, ChartSeries};

    let chart = ChartData {
        kind: ChartKind::Bar,
        title: Some("Итоги".to_owned()),
        categories: vec!["Q1".to_owned(), "Q2".to_owned()],
        series: vec![ChartSeries {
            name: "План".to_owned(),
            values: vec![1.0, 2.0],
        }],
    };
    let mut dl = DisplayList::new();
    let blob = dl.intern_bytes(&chart.to_blob());
    dl.push(DrawCommand::Chart {
        x: 1.0,
        y: 2.0,
        w: 300.0,
        h: 200.0,
        data: blob,
    });

    let bytes = dl.to_bytes();
    let rdr = DisplayList::from_bytes(&bytes).unwrap();
    let mut cmds = rdr.iter();
    match cmds.next().unwrap().unwrap() {
        DrawCommand::Chart { x, y, w, h, data } => {
            assert_eq!((x, y, w, h), (1.0, 2.0, 300.0, 200.0));
            let decoded = ChartData::from_blob(rdr.bytes(data)).expect("блоб разбирается");
            assert_eq!(decoded, chart);
        }
        other => panic!("ожидался Chart, а не {other:?}"),
    }
}
