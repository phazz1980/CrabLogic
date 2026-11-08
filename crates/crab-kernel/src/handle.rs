//! Generational handles: a removed entry's handle never reaches the entry that reuses its slot.

use alloc::vec::Vec;

/// A reference to an entry of a [`SlotMap`]; stale after the entry is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle {
    index: u16,
    generation: u16,
}

impl Handle {
    /// The slot number (stable while the entry lives; reused after removal).
    pub fn index(self) -> usize {
        usize::from(self.index)
    }
}

struct Slot<T> {
    generation: u16,
    value: Option<T>,
}

/// A vector of entries addressed by [`Handle`]; free slots are reused with a new generation.
pub struct SlotMap<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u16>,
}

impl<T> Default for SlotMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SlotMap<T> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    /// Stores `value`; `None` when all 65536 slots are taken.
    pub fn insert(&mut self, value: T) -> Option<Handle> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[usize::from(index)];
            slot.value = Some(value);
            return Some(Handle {
                index,
                generation: slot.generation,
            });
        }
        let index = u16::try_from(self.slots.len()).ok()?;
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        });
        Some(Handle {
            index,
            generation: 0,
        })
    }

    pub fn get(&self, h: Handle) -> Option<&T> {
        self.slots
            .get(h.index())
            .filter(|s| s.generation == h.generation)
            .and_then(|s| s.value.as_ref())
    }

    pub fn get_mut(&mut self, h: Handle) -> Option<&mut T> {
        self.slots
            .get_mut(h.index())
            .filter(|s| s.generation == h.generation)
            .and_then(|s| s.value.as_mut())
    }

    pub fn contains(&self, h: Handle) -> bool {
        self.get(h).is_some()
    }

    /// Takes the entry out; the handle (and every copy of it) goes stale.
    pub fn remove(&mut self, h: Handle) -> Option<T> {
        let slot = self
            .slots
            .get_mut(h.index())
            .filter(|s| s.generation == h.generation)?;
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(h.index);
        Some(value)
    }

    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Live entries in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.value.as_ref().map(|v| {
                (
                    Handle {
                        index: i as u16,
                        generation: s.generation,
                    },
                    v,
                )
            })
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| {
            let generation = s.generation;
            s.value.as_mut().map(|v| {
                (
                    Handle {
                        index: i as u16,
                        generation,
                    },
                    v,
                )
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_handle_after_slot_reuse_is_none() {
        let mut map = SlotMap::new();
        let a = map.insert("a").unwrap();
        let b = map.insert("b").unwrap();
        assert_eq!(map.remove(a), Some("a"));
        let c = map.insert("c").unwrap();
        assert_eq!(c.index(), a.index(), "the free slot is reused");
        assert_eq!(map.get(a), None);
        assert_eq!(map.remove(a), None);
        assert_eq!(map.get(c), Some(&"c"));
        assert_eq!(map.get(b), Some(&"b"));
        assert_eq!(map.len(), 2);
        let order: Vec<_> = map.iter().map(|(_, v)| *v).collect();
        assert_eq!(order, ["c", "b"]);
    }
}
