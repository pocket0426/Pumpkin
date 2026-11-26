use std::sync::Arc;

use pumpkin_core::{
    block::blocks::redstone::bell::ring_bell,
    entity::{
        Entity, EntityBase, ai::brain::memory::types, mob::Mob, passive::villager::VillagerEntity,
        r#type::from_type,
    },
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{Block, dimension::Dimension, effect::StatusEffect, entity::EntityType};

use super::offset_within_chunk;

/// Ticks from the ring until the raiders glow: 5 of swinging, then 40 of resonating, with a few
/// spare for the queued block event.
const RESONATION_TICKS: usize = 50;

/// Vanilla `BellBlockEntity`: a ring tells villagers in earshot when they heard it, and with a
/// raider nearby the bell resonates and makes the raiders glow.
pub fn run_bell_ring_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the bell test")?;
    let origin = player.get_entity().block_pos.load();

    // Above the bot, clear of the terrain
    let support_pos = offset_within_chunk(origin, 2).up_height(12);
    let bell_pos = support_pos.up();
    world.set_block_state(
        &support_pos,
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    world.set_block_state(
        &bell_pos,
        Block::BELL.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    if world.get_block(&bell_pos) != &Block::BELL || world.get_block_entity(&bell_pos).is_none() {
        return Err("Could not place the test bell".into());
    }

    let spawn_pos = origin.to_f64().add_raw(0.5, 0.0, 0.5);
    let villager =
        VillagerEntity::new(Entity::new(world.clone(), spawn_pos, &EntityType::VILLAGER));
    let pillager = from_type(
        &EntityType::PILLAGER,
        spawn_pos,
        &world,
        uuid::Uuid::new_v4(),
    );
    // Still a living raider for the bell, without shooting the bots
    if let Some(mob) = pillager.get_mob() {
        mob.get_mob_entity().set_no_ai(true);
    }
    if !world.spawn_entity(villager.clone()) || !world.spawn_entity(pillager.clone()) {
        return Err("Could not spawn the mobs for the bell test".into());
    }
    // The villager only gets a brain once it has ticked
    server.tick();
    let heard_bell = || {
        villager
            .get_mob_entity()
            .with_existing_brain(|brain| brain.get(types::HEARD_BELL_TIME).is_some())
            .unwrap_or(false)
    };
    let glowing = || {
        pillager
            .get_living_entity()
            .is_some_and(|living| living.has_effect(&StatusEffect::GLOWING))
    };
    let heard_before = heard_bell();

    ring_bell(bell_pos, &world, None, None);
    // The ring is a block event, handled at the start of the next tick
    server.tick();
    let heard_after = heard_bell();
    let glowing_early = glowing();
    for _ in 0..RESONATION_TICKS {
        server.tick();
    }
    let glowing_late = glowing();

    villager.get_entity().remove();
    pillager.get_entity().remove();
    for pos in [bell_pos, support_pos] {
        world.set_block_state(&pos, Block::AIR.default_state.id, BlockFlags::NOTIFY_ALL);
    }
    if heard_before || !heard_after || glowing_early || !glowing_late {
        return Err(format!(
            "Ringing a bell: villager heard it before={heard_before} (expected false), after={heard_after} (expected true); raider glowing right away={glowing_early} (expected false), after resonating={glowing_late} (expected true)"
        ));
    }

    summary_lines.push("[PASS] bot:bell_ring_alerts_villagers_and_reveals_raiders".into());
    Ok(())
}
