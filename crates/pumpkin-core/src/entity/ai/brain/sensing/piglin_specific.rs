use std::sync::Arc;

use pumpkin_data::block_properties::CampfireLikeProperties;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockState};
use pumpkin_util::math::position::BlockPos;

use super::super::memory::{MemoryModuleId, types};
use super::super::{BrainState, BrainTick};
use super::Sensor;
use crate::entity::EntityBase;
use crate::entity::living::LivingEntity;
use crate::entity::mob::hoglin::HoglinEntity;
use crate::entity::mob::piglin::PiglinEntity;
use crate::entity::mob::piglin_ai;
use crate::entity::player::Player;
use crate::world::World;

const REQUIRES: [MemoryModuleId; 12] = [
    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
    types::NEAREST_LIVING_ENTITIES.id(),
    types::NEAREST_VISIBLE_NEMESIS.id(),
    types::NEAREST_TARGETABLE_PLAYER_NOT_WEARING_GOLD.id(),
    types::NEAREST_PLAYER_HOLDING_WANTED_ITEM.id(),
    types::NEAREST_VISIBLE_HUNTABLE_HOGLIN.id(),
    types::NEAREST_VISIBLE_BABY_HOGLIN.id(),
    types::NEAREST_VISIBLE_ADULT_PIGLINS.id(),
    types::NEARBY_ADULT_PIGLINS.id(),
    types::VISIBLE_ADULT_PIGLIN_COUNT.id(),
    types::VISIBLE_ADULT_HOGLIN_COUNT.id(),
    types::NEAREST_REPELLENT.id(),
];

pub struct PiglinSpecificSensor;

fn is_valid_repellent(block: &Block, state: &BlockState) -> bool {
    block.has_tag(&tag::Block::MINECRAFT_PIGLIN_REPELLENTS)
        && (block != &Block::SOUL_CAMPFIRE || CampfireLikeProperties::from_state_id(state.id).lit)
}

fn find_nearest_repellent(world: &World, origin: BlockPos) -> Option<BlockPos> {
    BlockPos::within_manhattan(
        origin,
        piglin_ai::REPELLENT_DETECTION_RANGE_HORIZONTAL,
        piglin_ai::REPELLENT_DETECTION_RANGE_VERTICAL,
        piglin_ai::REPELLENT_DETECTION_RANGE_HORIZONTAL,
    )
    .find(|pos| {
        let (block, state) = world.get_block_and_state(pos);
        is_valid_repellent(block, state)
    })
}

fn is_baby(entity: &dyn EntityBase) -> bool {
    entity
        .cast_any()
        .downcast_ref::<PiglinEntity>()
        .map_or_else(
            || {
                entity
                    .get_living_entity()
                    .is_some_and(LivingEntity::is_baby)
            },
            PiglinEntity::is_baby,
        )
}

/// Vanilla `PiglinAi.findNearbyAdultPiglins`.
#[must_use]
pub fn find_nearby_adult_piglins(brain: &BrainState) -> Vec<Arc<dyn EntityBase>> {
    brain
        .get(types::NEAREST_LIVING_ENTITIES)
        .map(|nearby| {
            nearby
                .iter()
                .filter(|entity| {
                    let entity_type = entity.get_entity().entity_type;
                    (entity_type == &EntityType::PIGLIN || entity_type == &EntityType::PIGLIN_BRUTE)
                        && !is_baby(entity.as_ref())
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

impl Sensor for PiglinSpecificSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        &REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        let repellent = find_nearest_repellent(tick.world, tick.mob.get_entity().block_pos.load());
        tick.state.set_optional(types::NEAREST_REPELLENT, repellent);

        let mut nemesis = None;
        let mut huntable_hoglin = None;
        let mut baby_hoglin = None;
        let mut zombified = None;
        let mut player_not_wearing_gold: Option<Arc<Player>> = None;
        let mut player_holding_wanted_item: Option<Arc<Player>> = None;
        let mut adult_hoglin_count = 0;
        let mut visible_adult_piglins: Vec<Arc<dyn EntityBase>> = Vec::new();

        if let Some(visible) = tick.state.get(types::NEAREST_VISIBLE_LIVING_ENTITIES) {
            let ctx = tick.visibility();
            for entity in visible.find_all(&ctx, |_| true) {
                let entity_type = entity.get_entity().entity_type;
                if entity_type == &EntityType::HOGLIN {
                    if is_baby(entity.as_ref()) {
                        if baby_hoglin.is_none() {
                            baby_hoglin = Some(entity.clone());
                        }
                    } else {
                        adult_hoglin_count += 1;
                        if huntable_hoglin.is_none()
                            && entity
                                .cast_any()
                                .downcast_ref::<HoglinEntity>()
                                .is_some_and(HoglinEntity::can_be_hunted)
                        {
                            huntable_hoglin = Some(entity.clone());
                        }
                    }
                } else if entity_type == &EntityType::PIGLIN_BRUTE {
                    visible_adult_piglins.push(entity.clone());
                } else if entity_type == &EntityType::PIGLIN {
                    if !is_baby(entity.as_ref()) {
                        visible_adult_piglins.push(entity.clone());
                    }
                } else if let Some(player) = entity.get_player() {
                    if player_not_wearing_gold.is_none()
                        && !piglin_ai::is_wearing_safe_armor(entity.as_ref())
                        && tick.mob.can_attack(entity.as_ref())
                    {
                        player_not_wearing_gold =
                            tick.world.get_player_by_id(player.get_entity().entity_id);
                    }
                    if player_holding_wanted_item.is_none()
                        && !player.is_spectator()
                        && piglin_ai::is_player_holding_loved_item(entity.as_ref())
                    {
                        player_holding_wanted_item =
                            tick.world.get_player_by_id(player.get_entity().entity_id);
                    }
                } else if nemesis.is_none()
                    && (entity_type == &EntityType::WITHER_SKELETON
                        || entity_type == &EntityType::WITHER)
                {
                    nemesis = Some(entity.clone());
                } else if zombified.is_none() && piglin_ai::is_zombified(entity.as_ref()) {
                    zombified = Some(entity.clone());
                }
            }
        }

        let nearby_adult_piglins = find_nearby_adult_piglins(tick.state);
        let visible_adult_piglin_count = visible_adult_piglins.len() as i32;
        let state = &mut *tick.state;
        state.set_optional(types::NEAREST_VISIBLE_NEMESIS, nemesis);
        state.set_optional(types::NEAREST_VISIBLE_HUNTABLE_HOGLIN, huntable_hoglin);
        state.set_optional(types::NEAREST_VISIBLE_BABY_HOGLIN, baby_hoglin);
        state.set_optional(types::NEAREST_VISIBLE_ZOMBIFIED, zombified);
        state.set_optional(
            types::NEAREST_TARGETABLE_PLAYER_NOT_WEARING_GOLD,
            player_not_wearing_gold,
        );
        state.set_optional(
            types::NEAREST_PLAYER_HOLDING_WANTED_ITEM,
            player_holding_wanted_item,
        );
        state.set(types::NEARBY_ADULT_PIGLINS, nearby_adult_piglins);
        state.set(types::NEAREST_VISIBLE_ADULT_PIGLINS, visible_adult_piglins);
        state.set(
            types::VISIBLE_ADULT_PIGLIN_COUNT,
            visible_adult_piglin_count,
        );
        state.set(types::VISIBLE_ADULT_HOGLIN_COUNT, adult_hoglin_count);
    }
}
