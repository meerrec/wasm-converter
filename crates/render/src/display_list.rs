//! DisplayList — компактное сериализуемое представление кадра.
//! Формат фиксирован, painter читает напрямую из SAB без копирований.

use bytemuck::{Pod, Zeroable};

pub const DL_MAGIC: u32 = 0x444C_5354; // "DLST"
pub const DL_VERSION: u16 = 1;

/// RGBA8, порядок байт: 0xRRGGBBAA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Pod, Zeroable)]
#[repr(transparent)]
pub struct Color(pub u32);

impl Color {
    pub const TRANSPARENT: Color = Color(0);
    pub const BLACK: Color = Color(0x0000_00FF);
    pub const WHITE: Color = Color(0xFFFF_FFFF);

    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | a as u32)
    }

    #[inline]
    pub fn to_css(self) -> String {
        let r = (self.0 >> 24) & 0xFF;
        let g = (self.0 >> 16) & 0xFF;
        let b = (self.0 >> 8) & 0xFF;
        let a = self.0 & 0xFF;
        if a == 0xFF {
            format!("#{:02x}{:02x}{:02x}", r, g, b)
        } else {
            format!("rgba({},{},{},{:.3})", r, g, b, a as f32 / 255.0)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TextAlign {
    #[default]
    Left = 0,
    Center = 1,
    Right = 2,
    Justify = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TextBaseline {
    #[default]
    Alphabetic = 0,
    Top = 1,
    Middle = 2,
    Bottom = 3,
}

/// Ссылка в общий string pool DisplayList.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StringRef {
    pub off: u32,
    pub len: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    Clear,
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Color,
        stroke: Color,
        stroke_w: f32,
        radius: [f32; 4],
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: Color,
        stroke_w: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: StringRef,
        font_id: u32,
        size: f32,
        color: Color,
        align: TextAlign,
        baseline: TextBaseline,
    },
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: u32,
    },
    PushClip {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    PopClip,
    PushTransform {
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: f32,
        f: f32,
    },
    PopTransform,
}

// --- Wire tags -------------------------------------------------------------

const TAG_CLEAR: u8 = 0x00;
const TAG_RECT: u8 = 0x01;
const TAG_LINE: u8 = 0x02;
const TAG_TEXT: u8 = 0x03;
const TAG_IMAGE: u8 = 0x04;
const TAG_PUSH_CLIP: u8 = 0x05;
const TAG_POP_CLIP: u8 = 0x06;
const TAG_PUSH_XF: u8 = 0x07;
const TAG_POP_XF: u8 = 0x08;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Header {
    magic: u32,
    version: u16,
    flags: u16,
    cmd_count: u32,
    strings_off: u32,
    _pad: u32,
}

pub const HEADER_SIZE: usize = 20;

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    cmds: Vec<DrawCommand>,
    strings: Vec<u8>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cmds: usize) -> Self {
        Self {
            cmds: Vec::with_capacity(cmds),
            strings: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.cmds.clear();
        self.strings.clear();
    }

    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn push(&mut self, cmd: DrawCommand) {
        self.cmds.push(cmd);
    }

    /// Добавляет строку в pool, возвращает ссылку.
    pub fn intern(&mut self, s: &str) -> StringRef {
        let off = self.strings.len() as u32;
        self.strings.extend_from_slice(s.as_bytes());
        StringRef {
            off,
            len: s.len() as u32,
        }
    }

    pub fn cmd(&self, i: usize) -> Option<&DrawCommand> {
        self.cmds.get(i)
    }

    #[inline]
    pub fn string(&self, r: StringRef) -> &str {
        let s = r.off as usize;
        let e = s + r.len as usize;
        std::str::from_utf8(self.strings.get(s..e).unwrap_or(&[])).unwrap_or("")
    }

    // --- Serialization -----------------------------------------------------

    pub fn to_bytes_into(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(HEADER_SIZE + self.cmds.len() * 32 + self.strings.len());

        out.resize(HEADER_SIZE, 0);

        for cmd in &self.cmds {
            match cmd {
                DrawCommand::Clear => push_tag(out, TAG_CLEAR, 0),
                DrawCommand::Rect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    stroke,
                    stroke_w,
                    radius,
                } => {
                    push_tag(out, TAG_RECT, 44);
                    let mut p = [0u8; 44];
                    p[0..4].copy_from_slice(&x.to_le_bytes());
                    p[4..8].copy_from_slice(&y.to_le_bytes());
                    p[8..12].copy_from_slice(&w.to_le_bytes());
                    p[12..16].copy_from_slice(&h.to_le_bytes());
                    p[16..20].copy_from_slice(&fill.0.to_le_bytes());
                    p[20..24].copy_from_slice(&stroke.0.to_le_bytes());
                    p[24..28].copy_from_slice(&stroke_w.to_le_bytes());
                    for (i, r) in radius.iter().enumerate() {
                        p[28 + i * 4..32 + i * 4].copy_from_slice(&r.to_le_bytes());
                    }
                    out.extend_from_slice(&p);
                }
                DrawCommand::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    stroke,
                    stroke_w,
                } => {
                    push_tag(out, TAG_LINE, 24);
                    let mut p = [0u8; 24];
                    p[0..4].copy_from_slice(&x1.to_le_bytes());
                    p[4..8].copy_from_slice(&y1.to_le_bytes());
                    p[8..12].copy_from_slice(&x2.to_le_bytes());
                    p[12..16].copy_from_slice(&y2.to_le_bytes());
                    p[16..20].copy_from_slice(&stroke.0.to_le_bytes());
                    p[20..24].copy_from_slice(&stroke_w.to_le_bytes());
                    out.extend_from_slice(&p);
                }
                DrawCommand::Text {
                    x,
                    y,
                    text,
                    font_id,
                    size,
                    color,
                    align,
                    baseline,
                } => {
                    push_tag(out, TAG_TEXT, 32);
                    let mut p = [0u8; 32];
                    p[0..4].copy_from_slice(&x.to_le_bytes());
                    p[4..8].copy_from_slice(&y.to_le_bytes());
                    p[8..12].copy_from_slice(&text.off.to_le_bytes());
                    p[12..16].copy_from_slice(&text.len.to_le_bytes());
                    p[16..20].copy_from_slice(&font_id.to_le_bytes());
                    p[20..24].copy_from_slice(&size.to_le_bytes());
                    p[24..28].copy_from_slice(&color.0.to_le_bytes());
                    p[28] = *align as u8;
                    p[29] = *baseline as u8;
                    out.extend_from_slice(&p);
                }
                DrawCommand::Image {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                } => {
                    push_tag(out, TAG_IMAGE, 20);
                    let mut p = [0u8; 20];
                    p[0..4].copy_from_slice(&x.to_le_bytes());
                    p[4..8].copy_from_slice(&y.to_le_bytes());
                    p[8..12].copy_from_slice(&w.to_le_bytes());
                    p[12..16].copy_from_slice(&h.to_le_bytes());
                    p[16..20].copy_from_slice(&bitmap_id.to_le_bytes());
                    out.extend_from_slice(&p);
                }
                DrawCommand::PushClip { x, y, w, h } => {
                    push_tag(out, TAG_PUSH_CLIP, 16);
                    let mut p = [0u8; 16];
                    p[0..4].copy_from_slice(&x.to_le_bytes());
                    p[4..8].copy_from_slice(&y.to_le_bytes());
                    p[8..12].copy_from_slice(&w.to_le_bytes());
                    p[12..16].copy_from_slice(&h.to_le_bytes());
                    out.extend_from_slice(&p);
                }
                DrawCommand::PopClip => push_tag(out, TAG_POP_CLIP, 0),
                DrawCommand::PushTransform { a, b, c, d, e, f } => {
                    push_tag(out, TAG_PUSH_XF, 24);
                    let mut p = [0u8; 24];
                    p[0..4].copy_from_slice(&a.to_le_bytes());
                    p[4..8].copy_from_slice(&b.to_le_bytes());
                    p[8..12].copy_from_slice(&c.to_le_bytes());
                    p[12..16].copy_from_slice(&d.to_le_bytes());
                    p[16..20].copy_from_slice(&e.to_le_bytes());
                    p[20..24].copy_from_slice(&f.to_le_bytes());
                    out.extend_from_slice(&p);
                }
                DrawCommand::PopTransform => push_tag(out, TAG_POP_XF, 0),
            }
        }

        let strings_off = out.len() as u32;
        out.extend_from_slice(&self.strings);

        let hdr = Header {
            magic: DL_MAGIC,
            version: DL_VERSION,
            flags: 0,
            cmd_count: self.cmds.len() as u32,
            strings_off,
            _pad: 0,
        };
        out[..HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&hdr));
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        self.to_bytes_into(&mut v);
        v
    }

    pub fn from_bytes(buf: &[u8]) -> Result<DisplayListReader<'_>, DecodeError> {
        if buf.len() < HEADER_SIZE {
            return Err(DecodeError::TooShort);
        }
        let hdr: Header = *bytemuck::from_bytes(&buf[..HEADER_SIZE]);
        if hdr.magic != DL_MAGIC {
            return Err(DecodeError::BadMagic(hdr.magic));
        }
        if hdr.version != DL_VERSION {
            return Err(DecodeError::UnsupportedVersion(hdr.version));
        }
        let strings_off = hdr.strings_off as usize;
        if strings_off < HEADER_SIZE || strings_off > buf.len() {
            return Err(DecodeError::BadHeader);
        }
        Ok(DisplayListReader {
            cmds_buf: &buf[HEADER_SIZE..strings_off],
            strings: &buf[strings_off..],
            count: hdr.cmd_count as usize,
        })
    }
}

#[inline]
fn push_tag(out: &mut Vec<u8>, tag: u8, _payload_len: usize) {
    out.push(tag);
    out.push(0);
    out.push(0);
    out.push(0);
    debug_assert_eq!(out.len() % 4, 0);
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("display list too short")]
    TooShort,
    #[error("bad magic: {0:#x}")]
    BadMagic(u32),
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u16),
    #[error("malformed header")]
    BadHeader,
    #[error("truncated command at offset {0}")]
    Truncated(usize),
    #[error("unknown tag {0:#x}")]
    UnknownTag(u8),
}

pub struct DisplayListReader<'a> {
    cmds_buf: &'a [u8],
    strings: &'a [u8],
    count: usize,
}

impl<'a> DisplayListReader<'a> {
    pub fn cmd_count(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn string(&self, r: StringRef) -> &'a str {
        let s = r.off as usize;
        let e = s + r.len as usize;
        std::str::from_utf8(self.strings.get(s..e).unwrap_or(&[])).unwrap_or("")
    }

    pub fn iter(&self) -> CmdIter<'a> {
        CmdIter {
            buf: self.cmds_buf,
            pos: 0,
            remaining: self.count,
        }
    }
}

pub struct CmdIter<'a> {
    buf: &'a [u8],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for CmdIter<'a> {
    type Item = Result<DrawCommand, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(self.decode_one())
    }
}

impl<'a> CmdIter<'a> {
    fn decode_one(&mut self) -> Result<DrawCommand, DecodeError> {
        if self.pos + 4 > self.buf.len() {
            return Err(DecodeError::Truncated(self.pos));
        }
        let tag = self.buf[self.pos];
        let start = self.pos + 4;
        self.pos = start;

        macro_rules! take {
            ($n:expr) => {{
                if self.pos + $n > self.buf.len() {
                    return Err(DecodeError::Truncated(self.pos));
                }
                let s = self.pos;
                self.pos += $n;
                &self.buf[s..s + $n]
            }};
        }
        macro_rules! f32_at {
            ($b:expr, $o:expr) => {
                f32::from_le_bytes($b[$o..$o + 4].try_into().unwrap())
            };
        }
        macro_rules! u32_at {
            ($b:expr, $o:expr) => {
                u32::from_le_bytes($b[$o..$o + 4].try_into().unwrap())
            };
        }

        let cmd = match tag {
            TAG_CLEAR => DrawCommand::Clear,
            TAG_RECT => {
                let b = take!(44);
                let mut radius = [0.0f32; 4];
                for i in 0..4 {
                    radius[i] = f32_at!(b, 28 + i * 4);
                }
                DrawCommand::Rect {
                    x: f32_at!(b, 0),
                    y: f32_at!(b, 4),
                    w: f32_at!(b, 8),
                    h: f32_at!(b, 12),
                    fill: Color(u32_at!(b, 16)),
                    stroke: Color(u32_at!(b, 20)),
                    stroke_w: f32_at!(b, 24),
                    radius,
                }
            }
            TAG_LINE => {
                let b = take!(24);
                DrawCommand::Line {
                    x1: f32_at!(b, 0),
                    y1: f32_at!(b, 4),
                    x2: f32_at!(b, 8),
                    y2: f32_at!(b, 12),
                    stroke: Color(u32_at!(b, 16)),
                    stroke_w: f32_at!(b, 20),
                }
            }
            TAG_TEXT => {
                let b = take!(32);
                DrawCommand::Text {
                    x: f32_at!(b, 0),
                    y: f32_at!(b, 4),
                    text: StringRef {
                        off: u32_at!(b, 8),
                        len: u32_at!(b, 12),
                    },
                    font_id: u32_at!(b, 16),
                    size: f32_at!(b, 20),
                    color: Color(u32_at!(b, 24)),
                    align: match b[28] {
                        1 => TextAlign::Center,
                        2 => TextAlign::Right,
                        3 => TextAlign::Justify,
                        _ => TextAlign::Left,
                    },
                    baseline: match b[29] {
                        1 => TextBaseline::Top,
                        2 => TextBaseline::Middle,
                        3 => TextBaseline::Bottom,
                        _ => TextBaseline::Alphabetic,
                    },
                }
            }
            TAG_IMAGE => {
                let b = take!(20);
                DrawCommand::Image {
                    x: f32_at!(b, 0),
                    y: f32_at!(b, 4),
                    w: f32_at!(b, 8),
                    h: f32_at!(b, 12),
                    bitmap_id: u32_at!(b, 16),
                }
            }
            TAG_PUSH_CLIP => {
                let b = take!(16);
                DrawCommand::PushClip {
                    x: f32_at!(b, 0),
                    y: f32_at!(b, 4),
                    w: f32_at!(b, 8),
                    h: f32_at!(b, 12),
                }
            }
            TAG_POP_CLIP => DrawCommand::PopClip,
            TAG_PUSH_XF => {
                let b = take!(24);
                DrawCommand::PushTransform {
                    a: f32_at!(b, 0),
                    b: f32_at!(b, 4),
                    c: f32_at!(b, 8),
                    d: f32_at!(b, 12),
                    e: f32_at!(b, 16),
                    f: f32_at!(b, 20),
                }
            }
            TAG_POP_XF => DrawCommand::PopTransform,
            other => return Err(DecodeError::UnknownTag(other)),
        };
        Ok(cmd)
    }
}

// --- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dl() -> DisplayList {
        let mut dl = DisplayList::with_capacity(8);
        dl.push(DrawCommand::Clear);
        dl.push(DrawCommand::Rect {
            x: 1.0,
            y: 2.0,
            w: 10.0,
            h: 20.0,
            fill: Color::rgba(255, 0, 0, 255),
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0, 0.0, 0.0, 0.0],
        });
        let s = dl.intern("hello");
        dl.push(DrawCommand::Text {
            x: 3.0,
            y: 4.0,
            text: s,
            font_id: 7,
            size: 12.0,
            color: Color::BLACK,
            align: TextAlign::Center,
            baseline: TextBaseline::Middle,
        });
        dl.push(DrawCommand::PushClip {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        });
        dl.push(DrawCommand::PopClip);
        dl.push(DrawCommand::PushTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        });
        dl.push(DrawCommand::PopTransform);
        dl
    }

    #[test]
    fn roundtrip_preserves_commands() {
        let dl = sample_dl();
        let bytes = dl.to_bytes();
        let rdr = DisplayList::from_bytes(&bytes).unwrap();
        assert_eq!(rdr.cmd_count(), dl.len());
        let decoded: Vec<_> = rdr.iter().map(|c| c.unwrap()).collect();
        assert!(matches!(decoded[0], DrawCommand::Clear));
        assert!(matches!(decoded[1], DrawCommand::Rect { x, .. } if x == 1.0));
        match &decoded[2] {
            DrawCommand::Text {
                text,
                font_id,
                align,
                ..
            } => {
                assert_eq!(*font_id, 7);
                assert_eq!(*align, TextAlign::Center);
                assert_eq!(rdr.string(*text), "hello");
            }
            _ => panic!("expected Text"),
        }
        assert!(matches!(decoded[3], DrawCommand::PushClip { .. }));
        assert!(matches!(decoded[4], DrawCommand::PopClip));
        assert!(matches!(decoded[5], DrawCommand::PushTransform { .. }));
        assert!(matches!(decoded[6], DrawCommand::PopTransform));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = sample_dl().to_bytes();
        bytes[0] = 0xFF;
        assert!(matches!(
            DisplayList::from_bytes(&bytes),
            Err(DecodeError::BadMagic(_))
        ));
    }

    #[test]
    fn rejects_truncated() {
        let bytes = sample_dl().to_bytes();
        assert!(matches!(
            DisplayList::from_bytes(&bytes[..10]),
            Err(DecodeError::TooShort)
        ));
    }

    #[test]
    fn empty_dl_roundtrip() {
        let dl = DisplayList::new();
        let bytes = dl.to_bytes();
        let rdr = DisplayList::from_bytes(&bytes).unwrap();
        assert_eq!(rdr.cmd_count(), 0);
        assert_eq!(rdr.iter().count(), 0);
    }

    #[test]
    fn color_to_css() {
        assert_eq!(Color::rgba(255, 0, 0, 255).to_css(), "#ff0000");
        assert_eq!(Color::rgba(0, 0, 0, 0).to_css(), "rgba(0,0,0,0.000)");
    }
}
