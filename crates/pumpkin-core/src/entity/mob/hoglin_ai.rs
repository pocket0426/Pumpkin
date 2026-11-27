//! Vanilla `HoglinAi`.

use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::sound::Sound;
use pumpkin_util::math::position::BlockPos;
use rand::RngExt;

use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::crisis::set_walk_target_away_from;
use crate::entity::ai::brain::behavior::utils::{
    get_nearest_target, is_other_target_much_further_away_than_current_attack_target,
};
use crate::entity::ai::brain::behavior::{self, BehaviorControl, OneShot};
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::brain::sensing::{SensorType, is_entity_attackable};
use crate::entity::ai::brain::{BrainProvider, BrainState, BrainTick};
use crate::entity::mob::Mob;
use crate::entity::mob::hoglin::HoglinEntity;

pub const REPELLENT_DETECTION_RANGE_HORIZONTAL: i32 = 8;
pub const REPELLENT_DETECTION_RANGE_VERTICAL: i32 = 4;
/// Vanilla `RETREAT_DURATION`: 5 to 20 seconds.
const RETREAT_DURATION: std::ops::RangeInclusive<i64> = 100..=400;
const ATTACK_DURATION: i64 = 200;
const DESIRED_DISTANCE_FROM_PIGLIN_WHEN_IDLING: i32 = 8;
const DESIRED_DISTANCE_FROM_PIGLIN_WHEN_RETREATING: i32 = 15;
const ATTACK_INTERVAL: i32 = 40;
const BABY_ATTACK_INTERVAL: i32 = 15;
const REPELLENT_PACIFY_TIME: i64 = 200;
const ADULT_FOLLOW_RANGE: (i32, i32) = (5, 16);
const SPEED_MULTIPLIER_WHEN_AVOIDING_REPELLENT: f32 = 1.0;
const SPEED_MULTIPLIER_WHEN_RETREATING: f32 = 1.3;
const SPEED_MULTIPLIER_WHEN_MAKING_LOVE: f32 = 0.6;
const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 0.4;
const SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT: f32 = 0.6;

fn is_adult(mob: &dyn Mob) -> bool {
    !mob.get_mob_entity().living_entity.is_baby()
}

/// Vanilla `BecomePassiveIfMemoryPresent`: near a repellent the hoglin drops its target
/// and stays peaceful for a while.
fn become_passive_if_memory_present(memory: MemoryModuleId, pacify_time: i64) -> OneShot {
    OneShot::new(
        "BecomePassiveIfMemoryPresent",
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::Registered),
            (types::PACIFIED.id(), MemoryStatus::ValueAbsent),
            (memory, MemoryStatus::ValuePresent),
        ],
        move |tick| {
            tick.state
                .set_with_expiry(types::PACIFIED, true, pacify_time);
            tick.state.erase(types::ATTACK_TARGET.id());
            true
        },
    )
}

fn idle_movement_behaviors() -> Box<dyn BehaviorControl> {
    Box::new(behavior::run_one(vec![
        (
            Box::new(behavior::random_stroll::stroll(
                SPEED_MULTIPLIER_WHEN_IDLING,
            )),
            2,
        ),
        (
            Box::new(behavior::set_walk_target_from_look_target::create(
                SPEED_MULTIPLIER_WHEN_IDLING,
                3,
            )),
            2,
        ),
        (Box::new(behavior::DoNothing::new(30, 60)), 1),
    ]))
}

fn make_love() -> Box<dyn BehaviorControl> {
    Box::new(behavior::Timed::new(
        behavior::AnimalMakeLove::with_speed_and_distance(
            &EntityType::HOGLIN,
            SPEED_MULTIPLIER_WHEN_MAKING_LOVE,
            2,
        ),
    ))
}

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(behavior::Timed::new(behavior::LookAtTargetSink::new(
                45, 90,
            ))),
            Box::new(behavior::Timed::new(behavior::MoveToTargetSink::default())),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Idle,
        10,
        vec![
            Box::new(become_passive_if_memory_present(
                types::NEAREST_REPELLENT.id(),
                REPELLENT_PACIFY_TIME,
            )),
            make_love(),
            Box::new(set_walk_target_away_from::pos(
                types::NEAREST_REPELLENT,
                SPEED_MULTIPLIER_WHEN_AVOIDING_REPELLENT,
                8,
                true,
            )),
            Box::new(behavior::start_attacking::create(
                |_| true,
                |tick| find_nearest_valid_attack_target(tick.state),
            )),
            Box::new(behavior::one_shot::sequence(
                "KeepAwayFromAdultPiglin",
                |tick| is_adult(tick.mob),
                set_walk_target_away_from::entity(
                    types::NEAREST_VISIBLE_ADULT_PIGLIN,
                    SPEED_MULTIPLIER_WHEN_IDLING,
                    DESIRED_DISTANCE_FROM_PIGLIN_WHEN_IDLING,
                    false,
                ),
            )),
            Box::new(behavior::set_entity_look_target_sometimes::create(
                8.0, 30, 60,
            )),
            Box::new(behavior::baby_follow_adult::create(
                ADULT_FOLLOW_RANGE.0,
                ADULT_FOLLOW_RANGE.1,
                SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT,
            )),
            idle_movement_behaviors(),
        ],
    )
}

fn init_fight_activity() -> ActivityData {
    ActivityData::with_gate_memory(
        Activity::Fight,
        10,
        vec![
            Box::new(become_passive_if_memory_present(
                types::NEAREST_REPELLENT.id(),
                REPELLENT_PACIFY_TIME,
            )),
            make_love(),
            Box::new(
                behavior::set_walk_target_from_attack_target_if_target_out_of_reach::create(|_| {
                    1.0
                }),
            ),
            Box::new(behavior::melee_attack::create_with(
                is_adult,
                ATTACK_INTERVAL,
            )),
            Box::new(behavior::melee_attack::create_with(
                |mob| !is_adult(mob),
                BABY_ATTACK_INTERVAL,
            )),
            Box::new(behavior::stop_attacking_if_target_invalid::create()),
            Box::new(behavior::erase_memory_if::create(
                |tick| is_breeding(tick.state),
                types::ATTACK_TARGET.id(),
            )),
        ],
        types::ATTACK_TARGET.id(),
    )
}

fn init_retreat_activity() -> ActivityData {
    ActivityData::with_gate_memory(
        Activity::Avoid,
        10,
        vec![
            Box::new(set_walk_target_away_from::entity(
                types::AVOID_TARGET,
                SPEED_MULTIPLIER_WHEN_RETREATING,
                DESIRED_DISTANCE_FROM_PIGLIN_WHEN_RETREATING,
                false,
            )),
            idle_movement_behaviors(),
            Box::new(behavior::set_entity_look_target_sometimes::create(
                8.0, 30, 60,
            )),
            Box::new(behavior::erase_memory_if::create(
                |tick| wants_to_stop_fleeing(tick.mob, tick.state),
                types::AVOID_TARGET.id(),
            )),
        ],
        types::AVOID_TARGET.id(),
    )
}

/// Vanilla `HoglinAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    let old_activity = tick.state.get_active_non_core_activity();
    tick.state.set_active_activity_to_first_valid(&[
        Activity::Fight,
        Activity::Avoid,
        Activity::Idle,
    ]);
    let new_activity = tick.state.get_active_non_core_activity();
    if old_activity != new_activity
        && let Some(sound) = sound_for_current_activity(tick.mob, tick.state)
    {
        tick.mob.make_sound(sound);
    }
    let has_attack_target = tick.state.has_memory_value(types::ATTACK_TARGET.id());
    tick.mob.get_mob_entity().set_attacking(has_attack_target);
}

/// Vanilla `HoglinAi.onHitTarget`: a hoglin that just hit a piglin runs if the piglins
/// outnumber the hoglins, and otherwise calls the others in.
pub fn on_hit_target(tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) {
    if !is_adult(tick.mob) {
        return;
    }
    if target.get_entity().entity_type == &EntityType::PIGLIN
        && piglins_outnumber_hoglins(tick.mob, tick.state)
    {
        set_avoid_target(tick, target);
        broadcast_retreat(tick.state, target);
        return;
    }
    broadcast_attack_target(tick.state, target);
}

fn visible_adult_hoglins(state: &BrainState) -> Vec<Arc<dyn EntityBase>> {
    state
        .get(types::NEAREST_VISIBLE_ADULT_HOGLINS)
        .cloned()
        .unwrap_or_default()
}

/// The other hoglins have their own brains: they are told through their inboxes.
fn tell(
    hoglins: Vec<Arc<dyn EntityBase>>,
    message: fn(&mut BrainTick<'_>, &Arc<dyn EntityBase>),
    about: &Arc<dyn EntityBase>,
) {
    for hoglin in hoglins {
        if let Some(mob) = hoglin.get_mob() {
            let about = about.clone();
            mob.get_mob_entity()
                .post_to_brain(Box::new(move |tick| message(tick, &about)));
        }
    }
}

fn broadcast_retreat(state: &BrainState, target: &Arc<dyn EntityBase>) {
    tell(
        visible_adult_hoglins(state),
        retreat_from_nearest_target,
        target,
    );
}

fn broadcast_attack_target(state: &BrainState, target: &Arc<dyn EntityBase>) {
    tell(
        visible_adult_hoglins(state),
        set_attack_target_if_closer_than_current,
        target,
    );
}

fn retreat_from_nearest_target(tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) {
    let body = tick.mob.get_entity();
    let nearest = get_nearest_target(
        body,
        tick.state.get(types::AVOID_TARGET).cloned(),
        target.clone(),
    );
    let nearest = get_nearest_target(body, tick.state.get(types::ATTACK_TARGET).cloned(), nearest);
    set_avoid_target(tick, &nearest);
}

fn set_avoid_target(tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) {
    tick.state.erase(types::ATTACK_TARGET.id());
    tick.state.erase(types::WALK_TARGET.id());
    let duration = tick.mob.get_random().random_range(RETREAT_DURATION);
    tick.state
        .set_with_expiry(types::AVOID_TARGET, target.clone(), duration);
}

fn find_nearest_valid_attack_target(state: &BrainState) -> Option<Arc<dyn EntityBase>> {
    if is_pacified(state) || is_breeding(state) {
        return None;
    }
    state
        .get(types::NEAREST_VISIBLE_ATTACKABLE_PLAYER)
        .map(|player| player.clone() as Arc<dyn EntityBase>)
}

/// Vanilla `isPosNearNearestRepellent`, on the repellent the entity last sensed.
#[must_use]
pub fn is_pos_near_repellent(repellent: Option<BlockPos>, pos: &BlockPos) -> bool {
    repellent.is_some_and(|repellent| {
        let delta = repellent.0 - pos.0;
        f64::from(delta.x * delta.x + delta.y * delta.y + delta.z * delta.z) < 8.0 * 8.0
    })
}

fn wants_to_stop_fleeing(mob: &dyn Mob, state: &BrainState) -> bool {
    is_adult(mob) && !piglins_outnumber_hoglins(mob, state)
}

fn piglins_outnumber_hoglins(mob: &dyn Mob, state: &BrainState) -> bool {
    if !is_adult(mob) {
        return false;
    }
    let piglins = state
        .get(types::VISIBLE_ADULT_PIGLIN_COUNT)
        .copied()
        .unwrap_or(0);
    let hoglins = state
        .get(types::VISIBLE_ADULT_HOGLIN_COUNT)
        .copied()
        .unwrap_or(0)
        + 1;
    piglins > hoglins
}

/// Vanilla `HoglinAi.wasHurtBy`.
pub fn was_hurt_by(tick: &mut BrainTick<'_>, attacker: &Arc<dyn EntityBase>) {
    tick.state.erase(types::PACIFIED.id());
    tick.state.erase(types::BREED_TARGET.id());
    if is_adult(tick.mob) {
        maybe_retaliate(tick, attacker);
    } else {
        retreat_from_nearest_target(tick, attacker);
    }
}

fn maybe_retaliate(tick: &mut BrainTick<'_>, attacker: &Arc<dyn EntityBase>) {
    let attacker_type = attacker.get_entity().entity_type;
    if tick.state.is_active(Activity::Avoid) && attacker_type == &EntityType::PIGLIN {
        return;
    }
    if attacker_type == &EntityType::HOGLIN {
        return;
    }
    if is_other_target_much_further_away_than_current_attack_target(
        tick.state,
        tick.mob.get_entity(),
        attacker.as_ref(),
        4.0,
    ) || !is_entity_attackable(&tick.visibility(), attacker.as_ref())
    {
        return;
    }
    set_attack_target(tick, attacker);
    broadcast_attack_target(tick.state, attacker);
}

fn set_attack_target(tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) {
    tick.state.erase(types::CANT_REACH_WALK_TARGET_SINCE.id());
    tick.state.erase(types::BREED_TARGET.id());
    tick.state
        .set_with_expiry(types::ATTACK_TARGET, target.clone(), ATTACK_DURATION);
}

fn set_attack_target_if_closer_than_current(
    tick: &mut BrainTick<'_>,
    target: &Arc<dyn EntityBase>,
) {
    if is_pacified(tick.state) {
        return;
    }
    let nearest = get_nearest_target(
        tick.mob.get_entity(),
        tick.state.get(types::ATTACK_TARGET).cloned(),
        target.clone(),
    );
    set_attack_target(tick, &nearest);
}

/// Vanilla `getSoundForCurrentActivity`.
#[must_use]
pub fn sound_for_current_activity(mob: &dyn Mob, state: &BrainState) -> Option<Sound> {
    let activity = state.get_active_non_core_activity()?;
    let converting = mob
        .cast_any()
        .downcast_ref::<HoglinEntity>()
        .is_some_and(|hoglin| hoglin.is_converting(&mob.get_entity().world.load()));
    Some(if activity == Activity::Avoid || converting {
        Sound::EntityHoglinRetreat
    } else if activity == Activity::Fight {
        Sound::EntityHoglinAngry
    } else if state.has_memory_value(types::NEAREST_REPELLENT.id()) {
        Sound::EntityHoglinRetreat
    } else {
        Sound::EntityHoglinAmbient
    })
}

fn is_breeding(state: &BrainState) -> bool {
    state.has_memory_value(types::BREED_TARGET.id())
}

#[must_use]
pub fn is_pacified(state: &BrainState) -> bool {
    state.has_memory_value(types::PACIFIED.id())
}

pub const MEMORY_TYPES: &[MemoryModuleId] = &[
    types::PACIFIED.id(),
    types::BREED_TARGET.id(),
    types::AVOID_TARGET.id(),
    types::ATTACK_TARGET.id(),
    types::CANT_REACH_WALK_TARGET_SINCE.id(),
];

pub const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::NearestPlayers,
    SensorType::NearestAdult,
    SensorType::HoglinSpecific,
];

pub static HOGLIN_PROVIDER: BrainProvider = BrainProvider {
    memory_types: MEMORY_TYPES,
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_fight_activity(),
            init_retreat_activity(),
        ]
    },
};
