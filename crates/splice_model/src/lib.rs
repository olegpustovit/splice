//! Project data and reusable storage for integer-frame timeline edits.
#![deny(missing_docs)]

mod arena;
mod ids;
mod media;
mod timeline;
pub mod timing;

pub use arena::{Clips, Items, Slot};
pub use ids::*;
pub use media::*;
pub use timeline::*;

/// A time or duration in integer frames.
pub type Frame = i32;

/// Snapshot data. Persistent edits are applied by the core store.
#[derive(Clone, Debug)]
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
        }
    }
}

#[cfg(test)]
mod tests;
