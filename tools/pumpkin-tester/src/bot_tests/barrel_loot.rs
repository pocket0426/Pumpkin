use std::sync::Arc;

use pumpkin_core::{
    block::entities::barrel::BarrelBlockEntity, entity::EntityBase, net::ClientPlatform,
    server::Server, world::BlockFlags,
};
use pumpkin_data::{Block, dimension::Dimension};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{GameMode, math::vector3::Vector3};

use super::offset_within_chunk;

/// Vanilla `RandomizableContainerBlockEntity`: a barrel generated with a loot table stays shut
/// for a spectator and is filled for the first other player who opens it.
pub fn run_barrel_loot_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the barrel loot test")?;
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        return Err("Expected a Java player for the barrel loot test".into());
    };
    let barrel_pos = offset_within_chunk(player.get_entity().block_pos.load(), 2);
    let previous = world.get_block_state_id(&barrel_pos);
    world.set_block_state(
        &barrel_pos,
        Block::BARREL.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    let packed = BarrelBlockEntity::new(barrel_pos);
    packed
        .loot
        .set("minecraft:chests/village/village_fisher", 1);
    world.add_block_entity(Arc::new(packed));
    let barrel = world
        .get_block_entity(&barrel_pos)
        .ok_or("Could not place the test barrel")?;
    let use_barrel = || {
        client
            .handle_use_item_on(
                &player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: barrel_pos,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(400),
                },
                server,
            )
            .map_err(|error| format!("Could not use the test barrel: {error}"))
    };
    let item_count = || {
        barrel.clone().get_inventory().map_or(0, |inventory| {
            (0..inventory.size())
                .filter(|&slot| !inventory.get_stack(slot).is_empty())
                .count()
        })
    };

    let gamemode = player.gamemode.load();
    player.set_gamemode(GameMode::Spectator);
    use_barrel()?;
    let packed_after_spectator = barrel.has_loot_table();
    let items_after_spectator = item_count();

    player.set_gamemode(GameMode::Creative);
    use_barrel()?;
    let packed_after_player = barrel.has_loot_table();
    let items_after_player = item_count();

    player.close_handled_screen();
    player.set_gamemode(gamemode);
    // The loot must not litter the world for the tests that follow
    if let Some(inventory) = barrel.clone().get_inventory() {
        inventory.clear();
    }
    world.set_block_state(&barrel_pos, previous, BlockFlags::NOTIFY_ALL);
    if !packed_after_spectator
        || items_after_spectator != 0
        || packed_after_player
        || items_after_player == 0
    {
        return Err(format!(
            "Barrel with a loot table: after a spectator, still packed={packed_after_spectator} (expected true) with {items_after_spectator} filled slots (expected 0); after a creative player, still packed={packed_after_player} (expected false) with {items_after_player} filled slots (expected some)"
        ));
    }

    summary_lines.push("[PASS] bot:barrel_loot_table_unpacks_for_first_player".into());
    Ok(())
}
