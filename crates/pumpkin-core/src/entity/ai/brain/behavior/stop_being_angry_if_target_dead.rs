use pumpkin_data::entity::EntityType;

use super::super::memory::{MemoryStatus, types};
use super::{OneShot, utils};
use crate::entity::living::LivingEntity;

/// Forgets who the mob is angry at once that entity is dead.
#[must_use]
pub fn create() -> OneShot {
    OneShot::new(
        "StopBeingAngryIfTargetDead",
        vec![(types::ANGRY_AT.id(), MemoryStatus::ValuePresent)],
        |tick| {
            let forgiven =
                utils::get_living_entity_from_uuid_memory(tick.state, tick.world, types::ANGRY_AT)
                    .is_some_and(|target| {
                        target
                            .get_living_entity()
                            .is_some_and(LivingEntity::is_dead_or_dying)
                            && (target.get_entity().entity_type != &EntityType::PLAYER
                                || tick.world.level_info.load().game_rules.forgive_dead_players)
                    });
            if forgiven {
                tick.state.erase(types::ANGRY_AT.id());
            }
            true
        },
    )
}
