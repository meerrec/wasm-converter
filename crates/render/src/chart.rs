//! Данные диаграммы и их компактный бинарный блоб для `DrawCommand::Chart`.
//!
//! Формат — контракт между производителем кадра (`xlsx`) и painter'ом: painter
//! читает его из SAB и не может спросить вызывающего, поэтому блоб обязан быть
//! самодостаточным. Он лежит в пуле строк DisplayList как байты
//! (`intern_bytes`), а не как текст: значения — `f32`, и UTF-8 им не нужен.
//!
//! Раскладка блоба (little-endian):
//! `kind:u8`, `has_title:u8`, `categories:u32`, `series:u32`, `title:str`,
//! категории (`len:u32` + UTF-8), серии (`name:str` + `values:u32` + `f32`).
//!
//! Геометрия — в подмодулях: [`layout`] раскладывает данные в canvas-agnostic
//! примитивы ([`ChartPrim`], ADR-0011), а canvas- и PDF-бэкенды рисуют их
//! каждый по-своему. Блоб и `DisplayList` при этом не меняются.

mod layout;
mod prim;

pub use layout::layout;
pub use prim::{ChartPrim, Point, TextAlign};

/// Виды диаграмм, которые умеет рисовать painter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ChartKind {
    #[default]
    Bar = 0,
    Line = 1,
    Pie = 2,
    Scatter = 3,
    Area = 4,
}

impl ChartKind {
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Bar),
            1 => Some(Self::Line),
            2 => Some(Self::Pie),
            3 => Some(Self::Scatter),
            4 => Some(Self::Area),
            _ => None,
        }
    }
}

/// Одна серия: имя и значения по категориям.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f32>,
}

/// Модель диаграммы в кадре — без раскладки, только данные.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartData {
    pub kind: ChartKind,
    pub title: Option<String>,
    /// Подписи категорий; у круговой диаграммы — имена долей.
    pub categories: Vec<String>,
    pub series: Vec<ChartSeries>,
}

impl ChartData {
    /// Сериализует диаграмму в блоб пула строк.
    #[must_use]
    pub fn to_blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.kind as u8);
        out.push(u8::from(self.title.is_some()));
        out.extend_from_slice(&(self.categories.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.series.len() as u32).to_le_bytes());
        if let Some(title) = &self.title {
            push_str(&mut out, title);
        }
        for category in &self.categories {
            push_str(&mut out, category);
        }
        for series in &self.series {
            push_str(&mut out, &series.name);
            out.extend_from_slice(&(series.values.len() as u32).to_le_bytes());
            for value in &series.values {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    /// Разбирает блоб; повреждённый блоб — `None`, painter такой кадр пропускает.
    #[must_use]
    pub fn from_blob(bytes: &[u8]) -> Option<Self> {
        let mut pos = 0usize;
        let kind = ChartKind::from_tag(*bytes.get(pos)?)?;
        pos += 1;
        let has_title = *bytes.get(pos)? != 0;
        pos += 1;
        let categories = u32::from_le_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
        pos += 4;
        let series_count = u32::from_le_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
        pos += 4;

        let mut title = None;
        if has_title {
            let (s, next) = take_str(bytes, pos)?;
            title = Some(s);
            pos = next;
        }
        let mut cat_vec = Vec::with_capacity(categories);
        for _ in 0..categories {
            let (s, next) = take_str(bytes, pos)?;
            cat_vec.push(s);
            pos = next;
        }
        let mut series_vec = Vec::with_capacity(series_count);
        for _ in 0..series_count {
            let (name, next) = take_str(bytes, pos)?;
            pos = next;
            let values = u32::from_le_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
            pos += 4;
            let mut value_vec = Vec::with_capacity(values);
            for _ in 0..values {
                let raw = bytes.get(pos..pos + 4)?.try_into().ok()?;
                value_vec.push(f32::from_le_bytes(raw));
                pos += 4;
            }
            series_vec.push(ChartSeries {
                name,
                values: value_vec,
            });
        }
        // Лишние байты — будущее расширение, а не повреждение.
        Some(Self {
            kind,
            title,
            categories: cat_vec,
            series: series_vec,
        })
    }
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn take_str(bytes: &[u8], pos: usize) -> Option<(String, usize)> {
    let len = u32::from_le_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
    let start = pos + 4;
    let text = std::str::from_utf8(bytes.get(start..start + len)?)
        .ok()?
        .to_owned();
    Some((text, start + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ChartData {
        ChartData {
            kind: ChartKind::Line,
            title: Some("Продажи".to_owned()),
            categories: vec!["Янв".to_owned(), "Фев".to_owned()],
            series: vec![
                ChartSeries {
                    name: "План".to_owned(),
                    values: vec![1.5, 2.0],
                },
                ChartSeries {
                    name: "Факт".to_owned(),
                    values: vec![1.0, 2.5],
                },
            ],
        }
    }

    #[test]
    fn blob_roundtrip() {
        let data = sample();
        let decoded = ChartData::from_blob(&data.to_blob()).expect("блог разбирается");
        assert_eq!(decoded, data);
    }

    #[test]
    fn malformed_blob_is_none() {
        assert!(ChartData::from_blob(&[]).is_none());
        assert!(ChartData::from_blob(&[9, 0, 1, 0, 0, 0, 0, 0, 0, 0]).is_none());
        assert!(ChartData::from_blob(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0]).is_none());
    }

    #[test]
    fn unknown_kind_is_none() {
        let mut blob = sample().to_blob();
        blob[0] = 0xFF;
        assert!(ChartData::from_blob(&blob).is_none());
    }
}
