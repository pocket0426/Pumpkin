use std::sync::Arc;

use pumpkin_core::{
    entity::{Entity, EntityBase, mob::piglin::PiglinEntity},
    net::ClientPlatform,
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{Block, dimension::Dimension, entity::EntityType};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{GameMode, math::vector3::Vector3};

use super::offset_within_chunk;

pub fn run_piglin_anger_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the piglin anger test")?;
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        return Err("Expected a Java player for the piglin anger test".into());
    };
    let origin = player.get_entity().block_pos.load();
    let barrel_pos = offset_within_chunk(origin, 2);
    world.set_block_state(
        &barrel_pos,
        Block::BARREL.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    let use_barrel = SUseItemOn {
        hand: VarInt(0),
        position: barrel_pos,
        face: VarInt(1),
        cursor_pos: Vector3::new(0.5, 0.5, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: VarInt(300),
    };
    // In the middle of the cleared column above the bot, so nothing blocks the line of sight.
    for height in 2..=4 {
        world.set_block_state(
            &origin.up_height(height),
            Block::AIR.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
    }
    let piglin = PiglinEntity::new(Entity::new(
        world.clone(),
        origin.to_f64().add_raw(0.0, 3.0, 0.0),
        &EntityType::PIGLIN,
    ));
    if !world.spawn_entity(piglin.clone()) {
        return Err("Could not spawn the test piglin".into());
    }
    let is_angry_at_player = || {
        piglin
            .mob_entity
            .get_target()
            .is_some_and(|target| target.get_entity().entity_id == player.get_entity().entity_id)
    };

    // A creative player is not attackable, so opening the barrel angers nobody.
    client
        .handle_use_item_on(&player, &use_barrel, server)
        .map_err(|err| format!("Using the barrel in creative failed: {err:?}"))?;
    let angry_in_creative = is_angry_at_player();
    player.close_handled_screen();

    player.set_gamemode(GameMode::Survival);
    client
        .handle_use_item_on(&player, &use_barrel, server)
        .map_err(|err| format!("Using the barrel in survival failed: {err:?}"))?;
    let angry_in_survival = is_angry_at_player();
    player.close_handled_screen();

    player.set_gamemode(GameMode::Creative);
    piglin.get_entity().remove();
    world.set_block_state(
        &barrel_pos,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    if angry_in_creative || !angry_in_survival {
        return Err(format!(
            "Opening a barrel next to a piglin: angry at a creative player={angry_in_creative} (expected false), at a survival player={angry_in_survival} (expected true)"
        ));
    }

    summary_lines.push("[PASS] bot:piglin_angered_by_opening_barrel".into());
    Ok(())
}
