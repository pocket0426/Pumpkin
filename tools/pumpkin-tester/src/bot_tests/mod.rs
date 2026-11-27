mod barrel_loot;
mod bed_villager;
mod bell_ring;
mod block_tuning_and_invert;
mod commands_and_gamemode;
mod dripstone_mud;
mod embedded_stop;
mod login_and_movement;
mod loot_tables;
mod piglin_anger;
mod player_actions;
mod polar_bear;
mod sheep_shearing;
mod shulker_lid;
mod sign_authorization;
mod snow_golem;
mod spawner_data;
mod status_ping;
mod world_interaction;

use std::{net::SocketAddr, sync::Arc};

use pumpkin_core::server::Server;
use pumpkin_gametest::GameTestRunner;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

/// A position `distance` blocks from `origin` on X and Z that stays in `origin`'s chunk, which is
/// loaded when the bot stands in it. Its neighbours may still be loading.
fn offset_within_chunk(origin: BlockPos, distance: i32) -> BlockPos {
    let towards_middle = |coordinate: i32| {
        if coordinate.rem_euclid(16) < 8 {
            distance
        } else {
            -distance
        }
    };
    origin.offset(Vector3::new(
        towards_middle(origin.0.x),
        0,
        towards_middle(origin.0.z),
    ))
}

pub async fn run_bot_tests(
    server: &Arc<Server>,
    addr: SocketAddr,
    runner: &mut GameTestRunner,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    status_ping::run_bot_status_ping_test(addr, summary_lines).await?;

    let (mut bot1, bot2) =
        login_and_movement::run_login_and_movement_tests(server, addr, summary_lines).await?;

    commands_and_gamemode::run_commands_and_gamemode_tests(
        server,
        &mut bot1,
        runner,
        summary_lines,
    )
    .await?;

    loot_tables::run_loot_table_tests(server, &mut bot1, summary_lines).await?;

    world_interaction::run_world_interaction_tests(server, &mut bot1, summary_lines).await?;

    sheep_shearing::run_sheep_shearing_test(server, summary_lines)?;
    snow_golem::run_snow_golem_shearing_test(server, summary_lines)?;
    bed_villager::run_bed_kicks_sleeping_villager_test(server, summary_lines)?;
    shulker_lid::run_shulker_lid_pushes_entity_test(server, summary_lines)?;
    dripstone_mud::run_dripstone_dries_mud_test(server, summary_lines)?;
    bell_ring::run_bell_ring_test(server, summary_lines)?;
    barrel_loot::run_barrel_loot_test(server, summary_lines)?;
    spawner_data::run_spawner_data_test(server, summary_lines)?;
    piglin_anger::run_piglin_anger_test(server, summary_lines)?;
    vex_charge::run_vex_charge_test(server, summary_lines)?;
    xp_orb_merge::run_xp_orb_merge_test(server, summary_lines)?;
    hoglin_brain::run_hoglin_brain_test(server, summary_lines)?;

    polar_bear::run_polar_bear_defense_test(server, &bot1, summary_lines)?;

    block_tuning_and_invert::run_block_tuning_and_invert_tests(server, &mut bot1, summary_lines)
        .await?;

    allay::run_allay_interaction_test(server, &mut bot1, summary_lines).await?;

    player_actions::run_player_actions_and_disconnect_tests(server, bot1, bot2, summary_lines)
        .await?;

    sign_authorization::run_sign_authorization_tests(server, addr, summary_lines).await?;

    embedded_stop::run_embedded_stop_test(summary_lines).await?;

    Ok(())
}
mod allay;
mod hoglin_brain;
mod vex_charge;
mod xp_orb_merge;
