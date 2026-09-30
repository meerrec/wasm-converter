#[cfg(target_arch = "wasm32")]
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
        let mid = (lo + hi).div_ceil(2);
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
