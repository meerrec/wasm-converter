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
