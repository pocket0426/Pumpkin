use std::sync::Arc;

use pumpkin_data::entity::EntityType;

use super::super::BrainTick;
use super::super::memory::{MemoryStatus, types};
use super::OneShot;
use crate::entity::EntityBase;
use crate::entity::living::LivingEntity;

/// Remembers where a dead attack target fell so the mob can celebrate there, then drops the target.
#[must_use]
pub fn create(
    celebrate_duration: i32,
    dance_predicate: impl Fn(&BrainTick<'_>, &Arc<dyn EntityBase>) -> bool + Send + Sync + 'static,
) -> OneShot {
    OneShot::new(
        "StartCelebratingIfTargetDead",
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
            (types::ANGRY_AT.id(), MemoryStatus::Registered),
            (types::CELEBRATE_LOCATION.id(), MemoryStatus::ValueAbsent),
            (types::DANCING.id(), MemoryStatus::Registered),
        ],
        move |tick| {
            let Some(target) = tick.state.get(types::ATTACK_TARGET).cloned() else {
                return false;
            };
            if !target
                .get_living_entity()
                .is_some_and(LivingEntity::is_dead_or_dying)
            {
                return false;
            }
            let duration = i64::from(celebrate_duration);
            if dance_predicate(tick, &target) {
                tick.state.set_with_expiry(types::DANCING, true, duration);
            }
            tick.state.set_with_expiry(
                types::CELEBRATE_LOCATION,
                target.get_entity().block_pos.load(),
                duration,
            );
            if target.get_entity().entity_type != &EntityType::PLAYER
                || tick.world.level_info.load().game_rules.forgive_dead_players
            {
                tick.state.erase(types::ATTACK_TARGET.id());
                tick.state.erase(types::ANGRY_AT.id());
            }
            true
        },
    )
}
