use std::sync::Arc;

use pumpkin_core::{
    entity::{EntityBase, experience_orb::ExperienceOrbEntity},
    server::Server,
    world::World,
};
use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_util::GameMode;
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};

const DROPS: u32 = 120;
const VALUE: u32 = 7;

/// Live orbs around `center`: how many entities, and the experience they hold in all.
fn orbs_near(world: &World, center: Vector3<f64>) -> (usize, u32, Vec<Arc<dyn EntityBase>>) {
    let area = BoundingBox::new(
        center.add_raw(-12.0, -40.0, -12.0),
        center.add_raw(12.0, 12.0, 12.0),
    );
    let orbs: Vec<_> = world
        .get_entities_at_box(&area)
        .into_iter()
        .filter(|entity| {
            entity.get_entity().entity_type == &EntityType::EXPERIENCE_ORB
                && !entity.get_entity().is_removed()
        })
        .collect();
    let total = orbs
        .iter()
        .filter_map(|entity| entity.cast_any().downcast_ref::<ExperienceOrbEntity>())
        .map(|orb| orb.value() * orb.count().max(0) as u32)
        .sum();
    (orbs.len(), total, orbs)
}

/// Experience dropped on one spot piles into at most 40 orbs per value and none is lost,
/// neither on the drop nor while the orbs merge on the ground.
pub fn run_xp_orb_merge_test(
    server: &Arc<Server>,
    summary_lines: &mut Vec<String>,
) -> Result<(), String> {
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);
    let player = world
        .get_player_by_name("TestBot1")
        .ok_or("Could not find TestBot1 for the experience orb test")?;
    // Players pull orbs in from 8 blocks and pick them up, and where the second bot stands
    // is not known: both watch as spectators, which orbs ignore.
    let watchers: Vec<_> = world
        .players
        .load()
        .iter()
        .map(|watcher| (watcher.clone(), watcher.gamemode.load()))
        .collect();
    for (watcher, _) in &watchers {
        watcher.set_gamemode(GameMode::Spectator);
    }
    let center = player.get_entity().pos.load().add_raw(14.0, 6.0, 0.0);
    let (before, _, _) = orbs_near(&world, center);
    if before != 0 {
        return Err(format!(
            "{before} experience orbs were already lying around"
        ));
    }

    for _ in 0..DROPS {
        ExperienceOrbEntity::spawn(&world, center, VALUE);
    }
    let (dropped_count, dropped_total, _) = orbs_near(&world, center);
    for _ in 0..80 {
        server.tick();
    }
    let (settled_count, settled_total, orbs) = orbs_near(&world, center);
    for orb in orbs {
        orb.get_entity().remove();
    }
    for (watcher, gamemode) in watchers {
        watcher.set_gamemode(gamemode);
    }

    let expected_total = DROPS * VALUE;
    if dropped_count > 40 || dropped_total != expected_total {
        return Err(format!(
            "{DROPS} drops of {VALUE} experience on one spot made {dropped_count} orbs holding {dropped_total}, expected at most 40 holding {expected_total}"
        ));
    }
    if settled_count > dropped_count || settled_total != expected_total {
        return Err(format!(
            "After 80 ticks {settled_count} orbs hold {settled_total} experience, expected at most {dropped_count} holding {expected_total}"
        ));
    }

    summary_lines.push("[PASS] bot:xp_orbs_merge_without_losing_experience".into());
    Ok(())
}
