//! Ring из двух слотов для передачи DisplayList через SharedArrayBuffer.
//!
//! Layout:
//!   [0..4)    seq_writer: i32
//!   [4..8)    seq_reader: i32
//!   `[8..12)`   `slot_len[0]`: i32
//!   `[12..16)`  `slot_len[1]`: i32
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

    /// js-sys оборачивает вызовы `Atomics` в `catch`, поэтому они возвращают
    /// `Result`. Заголовок ring'а — всегда `Int32Array` поверх `SharedArrayBuffer`,
    /// так что ошибка здесь означает некорректно собранный буфер.
    #[inline]
    fn atomic_load(header: &Int32Array, index: u32) -> Result<u32, JsValue> {
        Atomics::load(header, index).map(|v| v as u32)
    }

    #[inline]
    fn atomic_store(header: &Int32Array, index: u32, value: i32) -> Result<(), JsValue> {
        Atomics::store(header, index, value).map(|_| ())
    }

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
        fn state(&self) -> Result<RingState, JsValue> {
            Ok(RingState {
                seq_writer: atomic_load(&self.header, 0)?,
                seq_reader: atomic_load(&self.header, 1)?,
                slot_len: [atomic_load(&self.header, 2)?, atomic_load(&self.header, 3)?],
            })
        }

        /// # Errors
        /// Если заголовок ring'а не читается как `Int32Array`.
        pub fn has_free_slot(&self) -> Result<bool, JsValue> {
            Ok(self.state()?.has_free_slot())
        }

        /// # Errors
        /// Если заголовок ring'а не читается как `Int32Array`.
        pub fn can_read(&self) -> Result<bool, JsValue> {
            Ok(self.state()?.can_read())
        }

        /// Пробует записать байты в следующий свободный слот.
        ///
        /// # Errors
        /// Если заголовок ring'а не читается как `Int32Array`.
        pub fn try_write(&mut self, bytes: &[u8]) -> Result<bool, JsValue> {
            let st = self.state()?;
            if !st.has_free_slot() {
                return Ok(false);
            }
            if bytes.len() > self.slot_capacity {
                return Ok(false);
            }
            let slot = st.writer_slot();
            let target = if slot == 0 { &self.slot0 } else { &self.slot1 };
            let src = Uint8Array::from(bytes);
            target.set(&src, 0);
            atomic_store(&self.header, 2 + slot as u32, bytes.len() as i32)?;
            atomic_store(&self.header, 0, st.seq_writer.wrapping_add(1) as i32)?;
            Ok(true)
        }

        /// Возвращает `(slot_idx, len)` — вызывающий читает из `slot0`/`slot1`.
        ///
        /// # Errors
        /// Если заголовок ring'а не читается как `Int32Array`.
        pub fn read_current(&self) -> Result<Option<(usize, usize)>, JsValue> {
            let st = self.state()?;
            if !st.can_read() {
                return Ok(None);
            }
            let slot = st.reader_slot();
            let len = st.slot_len[slot] as usize;
            if len > self.slot_capacity {
                return Ok(None);
            }
            Ok(Some((slot, len)))
        }

        /// # Errors
        /// Если заголовок ring'а не читается как `Int32Array`.
        pub fn release_current(&mut self) -> Result<(), JsValue> {
            let st = self.state()?;
            if st.can_read() {
                atomic_store(&self.header, 1, st.seq_reader.wrapping_add(1) as i32)?;
            }
            Ok(())
        }

        /// Копирует текущий слот в `out`.
        ///
        /// # Errors
        /// Если заголовок ring'а не читается как `Int32Array`.
        pub fn copy_current_into(&self, out: &mut Vec<u8>) -> Result<bool, JsValue> {
            let Some((slot, len)) = self.read_current()? else {
                return Ok(false);
            };
            let src = if slot == 0 { &self.slot0 } else { &self.slot1 };
            out.clear();
            out.resize(len, 0);
            src.subarray(0, len as u32).copy_to(out.as_mut_slice());
            Ok(true)
        }

        /// # Errors
        /// Если заголовок ring'а не читается как `Int32Array`.
        pub fn reset(&mut self) -> Result<(), JsValue> {
            for index in 0..4 {
                atomic_store(&self.header, index, 0)?;
            }
            Ok(())
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
