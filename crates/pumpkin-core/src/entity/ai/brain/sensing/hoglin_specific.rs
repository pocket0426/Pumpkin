use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, types};
use super::Sensor;
use crate::entity::EntityBase;
use crate::entity::living::LivingEntity;
use crate::entity::mob::hoglin_ai;
use crate::world::World;

const REQUIRES: [MemoryModuleId; 6] = [
    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
    types::NEAREST_REPELLENT.id(),
    types::NEAREST_VISIBLE_ADULT_PIGLIN.id(),
    types::NEAREST_VISIBLE_ADULT_HOGLINS.id(),
    types::VISIBLE_ADULT_PIGLIN_COUNT.id(),
    types::VISIBLE_ADULT_HOGLIN_COUNT.id(),
];

/// Vanilla `HoglinSpecificSensor`: the nearest warped fungus and the like, and the adult
/// piglins and hoglins in sight.
pub struct HoglinSpecificSensor;

fn find_nearest_repellent(world: &World, origin: BlockPos) -> Option<BlockPos> {
    BlockPos::within_manhattan(
        origin,
        hoglin_ai::REPELLENT_DETECTION_RANGE_HORIZONTAL,
        hoglin_ai::REPELLENT_DETECTION_RANGE_VERTICAL,
        hoglin_ai::REPELLENT_DETECTION_RANGE_HORIZONTAL,
    )
    .find(|pos| {
        world
            .get_block(pos)
            .has_tag(&tag::Block::MINECRAFT_HOGLIN_REPELLENTS)
    })
}

impl Sensor for HoglinSpecificSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        &REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        let repellent = find_nearest_repellent(tick.world, tick.mob.get_entity().block_pos.load());
        tick.state.set_optional(types::NEAREST_REPELLENT, repellent);

        let mut nearest_piglin = None;
        let mut piglin_count = 0;
        let mut hoglins: Vec<Arc<dyn EntityBase>> = Vec::new();
        if let Some(visible) = tick.state.get(types::NEAREST_VISIBLE_LIVING_ENTITIES) {
            let ctx = tick.visibility();
            // Vanilla tests `instanceof Piglin`, which leaves brutes out.
            for entity in visible.find_all(&ctx, |entity| {
                !entity
                    .get_living_entity()
                    .is_some_and(LivingEntity::is_baby)
            }) {
                let entity_type = entity.get_entity().entity_type;
                if entity_type == &EntityType::PIGLIN {
                    piglin_count += 1;
                    if nearest_piglin.is_none() {
                        nearest_piglin = Some(entity.clone());
                    }
                } else if entity_type == &EntityType::HOGLIN {
                    hoglins.push(entity.clone());
                }
            }
        }
        tick.state
            .set_optional(types::NEAREST_VISIBLE_ADULT_PIGLIN, nearest_piglin);
        tick.state
            .set(types::VISIBLE_ADULT_PIGLIN_COUNT, piglin_count);
        tick.state
            .set(types::VISIBLE_ADULT_HOGLIN_COUNT, hoglins.len() as i32);
        tick.state
            .set(types::NEAREST_VISIBLE_ADULT_HOGLINS, hoglins);
    }
}
