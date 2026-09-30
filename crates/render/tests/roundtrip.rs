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
fn native_ring_fifo_cycles() {
    use doc_converter_render::sab::reader::NativeRingReader;
    let mut r = NativeRingReader::new();
    for _ in 0..1000 {
        assert!(r.push(b"frame"));
        assert_eq!(r.pop(), Some(&b"frame"[..]));
    }
}
