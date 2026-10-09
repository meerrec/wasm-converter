//! Стабильные идентификаторы узлов модели (ADR-0019).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Идентификатор узла модели.
///
/// Уникален в пределах одного разбора и детерминирован для одного и того же
/// входа: ID выдаёт [`NodeIdAllocator`] в порядке обхода XML. Переоткрытие
/// документа, редактирование и сессии ID не переживают (ADR-0019 §5) — это
/// внутренний идентификатор для hit-test, snapshot-тестов и `tracing`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(u64);

impl NodeId {
    /// Корень документа: `NodeId(0)` зарезервирован (ADR-0019 §2).
    pub const ROOT: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for NodeId {
    /// То же, что [`Self::ROOT`].
    fn default() -> Self {
        Self::ROOT
    }
}

/// Монотонный счётчик [`NodeId`].
///
/// Один аллокатор на документ: порядок выдачи — порядок обхода XML, поэтому
/// один и тот же файл даёт одни и те же ID (ADR-0019 §2).
#[derive(Debug, Clone)]
pub struct NodeIdAllocator {
    next: u64,
}

impl NodeIdAllocator {
    /// Нумерация начинается с 1: `NodeId(0)` — корень.
    #[must_use]
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    #[must_use]
    pub fn alloc(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }

    /// Сколько ID уже выдано.
    #[must_use]
    pub const fn allocated(&self) -> u64 {
        self.next - 1
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.next == 1
    }
}

impl Default for NodeIdAllocator {
    /// Как [`Self::new`]: первый ID — 1, а не 0.
    ///
    /// Выведенный `Default` обнулил бы счётчик и выдал зарезервированный
    /// корневой ID первому же узлу.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn root_is_zero() {
        assert_eq!(NodeId::ROOT.value(), 0);
        assert_eq!(NodeId::default(), NodeId::ROOT);
    }

    #[test]
    fn allocator_numbers_from_one() {
        let mut alloc = NodeIdAllocator::new();
        assert!(alloc.is_empty());
        assert_eq!(alloc.allocated(), 0);

        assert_eq!(alloc.alloc(), NodeId::new(1));
        assert_eq!(alloc.alloc(), NodeId::new(2));
        assert_eq!(alloc.allocated(), 2);
        assert!(!alloc.is_empty());
    }

    #[test]
    fn default_allocator_numbers_from_one() {
        assert_eq!(NodeIdAllocator::default().alloc(), NodeId::new(1));
    }

    #[test]
    fn ids_are_unique() {
        let mut alloc = NodeIdAllocator::new();
        let ids: HashSet<NodeId> = (0..10_000).map(|_| alloc.alloc()).collect();

        assert_eq!(ids.len(), 10_000);
        assert_eq!(alloc.allocated(), 10_000);
        assert!(!ids.contains(&NodeId::ROOT));
    }

    #[test]
    fn same_traversal_gives_same_ids() {
        // Один и тот же порядок обхода XML обязан дать одинаковые ID — на этом
        // стоит и детерминизм snapshot-тестов, и кэш каскада.
        let walk = || {
            let mut alloc = NodeIdAllocator::new();
            (0..8).map(|_| alloc.alloc()).collect::<Vec<_>>()
        };

        assert_eq!(walk(), walk());
        assert_eq!(walk().len(), 8);
    }

    #[test]
    fn display_prints_the_number() {
        assert_eq!(NodeId::new(13).to_string(), "13");
    }

    #[test]
    fn serde_wire_form_is_a_number() {
        assert_eq!(
            serde_json::to_value(NodeId::new(7)).expect("serializes"),
            serde_json::json!(7)
        );
        assert_eq!(
            serde_json::to_value(NodeId::ROOT).expect("serializes"),
            serde_json::json!(0)
        );
    }

    #[test]
    fn serde_round_trips() {
        let wire = serde_json::to_string(&NodeId::new(42)).expect("serializes");
        assert_eq!(
            serde_json::from_str::<NodeId>(&wire).expect("deserializes"),
            NodeId::new(42)
        );
    }
}
