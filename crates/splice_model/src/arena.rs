use crate::{Clip, ClipData, ClipId, ClipTiming, Item, ItemData, ItemId, ItemTiming};

/// Arena slot. A slot stays fixed until its row is removed, after which it can be reused.
/// Use a typed ID for references that outlive an edit.
pub type Slot = u32;

#[derive(Debug, Default)]
struct Slots {
    order: Vec<Slot>,
    free: Vec<Slot>,
    by_id: Vec<(u64, Slot)>,
}

impl Slots {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            order: Vec::with_capacity(capacity),
            free: Vec::with_capacity(capacity),
            by_id: Vec::with_capacity(capacity),
        }
    }

    fn index(&self, serial: u64) -> Result<usize, usize> {
        self.by_id.binary_search_by_key(&serial, |&(id, _)| id)
    }

    fn find(&self, serial: u64) -> Option<Slot> {
        self.index(serial).ok().map(|index| self.by_id[index].1)
    }

    fn insert(&mut self, serial: u64) -> Option<Slot> {
        let index = self.index(serial).err()?;
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => Slot::try_from(self.order.len()).ok()?,
        };
        self.order.push(slot);
        self.by_id.insert(index, (serial, slot));
        Some(slot)
    }

    fn remove(&mut self, slot: Slot, serial: u64) -> bool {
        let Some(index) = self.order.iter().position(|&current| current == slot) else {
            return false;
        };
        self.order.remove(index);
        self.by_id.remove(self.index(serial).expect("live slot ID"));
        self.free.push(slot);
        true
    }
}

/// Column storage for clip IDs, timing and content, with independent spine order.
#[derive(Debug, Default)]
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
        let slot = self.slots.insert(row.id.serial())?;
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
        let Some(id) = self.id(slot) else {
            return false;
        };
        if !self.slots.remove(slot, id.serial()) {
            return false;
        }
        self.data[slot as usize] = None;
        true
    }

    /// Finds a live slot by stable ID.
    pub fn find(&self, id: ClipId) -> Option<Slot> {
        self.slots.find(id.serial())
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
#[derive(Debug, Default)]
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
        let slot = self.slots.insert(row.id.serial())?;
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
        let Some(id) = self.id(slot) else {
            return false;
        };
        if !self.slots.remove(slot, id.serial()) {
            return false;
        }
        self.data[slot as usize] = None;
        true
    }

    /// Finds a live slot by stable ID.
    pub fn find(&self, id: ItemId) -> Option<Slot> {
        self.slots.find(id.serial())
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

impl Clone for Slots {
    fn clone(&self) -> Self {
        Self {
            order: self.order.clone(),
            free: self.free.clone(),
            by_id: self.by_id.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.order.clone_from(&source.order);
        self.free.clone_from(&source.free);
        self.by_id.clone_from(&source.by_id);
    }
}

impl Clone for Clips {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            ids: self.ids.clone(),
            timing: self.timing.clone(),
            data: self.data.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.slots.clone_from(&source.slots);
        self.ids.clone_from(&source.ids);
        self.timing.clone_from(&source.timing);
        self.data.clone_from(&source.data);
    }
}

impl Clone for Items {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            ids: self.ids.clone(),
            timing: self.timing.clone(),
            data: self.data.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.slots.clone_from(&source.slots);
        self.ids.clone_from(&source.ids);
        self.timing.clone_from(&source.timing);
        self.data.clone_from(&source.data);
    }
}
