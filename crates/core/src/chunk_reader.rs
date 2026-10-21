//! Потоковый вход ZIP: `SegmentedBuffer` — `Read + Seek` над списком чанков
//! (ADR-0031).
//!
//! Central directory ZIP лежит в конце архива, поэтому разобрать пакет раньше
//! прихода последнего чанка нельзя. Буфер только копит чанки; парс стартует
//! после конца потока, когда `zip::ZipArchive` сможет читать и листать буфер
//! как обычный файл — без сборки контигуального `Vec<u8>`.

use std::io::{self, Read, Seek, SeekFrom};

/// Буфер чанков: список сегментов как единое адресное пространство.
///
/// `Read`/`Seek` реализованы на самом типе — чтение по ссылке получается
/// бланкетно (`impl Read for &T where T: Read`), отдельный trait не нужен.
/// Позиция чтения — внутреннее поле, как у `Cursor`.
pub struct SegmentedBuffer {
    segments: Vec<Box<[u8]>>,
    /// Сколько байт всего накопилось.
    total: u64,
    /// Текущая позиция чтения, байты от начала.
    pos: u64,
}

impl Default for SegmentedBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl SegmentedBuffer {
    /// Пустой буфер.
    #[must_use]
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
            total: 0,
            pos: 0,
        }
    }

    /// Дописать чанк (копия на границе JS→wasm неизбежна без SAB, ADR-0031 §6).
    ///
    /// Возвращает новый суммарный размер — его воркер использует для прогресса
    /// и диагностики. Чтение можно продолжать и после `append`: позиция не
    /// трогается, данные только дописываются в конец.
    pub fn append(&mut self, bytes: &[u8]) -> u64 {
        self.segments.push(bytes.to_vec().into_boxed_slice());
        self.total = self.total.saturating_add(bytes.len() as u64);
        self.total
    }

    /// Сколько байт накоплено.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Пуст ли буфер: пустой поток (0 байт) парсить нечего.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Первые `n` байт буфера без движения позиции чтения.
    ///
    /// Нужно проверкам сигнатуры (CFB-магия `.doc`) до открытия архива:
    /// читать через `Read` двигало бы позицию, с которой `ZipArchive` начнёт
    /// разбор. Сборка — одна маленькая копия (килобайты), а не весь файл.
    #[must_use]
    pub fn peek(&self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n.min(usize::try_from(self.total).unwrap_or(usize::MAX)));
        let mut taken = 0usize;
        for seg in &self.segments {
            if taken >= n {
                break;
            }
            let take = (n - taken).min(seg.len());
            out.extend_from_slice(&seg[..take]);
            taken += take;
        }
        out
    }
}

impl Read for SegmentedBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Чтение за границей известных байтов — UnexpectedEof, а не panic:
        // `zip` воспринимает это как оборванный архив и отдаёт свою ошибку.
        if self.pos >= self.total {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "segmented buffer exhausted",
            ));
        }

        let mut written = 0usize;
        let mut seg_start = 0u64;
        for seg in &self.segments {
            let seg_len = u64::try_from(seg.len()).unwrap_or(u64::MAX);
            let seg_end = seg_start.saturating_add(seg_len);
            if self.pos >= seg_end {
                seg_start = seg_end;
                continue;
            }
            let in_seg = usize::try_from(self.pos - seg_start).unwrap_or(usize::MAX);
            let avail = seg.len() - in_seg;
            let take = avail.min(buf.len() - written);
            buf[written..written + take].copy_from_slice(&seg[in_seg..in_seg + take]);
            written += take;
            self.pos = self.pos.saturating_add(take as u64);
            if written == buf.len() {
                break;
            }
            seg_start = seg_end;
        }
        Ok(written)
    }
}

impl Seek for SegmentedBuffer {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        // Позиции допустимы только в пределах накопленных байтов: за буфером
        // читать нечего, и zip обязан работать в границах total (ADR-0031 §5).
        let target = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(delta) => {
                let wide = i128::from(self.pos) + i128::from(delta);
                u64::try_from(wide).map_err(|_| seek_error(wide))?
            }
            SeekFrom::End(delta) => {
                let wide = i128::from(self.total) + i128::from(delta);
                u64::try_from(wide).map_err(|_| seek_error(wide))?
            }
        };
        if target > self.total {
            return Err(seek_error(i128::from(target)));
        }
        self.pos = target;
        Ok(target)
    }
}

/// Ошибка позиционирования: негативная позиция или за концом буфера.
fn seek_error(target: i128) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("seek position {target} outside the buffer"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf_with(parts: &[&[u8]]) -> SegmentedBuffer {
        let mut buf = SegmentedBuffer::new();
        for part in parts {
            buf.append(part);
        }
        buf
    }

    #[test]
    fn append_returns_running_total() {
        let mut buf = SegmentedBuffer::new();
        assert_eq!(buf.append(b"abc"), 3);
        assert_eq!(buf.append(b"de"), 5);
        assert_eq!(buf.total(), 5);
    }

    #[test]
    fn read_crosses_segment_boundaries() {
        let mut buf = buf_with(&[b"ab", b"cd", b"ef"]);
        let mut out = [0u8; 5];
        // Первое чтение: два сегмента и кусок третьего.
        assert_eq!(buf.read(&mut out).unwrap(), 5);
        assert_eq!(&out, b"abcde");
        // Второе чтение: остаток последнего сегмента.
        let mut tail = [0u8; 2];
        assert_eq!(buf.read(&mut tail).unwrap(), 1);
        assert_eq!(tail[0], b'f');
    }

    #[test]
    fn read_beyond_end_is_unexpected_eof() {
        let mut buf = buf_with(&[b"abc"]);
        let mut out = [0u8; 4];
        assert_eq!(buf.read(&mut out).unwrap(), 3);
        let err = buf.read(&mut out).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn seek_forward_and_backward() {
        let mut buf = buf_with(&[b"abcdef"]);
        assert_eq!(buf.seek(SeekFrom::Start(4)).unwrap(), 4);
        let mut out = [0u8; 2];
        assert_eq!(buf.read(&mut out).unwrap(), 2);
        assert_eq!(&out, b"ef");
        assert_eq!(buf.seek(SeekFrom::Current(-6)).unwrap(), 0);
        assert_eq!(buf.seek(SeekFrom::End(-1)).unwrap(), 5);
    }

    #[test]
    fn seek_beyond_end_is_an_error() {
        let mut buf = buf_with(&[b"abc"]);
        assert!(buf.seek(SeekFrom::Start(4)).is_err());
        assert!(buf.seek(SeekFrom::End(1)).is_err());
        assert!(buf.seek(SeekFrom::Current(-4)).is_err());
    }

    #[test]
    fn append_after_seek_keeps_position_and_extends() {
        let mut buf = buf_with(&[b"ab"]);
        assert_eq!(buf.seek(SeekFrom::Start(1)).unwrap(), 1);
        assert_eq!(buf.append(b"cd"), 4);
        let mut out = [0u8; 3];
        assert_eq!(buf.read(&mut out).unwrap(), 3);
        assert_eq!(&out, b"bcd");
    }

    #[test]
    fn peek_assembles_without_moving_position() {
        let mut buf = buf_with(&[b"ab", b"cd"]);
        assert_eq!(buf.peek(3), b"abc");
        assert_eq!(buf.stream_position().unwrap(), 0, "позиция не тронута");
        let mut out = [0u8; 2];
        assert_eq!(buf.read(&mut out).unwrap(), 2);
        assert_eq!(&out, b"ab");
    }
}
