use crate::{Clip, ClipData, ClipId, ClipTiming, Item, ItemData, ItemId, ItemTiming};

/// Arena slot. A slot stays fixed until its row is removed, after which it can be reused.
/// Use a typed ID for references that outlive an edit.
pub type Slot = u32;

#[derive(Clone, Debug, Default)]
struct Slots {
    order: Vec<Slot>,
    free: Vec<Slot>,
}

impl Slots {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            order: Vec::with_capacity(capacity),
            free: Vec::with_capacity(capacity),
        }
    }

    fn insert(&mut self) -> Option<Slot> {
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => Slot::try_from(self.order.len()).ok()?,
        };
        self.order.push(slot);
        Some(slot)
    }

    fn remove(&mut self, slot: Slot) -> bool {
        let Some(index) = self.order.iter().position(|&current| current == slot) else {
            return false;
        };
        self.order.remove(index);
        self.free.push(slot);
        true
    }
}

/// Column storage for clip IDs, timing and content, with independent spine order.
#[derive(Clone, Debug, Default)]
pub struct Clips {
    slots: Slots,
    ids: Vec<ClipId>,
    timing: Vec<ClipTiming>,
    data: Vec<Option<ClipData>>,
}

impl Clips {
    /// Reserves storage for this many live rows, including their order and free slots.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Slots::with_capacity(capacity),
            ids: Vec::with_capacity(capacity),
            timing: Vec::with_capacity(capacity),
            data: Vec::with_capacity(capacity),
        }
    }

    /// Live slots in spine order.
    pub fn order(&self) -> &[Slot] {
        &self.slots.order
    }

    /// Adds a row at the end of the order. Returns `None` for a duplicate ID or exhausted slots.
    pub fn insert(&mut self, row: Clip) -> Option<Slot> {
        if self.find(row.id).is_some() {
            return None;
        }
        let slot = self.slots.insert()?;
        let index = slot as usize;
        if index == self.ids.len() {
            self.ids.push(row.id);
            self.timing.push(row.timing);
            self.data.push(Some(row.data));
        } else {
            self.ids[index] = row.id;
            self.timing[index] = row.timing;
            self.data[index] = Some(row.data);
        }
        Some(slot)
    }

    /// Deletes a row and releases its slot.
    pub fn remove(&mut self, slot: Slot) -> bool {
        if !self.slots.remove(slot) {
            return false;
        }
        self.data[slot as usize] = None;
        true
    }

    /// Finds a live slot by stable ID.
    pub fn find(&self, id: ClipId) -> Option<Slot> {
        self.order()
            .iter()
            .copied()
            .find(|&slot| self.ids[slot as usize] == id)
    }

    /// Stable ID for a live slot.
    pub fn id(&self, slot: Slot) -> Option<ClipId> {
        self.data(slot)?;
        Some(self.ids[slot as usize])
    }

    /// Timing for a live slot.
    pub fn timing(&self, slot: Slot) -> Option<ClipTiming> {
        self.data(slot)?;
        Some(self.timing[slot as usize])
    }

    /// Mutable timing for a live slot.
    pub fn timing_mut(&mut self, slot: Slot) -> Option<&mut ClipTiming> {
        self.data(slot)?;
        Some(&mut self.timing[slot as usize])
    }

    /// Content for a live slot.
    pub fn data(&self, slot: Slot) -> Option<&ClipData> {
        self.data.get(slot as usize)?.as_ref()
    }

    /// Mutable content for a live slot.
    pub fn data_mut(&mut self, slot: Slot) -> Option<&mut ClipData> {
        self.data.get_mut(slot as usize)?.as_mut()
    }

    /// Sorts the spine by timeline start without moving any slots.
    pub fn sort_by_start(&mut self) {
        self.slots
            .order
            .sort_unstable_by_key(|&slot| (self.timing[slot as usize].start, slot));
    }
}

/// Column storage for item IDs, timing and content, with independent creation order.
#[derive(Clone, Debug, Default)]
pub struct Items {
    slots: Slots,
    ids: Vec<ItemId>,
    timing: Vec<ItemTiming>,
    data: Vec<Option<ItemData>>,
}

impl Items {
    /// Reserves storage for this many live rows, including their order and free slots.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Slots::with_capacity(capacity),
            ids: Vec::with_capacity(capacity),
            timing: Vec::with_capacity(capacity),
            data: Vec::with_capacity(capacity),
        }
    }

    /// Live slots in creation order.
    pub fn order(&self) -> &[Slot] {
        &self.slots.order
    }

    /// Adds a row at the end of the order. Returns `None` for a duplicate ID or exhausted slots.
    pub fn insert(&mut self, row: Item) -> Option<Slot> {
        if self.find(row.id).is_some() {
            return None;
        }
        let slot = self.slots.insert()?;
        let index = slot as usize;
        if index == self.ids.len() {
            self.ids.push(row.id);
            self.timing.push(row.timing);
            self.data.push(Some(row.data));
        } else {
            self.ids[index] = row.id;
            self.timing[index] = row.timing;
            self.data[index] = Some(row.data);
        }
        Some(slot)
    }

    /// Deletes a row and releases its slot.
    pub fn remove(&mut self, slot: Slot) -> bool {
        if !self.slots.remove(slot) {
            return false;
        }
        self.data[slot as usize] = None;
        true
    }

    /// Finds a live slot by stable ID.
    pub fn find(&self, id: ItemId) -> Option<Slot> {
        self.order()
            .iter()
            .copied()
            .find(|&slot| self.ids[slot as usize] == id)
    }

    /// Stable ID for a live slot.
    pub fn id(&self, slot: Slot) -> Option<ItemId> {
        self.data(slot)?;
        Some(self.ids[slot as usize])
    }

    /// Timing for a live slot.
    pub fn timing(&self, slot: Slot) -> Option<ItemTiming> {
        self.data(slot)?;
        Some(self.timing[slot as usize])
    }

    /// Mutable timing for a live slot.
    pub fn timing_mut(&mut self, slot: Slot) -> Option<&mut ItemTiming> {
        self.data(slot)?;
        Some(&mut self.timing[slot as usize])
    }

    /// Content for a live slot.
    pub fn data(&self, slot: Slot) -> Option<&ItemData> {
        self.data.get(slot as usize)?.as_ref()
    }

    /// Mutable content for a live slot.
    pub fn data_mut(&mut self, slot: Slot) -> Option<&mut ItemData> {
        self.data.get_mut(slot as usize)?.as_mut()
    }
}
