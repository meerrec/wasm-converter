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
