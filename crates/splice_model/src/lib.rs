//! Project data and reusable storage for integer-frame timeline edits.
#![deny(missing_docs)]

mod arena;
mod ids;
pub mod keys;
mod media;
mod timeline;
pub mod timing;

pub use arena::{Clips, Items, Slot};
pub use ids::*;
pub use keys::{Ease, Key};
pub use media::*;
pub use timeline::*;

/// A time or duration in integer frames.
pub type Frame = i32;

/// Snapshot data. Persistent edits are applied by the core store.
#[derive(Debug)]
pub struct Project {
    /// Current revision number.
    pub rev: u64,
    /// Journal chain hash at this revision.
    pub hash: u64,
    /// Next serial available for ID minting.
    pub serial: u64,
    /// Timeline frame rate in frames per second.
    pub fps: u32,
    /// Canvas width and height.
    pub size: [u32; 2],
    /// Canvas aspect.
    pub aspect: Aspect,
    /// Permission level for agent calls.
    pub access: Access,
    /// Project-wide effects.
    pub effects: Vec<Effect>,
    /// Primary video lane.
    pub spine: Spine,
    /// Secondary lanes in stack order.
    pub lanes: Vec<Lane>,
    /// Primary clips in spine order.
    pub clips: Clips,
    /// Overlay items in creation order.
    pub items: Items,
    /// Timeline markers.
    pub markers: Vec<Marker>,
    /// Media descriptors in insertion order.
    pub media: Vec<Media>,
    /// Transcripts for imported or recorded media.
    pub transcripts: Vec<Transcript>,
    /// Nested timelines behind `MediaKind::Compound` media.
    pub compounds: Vec<Compound>,
}

/// A nested timeline (T17.4). Its clips start at frame 0 and its items are pinned to its clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Compound {
    /// The `MediaKind::Compound` descriptor that plays it.
    pub media: MediaId,
    /// Spine clips on the compound's clock.
    pub clips: Vec<Clip>,
    /// Items pinned to the compound's clock.
    pub items: Vec<Item>,
    /// The lanes those items used, for breaking apart into a project that removed them.
    pub lanes: Vec<Lane>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            rev: 0,
            hash: 0,
            serial: 1,
            fps: 30,
            size: [1080, 1920],
            aspect: Aspect::Portrait,
            access: Access::Live,
            effects: Vec::new(),
            spine: Spine {
                id: LaneId::SPINE,
                name: "Video".into(),
                effects: Vec::new(),
            },
            lanes: Vec::new(),
            clips: Clips::default(),
            items: Items::default(),
            markers: Vec::new(),
            media: Vec::new(),
            transcripts: Vec::new(),
            compounds: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;

impl Clone for Project {
    fn clone(&self) -> Self {
        Self {
            rev: self.rev,
            hash: self.hash,
            serial: self.serial,
            fps: self.fps,
            size: self.size,
            aspect: self.aspect,
            access: self.access,
            effects: self.effects.clone(),
            spine: self.spine.clone(),
            lanes: self.lanes.clone(),
            clips: self.clips.clone(),
            items: self.items.clone(),
            markers: self.markers.clone(),
            media: self.media.clone(),
            transcripts: self.transcripts.clone(),
            compounds: self.compounds.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.rev = source.rev;
        self.hash = source.hash;
        self.serial = source.serial;
        self.fps = source.fps;
        self.size = source.size;
        self.aspect = source.aspect;
        self.access = source.access;
        self.effects.clone_from(&source.effects);
        self.spine.clone_from(&source.spine);
        // lane.reorder permutes lanes: keep each lane's buffers with its ID, as markers do.
        for (index, original) in source.lanes.iter().enumerate().take(self.lanes.len()) {
            let rest = &mut self.lanes[index..];
            if rest[0].id != original.id
                && let Some(found) = rest.iter().position(|lane| lane.id == original.id)
            {
                rest[..=found].rotate_right(1);
            }
        }
        self.lanes.clone_from(&source.lanes);
        self.clips.clone_from(&source.clips);
        self.items.clone_from(&source.items);
        // Moves reorder markers. Keep each marker's text buffer with its ID.
        for (index, original) in source.markers.iter().enumerate().take(self.markers.len()) {
            let rest = &mut self.markers[index..];
            if rest[0].id != original.id
                && let Some(found) = rest.iter().position(|marker| marker.id == original.id)
            {
                rest[..=found].rotate_right(1);
            }
        }
        self.markers.clone_from(&source.markers);
        self.media.clone_from(&source.media);
        self.transcripts.clone_from(&source.transcripts);
        self.compounds.clone_from(&source.compounds);
    }
}
