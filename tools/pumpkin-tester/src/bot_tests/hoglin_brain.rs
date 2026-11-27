use std::sync::Arc;

use pumpkin_core::{
    entity::{
        Entity, EntityBase,
        ai::brain::memory::types,
        mob::{Mob, hoglin::HoglinEntity},
    },
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{Block, dimension::Dimension, entity::EntityType};
use pumpkin_util::{GameMode, math::vector3::Vector3};

/// A hoglin goes for a survival player on sight, and a hoglin repellent nearby (a respawn
/// anchor here) makes it drop the target and leave the player alone.
pub fn run_hoglin_brain_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the hoglin test")?;
    let bystander = world.get_player_by_name("TestBot2");
    let bystander_gamemode = bystander.as_ref().map(|other| other.gamemode.load());
    if let Some(other) = &bystander {
        other.set_gamemode(GameMode::Creative);
    }
    let origin = player.get_entity().block_pos.load();
    // A flat stone pad with head room, so neither sight nor paths depend on the terrain.
    let mut previous = Vec::new();
    for dx in -3..=3 {
        for dz in -3..=3 {
            for dy in -1..=2 {
                let pos = origin.offset(Vector3::new(dx, dy, dz));
                let block = if dy == -1 { &Block::STONE } else { &Block::AIR };
                previous.push((pos, world.get_block_state_id(&pos)));
                world.set_block_state(&pos, block.default_state.id, BlockFlags::NOTIFY_ALL);
            }
        }
    }

    let hoglin = HoglinEntity::new(Entity::new(
        world.clone(),
        origin.to_f64().add_raw(2.5, 0.0, 0.5),
        &EntityType::HOGLIN,
    ));
    // It would turn into a zoglin in the overworld.
    hoglin.set_immune_to_zombification(true);
    if !world.spawn_entity(hoglin.clone()) {
        return Err("Could not spawn the test hoglin".into());
    }
    player.set_gamemode(GameMode::Survival);
    let health_before = player.living_entity.health.load();

    let mut hurt = false;
    for _ in 0..300 {
        server.tick();
        if player.living_entity.health.load() < health_before {
            hurt = true;
            break;
        }
    }
    player.living_entity.set_health(health_before);

    let anchor = origin.offset(Vector3::new(3, 0, 3));
    world.set_block_state(
        &anchor,
        Block::RESPAWN_ANCHOR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    let state = |hoglin: &Arc<HoglinEntity>| {
        hoglin.get_mob_entity().with_brain(&**hoglin, |brain| {
            (
                brain.has_memory_value(types::PACIFIED.id()),
                brain.has_memory_value(types::ATTACK_TARGET.id()),
            )
        })
    };
    // From here on the player is healed after every tick: a hit is still seen, but a few
    // of them in a row cannot kill the bot and upset the tests that follow.
    let mut pacified = false;
    for _ in 0..60 {
        server.tick();
        player.living_entity.set_health(health_before);
        if state(&hoglin) == (true, false) {
            pacified = true;
            break;
        }
    }
    let mut hurt_while_pacified = false;
    for _ in 0..150 {
        server.tick();
        hurt_while_pacified |= player.living_entity.health.load() < health_before;
        player.living_entity.set_health(health_before);
    }

    hoglin.get_entity().remove();
    player.set_gamemode(GameMode::Creative);
    player.living_entity.set_health(health_before);
    if let (Some(other), Some(gamemode)) = (&bystander, bystander_gamemode) {
        other.set_gamemode(gamemode);
    }
    for (pos, state) in previous {
        world.set_block_state(&pos, state, BlockFlags::NOTIFY_ALL);
    }
    if !hurt || !pacified || hurt_while_pacified {
        return Err(format!(
            "Hoglin next to a survival player: hurt the player={hurt} (expected true), pacified by a respawn anchor={pacified} (expected true), hurt the player while pacified={hurt_while_pacified} (expected false)"
        ));
    }

    summary_lines.push("[PASS] bot:hoglin_attacks_and_is_pacified_by_repellent".into());
    Ok(())
}
