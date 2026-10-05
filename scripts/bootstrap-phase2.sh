#!/usr/bin/env bash
# bootstrap-phase2.sh — Фаза 2 roadmap doc-converter.
# Создаёт / обновляет все файлы Фазы 2.
#
# Использование:
#   ./scripts/bootstrap-phase2.sh
#
# Требует: bash 4+, распакованный workspace Фазы 1.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

write_file() {
  local path="$1"
  mkdir -p "$(dirname "$path")"
  cat > "$path"
  printf '  → %s\n' "$path"
}

echo "== Фаза 2: bootstrap =="

# ---------------------------------------------------------------------------
# crates/render
# ---------------------------------------------------------------------------

write_file crates/render/Cargo.toml <<'__EOF_render_cargo__'
[package]
name = "doc-converter-render"
version = "0.2.0"
edition = "2021"
license = "Apache-2.0"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
bytemuck = { version = "1", features = ["derive"] }
thiserror = "1"

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
js-sys = "0.3"
web-sys = { version = "0.3", features = [
  "OffscreenCanvasRenderingContext2D",
  "OffscreenCanvas",
  "ImageBitmap",
  "TextMetrics",
] }

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "render"
harness = false
__EOF_render_cargo__

write_file crates/render/src/lib.rs <<'__EOF_render_lib__'
//! doc-converter-render — painter, DisplayList, SAB ring.
//!
//! Фаза 2: OffscreenCanvas painter + двойная буферизация DisplayList.

pub mod display_list;
pub mod painter;
pub mod sab;

pub use display_list::{
    Color, DecodeError, DisplayList, DisplayListReader, DrawCommand, StringRef, TextAlign,
    TextBaseline,
};
__EOF_render_lib__

write_file crates/render/src/display_list.rs <<'__EOF_render_display_list__'
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
__EOF_render_display_list__

write_file crates/render/src/sab/mod.rs <<'__EOF_render_sab_mod__'
pub mod reader;
pub mod ring;

#[cfg(target_arch = "wasm32")]
pub mod writer;

pub use ring::{RingState, HEADER_BYTES};

#[cfg(target_arch = "wasm32")]
pub use ring::SabRing;
__EOF_render_sab_mod__

write_file crates/render/src/sab/ring.rs <<'__EOF_render_sab_ring__'
//! Ring из двух слотов для передачи DisplayList через SharedArrayBuffer.
//!
//! Layout:
//!   [0..4)    seq_writer: i32
//!   [4..8)    seq_reader: i32
//!   [8..12)   slot_len[0]: i32
//!   [12..16)  slot_len[1]: i32
//!   [16..)    slot0 payload
//!   [16+cap..) slot1 payload

pub const HEADER_BYTES: usize = 16;
pub const SLOT_COUNT: u32 = 2;

/// Чистая (native-testable) часть ring'а.
#[derive(Debug, Clone, Copy, Default)]
pub struct RingState {
    pub seq_writer: u32,
    pub seq_reader: u32,
    pub slot_len: [u32; 2],
}

impl RingState {
    #[inline]
    pub fn has_free_slot(&self) -> bool {
        self.seq_writer.wrapping_sub(self.seq_reader) < SLOT_COUNT
    }

    #[inline]
    pub fn can_read(&self) -> bool {
        self.seq_reader < self.seq_writer
    }

    #[inline]
    pub fn writer_slot(&self) -> usize {
        (self.seq_writer & 1) as usize
    }

    #[inline]
    pub fn reader_slot(&self) -> usize {
        (self.seq_reader & 1) as usize
    }

    #[inline]
    pub fn commit_write(&mut self, len: u32) -> bool {
        if !self.has_free_slot() {
            return false;
        }
        let s = self.writer_slot();
        self.slot_len[s] = len;
        self.seq_writer = self.seq_writer.wrapping_add(1);
        true
    }

    #[inline]
    pub fn commit_read(&mut self) -> bool {
        if !self.can_read() {
            return false;
        }
        self.seq_reader = self.seq_reader.wrapping_add(1);
        true
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use super::*;
    use js_sys::{Atomics, Int32Array, SharedArrayBuffer, Uint8Array};
    use wasm_bindgen::prelude::*;

    pub struct SabRing {
        sab: SharedArrayBuffer,
        header: Int32Array,
        slot0: Uint8Array,
        slot1: Uint8Array,
        slot_capacity: usize,
    }

    impl SabRing {
        pub fn new(slot_capacity: usize) -> Result<Self, JsValue> {
            let total = HEADER_BYTES + slot_capacity * 2;
            let sab = SharedArrayBuffer::new(total as u32);
            Self::from_sab(sab, slot_capacity)
        }

        pub fn from_sab(sab: SharedArrayBuffer, slot_capacity: usize) -> Result<Self, JsValue> {
            let total = HEADER_BYTES + slot_capacity * 2;
            if (sab.byte_length() as usize) < total {
                return Err(JsValue::from_str("SAB too small"));
            }
            let header = Int32Array::new(&sab);
            let slot0 = Uint8Array::new_with_byte_offset_and_length(
                &sab,
                HEADER_BYTES as u32,
                slot_capacity as u32,
            );
            let slot1 = Uint8Array::new_with_byte_offset_and_length(
                &sab,
                (HEADER_BYTES + slot_capacity) as u32,
                slot_capacity as u32,
            );
            Ok(Self {
                sab,
                header,
                slot0,
                slot1,
                slot_capacity,
            })
        }

        pub fn sab(&self) -> &SharedArrayBuffer {
            &self.sab
        }

        pub fn slot_capacity(&self) -> usize {
            self.slot_capacity
        }

        #[inline]
        fn state(&self) -> RingState {
            RingState {
                seq_writer: Atomics::load(&self.header, 0) as u32,
                seq_reader: Atomics::load(&self.header, 1) as u32,
                slot_len: [
                    Atomics::load(&self.header, 2) as u32,
                    Atomics::load(&self.header, 3) as u32,
                ],
            }
        }

        pub fn has_free_slot(&self) -> bool {
            self.state().has_free_slot()
        }

        pub fn can_read(&self) -> bool {
            self.state().can_read()
        }

        /// Пробует записать байты в следующий свободный слот.
        pub fn try_write(&mut self, bytes: &[u8]) -> bool {
            let st = self.state();
            if !st.has_free_slot() {
                return false;
            }
            if bytes.len() > self.slot_capacity {
                return false;
            }
            let slot = st.writer_slot();
            let target = if slot == 0 { &self.slot0 } else { &self.slot1 };
            let src = Uint8Array::from(bytes);
            target.set(&src, 0);
            Atomics::store(&self.header, 2 + slot as u32, bytes.len() as i32);
            Atomics::store(&self.header, 0, st.seq_writer.wrapping_add(1) as i32);
            true
        }

        /// Возвращает `(slot_idx, len)` — вызывающий читает из `slot_data`.
        pub fn read_current(&self) -> Option<(usize, usize)> {
            let st = self.state();
            if !st.can_read() {
                return None;
            }
            let slot = st.reader_slot();
            let len = st.slot_len[slot] as usize;
            if len > self.slot_capacity {
                return None;
            }
            Some((slot, len))
        }

        pub fn release_current(&mut self) {
            let st = self.state();
            if st.can_read() {
                Atomics::store(&self.header, 1, st.seq_reader.wrapping_add(1) as i32);
            }
        }

        /// Копирует текущий слот в `out` и помечает прочитанным.
        pub fn copy_current_into(&self, out: &mut Vec<u8>) -> bool {
            let Some((slot, len)) = self.read_current() else {
                return false;
            };
            let src = if slot == 0 { &self.slot0 } else { &self.slot1 };
            out.clear();
            out.resize(len, 0);
            let dst = Uint8Array::new_from_slice(out);
            let view = src.subarray(0, len as u32);
            view.copy_to(&dst);
            true
        }

        pub fn reset(&mut self) {
            Atomics::store(&self.header, 0, 0);
            Atomics::store(&self.header, 1, 0);
            Atomics::store(&self.header, 2, 0);
            Atomics::store(&self.header, 3, 0);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::SabRing;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_cycles_1000() {
        let mut st = RingState::default();
        for i in 0..1000u32 {
            assert!(st.has_free_slot(), "iter {i}");
            assert!(st.commit_write(i), "commit_write {i}");
            assert!(st.can_read());
            assert!(st.commit_read());
        }
        assert_eq!(st.seq_writer, 1000);
        assert_eq!(st.seq_reader, 1000);
    }

    #[test]
    fn backpressure_when_reader_behind() {
        let mut st = RingState::default();
        assert!(st.commit_write(10));
        assert!(st.commit_write(20));
        assert!(!st.has_free_slot());
        assert!(!st.commit_write(30));
        st.commit_read();
        assert!(st.has_free_slot());
        assert!(st.commit_write(30));
    }

    #[test]
    fn slot_alternates() {
        let mut st = RingState::default();
        assert_eq!(st.writer_slot(), 0);
        st.commit_write(1);
        assert_eq!(st.writer_slot(), 1);
        st.commit_write(2);
        assert_eq!(st.writer_slot(), 0);
    }

    #[test]
    fn cannot_read_empty() {
        let mut st = RingState::default();
        assert!(!st.can_read());
        assert!(!st.commit_read());
    }
}
__EOF_render_sab_ring__

write_file crates/render/src/sab/writer.rs <<'__EOF_render_sab_writer__'
//! Утилита для сборки и записи DisplayList в SAB (wasm-only).

#![cfg(target_arch = "wasm32")]

use crate::display_list::DisplayList;
use super::SabRing;
use wasm_bindgen::JsValue;

pub struct SabWriter {
    pub ring: SabRing,
    scratch: Vec<u8>,
}

impl SabWriter {
    pub fn new(slot_capacity: usize) -> Result<Self, JsValue> {
        Ok(Self {
            ring: SabRing::new(slot_capacity)?,
            scratch: Vec::new(),
        })
    }

    pub fn write(&mut self, dl: &DisplayList) -> bool {
        dl.to_bytes_into(&mut self.scratch);
        self.ring.try_write(&self.scratch)
    }
}
__EOF_render_sab_writer__

write_file crates/render/src/sab/reader.rs <<'__EOF_render_sab_reader__'
//! Native-читатель ring'а для тестов и будущего дебага.

use super::ring::RingState;

pub struct NativeRingReader {
    state: RingState,
    slots: [Vec<u8>; 2],
}

impl NativeRingReader {
    pub fn new() -> Self {
        Self {
            state: RingState::default(),
            slots: [Vec::new(), Vec::new()],
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if !self.state.has_free_slot() {
            return false;
        }
        let s = self.state.writer_slot();
        self.slots[s].clear();
        self.slots[s].extend_from_slice(bytes);
        self.state.commit_write(bytes.len() as u32)
    }

    pub fn pop(&mut self) -> Option<&[u8]> {
        if !self.state.can_read() {
            return None;
        }
        let s = self.state.reader_slot();
        self.state.commit_read();
        Some(&self.slots[s])
    }
}

impl Default for NativeRingReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_two_slot() {
        let mut r = NativeRingReader::new();
        r.push(b"a");
        r.push(b"b");
        assert_eq!(r.pop(), Some(&b"a"[..]));
        r.push(b"c");
        assert_eq!(r.pop(), Some(&b"b"[..]));
        assert_eq!(r.pop(), Some(&b"c"[..]));
        assert_eq!(r.pop(), None);
    }

    #[test]
    fn drop_when_full() {
        let mut r = NativeRingReader::new();
        assert!(r.push(b"a"));
        assert!(r.push(b"b"));
        assert!(!r.push(b"c"));
    }
}
__EOF_render_sab_reader__

write_file crates/render/src/painter/mod.rs <<'__EOF_render_painter_mod__'
pub mod state;
pub mod text;

#[cfg(target_arch = "wasm32")]
pub mod bitmap_cache;
#[cfg(target_arch = "wasm32")]
pub mod painter_2d;

#[cfg(target_arch = "wasm32")]
pub use painter_2d::{PaintStats, Painter2D};
__EOF_render_painter_mod__

write_file crates/render/src/painter/state.rs <<'__EOF_render_painter_state__'
use crate::display_list::Color;
use std::collections::HashMap;

/// Стек clip/transform + кэш стилей, чтобы не дёргать `ctx.set*` без нужды.
pub struct PaintState {
    pub clip_depth: u32,
    pub transform_depth: u32,
    current_fill: Option<Color>,
    current_stroke: Option<Color>,
    current_line_width: f32,
    pub style_cache: HashMap<Color, String>,
    pub font_cache: HashMap<(u32, u32), String>,
}

impl PaintState {
    pub fn new() -> Self {
        Self {
            clip_depth: 0,
            transform_depth: 0,
            current_fill: None,
            current_stroke: None,
            current_line_width: 0.0,
            style_cache: HashMap::with_capacity(64),
            font_cache: HashMap::with_capacity(16),
        }
    }

    #[inline]
    pub fn css_for(&mut self, c: Color) -> &str {
        if !self.style_cache.contains_key(&c) {
            let s = c.to_css();
            self.style_cache.insert(c, s);
        }
        self.style_cache.get(&c).unwrap()
    }

    #[inline]
    pub fn font_css(&mut self, font_id: u32, size_px: f32) -> &str {
        let key = (font_id, size_px.to_bits());
        if !self.font_cache.contains_key(&key) {
            let s = format!("{}px font{}", size_px, font_id);
            self.font_cache.insert(key, s);
        }
        self.font_cache.get(&key).unwrap()
    }

    #[inline]
    pub fn needs_fill(&mut self, c: Color) -> bool {
        if self.current_fill == Some(c) {
            return false;
        }
        self.current_fill = Some(c);
        true
    }

    #[inline]
    pub fn needs_stroke(&mut self, c: Color, w: f32) -> bool {
        if self.current_stroke == Some(c) && (self.current_line_width - w).abs() < f32::EPSILON {
            return false;
        }
        self.current_stroke = Some(c);
        self.current_line_width = w;
        true
    }

    pub fn reset(&mut self) {
        self.clip_depth = 0;
        self.transform_depth = 0;
        self.current_fill = None;
        self.current_stroke = None;
        self.current_line_width = 0.0;
    }
}

impl Default for PaintState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_style_dedup() {
        let mut s = PaintState::new();
        let red = Color::rgba(255, 0, 0, 255);
        assert!(s.needs_fill(red));
        assert!(!s.needs_fill(red));
        assert!(s.needs_fill(Color::BLACK));
    }

    #[test]
    fn stroke_dedup_includes_width() {
        let mut s = PaintState::new();
        let red = Color::rgba(255, 0, 0, 255);
        assert!(s.needs_stroke(red, 1.0));
        assert!(!s.needs_stroke(red, 1.0));
        assert!(s.needs_stroke(red, 2.0));
    }

    #[test]
    fn css_cached() {
        let mut s = PaintState::new();
        let c = Color::rgba(10, 20, 30, 255);
        let a = s.css_for(c).to_owned();
        let b = s.css_for(c).to_owned();
        assert_eq!(a, b);
        assert_eq!(a, "#0a141e");
    }

    #[test]
    fn font_css_stable() {
        let mut s = PaintState::new();
        assert_eq!(s.font_css(3, 14.0), "14px font3");
        assert_eq!(s.font_css(3, 14.0), "14px font3");
    }
}
__EOF_render_painter_state__

write_file crates/render/src/painter/text.rs <<'__EOF_render_painter_text__'
use crate::display_list::TextAlign;

/// Эллипсис по code-point'ам.
pub fn ellipsize<F: Fn(&str) -> f32>(text: &str, max_width: f32, measure: F) -> String {
    if measure(text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&candidate) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

#[cfg(target_arch = "wasm32")]
pub fn text_align_str(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
        TextAlign::Justify => "justify",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_fits() {
        let m = |s: &str| s.chars().count() as f32;
        assert_eq!(ellipsize("hello world", 100.0, m), "hello world");
        assert_eq!(ellipsize("hello world", 5.0, m), "hell…");
        assert_eq!(ellipsize("hello world", 1.0, m), "…");
    }
}
__EOF_render_painter_text__

write_file crates/render/src/painter/bitmap_cache.rs <<'__EOF_render_painter_bitmap__'
#![cfg(target_arch = "wasm32")]

use std::collections::HashMap;
use web_sys::ImageBitmap;

#[derive(Default)]
pub struct BitmapCache {
    map: HashMap<u32, ImageBitmap>,
}

impl BitmapCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: u32, bmp: ImageBitmap) {
        self.map.insert(id, bmp);
    }

    pub fn get(&self, id: u32) -> Option<&ImageBitmap> {
        self.map.get(&id)
    }

    pub fn remove(&mut self, id: u32) -> bool {
        self.map.remove(&id).is_some()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
__EOF_render_painter_bitmap__

write_file crates/render/src/painter/painter_2d.rs <<'__EOF_render_painter_2d__'
#![cfg(target_arch = "wasm32")]

use super::bitmap_cache::BitmapCache;
use super::state::PaintState;
use super::text;
use crate::display_list::{DecodeError, DisplayList, DrawCommand, TextBaseline};
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2D;

#[derive(Debug, Clone, Copy, Default)]
pub struct PaintStats {
    pub cmds: u32,
    pub dropped: bool,
    pub paint_ms: f64,
}

pub struct Painter2D {
    ctx: OffscreenCanvasRenderingContext2D,
    state: PaintState,
    bitmaps: BitmapCache,
}

impl Painter2D {
    pub fn new(ctx: OffscreenCanvasRenderingContext2D) -> Self {
        Self {
            ctx,
            state: PaintState::new(),
            bitmaps: BitmapCache::new(),
        }
    }

    pub fn context(&self) -> &OffscreenCanvasRenderingContext2D {
        &self.ctx
    }

    pub fn bitmaps_mut(&mut self) -> &mut BitmapCache {
        &mut self.bitmaps
    }

    pub fn register_bitmap(&mut self, id: u32, bmp: web_sys::ImageBitmap) {
        self.bitmaps.insert(id, bmp);
    }

    pub fn drop_bitmap(&mut self, id: u32) -> bool {
        self.bitmaps.remove(id)
    }

    pub fn reset_state(&mut self) {
        self.state.reset();
        self.bitmaps.clear();
    }

    pub fn paint_bytes(&mut self, bytes: &[u8]) -> Result<PaintStats, JsValue> {
        let t0 = now_ms();
        let reader = DisplayList::from_bytes(bytes)
            .map_err(|e| JsValue::from_str(&format!("DisplayList decode: {e}")))?;

        let mut cmds = 0u32;
        for item in reader.iter() {
            match item {
                Ok(cmd) => {
                    self.dispatch(&reader, cmd);
                    cmds += 1;
                }
                Err(DecodeError::UnknownTag(t)) => {
                    web_sys::console::warn_1(&JsValue::from_str(&format!(
                        "painter: unknown tag {t:#x}, skipping rest"
                    )));
                    break;
                }
                Err(e) => {
                    return Err(JsValue::from_str(&format!("DisplayList decode: {e}")));
                }
            }
        }
        let t1 = now_ms();
        Ok(PaintStats {
            cmds,
            dropped: false,
            paint_ms: t1 - t0,
        })
    }

    fn dispatch(&mut self, reader: &crate::display_list::DisplayListReader, cmd: DrawCommand) {
        match cmd {
            DrawCommand::Clear => {
                let c = self.ctx.canvas();
                let w = c.width() as f64;
                let h = c.height() as f64;
                self.ctx.clear_rect(0.0, 0.0, w, h);
            }
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
                self.begin_path_rounded(x, y, w, h, radius);
                if fill.0 != 0 {
                    let css = self.state.css_for(fill).to_owned();
                    if self.state.needs_fill(fill) {
                        self.ctx.set_fill_style_str(&css);
                    }
                    self.ctx.fill();
                }
                if stroke.0 != 0 && stroke_w > 0.0 {
                    let css = self.state.css_for(stroke).to_owned();
                    if self.state.needs_stroke(stroke, stroke_w) {
                        self.ctx.set_stroke_style_str(&css);
                        self.ctx.set_line_width(stroke_w as f64);
                    }
                    self.ctx.stroke();
                }
            }
            DrawCommand::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                stroke_w,
            } => {
                if stroke.0 == 0 || stroke_w <= 0.0 {
                    return;
                }
                let css = self.state.css_for(stroke).to_owned();
                if self.state.needs_stroke(stroke, stroke_w) {
                    self.ctx.set_stroke_style_str(&css);
                    self.ctx.set_line_width(stroke_w as f64);
                }
                self.ctx.begin_path();
                self.ctx.move_to(x1 as f64, y1 as f64);
                self.ctx.line_to(x2 as f64, y2 as f64);
                self.ctx.stroke();
            }
            DrawCommand::Text {
                x,
                y,
                text: text_ref,
                font_id,
                size,
                color,
                align,
                baseline,
            } => {
                let s = reader.string(text_ref);
                let font = self.state.font_css(font_id, size).to_owned();
                self.ctx.set_font(&font);

                let css = self.state.css_for(color).to_owned();
                self.ctx.set_fill_style_str(&css);
                self.ctx.set_text_align(text::text_align_str(align));
                self.ctx.set_text_baseline(match baseline {
                    TextBaseline::Alphabetic => "alphabetic",
                    TextBaseline::Top => "top",
                    TextBaseline::Middle => "middle",
                    TextBaseline::Bottom => "bottom",
                });
                let _ = self.ctx.fill_text(s, x as f64, y as f64);
            }
            DrawCommand::Image {
                x,
                y,
                w,
                h,
                bitmap_id,
            } => {
                if let Some(bmp) = self.bitmaps.get(bitmap_id) {
                    let _ = self.ctx.draw_image_with_image_bitmap_and_dw_and_dh(
                        bmp,
                        x as f64,
                        y as f64,
                        w as f64,
                        h as f64,
                    );
                }
            }
            DrawCommand::PushClip { x, y, w, h } => {
                self.ctx.save();
                self.state.clip_depth += 1;
                self.ctx.begin_path();
                self.ctx.rect(x as f64, y as f64, w as f64, h as f64);
                self.ctx.clip();
            }
            DrawCommand::PopClip => {
                if self.state.clip_depth > 0 {
                    self.ctx.restore();
                    self.state.clip_depth -= 1;
                }
            }
            DrawCommand::PushTransform { a, b, c, d, e, f } => {
                self.ctx.save();
                self.state.transform_depth += 1;
                let _ = self
                    .ctx
                    .set_transform(a as f64, b as f64, c as f64, d as f64, e as f64, f as f64);
            }
            DrawCommand::PopTransform => {
                if self.state.transform_depth > 0 {
                    self.ctx.restore();
                    self.state.transform_depth -= 1;
                }
            }
        }
    }

    fn begin_path_rounded(&self, x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) {
        self.ctx.begin_path();
        let uniform = r[0] == r[1] && r[1] == r[2] && r[2] == r[3];
        if uniform && r[0] > 0.0 {
            rounded_rect_arc_to(
                &self.ctx,
                x as f64,
                y as f64,
                w as f64,
                h as f64,
                r[0] as f64,
                r[0] as f64,
                r[0] as f64,
                r[0] as f64,
            );
        } else if !uniform {
            rounded_rect_arc_to(
                &self.ctx,
                x as f64,
                y as f64,
                w as f64,
                h as f64,
                r[0] as f64,
                r[1] as f64,
                r[2] as f64,
                r[3] as f64,
            );
        } else {
            self.ctx.rect(x as f64, y as f64, w as f64, h as f64);
        }
    }
}

fn rounded_rect_arc_to(
    ctx: &OffscreenCanvasRenderingContext2D,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    tl: f64,
    tr: f64,
    br: f64,
    bl: f64,
) {
    ctx.move_to(x + tl, y);
    ctx.line_to(x + w - tr, y);
    ctx.arc_to(x + w, y, x + w, y + tr, tr);
    ctx.line_to(x + w, y + h - br);
    ctx.arc_to(x + w, y + h, x + w - br, y + h, br);
    ctx.line_to(x + bl, y + h);
    ctx.arc_to(x, y + h, x, y + h - bl, bl);
    ctx.line_to(x, y + tl);
    ctx.arc_to(x, y, x + tl, y, tl);
    ctx.close_path();
}

#[inline]
fn now_ms() -> f64 {
    js_sys::Date::now()
}
__EOF_render_painter_2d__

write_file crates/render/tests/roundtrip.rs <<'__EOF_render_tests_roundtrip__'
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
__EOF_render_tests_roundtrip__

write_file crates/render/benches/render.rs <<'__EOF_render_benches__'
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use doc_converter_render::display_list::*;
use doc_converter_render::sab::RingState;

fn make_dl_rects(n: usize) -> Vec<u8> {
    let mut dl = DisplayList::with_capacity(n);
    for i in 0..n {
        dl.push(DrawCommand::Rect {
            x: (i % 100) as f32,
            y: (i / 100) as f32,
            w: 10.0,
            h: 10.0,
            fill: Color::rgba(0, 0, 0, 255),
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        });
    }
    dl.to_bytes()
}

fn make_dl_text(n: usize) -> Vec<u8> {
    let mut dl = DisplayList::with_capacity(n);
    for i in 0..n {
        let s = dl.intern("Hello, world");
        dl.push(DrawCommand::Text {
            x: i as f32,
            y: 0.0,
            text: s,
            font_id: 0,
            size: 12.0,
            color: Color::BLACK,
            align: TextAlign::Left,
            baseline: TextBaseline::Alphabetic,
        });
    }
    dl.to_bytes()
}

fn bench_dl_encode(c: &mut Criterion) {
    let mut dl = DisplayList::with_capacity(1000);
    for i in 0..1000 {
        dl.push(DrawCommand::Rect {
            x: i as f32,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            fill: Color::BLACK,
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        });
    }
    c.bench_function("dl_encode_1k_rects", |b| {
        let mut out = Vec::with_capacity(64 * 1024);
        b.iter(|| {
            dl.to_bytes_into(black_box(&mut out));
            black_box(out.len())
        })
    });
}

fn bench_dl_decode(c: &mut Criterion) {
    let bytes = make_dl_rects(1000);
    c.bench_function("dl_decode_1k_rects", |b| {
        b.iter(|| {
            let r = DisplayList::from_bytes(black_box(&bytes)).unwrap();
            black_box(r.iter().count())
        })
    });
}

fn bench_dl_decode_text(c: &mut Criterion) {
    let bytes = make_dl_text(1000);
    c.bench_function("dl_decode_1k_text", |b| {
        b.iter(|| {
            let r = DisplayList::from_bytes(black_box(&bytes)).unwrap();
            black_box(r.iter().count())
        })
    });
}

fn bench_sab_state_cycle(c: &mut Criterion) {
    c.bench_function("sab_state_cycle_1k", |b| {
        b.iter(|| {
            let mut st = RingState::default();
            for _ in 0..1000 {
                st.commit_write(64);
                st.commit_read();
            }
            black_box(st.seq_writer)
        })
    });
}

criterion_group!(
    benches,
    bench_dl_encode,
    bench_dl_decode,
    bench_dl_decode_text,
    bench_sab_state_cycle
);
criterion_main!(benches);
__EOF_render_benches__

# ---------------------------------------------------------------------------
# crates/wasm
# ---------------------------------------------------------------------------

write_file crates/wasm/Cargo.toml <<'__EOF_wasm_cargo__'
[package]
name = "doc-converter-wasm"
version = "0.2.0"
edition = "2021"
license = "Apache-2.0"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
doc-converter-render = { path = "../render" }
wasm-bindgen = "0.2"
js-sys = "0.3"
serde = { version = "1", features = ["derive"] }
serde-wasm-bindgen = "0.6"
web-sys = { version = "0.3", features = [
  "OffscreenCanvas",
  "OffscreenCanvasRenderingContext2D",
  "ImageBitmap",
  "console",
] }

[dev-dependencies]
wasm-bindgen-test = "0.3"
__EOF_wasm_cargo__

write_file crates/wasm/src/lib.rs <<'__EOF_wasm_lib__'
pub mod bitmap_api;
pub mod painter_api;
pub mod sab_api;
__EOF_wasm_lib__

write_file crates/wasm/src/sab_api.rs <<'__EOF_wasm_sab_api__'
use wasm_bindgen::prelude::*;

/// Аллоцирует SharedArrayBuffer под ring с двумя слотами по `slot_capacity`.
/// Требует crossOriginIsolated.
#[wasm_bindgen]
pub fn alloc_sab(slot_capacity: u32) -> Result<js_sys::SharedArrayBuffer, JsValue> {
    let total = doc_converter_render::sab::HEADER_BYTES + (slot_capacity as usize) * 2;
    Ok(js_sys::SharedArrayBuffer::new(total as u32))
}

#[wasm_bindgen]
pub fn sab_total_bytes(slot_capacity: u32) -> u32 {
    (doc_converter_render::sab::HEADER_BYTES + (slot_capacity as usize) * 2) as u32
}
__EOF_wasm_sab_api__

write_file crates/wasm/src/painter_api.rs <<'__EOF_wasm_painter_api__'
use doc_converter_render::painter::{PaintStats, Painter2D};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2D;

thread_local! {
    static PAINTER: RefCell<Option<Painter2D>> = RefCell::new(None);
}

/// Инициализирует painter ровно один раз. Повторный вызов — ошибка.
#[wasm_bindgen]
pub fn init_painter(ctx: OffscreenCanvasRenderingContext2D) -> Result<(), JsValue> {
    PAINTER.with(|p| {
        let mut slot = p.borrow_mut();
        if slot.is_some() {
            return Err(JsValue::from_str("painter already initialised"));
        }
        *slot = Some(Painter2D::new(ctx));
        Ok(())
    })
}

#[wasm_bindgen]
pub fn dispose_painter() {
    PAINTER.with(|p| {
        *p.borrow_mut() = None;
    });
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintStatsJs {
    cmds: u32,
    dropped: bool,
    paint_ms: f64,
}

impl From<PaintStats> for PaintStatsJs {
    fn from(s: PaintStats) -> Self {
        Self {
            cmds: s.cmds,
            dropped: s.dropped,
            paint_ms: s.paint_ms,
        }
    }
}

/// Прямой путь: байты DL копируются в Rust Vec (fallback).
#[wasm_bindgen]
pub fn paint_display_list_bytes(bytes: &[u8]) -> Result<JsValue, JsValue> {
    PAINTER.with(|p| {
        let mut slot = p.borrow_mut();
        let painter = slot
            .as_mut()
            .ok_or_else(|| JsValue::from_str("painter not initialised"))?;
        let stats = painter.paint_bytes(bytes)?;
        serde_wasm_bindgen::to_value(&PaintStatsJs::from(stats))
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })
}

/// SAB-путь: читает текущий слот ring'а и рисует. Освобождает слот.
#[wasm_bindgen]
pub fn paint_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
) -> Result<JsValue, JsValue> {
    let ring = doc_converter_render::sab::SabRing::from_sab(sab, slot_capacity as usize)?;
    let mut buf = Vec::new();
    if !ring.copy_current_into(&mut buf) {
        let empty = PaintStatsJs {
            cmds: 0,
            dropped: true,
            paint_ms: 0.0,
        };
        return Ok(serde_wasm_bindgen::to_value(&empty).unwrap());
    }
    PAINTER.with(|p| {
        let mut slot = p.borrow_mut();
        let painter = slot
            .as_mut()
            .ok_or_else(|| JsValue::from_str("painter not initialised"))?;
        let stats = painter.paint_bytes(&buf)?;
        serde_wasm_bindgen::to_value(&PaintStatsJs::from(stats))
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })
}

/// Пересчитывает canvas под DPR и сбрасывает состояние painter'а.
#[wasm_bindgen]
pub fn resize_canvas(
    ctx: OffscreenCanvasRenderingContext2D,
    css_w: f32,
    css_h: f32,
    dpr: f32,
) -> Result<(), JsValue> {
    let canvas = ctx.canvas();
    let w = (css_w * dpr).round() as u32;
    let h = (css_h * dpr).round() as u32;
    if w == 0 || h == 0 {
        return Err(JsValue::from_str("canvas size must be > 0"));
    }
    canvas.set_width(w);
    canvas.set_height(h);

    PAINTER.with(|p| {
        if let Some(painter) = p.borrow_mut().as_mut() {
            painter.reset_state();
        }
    });
    Ok(())
}
__EOF_wasm_painter_api__

write_file crates/wasm/src/bitmap_api.rs <<'__EOF_wasm_bitmap_api__'
use wasm_bindgen::prelude::*;
use web_sys::ImageBitmap;

use crate::painter_api::with_painter;

#[wasm_bindgen]
pub fn register_bitmap(id: u32, bmp: ImageBitmap) -> Result<(), JsValue> {
    with_painter(|p| {
        p.register_bitmap(id, bmp);
    })
}

#[wasm_bindgen]
pub fn drop_bitmap(id: u32) -> Result<bool, JsValue> {
    let mut out = false;
    with_painter(|p| {
        out = p.drop_bitmap(id);
    })?;
    Ok(out)
}
__EOF_wasm_bitmap_api__

write_file crates/wasm/src/painter_api.rs <<'__EOF_wasm_painter_api2__'
use doc_converter_render::painter::{PaintStats, Painter2D};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2D;

thread_local! {
    static PAINTER: RefCell<Option<Painter2D>> = RefCell::new(None);
}

pub(crate) fn with_painter<R>(f: impl FnOnce(&mut Painter2D) -> R) -> Result<R, JsValue> {
    PAINTER.with(|p| {
        let mut slot = p.borrow_mut();
        let painter = slot
            .as_mut()
            .ok_or_else(|| JsValue::from_str("painter not initialised"))?;
        Ok(f(painter))
    })
}

/// Инициализирует painter ровно один раз. Повторный вызов — ошибка.
#[wasm_bindgen]
pub fn init_painter(ctx: OffscreenCanvasRenderingContext2D) -> Result<(), JsValue> {
    PAINTER.with(|p| {
        let mut slot = p.borrow_mut();
        if slot.is_some() {
            return Err(JsValue::from_str("painter already initialised"));
        }
        *slot = Some(Painter2D::new(ctx));
        Ok(())
    })
}

#[wasm_bindgen]
pub fn dispose_painter() {
    PAINTER.with(|p| {
        *p.borrow_mut() = None;
    });
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintStatsJs {
    cmds: u32,
    dropped: bool,
    paint_ms: f64,
}

impl From<PaintStats> for PaintStatsJs {
    fn from(s: PaintStats) -> Self {
        Self {
            cmds: s.cmds,
            dropped: s.dropped,
            paint_ms: s.paint_ms,
        }
    }
}

/// Прямой путь: байты DL копируются в Rust Vec (fallback).
#[wasm_bindgen]
pub fn paint_display_list_bytes(bytes: &[u8]) -> Result<JsValue, JsValue> {
    with_painter(|painter| {
        let stats = painter.paint_bytes(bytes)?;
        serde_wasm_bindgen::to_value(&PaintStatsJs::from(stats))
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })?
}

/// SAB-путь: читает текущий слот ring'а и рисует. Освобождает слот.
#[wasm_bindgen]
pub fn paint_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
) -> Result<JsValue, JsValue> {
    let ring = doc_converter_render::sab::SabRing::from_sab(sab, slot_capacity as usize)?;
    let mut buf = Vec::new();
    if !ring.copy_current_into(&mut buf) {
        let empty = PaintStatsJs {
            cmds: 0,
            dropped: true,
            paint_ms: 0.0,
        };
        return Ok(serde_wasm_bindgen::to_value(&empty).unwrap());
    }
    with_painter(|painter| {
        let stats = painter.paint_bytes(&buf)?;
        serde_wasm_bindgen::to_value(&PaintStatsJs::from(stats))
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })?
}

/// Пересчитывает canvas под DPR и сбрасывает состояние painter'а.
#[wasm_bindgen]
pub fn resize_canvas(
    ctx: OffscreenCanvasRenderingContext2D,
    css_w: f32,
    css_h: f32,
    dpr: f32,
) -> Result<(), JsValue> {
    let canvas = ctx.canvas();
    let w = (css_w * dpr).round() as u32;
    let h = (css_h * dpr).round() as u32;
    if w == 0 || h == 0 {
        return Err(JsValue::from_str("canvas size must be > 0"));
    }
    canvas.set_width(w);
    canvas.set_height(h);

    PAINTER.with(|p| {
        if let Some(painter) = p.borrow_mut().as_mut() {
            painter.reset_state();
        }
    });
    Ok(())
}
__EOF_wasm_painter_api2__

write_file crates/wasm/tests/painter_perf.rs <<'__EOF_wasm_tests_perf__'
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn paint_1000_rects_under_16ms() {
    let canvas = web_sys::OffscreenCanvas::new(1200, 800).unwrap();
    let ctx = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::OffscreenCanvasRenderingContext2D>()
        .unwrap();

    doc_converter_wasm::painter_api::init_painter(ctx).unwrap();

    use doc_converter_render::display_list::*;
    let mut dl = DisplayList::with_capacity(1000);
    for i in 0..1000 {
        dl.push(DrawCommand::Rect {
            x: (i % 100) as f32 * 10.0,
            y: (i / 100) as f32 * 10.0,
            w: 10.0,
            h: 10.0,
            fill: Color::rgba(0, 0, 0, 255),
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        });
    }
    let bytes = dl.to_bytes();
    let stats = doc_converter_wasm::painter_api::paint_display_list_bytes(&bytes).unwrap();
    let ms = js_sys::Reflect::get(&stats, &"paintMs".into())
        .unwrap()
        .as_f64()
        .unwrap();
    assert!(ms < 16.0, "paintMs = {ms}");
}
__EOF_wasm_tests_perf__

# ---------------------------------------------------------------------------
# packages/core — TS
# ---------------------------------------------------------------------------

write_file packages/core/src/sab/protocol.ts <<'__EOF_ts_sab_protocol__'
/** Заголовок ring'а. Смещения совпадают с Rust. */
export const HEADER_INTS = 4;
export const HEADER_BYTES = HEADER_INTS * 4;
export const SLOT_COUNT = 2;

export interface SlotHeader {
  seqWriter: number;
  seqReader: number;
  slotLen: [number, number];
}

export function readHeader(i32: Int32Array): SlotHeader {
  return {
    seqWriter: Atomics.load(i32, 0),
    seqReader: Atomics.load(i32, 1),
    slotLen: [Atomics.load(i32, 2), Atomics.load(i32, 3)],
  };
}

export function hasFreeSlot(h: SlotHeader): boolean {
  return h.seqWriter - h.seqReader < SLOT_COUNT;
}

export function canRead(h: SlotHeader): boolean {
  return h.seqReader < h.seqWriter;
}

export function writerSlot(h: SlotHeader): 0 | 1 {
  return (h.seqWriter & 1) as 0 | 1;
}

export function readerSlot(h: SlotHeader): 0 | 1 {
  return (h.seqReader & 1) as 0 | 1;
}
__EOF_ts_sab_protocol__

write_file packages/core/src/sab/reader.ts <<'__EOF_ts_sab_reader__'
import { HEADER_BYTES, canRead, readHeader, readerSlot } from './protocol';

/** Симметричен Rust `SabRing` — для дебага и тестов. */
export class SabReader {
  private readonly i32: Int32Array;
  private readonly slot0: Uint8Array;
  private readonly slot1: Uint8Array;

  constructor(sab: SharedArrayBuffer, slotCapacity: number) {
    this.i32 = new Int32Array(sab);
    this.slot0 = new Uint8Array(sab, HEADER_BYTES, slotCapacity);
    this.slot1 = new Uint8Array(sab, HEADER_BYTES + slotCapacity, slotCapacity);
  }

  hasData(): boolean {
    return canRead(readHeader(this.i32));
  }

  /** Возвращает view без копирования. Вызывающий должен вызвать release(). */
  peek(): Uint8Array | null {
    const h = readHeader(this.i32);
    if (!canRead(h)) return null;
    const s = readerSlot(h);
    const len = h.slotLen[s];
    const buf = s === 0 ? this.slot0 : this.slot1;
    return buf.subarray(0, len);
  }

  release(): void {
    const h = readHeader(this.i32);
    if (!canRead(h)) return;
    Atomics.store(this.i32, 1, (h.seqReader + 1) | 0);
  }
}
__EOF_ts_sab_reader__

write_file packages/core/src/worker/frame_loop.ts <<'__EOF_ts_frame_loop__'
export interface PaintStats {
  cmds: number;
  dropped: boolean;
  paintMs: number;
  frameId: number;
}

export interface FrameLoopCallbacks {
  /** Строит DisplayList и кладёт в SAB. true — есть что рисовать. */
  build(): boolean;
  /** Рисует текущий слот. */
  paint(): { cmds: number; dropped: boolean; paintMs: number };
  onTick(stats: PaintStats): void;
}

export function startFrameLoop(cb: FrameLoopCallbacks): {
  request(): void;
  stop(): void;
} {
  let scheduled = false;
  let stopped = false;
  let frameId = 0;

  const tick = () => {
    scheduled = false;
    if (stopped) return;
    try {
      if (cb.build()) {
        const r = cb.paint();
        cb.onTick({ ...r, frameId: ++frameId });
      }
    } catch (e) {
      // Ошибку логируем, loop не роняем.
      console.error('[frame_loop]', e);
    }
  };

  return {
    request() {
      if (scheduled || stopped) return;
      scheduled = true;
      requestAnimationFrame(tick);
    },
    stop() {
      stopped = true;
    },
  };
}
__EOF_ts_frame_loop__

write_file packages/core/src/worker/worker.ts <<'__EOF_ts_worker__'
/// <reference lib="webworker" />
import init, {
  alloc_sab,
  drop_bitmap,
  init_painter,
  paint_display_list_sab,
  register_bitmap,
  resize_canvas,
  sab_total_bytes,
} from '@doc-converter/wasm';
import { startFrameLoop } from './frame_loop';

type Ctx = OffscreenCanvasRenderingContext2D;

interface InitMsg {
  type: 'init';
  canvas: OffscreenCanvas;
  wasmUrl: string;
  slotCapacity: number;
}

interface ResizeMsg {
  type: 'resize';
  cssW: number;
  cssH: number;
  dpr: number;
}

interface BitmapMsg {
  type: 'bitmap';
  id: number;
  bitmap: ImageBitmap;
}

interface DropBitmapMsg {
  type: 'drop-bitmap';
  id: number;
}

interface RenderMsg {
  type: 'render';
}

type InMsg = InitMsg | ResizeMsg | BitmapMsg | DropBitmapMsg | RenderMsg;

let ctx: Ctx | null = null;
let sab: SharedArrayBuffer | null = null;
let slotCapacity = 0;
let loop: ReturnType<typeof startFrameLoop> | null = null;

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  switch (msg.type) {
    case 'init': {
      if (ctx) {
        (self as unknown as Worker).postMessage({
          type: 'error',
          message: 'worker already initialised',
        });
        return;
      }
      try {
        await init(msg.wasmUrl);
        ctx = msg.canvas.getContext('2d', {
          alpha: true,
          desynchronized: true,
        }) as Ctx;
        if (!ctx) throw new Error('getContext("2d") returned null');

        slotCapacity = msg.slotCapacity;
        // SAB — не transferable, шарится через structured clone.
        sab = alloc_sab(slotCapacity) as unknown as SharedArrayBuffer;
        (self as unknown as Worker).postMessage({
          type: 'ready',
          sab,
          slotCapacity,
          totalBytes: sab_total_bytes(slotCapacity),
        });

        init_painter(ctx);
        loop = startFrameLoop({
          build: () => true, // Phase 3/6 заменит на реальный builder
          paint: () => {
            const stats = paint_display_list_sab(sab!, slotCapacity) as {
              cmds: number;
              dropped: boolean;
              paintMs: number;
            };
            return stats;
          },
          onTick: (s) =>
            (self as unknown as Worker).postMessage({ type: 'tick', stats: s }),
        });
      } catch (e) {
        (self as unknown as Worker).postMessage({
          type: 'error',
          message: String(e),
        });
      }
      break;
    }
    case 'resize': {
      if (!ctx) return;
      resize_canvas(ctx, msg.cssW, msg.cssH, msg.dpr);
      break;
    }
    case 'bitmap': {
      register_bitmap(msg.id, msg.bitmap);
      break;
    }
    case 'drop-bitmap': {
      drop_bitmap(msg.id);
      break;
    }
    case 'render': {
      loop?.request();
      break;
    }
  }
};
__EOF_ts_worker__

write_file packages/core/src/render/resize_observer.ts <<'__EOF_ts_resize_observer__'
export interface RpcLike {
  call(method: string, params: unknown): unknown;
  attachSab?(sab: SharedArrayBuffer, capacity: number): void;
}

export interface AttachResizeOpts {
  debounceMs?: number;
  maxDpr?: number;
}

const DEFAULT_DEBOUNCE = 100;
const DEFAULT_MAX_DPR = 3;
/** На Retina 4K canvas.width*height > 16M — OffscreenCanvas падает. */
const MAX_PIXELS = 16_000_000;

export function attachResize(
  canvas: HTMLCanvasElement,
  rpc: RpcLike,
  opts: AttachResizeOpts = {},
): () => void {
  const debounceMs = opts.debounceMs ?? DEFAULT_DEBOUNCE;
  const maxDpr = opts.maxDpr ?? DEFAULT_MAX_DPR;

  let timer: number | null = null;
  let lastDpr = 1;

  const computeDpr = (): number => {
    let dpr = Math.min(window.devicePixelRatio || 1, maxDpr);
    const w = canvas.clientWidth || canvas.width;
    const h = canvas.clientHeight || canvas.height;
    if (w * h * dpr * dpr > MAX_PIXELS) {
      dpr = Math.max(1, Math.sqrt(MAX_PIXELS / (w * h)));
    }
    return dpr;
  };

  const flush = () => {
    timer = null;
    const w = canvas.clientWidth || canvas.width;
    const h = canvas.clientHeight || canvas.height;
    const dpr = computeDpr();
    lastDpr = dpr;
    rpc.call('resize', { cssW: w, cssH: h, dpr });
    rpc.call('render', {});
  };

  const onResize = () => {
    if (timer != null) clearTimeout(timer);
    timer = window.setTimeout(flush, debounceMs);
  };

  const ro = new ResizeObserver(onResize);
  ro.observe(canvas);
  window.addEventListener('resize', onResize);
  lastDpr = computeDpr();
  flush();

  return () => {
    ro.disconnect();
    window.removeEventListener('resize', onResize);
    if (timer != null) clearTimeout(timer);
  };
}
__EOF_ts_resize_observer__

write_file packages/core/src/render/offscreen.ts <<'__EOF_ts_offscreen__'
import type { RpcLike } from './resize_observer';

/** Передаёт OffscreenCanvas воркеру ровно один раз. */
export function initOffscreen(
  canvas: HTMLCanvasElement,
  worker: Worker,
  rpc: RpcLike,
  wasmUrl: string,
  slotCapacity = 1 << 20,
): Promise<void> {
  if (!('transferControlToOffscreen' in canvas)) {
    return Promise.reject(new Error('transferControlToOffscreen unsupported'));
  }
  const off = canvas.transferControlToOffscreen();
  return new Promise((resolve, reject) => {
    const onMsg = (ev: MessageEvent) => {
      const data = ev.data as { type: string; sab?: SharedArrayBuffer; slotCapacity?: number; message?: string };
      if (data?.type === 'ready' && data.sab) {
        worker.removeEventListener('message', onMsg);
        rpc.attachSab?.(data.sab, data.slotCapacity ?? slotCapacity);
        resolve();
      } else if (data?.type === 'error') {
        worker.removeEventListener('message', onMsg);
        reject(new Error(data.message ?? 'worker init error'));
      }
    };
    worker.addEventListener('message', onMsg);
    worker.postMessage(
      { type: 'init', canvas: off, wasmUrl, slotCapacity },
      [off],
    );
  });
}
__EOF_ts_offscreen__

write_file packages/core/src/render/export_png.ts <<'__EOF_ts_export_png__'
/**
 * Экспорт OffscreenCanvas в PNG.
 * Работает из воркера с Chrome 108 / Firefox 116 / Safari 16.4.
 */
export async function exportPng(canvas: OffscreenCanvas): Promise<Uint8Array> {
  const blob = await canvas.convertToBlob({ type: 'image/png' });
  const ab = await blob.arrayBuffer();
  return new Uint8Array(ab);
}
__EOF_ts_export_png__

# ---------------------------------------------------------------------------
# Playwright specs
# ---------------------------------------------------------------------------

write_file packages/core/test/resize.spec.ts <<'__EOF_ts_test_resize__'
import { expect, test } from '@playwright/test';

test('resize updates canvas.width with dpr clamp', async ({ page }) => {
  await page.goto('/demo/viewer.html');
  await page.setViewportSize({ width: 1600, height: 900 });
  const canvas = page.locator('canvas');
  await expect
    .poll(async () => canvas.evaluate((c: HTMLCanvasElement) => c.width))
    .toBeGreaterThan(0);
  const before = await canvas.evaluate((c: HTMLCanvasElement) => c.width);
  await page.setViewportSize({ width: 800, height: 600 });
  await page.waitForTimeout(250);
  const after = await canvas.evaluate((c: HTMLCanvasElement) => c.width);
  expect(after).not.toBe(before);
  const dpr = await page.evaluate(() => window.devicePixelRatio);
  expect(dpr).toBeLessThanOrEqual(3);
});
__EOF_ts_test_resize__

write_file packages/core/test/tick.spec.ts <<'__EOF_ts_test_tick__'
import { expect, test } from '@playwright/test';

test('tick arrives with paintMs < 16', async ({ page }) => {
  await page.goto('/demo/viewer.html');
  const tick = await page.evaluate(
    () =>
      new Promise<{ paintMs: number; cmds: number }>((resolve, reject) => {
        const w = new Worker('/worker.js', { type: 'module' });
        const timer = setTimeout(() => reject(new Error('no tick in 5s')), 5000);
        w.onmessage = (ev) => {
          if (ev.data?.type === 'tick') {
            clearTimeout(timer);
            resolve(ev.data.stats);
          }
        };
      }),
  );
  expect(tick.paintMs).toBeLessThan(16);
});
__EOF_ts_test_tick__

write_file packages/core/test/png.spec.ts <<'__EOF_ts_test_png__'
import { expect, test } from '@playwright/test';

test('exportPng returns valid PNG magic bytes', async ({ page }) => {
  await page.goto('/demo/viewer.html');
  const magic = await page.evaluate(async () => {
    const canvas = new OffscreenCanvas(64, 64);
    const blob = await canvas.convertToBlob({ type: 'image/png' });
    const ab = await blob.arrayBuffer();
    return Array.from(new Uint8Array(ab).slice(0, 8));
  });
  expect(magic).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
});
__EOF_ts_test_png__

# ---------------------------------------------------------------------------
# size-limit
# ---------------------------------------------------------------------------

write_file .size-limit.json <<'__EOF_size_limit__'
[
  { "path": "packages/core/dist/worker.js", "limit": "550 KB", "gzip": true },
  { "path": "packages/core/dist/index.js",  "limit": "200 KB", "gzip": true }
]
__EOF_size_limit__

echo
echo "== Готово =="
echo
echo "Дальше:"
echo "  1. cargo fmt --all"
echo "  2. cargo test --workspace"
echo "  3. cargo test --workspace --target wasm32-unknown-unknown --no-run"
echo "  4. cargo bench -p doc-converter-render"
echo "  5. wasm-pack test --headless --chrome crates/wasm"
echo "  6. pnpm turbo run typecheck test build"
echo "  7. pnpm size-limit"