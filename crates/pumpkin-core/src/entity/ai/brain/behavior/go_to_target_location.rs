use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleType, MemoryStatus, types};
use super::{OneShot, utils};

fn get_random_offset(tick: &BrainTick<'_>) -> i32 {
    tick.mob.get_random().random_range(0..3) - 1
}

fn get_nearby_pos(tick: &BrainTick<'_>, pos: BlockPos) -> BlockPos {
    pos.offset(Vector3::new(
        get_random_offset(tick),
        0,
        get_random_offset(tick),
    ))
}

/// Walks to a block next to the remembered location while further than `close_enough_dist` away.
#[must_use]
pub fn create(
    location_memory: MemoryModuleType<BlockPos>,
    close_enough_dist: i32,
    speed_modifier: f32,
) -> OneShot {
    OneShot::new(
        "GoToTargetLocation",
        vec![
            (location_memory.id(), MemoryStatus::ValuePresent),
            (types::ATTACK_TARGET.id(), MemoryStatus::ValueAbsent),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
            (types::LOOK_TARGET.id(), MemoryStatus::Registered),
        ],
        move |tick| {
            let Some(target_pos) = tick.state.get(location_memory).copied() else {
                return false;
            };
            let mob_pos = tick.mob.get_entity().block_pos.load();
            // Vanilla `BlockPos.closerThan`.
            let close_enough = f64::from(close_enough_dist);
            if f64::from(target_pos.squared_distance(&mob_pos)) >= close_enough * close_enough {
                utils::set_walk_and_look_target_memories_to_block(
                    tick.state,
                    get_nearby_pos(tick, target_pos),
                    speed_modifier,
                    close_enough_dist,
                );
            }
            true
        },
    )
}
