//! The item stack value shared by the game state and the packet writers.

/// Item stack. Only the damage component is modelled (`minecraft:damage`);
/// everything else on an item is the prototype default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    pub item: u32,
    pub count: i32,
    /// `minecraft:damage`: wear taken so far (0 = undamaged).
    pub damage: u16,
}

impl Stack {
    pub const EMPTY: Stack = Stack { item: 0, count: 0, damage: 0 };
    pub fn new(item: u32, count: i32) -> Self {
        Self::damaged(item, count, 0)
    }
    pub fn damaged(item: u32, count: i32, damage: u16) -> Self {
        if item == 0 || count <= 0 {
            Self::EMPTY
        } else {
            Self { item, count, damage }
        }
    }
    /// Same item and damage, another count. Keeps the wear of this stack.
    pub fn with_count(&self, count: i32) -> Self {
        Self::damaged(self.item, count, self.damage)
    }
    pub fn is_empty(&self) -> bool {
        self.item == 0 || self.count <= 0
    }
    /// Whether two stacks may merge (same item, same wear).
    pub fn same_kind(&self, other: &Stack) -> bool {
        self.item == other.item && self.damage == other.damage
    }
}
