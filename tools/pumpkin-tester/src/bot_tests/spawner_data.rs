use std::sync::Arc;

use pumpkin_core::{
    block::entities::{BlockEntity, mob_spawner::MobSpawnerBlockEntity},
    entity::EntityBase,
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{Block, dimension::Dimension, entity::EntityType};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::boundingbox::BoundingBox;

use super::offset_within_chunk;

/// Vanilla `BaseSpawner.serverTick`: a spawner whose spawn data describes the entity makes it
/// as described, here pinned to one spot and floating there.
pub fn run_spawner_data_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the spawner test")?;
    // High above the bot, where nothing else is
    let spawner_pos = offset_within_chunk(player.get_entity().block_pos.load(), 2).up_height(25);
    let perch = spawner_pos.up_height(3).to_centered_f64();

    let mut stand = NbtCompound::new();
    stand.put_string("id", "minecraft:armor_stand".to_string());
    stand.put_bool("NoGravity", true);
    stand.put_list(
        "Pos",
        vec![
            NbtTag::Double(perch.x),
            NbtTag::Double(perch.y),
            NbtTag::Double(perch.z),
        ],
    );
    let mut spawn_data = NbtCompound::new();
    spawn_data.put_compound("entity", stand);
    // Any light will do, and an armor stand has no spawn rules of its own
    spawn_data.put_compound("custom_spawn_rules", NbtCompound::new());
    let mut nbt = NbtCompound::new();
    nbt.put_compound("SpawnData", spawn_data);
    nbt.put_short("Delay", 0);
    nbt.put_short("SpawnCount", 1);
    nbt.put_short("RequiredPlayerRange", 64);

    world.set_block_state(
        &spawner_pos,
        Block::SPAWNER.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    world.remove_block_entity(&spawner_pos);
    world.add_block_entity(Arc::new(MobSpawnerBlockEntity::from_nbt(&nbt, spawner_pos)));
    for _ in 0..3 {
        server.tick();
    }

    let around = BoundingBox::from_block(&spawner_pos).expand_all(8.0);
    let stands: Vec<_> = world
        .get_entities_at_box(&around)
        .into_iter()
        .filter(|entity| entity.get_entity().entity_type == &EntityType::ARMOR_STAND)
        .collect();
    let report: Vec<_> = stands
        .iter()
        .map(|stand| {
            let base = stand.get_entity();
            (base.pos.load(), base.has_no_gravity())
        })
        .collect();
    for stand in &stands {
        stand.get_entity().remove();
    }
    world.set_block_state(
        &spawner_pos,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    if report != vec![(perch, true)] {
        return Err(format!(
            "Spawner with a described armor stand: found {report:?} (position, floating), expected one floating at {perch:?}"
        ));
    }

    summary_lines.push("[PASS] bot:spawner_spawns_the_entity_its_data_describes".into());
    Ok(())
}
