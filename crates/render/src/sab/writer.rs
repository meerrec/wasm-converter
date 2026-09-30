//! Утилита для сборки и записи DisplayList в SAB (wasm-only).

#![cfg(target_arch = "wasm32")]

use super::SabRing;
use crate::display_list::DisplayList;
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

    /// # Errors
    /// Если заголовок ring'а не читается как `Int32Array`.
    pub fn write(&mut self, dl: &DisplayList) -> Result<bool, JsValue> {
        dl.to_bytes_into(&mut self.scratch);
        self.ring.try_write(&self.scratch)
    }
}
