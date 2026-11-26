//! Vanilla `SpearApproach`, `SpearAttack` and `SpearRetreat`: the three stages a brain mob
//! cycles through while holding a kinetic weapon, handed over through `SPEAR_STATUS`.

use std::sync::Arc;

use pumpkin_data::data_component_impl::{EquipmentSlot, KineticWeaponImpl};
use pumpkin_util::Hand;
use pumpkin_util::math::vector3::Vector3;

use super::super::BrainTick;
use super::super::memory::position_tracker::{EntityTracker, PositionTracker};
use super::super::memory::value::SpearStatus;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::entity::EntityBase;
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::ai::util::land_random_pos;
use crate::entity::mob::Mob;

const MIN_REPOSITION_DISTANCE: f64 = 6.0;
const MAX_REPOSITION_DISTANCE: f64 = 7.0;
const MIN_COOLDOWN_DISTANCE: f64 = 9.0;
const MAX_COOLDOWN_DISTANCE: f64 = 11.0;
const MAX_FLEEING_TIME: i32 = 100;
/// `Item.getUseDuration` of a kinetic weapon.
const SPEAR_USE_DURATION: i32 = 72_000;

fn kinetic_weapon(mob: &dyn Mob) -> Option<KineticWeaponImpl> {
    mob.get_mob_entity()
        .living_entity
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&EquipmentSlot::MAIN_HAND)
        .get_data_component::<KineticWeaponImpl>()
        .cloned()
}

fn able_to_attack(tick: &BrainTick<'_>) -> bool {
    tick.state.has_memory_value(types::ATTACK_TARGET.id()) && kinetic_weapon(tick.mob).is_some()
}

fn is_using_item(mob: &dyn Mob) -> bool {
    mob.get_mob_entity()
        .living_entity
        .item_in_use
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some()
}

/// Vanilla `KineticWeapon.computeDamageUseDuration`.
fn kinetic_weapon_use_duration(mob: &dyn Mob) -> i32 {
    kinetic_weapon(mob).map_or(0, |weapon| {
        weapon.delay_ticks
            + weapon
                .damage_conditions
                .as_ref()
                .map_or(0, |condition| condition.max_duration_ticks)
    })
}

/// The mount's `chargeSpeedModifier`, or 1 on foot.
fn charge_speed_modifier(mob: &dyn Mob) -> f64 {
    mob.get_entity()
        .get_root_vehicle()
        .as_deref()
        .and_then(|vehicle| vehicle.get_mob())
        .map_or(1.0, |vehicle| f64::from(vehicle.charge_speed_modifier()))
}

fn mount_distance(mob: &dyn Mob) -> f64 {
    if mob.get_entity().has_vehicle() {
        2.0
    } else {
        0.0
    }
}

fn move_to(mob: &dyn Mob, destination: Vector3<f64>, speed: f64) {
    mob.get_mob_entity()
        .steered()
        .navigator()
        .set_progress(NavigatorGoal {
            current_progress: mob.get_entity().pos.load(),
            destination,
            speed,
        });
}

fn stop_navigation(mob: &dyn Mob) {
    mob.get_mob_entity().steered().navigator().stop();
}

fn look_at(tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) {
    tick.state.set(
        types::LOOK_TARGET,
        Arc::new(EntityTracker::new(Arc::clone(target), true)) as Arc<dyn PositionTracker>,
    );
}

fn squared_distance_to(mob: &dyn Mob, target: &dyn EntityBase) -> f64 {
    mob.get_entity()
        .pos
        .load()
        .squared_distance_to_vec(&target.get_entity().pos.load())
}

pub struct SpearApproach {
    conditions: [(MemoryModuleId, MemoryStatus); 1],
    speed_modifier_when_repositioning: f64,
    approach_distance_sq: f64,
}

impl SpearApproach {
    #[must_use]
    pub fn new(speed_modifier_when_repositioning: f64, approach_distance: f32) -> Self {
        Self {
            conditions: [(types::SPEAR_STATUS.id(), MemoryStatus::ValueAbsent)],
            speed_modifier_when_repositioning,
            approach_distance_sq: f64::from(approach_distance * approach_distance),
        }
    }
}

impl Behavior for SpearApproach {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn can_time_out(&self) -> bool {
        false
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        able_to_attack(tick) && !is_using_item(tick.mob)
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        tick.mob.get_mob_entity().set_attacking(true);
        tick.state.set(types::SPEAR_STATUS, SpearStatus::Approach);
    }

    fn can_still_use(&mut self, tick: &mut BrainTick<'_>) -> bool {
        able_to_attack(tick)
            && tick.state.get(types::ATTACK_TARGET).is_some_and(|target| {
                squared_distance_to(tick.mob, target.as_ref()) > self.approach_distance_sq
            })
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(target) = tick.state.get(types::ATTACK_TARGET).cloned() else {
            return;
        };
        let speed_modifier = charge_speed_modifier(tick.mob);
        look_at(tick, &target);
        move_to(
            tick.mob,
            target.get_entity().pos.load(),
            speed_modifier * self.speed_modifier_when_repositioning,
        );
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>, _timed_out: bool) {
        stop_navigation(tick.mob);
        tick.state.set(types::SPEAR_STATUS, SpearStatus::Charging);
    }

    fn debug_name(&self) -> &'static str {
        "SpearApproach"
    }
}

pub struct SpearAttack {
    conditions: [(MemoryModuleId, MemoryStatus); 1],
    speed_modifier_when_charging: f64,
    speed_modifier_when_repositioning: f64,
    target_in_range_radius_sq: f64,
}

impl SpearAttack {
    #[must_use]
    pub fn new(
        speed_modifier_when_charging: f64,
        speed_modifier_when_repositioning: f64,
        target_in_range_radius: f32,
    ) -> Self {
        Self {
            conditions: [(types::SPEAR_STATUS.id(), MemoryStatus::ValuePresent)],
            speed_modifier_when_charging,
            speed_modifier_when_repositioning,
            target_in_range_radius_sq: f64::from(target_in_range_radius * target_in_range_radius),
        }
    }
}

impl Behavior for SpearAttack {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn can_time_out(&self) -> bool {
        false
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        tick.state.get(types::SPEAR_STATUS).copied() == Some(SpearStatus::Charging)
            && able_to_attack(tick)
            && !is_using_item(tick.mob)
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let mob_entity = tick.mob.get_mob_entity();
        mob_entity.set_attacking(true);
        tick.state.set(
            types::SPEAR_ENGAGE_TIME,
            kinetic_weapon_use_duration(tick.mob),
        );
        tick.state.erase(types::SPEAR_CHARGE_POSITION.id());
        let stack = mob_entity
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND);
        mob_entity
            .living_entity
            .set_active_hand(Hand::Right, stack, SPEAR_USE_DURATION);
    }

    fn can_still_use(&mut self, tick: &mut BrainTick<'_>) -> bool {
        tick.state
            .get(types::SPEAR_ENGAGE_TIME)
            .copied()
            .unwrap_or(0)
            > 0
            && able_to_attack(tick)
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(target) = tick.state.get(types::ATTACK_TARGET).cloned() else {
            return;
        };
        let target_pos = target.get_entity().pos.load();
        let target_dist_sq = squared_distance_to(tick.mob, target.as_ref());
        let speed_modifier = charge_speed_modifier(tick.mob);
        let mount_distance = mount_distance(tick.mob);
        look_at(tick, &target);
        let engage_time = tick
            .state
            .get(types::SPEAR_ENGAGE_TIME)
            .copied()
            .unwrap_or(0);
        tick.state.set(types::SPEAR_ENGAGE_TIME, engage_time - 1);

        if let Some(charge_pos) = tick.state.get(types::SPEAR_CHARGE_POSITION).copied() {
            move_to(
                tick.mob,
                charge_pos,
                speed_modifier * self.speed_modifier_when_repositioning,
            );
            if tick.mob.is_navigator_idle() {
                tick.state.erase(types::SPEAR_CHARGE_POSITION.id());
            }
        } else {
            move_to(
                tick.mob,
                target_pos,
                speed_modifier * self.speed_modifier_when_charging,
            );
            if target_dist_sq < self.target_in_range_radius_sq || tick.mob.is_navigator_idle() {
                let distance = target_dist_sq.sqrt();
                let away_pos = land_random_pos::get_pos_away_between(
                    tick.mob,
                    MIN_REPOSITION_DISTANCE + mount_distance - distance,
                    MAX_REPOSITION_DISTANCE + mount_distance - distance,
                    7,
                    target_pos,
                );
                // Vanilla `setMemory` with a null value erases, and the memory is already absent
                if let Some(away_pos) = away_pos {
                    tick.state.set(types::SPEAR_CHARGE_POSITION, away_pos);
                }
            }
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>, _timed_out: bool) {
        stop_navigation(tick.mob);
        tick.mob.get_mob_entity().living_entity.clear_active_hand();
        tick.state.erase(types::SPEAR_CHARGE_POSITION.id());
        tick.state.erase(types::SPEAR_ENGAGE_TIME.id());
        tick.state.set(types::SPEAR_STATUS, SpearStatus::Retreat);
    }

    fn debug_name(&self) -> &'static str {
        "SpearAttack"
    }
}

pub struct SpearRetreat {
    conditions: [(MemoryModuleId, MemoryStatus); 1],
    speed_modifier_when_repositioning: f64,
}

impl SpearRetreat {
    #[must_use]
    pub const fn new(speed_modifier_when_repositioning: f64) -> Self {
        Self {
            conditions: [(types::SPEAR_STATUS.id(), MemoryStatus::ValuePresent)],
            speed_modifier_when_repositioning,
        }
    }
}

impl Behavior for SpearRetreat {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn min_duration(&self) -> i32 {
        MAX_FLEEING_TIME
    }

    fn max_duration(&self) -> i32 {
        MAX_FLEEING_TIME
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        if !able_to_attack(tick) || is_using_item(tick.mob) {
            return false;
        }
        if tick.state.get(types::SPEAR_STATUS).copied() != Some(SpearStatus::Retreat) {
            return false;
        }
        let Some(target) = tick.state.get(types::ATTACK_TARGET).cloned() else {
            return false;
        };
        let distance = squared_distance_to(tick.mob, target.as_ref()).sqrt();
        let mount_distance = mount_distance(tick.mob);
        let Some(away_pos) = land_random_pos::get_pos_away_between(
            tick.mob,
            (MIN_COOLDOWN_DISTANCE + mount_distance - distance).max(0.0),
            (MAX_COOLDOWN_DISTANCE + mount_distance - distance).max(1.0),
            7,
            target.get_entity().pos.load(),
        ) else {
            return false;
        };
        tick.state.set(types::SPEAR_FLEEING_POSITION, away_pos);
        true
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        tick.mob.get_mob_entity().set_attacking(true);
        tick.state.set(types::SPEAR_FLEEING_TIME, 0);
    }

    fn can_still_use(&mut self, tick: &mut BrainTick<'_>) -> bool {
        tick.state
            .get(types::SPEAR_FLEEING_TIME)
            .copied()
            .unwrap_or(MAX_FLEEING_TIME)
            < MAX_FLEEING_TIME
            && tick
                .state
                .has_memory_value(types::SPEAR_FLEEING_POSITION.id())
            && !tick.mob.is_navigator_idle()
            && able_to_attack(tick)
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(target) = tick.state.get(types::ATTACK_TARGET).cloned() else {
            return;
        };
        let speed_modifier = charge_speed_modifier(tick.mob);
        look_at(tick, &target);
        let fleeing_time = tick
            .state
            .get(types::SPEAR_FLEEING_TIME)
            .copied()
            .unwrap_or(0);
        tick.state.set(types::SPEAR_FLEEING_TIME, fleeing_time + 1);
        if let Some(fleeing_pos) = tick.state.get(types::SPEAR_FLEEING_POSITION).copied() {
            move_to(
                tick.mob,
                fleeing_pos,
                speed_modifier * self.speed_modifier_when_repositioning,
            );
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>, _timed_out: bool) {
        let mob_entity = tick.mob.get_mob_entity();
        stop_navigation(tick.mob);
        mob_entity.set_attacking(false);
        mob_entity.living_entity.clear_active_hand();
        tick.state.erase(types::SPEAR_FLEEING_TIME.id());
        tick.state.erase(types::SPEAR_FLEEING_POSITION.id());
        tick.state.erase(types::SPEAR_STATUS.id());
    }

    fn debug_name(&self) -> &'static str {
        "SpearRetreat"
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::entity::EntityType;
    use pumpkin_data::environment_attribute::Activity;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;

    use super::*;
    use crate::entity::Entity;
    use crate::entity::ai::brain::Brain;
    use crate::entity::ai::brain::activity::ActivityData;
    use crate::entity::ai::brain::behavior::Timed;
    use crate::entity::ai::test_support::world;
    use crate::entity::mob::piglin::PiglinEntity;
    use crate::entity::passive::cow::CowEntity;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn spear_holder_approaches_then_charges_then_hands_over_to_retreat() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let body = PiglinEntity::new(Entity::new(
            world.clone(),
            Vector3::new(0.0, 64.0, 0.0),
            &EntityType::PIGLIN,
        ));
        body.hold_in_main_hand(ItemStack::new(1, &Item::GOLDEN_SPEAR));
        let target = CowEntity::new(Entity::new(
            world.clone(),
            Vector3::new(20.0, 64.0, 0.0),
            &EntityType::COW,
        ));
        let use_duration = kinetic_weapon_use_duration(&*body);

        let mut brain = Brain::default();
        brain.add_activity(ActivityData::with_priority_start(
            Activity::Core,
            0,
            vec![
                Box::new(Timed::new(SpearApproach::new(1.0, 10.0))),
                Box::new(Timed::new(SpearAttack::new(1.0, 1.0, 2.0))),
            ],
        ));
        // A real mob gets these from its provider and its fight activity's gate
        for id in [
            types::ATTACK_TARGET.id(),
            types::LOOK_TARGET.id(),
            types::SPEAR_ENGAGE_TIME.id(),
            types::SPEAR_CHARGE_POSITION.id(),
        ] {
            brain.register_memory(id);
        }
        brain.set(types::ATTACK_TARGET, target.clone() as Arc<dyn EntityBase>);

        let mut time = 1;
        brain.tick(&world, &*body, time);
        let approaching = brain.get(types::SPEAR_STATUS).copied();
        let using_while_approaching = is_using_item(&*body);

        // Within the approach distance the approach ends and hands over to the charge
        target.get_entity().set_pos(Vector3::new(5.0, 64.0, 0.0));
        time += 1;
        brain.tick(&world, &*body, time);
        let handed_over = brain.get(types::SPEAR_STATUS).copied();
        time += 1;
        brain.tick(&world, &*body, time);
        let engage_time = brain.get(types::SPEAR_ENGAGE_TIME).copied();
        let using_while_charging = is_using_item(&*body);

        // The charge lasts exactly the weapon's damage window
        let mut charge_ticks = 0;
        while brain.get(types::SPEAR_STATUS).copied() == Some(SpearStatus::Charging)
            && charge_ticks < 10_000
        {
            time += 1;
            brain.tick(&world, &*body, time);
            charge_ticks += 1;
        }
        let after_charge = brain.get(types::SPEAR_STATUS).copied();
        let using_after_charge = is_using_item(&*body);
        let engage_time_after = brain.get(types::SPEAR_ENGAGE_TIME).copied();
        world.level.shutdown().await;

        assert!(use_duration > 0, "a golden spear has a damage window");
        assert_eq!(approaching, Some(SpearStatus::Approach));
        assert!(!using_while_approaching);
        assert_eq!(handed_over, Some(SpearStatus::Charging));
        // A behavior ticks on the tick it starts, as in vanilla `Brain.tick`
        assert_eq!(engage_time, Some(use_duration - 1));
        assert!(using_while_charging);
        assert_eq!(charge_ticks, use_duration);
        assert_eq!(after_charge, Some(SpearStatus::Retreat));
        assert!(!using_after_charge);
        assert_eq!(engage_time_after, None);
        Ok(())
    }
}
