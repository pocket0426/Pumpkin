use std::sync::Arc;

use pumpkin_core::{
    entity::{Entity, EntityBase, passive::villager::VillagerEntity},
    net::ClientPlatform,
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{
    Block,
    block_properties::{BedPart, HorizontalFacing, WhiteBedLikeProperties},
    dimension::Dimension,
    entity::EntityType,
};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItemOn};
use pumpkin_util::math::vector3::Vector3;

use super::offset_within_chunk;

pub fn run_bed_kicks_sleeping_villager_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the bed villager test")?;
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        return Err("Expected a Java player for the bed villager test".into());
    };

    let foot_pos = offset_within_chunk(player.get_entity().block_pos.load(), 2);
    let head_pos = foot_pos.offset(Vector3::new(0, 0, -1));
    // The spot can be inside terrain, and a bed with a solid block above it counts as obstructed.
    for pos in [foot_pos, head_pos, foot_pos.up(), head_pos.up()] {
        world.set_block_state(&pos, Block::AIR.default_state.id, BlockFlags::NOTIFY_ALL);
    }
    let mut foot = WhiteBedLikeProperties::default(&Block::RED_BED);
    foot.facing = HorizontalFacing::North;
    foot.part = BedPart::Foot;
    // Placing the foot also places the head.
    world.set_block_state(
        &foot_pos,
        foot.to_state_id(&Block::RED_BED),
        BlockFlags::NOTIFY_ALL,
    );

    let villager = VillagerEntity::new(Entity::new(
        world.clone(),
        head_pos.to_f64(),
        &EntityType::VILLAGER,
    ));
    if !world.spawn_entity(villager.clone()) {
        return Err("Could not spawn the test villager".into());
    }
    if !villager.start_sleeping(head_pos) || !villager.is_sleeping() {
        return Err("The test villager did not fall asleep in the bed".into());
    }

    let use_bed = SUseItemOn {
        hand: VarInt(0),
        position: head_pos,
        face: VarInt(1),
        cursor_pos: Vector3::new(0.5, 0.5, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: VarInt(200),
    };
    client
        .handle_use_item_on(&player, &use_bed, server)
        .map_err(|err| format!("Using the occupied bed failed: {err:?}"))?;

    let still_sleeping = villager.is_sleeping();
    let still_occupied =
        WhiteBedLikeProperties::from_state_id(world.get_block_state_id(&head_pos)).occupied;
    villager.get_entity().remove();
    world.set_block_state(
        &foot_pos,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    world.set_block_state(
        &head_pos,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    if still_sleeping || still_occupied {
        return Err(format!(
            "Using an occupied bed did not kick the villager out: sleeping={still_sleeping}, occupied={still_occupied}"
        ));
    }

    summary_lines.push("[PASS] bot:bed_kicks_sleeping_villager".into());
    Ok(())
}
