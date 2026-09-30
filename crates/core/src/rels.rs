//! `_rels/*.rels` — карта relationships.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::xml::XmlReader;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub target_mode: Option<String>,
}

#[derive(Debug, Default)]
pub struct RelMap {
    pub items: HashMap<String, Relationship>,
}

impl RelMap {
    /// Разбирает `_rels/*.rels`.
    ///
    /// # Errors
    /// Если XML некорректен или `<Relationship/>` неполный.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = XmlReader::new(bytes, "_rels");
        let mut items = HashMap::new();
        while let Some(ev) = r.next_significant()? {
            if let quick_xml::events::Event::Empty(e) | quick_xml::events::Event::Start(e) = ev {
                if e.name().as_ref() == b"Relationship" {
                    let mut id = None;
                    let mut rel_type = None;
                    let mut target = None;
                    let mut target_mode = None;
                    for attr in e.attributes().flatten() {
                        let key = attr.key.as_ref();
                        let val = String::from_utf8_lossy(&attr.value).into_owned();
                        match key {
                            b"Id" => id = Some(val),
                            b"Type" => rel_type = Some(val),
                            b"Target" => target = Some(val),
                            b"TargetMode" => target_mode = Some(val),
                            _ => {}
                        }
                    }
                    match (id, rel_type, target) {
                        (Some(id), Some(rel_type), Some(target)) => {
                            items.insert(
                                id.clone(),
                                Relationship {
                                    id,
                                    rel_type,
                                    target,
                                    target_mode,
                                },
                            );
                        }
                        _ => return Err(Error::Malformed("incomplete <Relationship/>".into())),
                    }
                }
            }
        }
        Ok(Self { items })
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.items.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_rels() {
        let xml = br#"<?xml version="1.0"?>
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId1" Type="http://x/officeDocument" Target="word/document.xml"/>
            </Relationships>"#;
        let m = RelMap::parse(xml).unwrap();
        assert_eq!(m.get("rId1").unwrap().target, "word/document.xml");
    }
}
