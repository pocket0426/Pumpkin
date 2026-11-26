use std::sync::Arc;

use pumpkin_core::{
    block::blocks::dripstone::maybe_transfer_fluid,
    entity::{Entity, EntityBase, decoration::armor_stand::ArmorStandEntity},
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{
    Block,
    block_properties::{PointedDripstoneLikeProperties, SpeleothemThickness, VerticalDirection},
    dimension::Dimension,
    entity::EntityType,
};

use super::offset_within_chunk;

/// Vanilla `maybeTransferFluid`: a stalactite hanging from the block under mud dries the mud into clay, and whoever stands
/// on the two pixel lower mud is lifted onto the clay (`Block.pushEntitiesUp`).
pub fn run_dripstone_dries_mud_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the dripstone test")?;

    // High above the bot, where no terrain is in the way.
    let mud_pos = offset_within_chunk(player.get_entity().block_pos.load(), 3).up_height(40);
    let root_pos = mud_pos.down();
    let tip_pos = root_pos.down();
    let mut tip = PointedDripstoneLikeProperties::default(&Block::POINTED_DRIPSTONE);
    tip.vertical_direction = VerticalDirection::Down;
    tip.thickness = SpeleothemThickness::Tip;
    let tip_state = tip.to_state_id(&Block::POINTED_DRIPSTONE);
    world.set_block_state(
        &mud_pos,
        Block::MUD.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.set_block_state(
        &root_pos,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.set_block_state(&tip_pos, tip_state, BlockFlags::FORCE_STATE);
    if world.get_block(&mud_pos) != &Block::MUD
        || world.get_block(&tip_pos) != &Block::POINTED_DRIPSTONE
    {
        return Err("Could not place the mud and stalactite for the dripstone test".into());
    }

    let mud_top = f64::from(mud_pos.0.y) + 14.0 / 16.0;
    let stand = Arc::new(ArmorStandEntity::new(Entity::new(
        world.clone(),
        mud_pos.to_f64().add_raw(0.5, 14.0 / 16.0, 0.5),
        &EntityType::ARMOR_STAND,
    )));
    if !world.spawn_entity(stand.clone()) {
        return Err("Could not spawn the test armor stand".into());
    }

    // A roll of 0 always transfers
    maybe_transfer_fluid(&world, &tip_pos, tip_state, 0.0);
    let dried = world.get_block(&mud_pos) == &Block::CLAY;
    let lifted = stand.get_entity().pos.load().y - mud_top;

    stand.get_entity().remove();
    for pos in [tip_pos, root_pos, mud_pos] {
        world.set_block_state(&pos, Block::AIR.default_state.id, BlockFlags::FORCE_STATE);
    }
    if !dried || (lifted - 2.0 / 16.0).abs() > 1.0e-3 {
        return Err(format!(
            "Stalactite under mud: mud became clay={dried} (expected true), armor stand lifted by {lifted} (expected 0.125)"
        ));
    }

    summary_lines.push("[PASS] bot:dripstone_dries_mud_and_lifts_entity".into());
    Ok(())
}
