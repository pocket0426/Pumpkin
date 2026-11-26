use std::sync::Arc;

use pumpkin_core::{
    entity::{Entity, EntityBase, ai::brain::memory::types, mob::piglin::PiglinEntity},
    net::ClientPlatform,
    server::Server,
    world::BlockFlags,
};
use pumpkin_data::{
    Block, attributes::Attributes, data_component_impl::EquipmentSlot, dimension::Dimension,
    entity::EntityType, item::Item, item_stack::ItemStack,
};
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
    // Piglins attack any survival player without gold on sight, which is not what this covers:
    // the bot wears gold, and the bystander is made untargetable for the duration.
    let bystander = world.get_player_by_name("TestBot2");
    let bystander_gamemode = bystander.as_ref().map(|other| other.gamemode.load());
    if let Some(other) = &bystander {
        other.set_gamemode(GameMode::Creative);
    }
    let set_helmet = |stack: ItemStack| {
        player
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(&EquipmentSlot::HEAD, stack);
    };
    set_helmet(ItemStack::new(1, &Item::GOLDEN_HELMET));
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
    // Kept from strolling out of sight while its sensors warm up.
    piglin
        .mob_entity
        .living_entity
        .set_attribute_base(&Attributes::MOVEMENT_SPEED, 0.0);
    if !world.spawn_entity(piglin.clone()) {
        return Err("Could not spawn the test piglin".into());
    }
    let is_angry_at_player = || {
        piglin.mob_entity.with_brain(piglin.as_ref(), |brain| {
            brain.get(types::ANGRY_AT).copied() == Some(player.get_entity().entity_uuid)
        })
    };
    // Each piglin reacts on its own brain tick, and only once its sensors have seen the player.
    for _ in 0..40 {
        server.tick();
    }

    // A creative player is not attackable, so opening the barrel angers nobody.
    client
        .handle_use_item_on(&player, &use_barrel, server)
        .map_err(|err| format!("Using the barrel in creative failed: {err:?}"))?;
    for _ in 0..5 {
        server.tick();
    }
    let angry_in_creative = is_angry_at_player();
    player.close_handled_screen();

    let activity_before = piglin.mob_entity.with_brain(piglin.as_ref(), |brain| {
        format!(
            "{:?} attack_target={:?} angry_at={}",
            brain.get_active_non_core_activity(),
            brain.get(types::ATTACK_TARGET).map(|target| (
                target.get_entity().entity_type.resource_name,
                target
                    .get_player()
                    .map(|p| (p.gamemode.load(), p.gameprofile.name.clone())),
            )),
            brain.has_memory_value(types::ANGRY_AT.id()),
        )
    });
    player.set_gamemode(GameMode::Survival);
    client
        .handle_use_item_on(&player, &use_barrel, server)
        .map_err(|err| format!("Using the barrel in survival failed: {err:?}"))?;
    let mut angry_in_survival = false;
    for _ in 0..5 {
        server.tick();
        angry_in_survival = is_angry_at_player();
        if angry_in_survival {
            break;
        }
    }
    player.close_handled_screen();

    let brain_state = piglin.mob_entity.with_brain(piglin.as_ref(), |brain| {
        format!(
            "before={activity_before}; now={:?} idle={}, sees_living={}, sees_player={}, targetable_player={}, pos={:?}, alive={}, ticks={}, no_ai={}, player_pos={:?}",
            brain.get_active_non_core_activity(),
            brain.is_active(pumpkin_data::environment_attribute::Activity::Idle),
            brain.has_memory_value(types::NEAREST_VISIBLE_LIVING_ENTITIES.id()),
            brain.has_memory_value(types::NEAREST_VISIBLE_PLAYER.id()),
            brain.has_memory_value(types::NEAREST_VISIBLE_ATTACKABLE_PLAYER.id()),
            piglin.get_entity().pos.load(),
            piglin.get_entity().is_alive(),
            piglin
                .get_entity()
                .tick_count
                .load(std::sync::atomic::Ordering::Relaxed),
            piglin.mob_entity.is_no_ai(),
            player.get_entity().pos.load(),
        )
    });
    player.set_gamemode(GameMode::Creative);
    set_helmet(ItemStack::EMPTY.clone());
    if let (Some(other), Some(gamemode)) = (&bystander, bystander_gamemode) {
        other.set_gamemode(gamemode);
    }
    piglin.get_entity().remove();
    world.set_block_state(
        &barrel_pos,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    if angry_in_creative || !angry_in_survival {
        return Err(format!(
            "Opening a barrel next to a piglin: angry at a creative player={angry_in_creative} (expected false), at a survival player={angry_in_survival} (expected true); piglin brain: {brain_state}"
        ));
    }

    summary_lines.push("[PASS] bot:piglin_angered_by_opening_barrel".into());
    Ok(())
}
