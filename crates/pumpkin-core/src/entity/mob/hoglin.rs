use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use pumpkin_data::BlockState;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};

use crate::entity::ai::brain::memory::types;
use crate::entity::living::LivingEntity;
use crate::entity::mob::hoglin_ai;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::passive::animal::Animal;
use crate::entity::player::Player;
use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    mob::{Mob, MobEntity},
};
use crate::world::World;

pub struct HoglinEntity {
    pub mob_entity: MobEntity,
    pub immune_to_zombification: AtomicBool,
    pub time_in_overworld: AtomicI32,
    pub cannot_be_hunted: AtomicBool,
    pub ageable_data: AgeableData,
    /// The brain's `NEAREST_REPELLENT` and `PACIFIED` as of its last tick, for callers that
    /// run while the brain is locked or on another thread.
    nearest_repellent: AtomicCell<Option<BlockPos>>,
    pacified: AtomicBool,
}

impl HoglinEntity {
    pub const CONVERSION_TIME: i32 = 300;
    pub const XP_REWARD: u32 = 5;
    pub const BABY_XP_REWARD: u32 = 3;

    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        {
            let mut attributes = mob_entity
                .living_entity
                .attributes
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(health) = attributes.get_mut(&Attributes::MAX_HEALTH.id) {
                health.base_value = 40.0;
                health.dirty.store(true, Ordering::Relaxed);
            }
            if let Some(speed) = attributes.get_mut(&Attributes::MOVEMENT_SPEED.id) {
                speed.base_value = 0.3;
                speed.dirty.store(true, Ordering::Relaxed);
            }
            if let Some(knockback_res) = attributes.get_mut(&Attributes::KNOCKBACK_RESISTANCE.id) {
                knockback_res.base_value = 0.6;
                knockback_res.dirty.store(true, Ordering::Relaxed);
            }
            if let Some(attack_kb) = attributes.get_mut(&Attributes::ATTACK_KNOCKBACK.id) {
                attack_kb.base_value = 1.0;
                attack_kb.dirty.store(true, Ordering::Relaxed);
            }
            if let Some(damage) = attributes.get_mut(&Attributes::ATTACK_DAMAGE.id) {
                damage.base_value = 6.0;
                damage.dirty.store(true, Ordering::Relaxed);
            }
        }
        mob_entity.living_entity.health.store(40.0);

        let hoglin = Self {
            mob_entity,
            immune_to_zombification: AtomicBool::new(false),
            time_in_overworld: AtomicI32::new(0),
            cannot_be_hunted: AtomicBool::new(false),
            ageable_data: AgeableData::default(),
            nearest_repellent: AtomicCell::new(None),
            pacified: AtomicBool::new(false),
        };
        Arc::new(hoglin)
    }

    pub fn can_be_hunted(&self) -> bool {
        !self.mob_entity.living_entity.is_baby() && !self.cannot_be_hunted.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_immune_to_zombification(&self) -> bool {
        self.immune_to_zombification.load(Ordering::Relaxed)
    }

    pub fn set_immune_to_zombification(&self, immune: bool) {
        self.immune_to_zombification
            .store(immune, Ordering::Relaxed);
        self.mob_entity.living_entity.entity.set_synced_data(
            pumpkin_data::tracked_data::hoglin::DATA_IMMUNE_TO_ZOMBIFICATION,
            immune,
        );
    }

    pub fn is_converting(&self, world: &World) -> bool {
        !self.is_immune_to_zombification()
            && !self.mob_entity.is_no_ai()
            && world.dimension.piglins_zombify
    }

    fn convert_to_zoglin(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load();
        let pos = entity.pos.load();

        world.play_sound(
            Sound::EntityHoglinConvertedToZombified,
            SoundCategory::Hostile,
            &pos,
        );

        let zoglin = crate::entity::r#type::from_type(
            &EntityType::ZOGLIN,
            pos,
            &world,
            uuid::Uuid::new_v4(),
        );

        let zoglin_base = zoglin.get_entity();
        zoglin_base.set_rotation(entity.yaw.load(), entity.pitch.load());
        zoglin_base.head_yaw.store(entity.head_yaw.load());
        zoglin_base.velocity.store(entity.velocity.load());

        if let Some(living) = zoglin.get_living_entity() {
            living.set_health(self.mob_entity.living_entity.health.load());
        }

        if let Some(custom_name) = &**entity.custom_name.load() {
            zoglin_base.set_custom_name(custom_name.clone());
        }

        // Vanilla ConversionType.SINGLE keeps the baby state.
        if self.is_baby()
            && let Some(mob) = zoglin.get_mob()
        {
            mob.spawn_as_baby();
        }

        world.spawn_entity(zoglin);
        entity.remove();
    }
}

impl AgeableMob for HoglinEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    fn age_boundary_reached(&self) {
        let damage = if self.is_baby() { 0.5 } else { 6.0 };
        self.mob_entity
            .living_entity
            .set_attribute_base(&Attributes::ATTACK_DAMAGE, damage);
    }
}

impl Animal for HoglinEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_HOGLIN_FOOD)
    }

    /// A bred hoglin never despawns.
    fn get_breed_offspring(&self, _partner: &dyn EntityBase) -> Arc<dyn EntityBase> {
        let entity = self.get_entity();
        let baby = crate::entity::r#type::from_type(
            entity.entity_type,
            entity.pos.load(),
            &entity.world.load(),
            uuid::Uuid::new_v4(),
        );
        if let Some(mob) = baby.get_mob() {
            mob.get_mob_entity().set_persistence_required();
        }
        baby
    }

    /// A hoglin that smelled warped fungus is in no mood.
    fn can_fall_in_love(&self) -> bool {
        !self.pacified.load(Ordering::Relaxed) && !self.mob_entity.is_in_love()
    }
}

impl Mob for HoglinEntity {
    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    /// Unlike other animals, hoglins despawn like monsters.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        true
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let consumed = self.animal_interact(player, item_stack);
        if consumed {
            self.mob_entity.set_persistence_required();
        }
        consumed
    }

    fn make_brain(
        &self,
        packed: &crate::entity::ai::brain::memory::PackedMemories,
    ) -> crate::entity::ai::brain::Brain {
        hoglin_ai::HOGLIN_PROVIDER.make_brain(self, packed)
    }

    fn after_brain_tick(&self, tick: &mut crate::entity::ai::brain::BrainTick<'_>) {
        hoglin_ai::update_activity(tick);
        self.nearest_repellent
            .store(tick.state.get(types::NEAREST_REPELLENT).copied());
        self.pacified
            .store(hoglin_ai::is_pacified(tick.state), Ordering::Relaxed);
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !crate::entity::ai::brain::behavior::utils::is_alive(self) {
            // No AI while dying, but still drain the inbox
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
        let world = self.get_entity().world.load();
        if self.is_converting(&world) {
            let time = self.time_in_overworld.fetch_add(1, Ordering::Relaxed) + 1;
            if time > Self::CONVERSION_TIME {
                self.convert_to_zoglin();
            }
        } else {
            self.time_in_overworld.store(0, Ordering::Relaxed);
        }
    }

    /// Vanilla `Hoglin.doHurtTarget`.
    fn do_hurt_target(&self, target: &dyn EntityBase) -> bool {
        if target.get_living_entity().is_none() {
            return false;
        }
        let entity = self.get_entity();
        let world = entity.world.load();
        world.send_entity_status(
            entity,
            EntityStatus::StartAttacking,
            pumpkin_protocol::bedrock::server::ActorEventID::None,
        );
        self.make_sound(Sound::EntityHoglinAttack);
        // The brain is locked while it attacks, so the aftermath waits in its inbox.
        if let Some(target) = world
            .get_entity_by_id(target.get_entity().entity_id)
            .or_else(|| {
                world
                    .get_player_by_id(target.get_entity().entity_id)
                    .map(|player| player as Arc<dyn EntityBase>)
            })
        {
            self.mob_entity.post_to_brain(Box::new(move |tick| {
                hoglin_ai::on_hit_target(tick, &target);
            }));
        }
        hurt_and_throw_target(
            &self.mob_entity.living_entity,
            self,
            target,
            !self.is_baby(),
        )
    }

    fn on_damage(&self, _damage_type: DamageType, source: Option<&dyn EntityBase>) {
        let Some(attacker) = source else {
            return;
        };
        let world = self.get_entity().world.load_full();
        let id = attacker.get_entity().entity_id;
        let Some(attacker) = world
            .get_entity_by_id(id)
            .or_else(|| {
                world
                    .get_player_by_id(id)
                    .map(|player| player as Arc<dyn EntityBase>)
            })
            .filter(|attacker| attacker.get_living_entity().is_some())
        else {
            return;
        };
        self.mob_entity.post_to_brain(Box::new(move |tick| {
            hoglin_ai::was_hurt_by(tick, &attacker);
        }));
    }

    /// Hoglins keep clear of warped fungus and like crimson nylium underfoot.
    fn get_walk_target_value(&self, pos: &BlockPos) -> f32 {
        if hoglin_ai::is_pos_near_repellent(self.nearest_repellent.load(), pos) {
            -1.0
        } else if self.get_entity().world.load().get_block(&pos.down()) == &Block::CRIMSON_NYLIUM {
            10.0
        } else {
            0.0
        }
    }

    /// Vanilla `getAmbientSound`: what it is doing decides how it sounds.
    fn get_ambient_sound(&self) -> Option<Sound> {
        self.mob_entity.with_brain(self, |brain| {
            hoglin_ai::sound_for_current_activity(self, brain)
        })
    }

    fn mob_play_step_sound(&self, _pos: &BlockPos, _state: &BlockState) {
        self.play_entity_sound(Sound::EntityHoglinStep, 0.15, 1.0);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        if self.get_random().random::<f32>() < 0.2 {
            self.set_baby(true);
        }
        self.default_finalize_spawn(world, group_data)
    }

    fn should_drop_experience(&self) -> bool {
        true
    }

    fn get_base_experience_reward(&self) -> u32 {
        if self.is_baby() {
            Self::BABY_XP_REWARD
        } else {
            Self::XP_REWARD
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        if self.is_immune_to_zombification() {
            nbt.put_bool("IsImmuneToZombification", true);
        }
        let time = self.time_in_overworld.load(Ordering::Relaxed);
        if time > 0 {
            nbt.put_int("TimeInOverworld", time);
        }
        if self.cannot_be_hunted.load(Ordering::Relaxed) {
            nbt.put_bool("CannotBeHunted", true);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(immune) = nbt.get_bool("IsImmuneToZombification") {
            self.set_immune_to_zombification(immune);
        }
        if let Some(time) = nbt.get_int("TimeInOverworld") {
            self.time_in_overworld.store(time, Ordering::Relaxed);
        }
        if let Some(cannot_hunt) = nbt.get_bool("CannotBeHunted") {
            self.cannot_be_hunted.store(cannot_hunt, Ordering::Relaxed);
        }
    }
}

/// Vanilla `HoglinBase.hurtAndThrowTarget`, shared with zoglins: an adult hits for half
/// its attack damage plus a random part, and throws what it hit.
pub fn hurt_and_throw_target(
    attacker: &LivingEntity,
    caller: &dyn EntityBase,
    target: &dyn EntityBase,
    adult: bool,
) -> bool {
    let damage = attacker.get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
    let dealt = if adult && damage as i32 > 0 {
        damage / 2.0 + rand::rng().random_range(0..damage as i32) as f32
    } else {
        damage
    };
    let hurt = target.damage_with_context(
        target,
        dealt,
        DamageType::MOB_ATTACK,
        None,
        Some(caller),
        Some(caller),
    );
    if hurt && adult {
        throw_target(attacker, target);
    }
    hurt
}

/// Vanilla `HoglinBase.throwTarget`: attack knockback less the target's resistance, in a
/// random direction and upwards.
pub fn throw_target(attacker: &LivingEntity, target: &dyn EntityBase) {
    let resistance = target.get_living_entity().map_or(0.0, |living| {
        living.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE)
    });
    let knockback = attacker.get_attribute_value(&Attributes::ATTACK_KNOCKBACK) - resistance;
    if knockback <= 0.0 {
        return;
    }
    let mut rng = rand::rng();
    let from = attacker.entity.pos.load();
    let to = target.get_entity().pos.load();
    // Vanilla turns the direction by -10..=10, read as radians.
    let angle = (rng.random_range(0..21) - 10) as f32;
    let horizontal = knockback * f64::from(rng.random::<f32>() * 0.5 + 0.2);
    let direction = Vector3::new(to.x - from.x, 0.0, to.z - from.z).normalize() * horizontal;
    let (sin, cos) = (f64::from(angle.sin()), f64::from(angle.cos()));
    let vertical = knockback * f64::from(rng.random::<f32>()) * 0.5;
    target.get_entity().add_velocity(Vector3::new(
        direction.x * cos + direction.z * sin,
        vertical,
        direction.z * cos - direction.x * sin,
    ));
}

#[cfg(test)]
mod tests {
    use pumpkin_data::environment_attribute::Activity;

    use super::*;
    use crate::entity::ai::test_support::world;
    use crate::entity::r#type::from_type;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn hoglin(world: &Arc<World>) -> Arc<HoglinEntity> {
        let hoglin = HoglinEntity::new(Entity::new(
            world.clone(),
            Vector3::new(0.5, 64.0, 0.5),
            &EntityType::HOGLIN,
        ));
        world.spawn_entity(hoglin.clone());
        hoglin
    }

    fn spawn(world: &Arc<World>, entity_type: &'static EntityType, x: f64) -> Arc<dyn EntityBase> {
        let entity = from_type(
            entity_type,
            Vector3::new(x, 64.0, 0.5),
            world,
            uuid::Uuid::new_v4(),
        );
        world.spawn_entity(entity.clone());
        entity
    }

    fn think(hoglin: &Arc<HoglinEntity>, ticks: usize) {
        for _ in 0..ticks {
            hoglin.custom_server_ai_step(&**hoglin);
        }
    }

    fn memory_id(
        hoglin: &Arc<HoglinEntity>,
        memory: crate::entity::ai::brain::memory::MemoryModuleType<Arc<dyn EntityBase>>,
    ) -> Option<i32> {
        hoglin.mob_entity.with_brain(&**hoglin, |brain| {
            brain
                .get(memory)
                .map(|entity| entity.get_entity().entity_id)
        })
    }

    fn activity(hoglin: &Arc<HoglinEntity>) -> Option<Activity> {
        hoglin
            .mob_entity
            .with_brain(&**hoglin, |brain| brain.get_active_non_core_activity())
    }

    #[tokio::test]
    async fn adult_fights_back_and_baby_runs() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let cow = spawn(&world, &EntityType::COW, 3.5);
        let adult = hoglin(&world);
        let baby = hoglin(&world);
        baby.set_baby(true);
        think(&adult, 2);
        think(&baby, 2);
        let calm = (activity(&adult), activity(&baby));

        adult.on_damage(DamageType::MOB_ATTACK, Some(cow.as_ref()));
        baby.on_damage(DamageType::MOB_ATTACK, Some(cow.as_ref()));
        think(&adult, 2);
        think(&baby, 2);
        let cow_id = Some(cow.get_entity().entity_id);
        let provoked = (
            activity(&adult),
            memory_id(&adult, types::ATTACK_TARGET),
            activity(&baby),
            memory_id(&baby, types::AVOID_TARGET),
        );
        world.level.shutdown().await;

        assert_eq!(calm, (Some(Activity::Idle), Some(Activity::Idle)));
        assert_eq!(
            provoked,
            (Some(Activity::Fight), cow_id, Some(Activity::Avoid), cow_id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn outnumbered_hoglin_retreats_from_the_piglin_it_hit() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let piglins = [
            spawn(&world, &EntityType::PIGLIN, 3.5),
            spawn(&world, &EntityType::PIGLIN, 4.5),
            spawn(&world, &EntityType::PIGLIN, 5.5),
        ];
        let lone = hoglin(&world);
        // Long enough for every sensor to have looked around once.
        think(&lone, 45);
        let seen = lone.mob_entity.with_brain(&*lone, |brain| {
            brain.get(types::VISIBLE_ADULT_PIGLIN_COUNT).copied()
        });
        let hit = piglins[0].clone();
        lone.mob_entity.post_to_brain(Box::new(move |tick| {
            hoglin_ai::on_hit_target(tick, &hit);
        }));
        think(&lone, 2);
        let after = (activity(&lone), memory_id(&lone, types::AVOID_TARGET));
        world.level.shutdown().await;

        assert_eq!(seen, Some(3));
        assert_eq!(
            after,
            (
                Some(Activity::Avoid),
                Some(piglins[0].get_entity().entity_id)
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn adult_hits_for_half_plus_a_roll_and_throws() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let attacker = hoglin(&world);
        let living = &attacker.mob_entity.living_entity;
        let mut dealt = Vec::new();
        let mut thrown = Vec::new();
        for _ in 0..60 {
            let cow = spawn(&world, &EntityType::COW, 2.5);
            let hurt = hurt_and_throw_target(living, &*attacker, cow.as_ref(), true);
            // The fixture has no server to finish a hit, so the recorded amount is read.
            assert!(!hurt);
            if let Some(cow_living) = cow.get_living_entity() {
                dealt.push(cow_living.last_damage_taken.load());
            }
            throw_target(living, cow.as_ref());
            thrown.push(cow.get_entity().velocity.load());
        }
        let baby_cow = spawn(&world, &EntityType::COW, 2.5);
        living.set_attribute_base(&Attributes::ATTACK_DAMAGE, 0.5);
        hurt_and_throw_target(living, &*attacker, baby_cow.as_ref(), false);
        let baby_dealt = baby_cow
            .get_living_entity()
            .map(|cow| cow.last_damage_taken.load());
        world.level.shutdown().await;

        // 6 attack damage: 3 plus 0..=5.
        assert!(
            dealt
                .iter()
                .all(|damage| (3.0..=8.0).contains(damage) && damage.fract() == 0.0)
        );
        assert!(dealt.iter().any(|damage| *damage != dealt[0]));
        // Knockback 1 against a cow with no resistance: 0.2..0.7 sideways, up to 0.5 up.
        for velocity in &thrown {
            let sideways = velocity.x.hypot(velocity.z);
            assert!(
                (0.2 - 1e-6..=0.7 + 1e-6).contains(&sideways),
                "{velocity:?}"
            );
            assert!((0.0..=0.5).contains(&velocity.y), "{velocity:?}");
        }
        assert_eq!(baby_dealt, Some(0.5));
        Ok(())
    }

    #[tokio::test]
    async fn fed_hoglin_is_an_animal_that_never_despawns() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let parent = hoglin(&world);
        let fungus = ItemStack::new(1, &pumpkin_data::item::Item::CRIMSON_FUNGUS);
        let warped = ItemStack::new(1, &pumpkin_data::item::Item::WARPED_FUNGUS);
        let food = (parent.is_food(&fungus), parent.is_food(&warped));
        let child = parent.get_breed_offspring(&*parent);
        let child_stays = child.get_mob().is_some_and(|mob| {
            mob.get_mob_entity()
                .persistence_required
                .load(Ordering::Relaxed)
        });
        let wild_despawns = parent.remove_when_far_away(40_000.0);
        parent.pacified.store(true, Ordering::Relaxed);
        let pacified_loves = parent.can_fall_in_love();
        world.level.shutdown().await;

        assert_eq!(food, (true, false));
        assert!(child_stays);
        assert!(wild_despawns);
        assert!(!pacified_loves);
        Ok(())
    }

    #[test]
    fn positions_within_eight_blocks_of_the_repellent_are_shunned() {
        let fungus = Some(BlockPos::new(0, 64, 0));
        assert!(hoglin_ai::is_pos_near_repellent(
            fungus,
            &BlockPos::new(7, 64, 0)
        ));
        assert!(!hoglin_ai::is_pos_near_repellent(
            fungus,
            &BlockPos::new(8, 64, 0)
        ));
        assert!(!hoglin_ai::is_pos_near_repellent(
            None,
            &BlockPos::new(0, 64, 0)
        ));
    }
}
