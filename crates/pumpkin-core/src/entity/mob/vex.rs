use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use uuid::Uuid;

use crate::entity::ai::behavior::hunting::{self, Hunt};
use crate::entity::ai::brain::memory::codec::{block_pos_from_tag, block_pos_to_tag};
use crate::entity::ai::control::move_control::{MoveControl, Operation};
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::ai::goal::track_target::TrackTargetGoal;
use crate::entity::ai::goal::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::target_match::TargetMatch;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::raider;
use crate::entity::mob::spawn::{EntitySpawnReason, SpawnGroupData};
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{look_at_entity::LookAtEntityGoal, swim::SwimGoal},
    mob::{Mob, MobEntity},
};
use crate::plugin::api::events::entity::entity_target::TargetReason;
use crate::world::World;

const VEX_HUNTS: &[Hunt] = &[Hunt::new(3, TargetMatch::Type(&EntityType::PLAYER), true)];

/// Vanilla `TICKS_PER_FLAP`: `ceil(3.9269907)`.
const TICKS_PER_FLAP: i32 = 4;
const FLAG_IS_CHARGING: i8 = 1;
/// Ticks between two starvation hits once the summoned life has run out.
const LIFE_OVER_DAMAGE_INTERVAL: i32 = 20;

#[derive(Default)]
struct Owner {
    uuid: Option<Uuid>,
    /// The entity behind `uuid`, kept so the per-tick target copy skips the world lookup.
    cached: Option<Weak<dyn EntityBase>>,
}

pub struct VexEntity {
    pub mob_entity: MobEntity,
    owner: Mutex<Owner>,
    bound_origin: AtomicCell<Option<BlockPos>>,
    has_limited_life: AtomicBool,
    limited_life_ticks: AtomicI32,
    charging: AtomicBool,
}

impl VexEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(VexMoveControl::default());
        let vex = Self {
            mob_entity,
            owner: Mutex::new(Owner::default()),
            bound_origin: AtomicCell::new(None),
            has_limited_life: AtomicBool::new(false),
            limited_life_ticks: AtomicI32::new(0),
            charging: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(vex);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let goal_selector = &mob_arc.mob_entity.goals_selector;

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            goal_selector.add_goal(4, Box::new(VexChargeAttackGoal));
            goal_selector.add_goal(8, Box::new(VexRandomMoveGoal));
            goal_selector.add_goal(
                9,
                Box::new(LookAtEntityGoal::new(
                    mob_weak.clone(),
                    &EntityType::PLAYER,
                    3.0,
                    1.0,
                    false,
                )),
            );
            goal_selector.add_goal(
                10,
                Box::new(LookAtEntityGoal::matching(
                    mob_weak,
                    TargetMatch::Mob,
                    8.0,
                    0.02,
                    false,
                )),
            );

            let target_selector = &mob_arc.mob_entity.target_selector;
            target_selector.add_goal(1, raider::revenge_goal());
            target_selector.add_goal(2, Box::new(VexCopyOwnerTargetGoal::new()));
            hunting::apply(target_selector, &mob_arc.mob_entity, VEX_HUNTS);
        };

        mob_arc
    }

    fn as_vex(mob: &dyn Mob) -> Option<&Self> {
        mob.cast_any().downcast_ref::<Self>()
    }

    #[must_use]
    pub fn is_charging(&self) -> bool {
        self.charging.load(Ordering::Relaxed)
    }

    pub fn set_charging(&self, charging: bool) {
        if self.charging.swap(charging, Ordering::Relaxed) != charging {
            self.get_entity().set_synced_data(
                tracked_data::vex::DATA_FLAGS_ID,
                if charging { FLAG_IS_CHARGING } else { 0 },
            );
        }
    }

    pub fn set_owner(&self, owner: &Arc<dyn EntityBase>) {
        *self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Owner {
            uuid: Some(owner.get_entity().entity_uuid),
            cached: Some(Arc::downgrade(owner)),
        };
    }

    #[must_use]
    pub fn owner_uuid(&self) -> Option<Uuid> {
        self.owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .uuid
    }

    /// The living owner, if it is loaded in this world.
    #[must_use]
    pub fn get_owner(&self) -> Option<Arc<dyn EntityBase>> {
        let (uuid, cached) = {
            let owner = self
                .owner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (owner.uuid?, owner.cached.as_ref().and_then(Weak::upgrade))
        };
        if let Some(cached) = cached
            && !cached.get_entity().is_removed()
        {
            return Some(cached);
        }
        let found = self.get_entity().world.load().get_entity_by_uuid(uuid);
        self.owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cached = found.as_ref().map(Arc::downgrade);
        found
    }

    #[must_use]
    pub fn bound_origin(&self) -> Option<BlockPos> {
        self.bound_origin.load()
    }

    pub fn set_bound_origin(&self, origin: Option<BlockPos>) {
        self.bound_origin.store(origin);
    }

    pub fn set_limited_life(&self, ticks: i32) {
        self.has_limited_life.store(true, Ordering::Relaxed);
        self.limited_life_ticks.store(ticks, Ordering::Relaxed);
    }

    #[must_use]
    pub fn limited_life_ticks(&self) -> Option<i32> {
        self.has_limited_life
            .load(Ordering::Relaxed)
            .then(|| self.limited_life_ticks.load(Ordering::Relaxed))
    }

    fn move_control(&self) -> std::sync::MutexGuard<'_, Box<dyn MoveControlTrait>> {
        self.mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn has_wanted(&self) -> bool {
        self.move_control().has_wanted()
    }

    fn fly_to(&self, pos: Vector3<f64>, speed: f64) {
        self.move_control()
            .set_wanted_position(pos.x, pos.y, pos.z, speed);
    }

    fn living_target(&self) -> Option<Arc<dyn EntityBase>> {
        self.mob_entity.get_target().filter(|target| {
            target
                .get_living_entity()
                .is_some_and(crate::entity::living::LivingEntity::is_alive)
        })
    }
}

impl Mob for VexEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_is_flapping(&self) -> bool {
        self.get_entity().tick_count.load(Ordering::Relaxed) % TICKS_PER_FLAP == 0
    }

    fn mob_init_data_tracker(&self) {
        if self.is_charging() {
            self.get_entity()
                .set_synced_data(tracked_data::vex::DATA_FLAGS_ID, FLAG_IS_CHARGING);
        }
    }

    /// Vanilla `tick` runs the whole entity tick with `noPhysics` set, which is what
    /// lets a vex pass through blocks, and clears it again afterwards.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.get_entity().no_physics.store(true, Ordering::Relaxed);
    }

    fn post_tick(&self) {
        let entity = self.get_entity();
        entity.no_physics.store(false, Ordering::Relaxed);
        if !entity.has_no_gravity() {
            entity.set_has_no_gravity(true);
        }
        if self.has_limited_life.load(Ordering::Relaxed)
            && self.limited_life_ticks.fetch_sub(1, Ordering::Relaxed) <= 1
        {
            self.limited_life_ticks
                .store(LIFE_OVER_DAMAGE_INTERVAL, Ordering::Relaxed);
            self.damage(self, 1.0, DamageType::STARVE);
        }
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let difficulty = RegionalDifficulty::at(world, self.get_entity().pos.load());
        self.populate_default_equipment_slots(world, &difficulty);
        self.populate_default_equipment_enchantments(&difficulty);
        self.default_finalize_spawn(world, group_data)
    }

    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        _difficulty: &RegionalDifficulty,
    ) {
        self.mob_entity.set_item_slot(
            &EquipmentSlot::MAIN_HAND,
            ItemStack::new(1, &Item::IRON_SWORD),
        );
        self.mob_entity
            .living_entity
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(EquipmentSlot::MAIN_HAND, 0.0);
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(origin) = self.bound_origin() {
            nbt.put("bound_pos", block_pos_to_tag(origin));
        }
        if let Some(ticks) = self.limited_life_ticks() {
            nbt.put_int("life_ticks", ticks);
        }
        if let Some(owner) = self.owner_uuid() {
            nbt.put_uuid("owner", owner);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_bound_origin(nbt.get("bound_pos").and_then(block_pos_from_tag));
        match nbt.get_int("life_ticks") {
            Some(ticks) => self.set_limited_life(ticks),
            None => self.has_limited_life.store(false, Ordering::Relaxed),
        }
        *self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Owner {
            uuid: nbt.get_uuid("owner"),
            cached: None,
        };
    }
}

/// Vanilla `VexMoveControl`: accelerates straight at the wanted point, through anything.
#[derive(Default)]
struct VexMoveControl {
    inner: MoveControl,
}

impl Control for VexMoveControl {}

impl MoveControlTrait for VexMoveControl {
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.inner.set_wanted_position(x, y, z, speed_modifier);
    }

    fn strafe(&mut self, forward: f32, right: f32) {
        self.inner.strafe(forward, right);
    }

    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }

    fn get_speed_modifier(&self) -> f64 {
        self.inner.get_speed_modifier()
    }

    fn get_wanted_position(&self) -> Vector3<f64> {
        Vector3::new(
            self.inner.wanted_x,
            self.inner.wanted_y,
            self.inner.wanted_z,
        )
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if self.inner.operation != Operation::MoveTo {
            return;
        }
        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        let pos = entity.pos.load();
        let delta = self.get_wanted_position() - pos;
        let distance = delta.length();
        if distance < entity.bounding_box.load().get_average_side_length() {
            self.inner.operation = Operation::Wait;
            entity.velocity.store(entity.velocity.load() * 0.5);
            return;
        }
        let velocity =
            entity.velocity.load() + delta * (self.inner.speed_modifier * 0.05 / distance);
        entity.velocity.store(velocity);
        let (facing_x, facing_z) =
            mob_entity
                .get_target()
                .map_or((velocity.x, velocity.z), |target| {
                    let target = target.get_entity().pos.load();
                    (target.x - pos.x, target.z - pos.z)
                });
        let yaw = -(facing_x.atan2(facing_z) as f32) * 57.295_776;
        entity.yaw.store(yaw);
        entity.body_yaw.store(yaw);
    }
}

/// Vanilla `VexChargeAttackGoal`: dive at the target's eyes and hit on contact.
struct VexChargeAttackGoal;

impl Goal for VexChargeAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(vex) = VexEntity::as_vex(mob) else {
            return false;
        };
        let Some(target) = vex.living_target() else {
            return false;
        };
        !vex.has_wanted()
            && mob.get_random().random_range(0..to_goal_ticks(7)) == 0
            && vex
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&target.get_entity().pos.load())
                > 4.0
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        VexEntity::as_vex(mob).is_some_and(|vex| {
            vex.has_wanted() && vex.is_charging() && vex.living_target().is_some()
        })
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(vex) = VexEntity::as_vex(mob) else {
            return;
        };
        if let Some(target) = vex.mob_entity.get_target() {
            vex.fly_to(target.get_entity().get_eye_pos(), 1.0);
        }
        vex.set_charging(true);
        vex.play_entity_sound(Sound::EntityVexCharge, 1.0, 1.0);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(vex) = VexEntity::as_vex(mob) {
            vex.set_charging(false);
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(vex) = VexEntity::as_vex(mob) else {
            return;
        };
        let Some(target) = vex.mob_entity.get_target() else {
            return;
        };
        let entity = vex.get_entity();
        let target_entity = target.get_entity();
        if entity
            .bounding_box
            .load()
            .intersects(&target_entity.bounding_box.load())
        {
            mob.do_hurt_target(target.as_ref());
            vex.set_charging(false);
        } else if entity
            .pos
            .load()
            .squared_distance_to_vec(&target_entity.pos.load())
            < 9.0
        {
            vex.fly_to(target_entity.get_eye_pos(), 1.0);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `VexRandomMoveGoal`: drift to a free block near the bound position.
struct VexRandomMoveGoal;

impl Goal for VexRandomMoveGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        VexEntity::as_vex(mob).is_some_and(|vex| {
            !vex.has_wanted() && mob.get_random().random_range(0..to_goal_ticks(7)) == 0
        })
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(vex) = VexEntity::as_vex(mob) else {
            return;
        };
        let entity = vex.get_entity();
        let origin = vex
            .bound_origin()
            .unwrap_or_else(|| entity.block_pos.load());
        let world = entity.world.load();
        let mut random = mob.get_random();
        for _ in 0..3 {
            let pos = BlockPos::new(
                origin.0.x + random.random_range(0..15) - 7,
                origin.0.y + random.random_range(0..11) - 5,
                origin.0.z + random.random_range(0..15) - 7,
            );
            if !world.get_block_state(&pos).is_air() {
                continue;
            }
            let center = pos.to_centered_f64();
            vex.fly_to(center, 0.25);
            if vex.mob_entity.get_target().is_none() {
                vex.mob_entity
                    .look_control
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .look_at_with_range(center.x, center.y, center.z, 180.0, 20.0);
            }
            break;
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `VexCopyOwnerTargetGoal`: whatever the summoner fights, the vex fights.
struct VexCopyOwnerTargetGoal {
    track_target_goal: TrackTargetGoal,
    target_predicate: TargetPredicate,
}

impl VexCopyOwnerTargetGoal {
    fn new() -> Self {
        Self {
            track_target_goal: TrackTargetGoal::with_default(false),
            // `forNonCombat().ignoreLineOfSight().ignoreInvisibilityTesting()`
            target_predicate: TargetPredicate::create_non_attackable()
                .ignore_visibility()
                .ignore_distance_scaling_factor(),
        }
    }

    fn owner_target(mob: &dyn Mob) -> Option<Arc<dyn EntityBase>> {
        VexEntity::as_vex(mob)?
            .get_owner()?
            .get_mob()?
            .get_mob_entity()
            .get_target()
    }
}

impl Goal for VexCopyOwnerTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        Self::owner_target(mob).is_some_and(|target| {
            self.track_target_goal
                .can_track(mob, Some(target.as_ref()), &self.target_predicate)
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.track_target_goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.track_target_goal.start_targeting(
            mob,
            Self::owner_target(mob),
            TargetReason::OwnerAttackedTarget,
        );
        self.track_target_goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.track_target_goal.stop(mob);
    }

    fn controls(&self) -> Controls {
        self.track_target_goal.controls()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::ai::goal::goal_selector::GoalSelector;
    use crate::entity::ai::test_support::world;
    use crate::entity::mob::evoker::EvokerEntity;
    use crate::entity::r#type::from_type;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn vex_at(world: &Arc<World>, pos: Vector3<f64>) -> Arc<VexEntity> {
        VexEntity::new(Entity::new(world.clone(), pos, &EntityType::VEX))
    }

    fn vex(world: &Arc<World>) -> Arc<VexEntity> {
        vex_at(world, Vector3::new(0.5, 64.0, 0.5))
    }

    #[tokio::test]
    async fn bound_position_life_and_owner_are_saved() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let owner = Uuid::new_v4();
        let summoned = vex(&world);
        summoned.set_bound_origin(Some(BlockPos::new(3, 70, -4)));
        summoned.set_limited_life(777);
        summoned
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .uuid = Some(owner);
        let mut saved = NbtCompound::new();
        summoned.mob_write_nbt(&mut saved);

        let loaded = vex(&world);
        loaded.mob_read_nbt(&saved);
        let restored = (
            loaded.bound_origin(),
            loaded.limited_life_ticks(),
            loaded.owner_uuid(),
        );
        // A wild vex saves none of it, and loading that clears a limited life.
        let mut wild = NbtCompound::new();
        vex(&world).mob_write_nbt(&mut wild);
        loaded.mob_read_nbt(&wild);
        let cleared = (
            loaded.bound_origin(),
            loaded.limited_life_ticks(),
            loaded.owner_uuid(),
        );
        world.level.shutdown().await;

        assert_eq!(
            restored,
            (Some(BlockPos::new(3, 70, -4)), Some(777), Some(owner))
        );
        assert!(wild.child_tags.is_empty());
        assert_eq!(cleared, (None, None, None));
        Ok(())
    }

    #[tokio::test]
    async fn summoned_life_runs_out_into_starvation_every_second() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let summoned = vex(&world);
        let living = &summoned.mob_entity.living_entity;
        // The fixture has no server to finish a hit, so the recorded hit is checked.
        let hit = || living.last_damage_taken.load();
        summoned.set_limited_life(2);
        summoned.mob_tick(&*summoned);
        let flying_through = summoned.get_entity().no_physics.load(Ordering::Relaxed);
        summoned.post_tick();
        let after_first = (hit(), summoned.limited_life_ticks());
        summoned.post_tick();
        let after_second = (hit(), summoned.limited_life_ticks());
        let entity = summoned.get_entity();
        let settled = (
            entity.no_physics.load(Ordering::Relaxed),
            entity.has_no_gravity(),
        );
        world.level.shutdown().await;

        assert!(flying_through);
        assert_eq!(after_first, (0.0, Some(1)));
        assert_eq!(after_second, (1.0, Some(20)));
        assert_eq!(settled, (false, true));
        Ok(())
    }

    #[tokio::test]
    async fn move_control_accelerates_at_the_point_and_brakes_on_arrival() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let flyer = vex(&world);
        let entity = flyer.get_entity();
        flyer.fly_to(Vector3::new(0.5, 64.0, 10.5), 1.0);
        flyer.move_control().tick(&*flyer);
        let pushed = entity.velocity.load();
        let yaw = entity.yaw.load();
        let still_wanted = flyer.has_wanted();

        // Closer than its own size: stop wanting and halve the speed.
        flyer.fly_to(Vector3::new(0.5, 64.0, 0.6), 1.0);
        flyer.move_control().tick(&*flyer);
        let braked = entity.velocity.load();
        let arrived = !flyer.has_wanted();
        world.level.shutdown().await;

        assert!((pushed.z - 0.05).abs() < 1e-9, "{pushed:?}");
        assert!(pushed.x.abs() < 1e-9 && pushed.y.abs() < 1e-9);
        assert!(yaw.abs() < 1e-3, "south is yaw 0, got {yaw}");
        assert!(still_wanted);
        assert!((braked.z - 0.025).abs() < 1e-9, "{braked:?}");
        assert!(arrived);
        Ok(())
    }

    #[tokio::test]
    async fn vex_takes_its_summoners_target_and_evoker_sides_with_it() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let spawn = |entity_type, x: f64| {
            let entity = from_type(
                entity_type,
                Vector3::new(x, 64.0, 0.5),
                &world,
                Uuid::new_v4(),
            );
            world.spawn_entity(entity.clone());
            entity
        };
        let evoker = spawn(&EntityType::EVOKER, 2.5);
        let stranger = spawn(&EntityType::EVOKER, 4.5);
        let cow = spawn(&EntityType::COW, 6.5);
        let minion = vex(&world);
        let target_of = |minion: &Arc<VexEntity>| {
            for _ in 0..4 {
                GoalSelector::tick(&minion.mob_entity.target_selector, &**minion);
            }
            minion
                .mob_entity
                .get_target()
                .map(|target| target.get_entity().entity_id)
        };
        let evoker_mob = evoker.cast_any().downcast_ref::<EvokerEntity>();
        let ownerless = (
            target_of(&minion),
            evoker.is_allied_to(&*minion as &dyn EntityBase),
        );
        minion.set_owner(&evoker);
        let idle_owner = target_of(&minion);
        if let Some(evoker_mob) = evoker_mob {
            evoker_mob.mob_entity.set_target(Some(cow.clone()));
        }
        let copied = target_of(&minion);
        let owned = (
            evoker.is_allied_to(&*minion as &dyn EntityBase),
            stranger.is_allied_to(&*minion as &dyn EntityBase),
        );
        world.level.shutdown().await;

        assert_eq!(ownerless, (None, false));
        assert_eq!(idle_owner, None);
        assert_eq!(copied, Some(cow.get_entity().entity_id));
        // Any illager without a team sides with an illager's vex.
        assert_eq!(owned, (true, true));
        Ok(())
    }

    #[tokio::test]
    async fn charge_hits_on_contact_and_re_aims_when_close() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let spawn_cow = |pos| {
            let cow = from_type(&EntityType::COW, pos, &world, Uuid::new_v4());
            world.spawn_entity(cow.clone());
            cow
        };
        let hit = |cow: &Arc<dyn EntityBase>| {
            cow.get_living_entity()
                .map_or(0.0, |living| living.last_damage_taken.load())
        };
        let mut goal = VexChargeAttackGoal;

        // Two blocks off: no contact, the dive is aimed at the eyes again.
        let near = spawn_cow(Vector3::new(2.5, 64.0, 0.5));
        let diver = vex(&world);
        diver.mob_entity.set_target(Some(near.clone()));
        diver.set_charging(true);
        goal.tick(&*diver);
        let aimed = diver.move_control().get_wanted_position();
        let near_hit = (hit(&near), diver.is_charging());

        // Inside the target: it is hit and the charge is over.
        let touching = spawn_cow(Vector3::new(0.5, 64.0, 0.5));
        diver.mob_entity.set_target(Some(touching.clone()));
        goal.tick(&*diver);
        let contact = (hit(&touching) > 0.0, diver.is_charging());
        world.level.shutdown().await;

        assert_eq!(aimed, near.get_entity().get_eye_pos());
        assert_eq!(near_hit, (0.0, true));
        assert_eq!(contact, (true, false));
        Ok(())
    }

    #[tokio::test]
    async fn spawned_vex_holds_an_iron_sword_it_never_drops() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let armed = vex(&world);
        armed.finalize_spawn(&world, EntitySpawnReason::MobSummoned, None);
        let living = &armed.mob_entity.living_entity;
        let held = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
            .item;
        let drop_chance = living
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
            .copied();
        let xp = armed.xp_reward();
        world.level.shutdown().await;

        assert_eq!(held, &Item::IRON_SWORD);
        assert_eq!(drop_chance, Some(0.0));
        assert_eq!(xp, 3);
        Ok(())
    }
}
