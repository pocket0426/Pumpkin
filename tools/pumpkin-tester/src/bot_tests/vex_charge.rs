use std::sync::Arc;

use pumpkin_core::{
    entity::{Entity, EntityBase, mob::vex::VexEntity},
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{Block, dimension::Dimension, entity::EntityType};
use pumpkin_util::{GameMode, math::vector3::Vector3};

/// A vex above a stone ceiling charges straight through it at the player underneath.
/// Whether a dive ends in a hit depends on the angle (as in vanilla), so the hit itself is
/// covered by a unit test and this checks the flight.
pub fn run_vex_charge_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the vex charge test")?;
    // The vex is handed its target, the bystander is kept from becoming a second one.
    let bystander = world.get_player_by_name("TestBot2");
    let bystander_gamemode = bystander.as_ref().map(|other| other.gamemode.load());
    if let Some(other) = &bystander {
        other.set_gamemode(GameMode::Creative);
    }
    let origin = player.get_entity().block_pos.load();
    // An open, floored cell for the bot (rough terrain could hurt it on its own), a stone
    // ceiling two blocks thick over it, and open air above that for the vex.
    let mut previous = Vec::new();
    for dx in -1..=1 {
        for dz in -1..=1 {
            for dy in -1..=7 {
                let pos = origin.offset(Vector3::new(dx, dy, dz));
                let block = if dy == -1 || dy == 3 || dy == 4 {
                    &Block::STONE
                } else {
                    &Block::AIR
                };
                previous.push((pos, world.get_block_state_id(&pos)));
                world.set_block_state(&pos, block.default_state.id, BlockFlags::NOTIFY_ALL);
            }
        }
    }

    let vex = VexEntity::new(Entity::new(
        world.clone(),
        origin.to_f64().add_raw(0.5, 6.0, 0.5),
        &EntityType::VEX,
    ));
    if !world.spawn_entity(vex.clone()) {
        return Err("Could not spawn the test vex".into());
    }
    player.set_gamemode(GameMode::Survival);
    let health_before = player.living_entity.health.load();
    let target: Arc<dyn EntityBase> = player.clone();
    vex.mob_entity.set_target(Some(target));

    let mut charged = false;
    let mut was_inside_stone = false;
    let mut closest = f64::MAX;
    for _ in 0..400 {
        // A vex inside stone cannot see, and its own target goals let go of what they
        // cannot see: the target is held for it, the test is about the flight.
        let target: Arc<dyn EntityBase> = player.clone();
        vex.mob_entity.set_target(Some(target));
        server.tick();
        charged |= vex.is_charging();
        let vex_entity = vex.get_entity();
        was_inside_stone |= world.get_block(&vex_entity.block_pos.load()) == &Block::STONE;
        closest = closest.min(
            vex_entity
                .pos
                .load()
                .squared_distance_to_vec(&player.get_entity().get_eye_pos())
                .sqrt(),
        );
        if charged && was_inside_stone && closest < 1.0 {
            break;
        }
    }
    let vex_pos = vex.get_entity().pos.load();
    let no_gravity = vex.get_entity().has_no_gravity();

    vex.get_entity().remove();
    player.set_gamemode(GameMode::Creative);
    player.living_entity.set_health(health_before);
    if let (Some(other), Some(gamemode)) = (&bystander, bystander_gamemode) {
        other.set_gamemode(gamemode);
    }
    for (pos, state) in previous {
        world.set_block_state(&pos, state, BlockFlags::NOTIFY_ALL);
    }
    if !charged || !was_inside_stone || closest >= 1.0 || !no_gravity {
        return Err(format!(
            "Vex above a stone ceiling: charged={charged}, passed through stone={was_inside_stone}, no gravity={no_gravity} (all expected true), closest to the player's eyes={closest:.2} (expected under 1); vex ended at {vex_pos:?}, player block at {origin:?}, player now at {:?}, vex velocity {:?}, vex target set={}",
            player.get_entity().pos.load(),
            vex.get_entity().velocity.load(),
            vex.mob_entity.get_target().is_some()
        ));
    }

    summary_lines.push("[PASS] bot:vex_charges_through_blocks".into());
    Ok(())
}
