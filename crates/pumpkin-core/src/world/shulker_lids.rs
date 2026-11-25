//! Space taken up by shulker box lids that are not closed.
//!
//! Collision shapes come from the block state alone. A shulker box is the one block whose
//! collision follows its block entity, so the part of an open lid that sticks out of the block is
//! kept here and added on top of the static shapes.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};

struct Lid {
    position: BlockPos,
    bounds: BoundingBox,
    updated: i64,
}

#[derive(Default)]
pub struct ShulkerLids {
    /// Mirrors `lids.len()` so collision checks skip the lock while every lid is closed.
    count: AtomicUsize,
    lids: Mutex<Vec<Lid>>,
}

impl ShulkerLids {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0
    }

    /// Stores the space the lid of the shulker box at `position` covers outside its block.
    pub fn set(&self, position: BlockPos, bounds: BoundingBox, world_age: i64) {
        let mut lids = self
            .lids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(lid) = lids.iter_mut().find(|lid| lid.position == position) {
            lid.bounds = bounds;
            lid.updated = world_age;
        } else {
            lids.push(Lid {
                position,
                bounds,
                updated: world_age,
            });
            self.count.store(lids.len(), Ordering::Relaxed);
        }
    }

    pub fn remove(&self, position: &BlockPos) {
        if self.is_empty() {
            return;
        }
        let mut lids = self
            .lids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lids.retain(|lid| lid.position != *position);
        self.count.store(lids.len(), Ordering::Relaxed);
    }

    /// Drops lids whose block entity stopped ticking, such as in an unloaded chunk.
    pub fn remove_stale(&self, world_age: i64) {
        if self.is_empty() {
            return;
        }
        let mut lids = self
            .lids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lids.retain(|lid| lid.updated + 1 >= world_age);
        self.count.store(lids.len(), Ordering::Relaxed);
    }

    /// Calls `on_lid` for every open lid that overlaps `bounding_box`.
    pub fn for_each_intersecting(
        &self,
        bounding_box: &BoundingBox,
        mut on_lid: impl FnMut(BlockPos, BoundingBox),
    ) {
        if self.is_empty() {
            return;
        }
        let lids = self
            .lids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for lid in lids.iter() {
            if lid.bounds.intersects(bounding_box) {
                on_lid(lid.position, lid.bounds);
            }
        }
    }

    #[must_use]
    pub fn intersects(&self, bounding_box: &BoundingBox) -> bool {
        let mut intersects = false;
        self.for_each_intersecting(bounding_box, |_, _| intersects = true);
        intersects
    }
}
