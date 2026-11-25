use std::sync::Arc;

use pumpkin_core::{
    block::entities::shulker_box::ShulkerBoxBlockEntity,
    entity::{Entity, EntityBase, decoration::armor_stand::ArmorStandEntity},
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{Block, dimension::Dimension, entity::EntityType};

use super::offset_within_chunk;

/// Ticks for the lid to finish moving, with a few spare for the armor stand to settle.
const LID_TICKS: usize = 16;

pub fn run_shulker_lid_pushes_entity_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the shulker lid test")?;

    // The default state faces up, so the lid rises half a block above the box.
    // High above the bot, where no terrain is in the way of the armor stand.
    let box_pos = offset_within_chunk(player.get_entity().block_pos.load(), 3).up_height(30);
    world.set_block_state(
        &box_pos,
        Block::SHULKER_BOX.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    if world.get_block(&box_pos) != &Block::SHULKER_BOX {
        return Err("Could not place the test shulker box".into());
    }
    let top = f64::from(box_pos.0.y) + 1.0;
    let stand = Arc::new(ArmorStandEntity::new(Entity::new(
        world.clone(),
        box_pos.to_f64().add_raw(0.0, 1.0, 0.0),
        &EntityType::ARMOR_STAND,
    )));
    if !world.spawn_entity(stand.clone()) {
        return Err("Could not spawn the test armor stand".into());
    }
    let height_after = |open_count: u8| {
        world.add_synced_block_event(
            box_pos,
            ShulkerBoxBlockEntity::OPEN_ANIMATION_EVENT_TYPE,
            open_count,
        );
        for _ in 0..LID_TICKS {
            server.tick();
        }
        stand.get_entity().pos.load().y - top
    };

    let opened = height_after(1);
    let closed = height_after(0);
    stand.get_entity().remove();
    world.set_block_state(
        &box_pos,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    if (opened - 0.5).abs() > 1.0e-3 || closed.abs() > 1.0e-3 {
        return Err(format!(
            "Armor stand on a shulker box: {opened} above the box when open (expected 0.5), {closed} when closed again (expected 0)"
        ));
    }

    summary_lines.push("[PASS] bot:shulker_lid_pushes_entity".into());
    Ok(())
}
