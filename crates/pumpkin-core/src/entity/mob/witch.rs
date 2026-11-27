use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use rand::RngExt;

use crate::entity::attributes::{Modifier, ModifierOperation, send_attribute_updates_for_living};
use crate::entity::living::LivingEntity;
use crate::world::World;

use crate::entity::ai::target_match::TargetMatch;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        Controls, Goal, GoalReach, active_target::ActiveTargetGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
        ranged_attack::RangedAttackGoal, revenge::RevengeGoal, swim::SwimGoal, to_goal_ticks,
        wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity, RangedAttackMob,
        patrol::{PatrolData, PatrollingMonster},
        raider::{
            PathfindToRaidGoal, Raider, RaiderCelebrationGoal, RaiderData,
            RaiderMoveThroughVillageGoal, is_raider,
        },
    },
    projectile::splash_potion::SplashPotionEntity,
};

fn create_potion_stack(item: &'static Item, potion: &'static Potion) -> ItemStack {
    use pumpkin_data::data_component::DataComponent;
    use pumpkin_data::data_component_impl::{DataComponentImpl, PotionContentsImpl};
    let mut stack = ItemStack::new(1, item);
    stack.patch.push((
        DataComponent::PotionContents,
        Some(
            PotionContentsImpl {
                potion_id: Some(i32::from(potion.id)),
                custom_color: None,
                custom_effects: Vec::new(),
                custom_name: None,
            }
            .to_dyn(),
        ),
    ));
    stack
}

/// Represents a Witch, a hostile ranged mob that throws splash potions and drinks restorative potions.
///
/// Wiki: <https://minecraft.wiki/w/Witch>
pub struct WitchEntity {
    pub mob_entity: MobEntity,
    pub raider_data: RaiderData,
    drinking_potion: AtomicBool,
    using_time: AtomicI32,
    /// Vanilla `healRaidersGoal.cooldown`: counts down every tick, and while it is above
    /// zero the witch does not pick players.
    heal_cooldown: AtomicI32,
}

/// Vanilla `SPEED_MODIFIER_DRINKING`: a witch stands still while it drinks.
const SPEED_MODIFIER_DRINKING_ID: &str = "minecraft:drinking";
const SPEED_MODIFIER_DRINKING: f64 = -0.25;
/// Vanilla `NearestHealableRaiderTargetGoal.DEFAULT_COOLDOWN`.
const HEAL_COOLDOWN: i32 = 200;
/// Use duration of a drinkable potion.
const POTION_USE_DURATION: i32 = 32;

/// Vanilla `NearestHealableRaiderTargetGoal`: during a raid, every other try once the
/// cooldown is over, the witch turns to the nearest raider that is no witch and throws
/// it a potion.
struct HealRaidersGoal {
    search: ActiveTargetGoal,
}

impl Goal for HealRaidersGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(witch) = mob.cast_any().downcast_ref::<WitchEntity>() else {
            return false;
        };
        if witch.heal_cooldown.load(Ordering::Relaxed) > 0 || !mob.get_random().random::<bool>() {
            return false;
        }
        witch.has_active_raid() && self.search.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.search.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(witch) = mob.cast_any().downcast_ref::<WitchEntity>() {
            witch
                .heal_cooldown
                .store(to_goal_ticks(HEAL_COOLDOWN), Ordering::Relaxed);
        }
        self.search.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.search.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.search.tick(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        self.search.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.search.controls()
    }

    fn tick_reach(&self, mob: &dyn Mob, running: bool) -> Option<GoalReach> {
        self.search.tick_reach(mob, running)
    }
}

impl WitchEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let witch = Self {
            mob_entity,
            raider_data: RaiderData::default(),
            drinking_potion: AtomicBool::new(false),
            using_time: AtomicI32::new(0),
            heal_cooldown: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(witch);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };
        let ranged_weak: Weak<dyn RangedAttackMob> = {
            let ranged_arc: Arc<dyn RangedAttackMob> = mob_arc.clone();
            Arc::downgrade(&ranged_arc)
        };

        {
            let goal_selector = &mob_arc.mob_entity.goals_selector;
            let target_selector = &mob_arc.mob_entity.target_selector;

            goal_selector.add_goal(1, Box::new(SwimGoal::default()));
            goal_selector.add_goal(
                2,
                Box::new(RangedAttackGoal::new(ranged_weak, 1.0, 60, 10.0)),
            );
            goal_selector.add_goal(2, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(3, Box::new(RaiderMoveThroughVillageGoal::new(1.05)));
            goal_selector.add_goal(3, Box::new(PathfindToRaidGoal::default()));
            goal_selector.add_goal(
                3,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(3, Box::new(RandomLookAroundGoal::default()));
            goal_selector.add_goal(4, Box::new(RaiderCelebrationGoal));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true).ignoring(is_raider)));
            target_selector.add_goal(
                2,
                Box::new(HealRaidersGoal {
                    search: ActiveTargetGoal::new(
                        &mob_arc.mob_entity,
                        TargetMatch::Tag(&tag::EntityType::MINECRAFT_RAIDERS),
                        0,
                        true,
                        false,
                        Some(|target: &LivingEntity, _: &World| {
                            target.entity.entity_type != &EntityType::WITCH
                        }),
                    ),
                }),
            );
            // Vanilla `NearestAttackableWitchTargetGoal`: players, but not while the heal
            // cooldown runs.
            target_selector.add_goal(
                3,
                Box::new(ActiveTargetGoal::new(
                    &mob_arc.mob_entity,
                    TargetMatch::Type(&EntityType::PLAYER),
                    10,
                    true,
                    false,
                    None::<fn(&LivingEntity, &World) -> bool>,
                ))
                .every(10)
                .gated_by(|mob| {
                    mob.cast_any()
                        .downcast_ref::<Self>()
                        .is_some_and(Self::can_attack_players)
                }),
            );
        };

        mob_arc
    }

    pub fn set_drinking_potion(&self, drinking: bool) {
        self.drinking_potion.store(drinking, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(tracked_data::witch::DATA_USING_ITEM, drinking);
    }

    #[must_use]
    pub fn is_drinking_potion(&self) -> bool {
        self.drinking_potion.load(Ordering::Relaxed)
    }

    /// Vanilla `attackPlayersGoal.setCanAttack(healRaidersGoal.getCooldown() <= 0)`.
    #[must_use]
    pub fn can_attack_players(&self) -> bool {
        self.heal_cooldown.load(Ordering::Relaxed) <= 0
    }

    /// The splash potion vanilla `performRangedAttack` picks for a target `dist` away.
    fn potion_for(&self, target: &dyn EntityBase, dist: f64) -> &'static Potion {
        let Some(living) = target.get_living_entity() else {
            return &Potion::HARMING;
        };
        if is_raider(target.get_entity().entity_type) {
            return if living.health.load() <= 4.0 {
                &Potion::HEALING
            } else {
                &Potion::REGENERATION
            };
        }
        if dist >= 8.0 && !living.has_effect(&StatusEffect::SLOWNESS) {
            &Potion::SLOWNESS
        } else if living.health.load() >= 8.0 && !living.has_effect(&StatusEffect::POISON) {
            &Potion::POISON
        } else if dist <= 3.0
            && !living.has_effect(&StatusEffect::WEAKNESS)
            && self.get_random().random::<f32>() < 0.25
        {
            &Potion::WEAKNESS
        } else {
            &Potion::HARMING
        }
    }

    fn hold(&self, stack: ItemStack) {
        let living = &self.mob_entity.living_entity;
        living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(&EquipmentSlot::MAIN_HAND, stack.clone());
        living.send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, stack)]);
    }

    fn set_drinking_slowdown(&self, slowed: bool) {
        let living = &self.mob_entity.living_entity;
        living.update_attribute(&Attributes::MOVEMENT_SPEED, |speed| {
            speed.remove_modifier(SPEED_MODIFIER_DRINKING_ID);
            if slowed {
                speed.add_or_replace_modifier(Modifier {
                    id: SPEED_MODIFIER_DRINKING_ID.to_string(),
                    amount: SPEED_MODIFIER_DRINKING,
                    operation: ModifierOperation::Add,
                });
            }
        });
        send_attribute_updates_for_living(living, vec![Attributes::MOVEMENT_SPEED]);
    }

    fn start_drinking(&self, potion: &'static Potion) {
        let entity = self.get_entity();
        self.hold(create_potion_stack(&Item::POTION, potion));
        self.using_time
            .store(POTION_USE_DURATION, Ordering::Relaxed);
        self.set_drinking_potion(true);
        if !entity.silent.load(Ordering::Relaxed) {
            entity.world.load().play_sound_fine(
                Sound::EntityWitchDrink,
                SoundCategory::Hostile,
                &entity.pos.load(),
                1.0,
                rand::random::<f32>().mul_add(0.4, 0.8),
            );
        }
        self.set_drinking_slowdown(true);
    }

    fn finish_drinking(&self) {
        let living = &self.mob_entity.living_entity;
        self.set_drinking_potion(false);
        let stack = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND);
        self.hold(ItemStack::EMPTY.clone());
        if stack.item == &Item::POTION {
            let effects = crate::item::potion::PotionContents::read_potion_effects(&stack);
            crate::item::potion::PotionContents::apply_effects_to(
                living,
                effects,
                1.0,
                crate::item::potion::PotionApplicationSource::Normal,
                None,
                None,
            );
        }
        let entity = self.get_entity();
        entity
            .world
            .load()
            .emit_game_event(GameEvent::Drink.name(), entity.pos.load());
        self.set_drinking_slowdown(false);
    }

    /// The potion vanilla `aiStep` reaches for, each condition on its own roll.
    fn potion_to_drink(&self) -> Option<&'static Potion> {
        let entity = self.get_entity();
        let living = &self.mob_entity.living_entity;
        let roll = || rand::random::<f32>();
        if roll() < 0.15
            && entity.is_submerged_in_water()
            && !living.has_effect(&StatusEffect::WATER_BREATHING)
        {
            return Some(&Potion::WATER_BREATHING);
        }
        if roll() < 0.15
            && (entity.is_on_fire()
                || living
                    .get_last_damage_type()
                    .is_some_and(|kind| kind.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE)))
            && !living.has_effect(&StatusEffect::FIRE_RESISTANCE)
        {
            return Some(&Potion::FIRE_RESISTANCE);
        }
        if roll() < 0.05 && living.health.load() < living.get_max_health() {
            return Some(&Potion::HEALING);
        }
        if roll() < 0.5
            && let Some(target) = self.mob_entity.get_target()
            && !living.has_effect(&StatusEffect::SPEED)
            && entity
                .pos
                .load()
                .squared_distance_to_vec(&target.get_entity().pos.load())
                > 121.0
        {
            return Some(&Potion::SWIFTNESS);
        }
        None
    }

    pub fn throw_potion(&self, target: &Arc<dyn EntityBase>) {
        if self.is_drinking_potion() {
            return;
        }

        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load_full();

        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();
        let target_vel = target_entity.velocity.load();
        let witch_pos = entity.pos.load();

        let xd = target_pos.x + target_vel.x - witch_pos.x;
        let yd = target_pos.y + target_entity.get_eye_height() - 1.1 - witch_pos.y;
        let zd = target_pos.z + target_vel.z - witch_pos.z;
        let dist = xd.hypot(zd);

        let potion = self.potion_for(target.as_ref(), dist);
        if is_raider(target.get_entity().entity_type) {
            // A raider was a patient, not a foe: one potion and the witch lets go.
            self.mob_entity.set_target(None);
        }

        let potion_stack = create_potion_stack(&Item::SPLASH_POTION, potion);

        let splash_entity = Entity::new(world.clone(), witch_pos, &EntityType::SPLASH_POTION);
        let splash = SplashPotionEntity::new_shot(splash_entity, entity);
        splash.set_item_stack(potion_stack);

        let speed = if dist <= 2.0 { 0.45 } else { 0.75 };
        let yo = dist * 0.2;

        splash.thrown.set_velocity(xd, yd + yo, zd, speed, 8.0);

        if !entity.silent.load(Ordering::Relaxed) {
            world.play_sound_fine(
                Sound::EntityWitchThrow,
                SoundCategory::Hostile,
                &witch_pos,
                1.0,
                rand::random::<f32>().mul_add(0.4, 0.8),
            );
        }

        let splash_arc: Arc<dyn EntityBase> = Arc::new(splash);
        world.spawn_entity(splash_arc);
    }
}

impl Mob for WitchEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_patrolling_monster(&self) -> Option<&dyn PatrollingMonster> {
        Some(self)
    }

    fn as_raider(&self) -> Option<&dyn Raider> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.write_raider_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.read_raider_nbt(nbt);
    }

    fn pre_damage(&self, _damage_type: DamageType, source: Option<&dyn EntityBase>) -> bool {
        if let Some(src) = source
            && src.get_entity().entity_id == self.mob_entity.living_entity.entity.entity_id
        {
            return false;
        }
        true
    }

    fn modify_incoming_damage(&self, mut amount: f32, damage_type: DamageType) -> f32 {
        // Vanilla `Witch.getDamageAfterMagicAbsorb`.
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_WITCH_RESISTANT_TO) {
            amount *= 0.15;
        }
        amount
    }

    /// Vanilla `Witch.aiStep`.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = self.get_entity();
        if !entity.is_alive() {
            return;
        }
        self.heal_cooldown.fetch_sub(1, Ordering::Relaxed);
        if self.is_drinking_potion() {
            // Vanilla tests the counter before lowering it: 32 ticks of drinking, done on the 33rd.
            if self.using_time.fetch_sub(1, Ordering::Relaxed) <= 0 {
                self.finish_drinking();
            }
        } else if let Some(potion) = self.potion_to_drink() {
            self.start_drinking(potion);
        }
        if rand::random::<f32>() < 7.5e-4 {
            entity.world.load().send_entity_status(
                entity,
                EntityStatus::WitchHatMagic,
                pumpkin_protocol::bedrock::server::ActorEventID::WitchHatMagic,
            );
        }
    }
}

impl RangedAttackMob for WitchEntity {
    fn perform_ranged_attack(&self, target: &Arc<dyn EntityBase>, _power: f32) {
        self.throw_potion(target);
    }
}

impl PatrollingMonster for WitchEntity {
    fn get_patrol_data(&self) -> &PatrolData {
        &self.raider_data.patrol_data
    }

    fn can_be_leader(&self) -> bool {
        false
    }
}

impl Raider for WitchEntity {
    fn get_raider_data(&self) -> &RaiderData {
        &self.raider_data
    }

    fn get_celebrate_sound(&self) -> Sound {
        Sound::EntityWitchCelebrate
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_util::math::vector3::Vector3;
    use uuid::Uuid;

    use super::*;
    use crate::entity::ai::goal::goal_selector::GoalSelector;
    use crate::entity::ai::test_support::world;
    use crate::entity::r#type::from_type;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn witch(world: &Arc<World>) -> Arc<WitchEntity> {
        WitchEntity::new(Entity::new(
            world.clone(),
            Vector3::new(0.5, 64.0, 0.5),
            &EntityType::WITCH,
        ))
    }

    fn spawn(world: &Arc<World>, entity_type: &'static EntityType, x: f64) -> Arc<dyn EntityBase> {
        let entity = from_type(
            entity_type,
            Vector3::new(x, 64.0, 0.5),
            world,
            Uuid::new_v4(),
        );
        world.spawn_entity(entity.clone());
        entity
    }

    fn target_after(witch: &Arc<WitchEntity>, ticks: usize) -> Option<i32> {
        for _ in 0..ticks {
            GoalSelector::tick(&witch.mob_entity.target_selector, &**witch);
        }
        witch
            .mob_entity
            .get_target()
            .map(|target| target.get_entity().entity_id)
    }

    #[tokio::test]
    async fn witch_in_a_raid_turns_to_the_nearest_raider_that_is_no_witch() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        // A closer witch is passed over for the pillager behind it.
        spawn(&world, &EntityType::WITCH, 2.5);
        let pillager = spawn(&world, &EntityType::PILLAGER, 4.5);
        let healer = witch(&world);

        let outside_raid = target_after(&healer, 60);
        healer.raider_data.raid_id.store(Some(1));
        let in_raid = target_after(&healer, 60);
        let cooldown = healer.heal_cooldown.load(Ordering::Relaxed);
        let attacks_players_meanwhile = healer.can_attack_players();
        for _ in 0..100 {
            healer.mob_tick(&*healer);
        }
        let attacks_players_after = healer.can_attack_players();
        world.level.shutdown().await;

        assert_eq!(outside_raid, None);
        assert_eq!(in_raid, Some(pillager.get_entity().entity_id));
        assert_eq!(cooldown, 100);
        assert!(!attacks_players_meanwhile);
        assert!(attacks_players_after);
        Ok(())
    }

    #[tokio::test]
    async fn raiders_get_a_helpful_potion_and_are_let_go() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let pillager = spawn(&world, &EntityType::PILLAGER, 4.5);
        let cow = spawn(&world, &EntityType::COW, 14.5);
        let healer = witch(&world);

        let healthy = healer.potion_for(pillager.as_ref(), 4.0).id;
        if let Some(living) = pillager.get_living_entity() {
            living.health.store(4.0);
        }
        let dying = healer.potion_for(pillager.as_ref(), 4.0).id;
        let far_foe = healer.potion_for(cow.as_ref(), 14.0).id;

        healer.mob_entity.set_target(Some(pillager.clone()));
        healer.throw_potion(&pillager);
        let after_healing = healer.mob_entity.get_target().is_none();
        healer.mob_entity.set_target(Some(cow.clone()));
        healer.throw_potion(&cow);
        let after_attacking = healer.mob_entity.get_target().is_some();
        world.level.shutdown().await;

        assert_eq!(healthy, Potion::REGENERATION.id);
        assert_eq!(dying, Potion::HEALING.id);
        assert_eq!(far_foe, Potion::SLOWNESS.id);
        assert!(after_healing);
        assert!(after_attacking);
        Ok(())
    }

    #[tokio::test]
    async fn drinking_takes_33_ticks_and_roots_the_witch() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let drinker = witch(&world);
        let living = &drinker.mob_entity.living_entity;
        let speed = || living.get_attribute_value(&Attributes::MOVEMENT_SPEED);
        let normal = speed();

        drinker.start_drinking(&Potion::FIRE_RESISTANCE);
        let slowed = speed();
        for _ in 0..32 {
            drinker.mob_tick(&*drinker);
        }
        let still_drinking = drinker.is_drinking_potion();
        drinker.mob_tick(&*drinker);
        let done = !drinker.is_drinking_potion();
        let effect = living.has_effect(&StatusEffect::FIRE_RESISTANCE);
        let hand_is_empty = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
            .is_empty();
        let restored = speed();
        world.level.shutdown().await;

        assert!((normal - 0.25).abs() < 1e-6, "{normal}");
        assert!(slowed.abs() < 1e-6, "{slowed}");
        assert!(still_drinking);
        assert!(done && effect && hand_is_empty);
        assert!((restored - normal).abs() < 1e-6);
        Ok(())
    }
}
