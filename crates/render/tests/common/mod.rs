//! Native-читатель ring'а для интеграционных тестов.
//!
//! Жил в `crates/render/src/sab/reader.rs`, но прод его не использует — это
//! тестовый двойник SAB-пути painter'а на обычной памяти, — поэтому он живёт
//! здесь, рядом с единственным потребителем (`roundtrip.rs`).

use doc_converter_render::sab::RingState;

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

    /// Отпускает готовые кадры, кроме самого свежего, — как это делает painter
    /// на SAB-пути. Возвращает число пропущенных.
    pub fn advance_to_latest(&mut self) -> u32 {
        self.state.advance_to_latest()
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

    #[test]
    fn latest_wins_when_both_slots_are_full() {
        let mut r = NativeRingReader::new();
        assert!(r.push(b"first"));
        assert!(r.push(b"second"));
        assert_eq!(r.advance_to_latest(), 1);
        assert_eq!(r.pop(), Some(&b"second"[..]));
        assert_eq!(r.pop(), None);
        // Оба слота отпущены: следующий кадр пишется без ожидания.
        assert!(r.push(b"third"));
    }

    #[test]
    fn advance_to_latest_keeps_the_only_frame() {
        let mut r = NativeRingReader::new();
        assert!(r.push(b"only"));
        assert_eq!(r.advance_to_latest(), 0);
        assert_eq!(r.pop(), Some(&b"only"[..]));
    }
}
