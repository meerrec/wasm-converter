#[cfg(target_arch = "wasm32")]
use crate::display_list::TextAlign;

#[cfg(target_arch = "wasm32")]
pub fn text_align_str(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
        TextAlign::Justify => "justify",
    }
}
