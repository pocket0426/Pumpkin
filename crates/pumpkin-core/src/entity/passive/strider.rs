use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::BlockState;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    item_steerable::{ItemBasedSteering, ItemSteerable},
    mob::{
        Mob, MobEntity,
        equipment::equip_on_target,
        spawn::{EntitySpawnReason, SpawnGroupData, finalize_spawn},
    },
    passive::animal::Animal,
    player::Player,
    r#type::from_type,
};
use crate::world::World;
use rand::RngExt;
use uuid::Uuid;

/// One strider in this many spawns with a zombified piglin on its back.
const PIGLIN_JOCKEY_ODDS: i32 = 30;
/// Of the rest, one in this many carries a baby strider.
const BABY_JOCKEY_ODDS: i32 = 10;
/// Vanilla `Zombie.getSpawnAsBabyOdds`.
const ZOMBIE_BABY_ODDS: f32 = 0.05;

pub struct StriderEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub steering: ItemBasedSteering,
    pub suffocating: AtomicBool,
}

impl StriderEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let strider = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            steering: ItemBasedSteering::new(pumpkin_data::tracked_data::strider::DATA_BOOST_TIME),
            suffocating: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(strider);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let goal_selector = &mob_arc.mob_entity.goals_selector;

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            goal_selector.add_goal(1, EscapeDangerGoal::new(1.65));
            goal_selector.add_goal(2, BreedGoal::new(1.0));
            goal_selector.add_goal(
                3,
                Box::new(TemptGoal::with_tag(
                    1.4,
                    &tag::Item::MINECRAFT_STRIDER_TEMPT_ITEMS,
                    false,
                )),
            );
            goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.1)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    #[must_use]
    pub fn is_suffocating(&self) -> bool {
        self.suffocating.load(Ordering::Relaxed)
    }

    pub fn set_suffocating(&self, suffocating: bool) {
        self.suffocating.store(suffocating, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::strider::DATA_SUFFOCATING,
            suffocating,
        );
    }
}

impl AgeableMob for StriderEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    fn default_baby_spawn_chance(&self) -> f32 {
        crate::entity::ageable::STRIDER_BABY_SPAWN_CHANCE
    }
}

impl Animal for StriderEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_STRIDER_FOOD)
    }
}

impl StriderEntity {
    /// Vanilla `Strider.spawnJockey`: the rider is finalized on the strider's spot and
    /// enters the world mounted, and the group stops rolling babies.
    fn spawn_jockey(
        &self,
        world: &Arc<World>,
        rider: Arc<dyn EntityBase>,
        rider_group: Option<SpawnGroupData>,
    ) -> SpawnGroupData {
        rider
            .get_entity()
            .set_rotation(self.get_entity().yaw.load(), 0.0);
        finalize_spawn(&rider, world, EntitySpawnReason::Jockey, rider_group);
        self.mob_entity.add_pending_rider(rider);
        SpawnGroupData::Ageable {
            group_size: 0,
            should_spawn_baby: true,
            baby_spawn_chance: 0.0,
        }
    }
}

impl Mob for StriderEntity {
    /// Vanilla `Strider.finalizeSpawn`: 1 in 30 carries a zombified piglin (where monsters
    /// spawn), otherwise 1 in 10 a baby strider. Every strider starts its group over, so
    /// the only naturally spawned babies are those riders.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        if self.is_baby() {
            return self.default_finalize_spawn(world, group_data);
        }
        let mut random = self.get_random();
        let pos = self.get_entity().pos.load();
        let group =
            if random.random_range(0..PIGLIN_JOCKEY_ODDS) == 0 && world.should_spawn_monsters() {
                let rider = from_type(&EntityType::ZOMBIFIED_PIGLIN, pos, world, Uuid::new_v4());
                let group = self.spawn_jockey(
                    world,
                    rider.clone(),
                    Some(SpawnGroupData::Zombie {
                        is_baby: random.random::<f32>() < ZOMBIE_BABY_ODDS,
                        can_spawn_jockey: false,
                    }),
                );
                if let Some(rider) = rider.get_mob() {
                    rider.get_mob_entity().set_item_slot(
                        &EquipmentSlot::MAIN_HAND,
                        ItemStack::new(1, &Item::WARPED_FUNGUS_ON_A_STICK),
                    );
                }
                self.mob_entity.set_item_slot_and_drop_when_killed(
                    &EquipmentSlot::SADDLE,
                    ItemStack::new(1, &Item::SADDLE),
                );
                group
            } else if random.random_range(0..BABY_JOCKEY_ODDS) == 0 {
                let rider = from_type(&EntityType::STRIDER, pos, world, Uuid::new_v4());
                if let Some(baby) = rider.get_mob().and_then(Mob::as_ageable) {
                    baby.set_baby(true);
                }
                self.spawn_jockey(world, rider, None)
            } else {
                SpawnGroupData::Ageable {
                    group_size: 0,
                    should_spawn_baby: true,
                    baby_spawn_chance: crate::entity::ageable::STRIDER_BABY_SPAWN_CHANCE,
                }
            };
        self.default_finalize_spawn(world, Some(group))
    }

    fn can_dispenser_equip_into_slot(&self, slot: &EquipmentSlot) -> bool {
        *slot == EquipmentSlot::SADDLE || self.mob_entity.can_pick_up_loot()
    }

    fn mob_controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        self.saddled_controlling_passenger(Some(&Item::WARPED_FUNGUS_ON_A_STICK))
    }

    fn mob_play_step_sound(&self, _pos: &BlockPos, _state: &BlockState) {
        let sound = if self.get_entity().touching_lava.load(Ordering::Relaxed) {
            Sound::EntityStriderStepLava
        } else {
            Sound::EntityStriderStep
        };
        self.play_entity_sound(sound, 1.0, 1.0);
    }

    fn mob_ai_step(&self) {
        if self
            .mob_controlling_passenger()
            .is_some_and(|rider| rider.get_player().is_some())
        {
            self.steering.tick_boost();
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        self.mob_entity
            .read_legacy_saddle(nbt.get_bool("Saddle") == Some(true));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_item_steerable(&self) -> Option<&dyn ItemSteerable> {
        Some(self)
    }

    fn can_use_slot(&self, slot: &EquipmentSlot) -> bool {
        *slot != EquipmentSlot::SADDLE
            || (self.mob_entity.living_entity.entity.is_alive() && !self.is_baby())
    }

    fn get_equip_sound(&self, slot: &EquipmentSlot) -> Option<Sound> {
        (*slot == EquipmentSlot::SADDLE).then_some(Sound::EntityStriderSaddle)
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::strider::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::strider::DATA_SUFFOCATING,
            self.is_suffocating(),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if !self.is_food(item_stack)
            && self.is_saddled()
            && !self.get_entity().has_passengers()
            && !player.get_entity().is_sneaking()
        {
            let world = player.world();
            let ent = &self.mob_entity.living_entity.entity;
            if let Some(vehicle) = world.get_entity_by_id(ent.entity_id)
                && let Some(passenger) = world.get_player_by_id(player.entity_id())
            {
                ent.add_passenger(vehicle, passenger as Arc<dyn EntityBase>);
            }
            return true;
        }

        if self.animal_interact(player, item_stack) {
            return true;
        }
        self.is_equippable_in_slot(item_stack, &EquipmentSlot::SADDLE)
            && equip_on_target(self, player, item_stack)
    }
}

impl ItemSteerable for StriderEntity {
    fn boost(&self) -> bool {
        let mut rng = self.get_random();
        self.steering.boost(self.get_entity(), &mut rng)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_util::math::vector3::Vector3;

    use super::*;
    use crate::entity::ai::test_support::world;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn natural_striders_are_adults_and_some_carry_a_rider() -> TestResult {
        const SPAWNS: usize = 1500;
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let monsters = world.should_spawn_monsters();
        let mut piglin_riders = 0;
        let mut baby_riders = 0;
        let mut broken_rules = Vec::new();
        let mut group = None;
        for _ in 0..SPAWNS {
            let strider = StriderEntity::new(Entity::new(
                world.clone(),
                Vector3::new(0.5, 64.0, 0.5),
                &EntityType::STRIDER,
            ));
            group = strider.finalize_spawn(&world, EntitySpawnReason::Natural, group);
            let riders = strider.mob_entity.take_pending_riders();
            let saddled = strider.is_saddled();
            if strider.is_baby() {
                broken_rules.push("a strider spawned as a baby on its own");
            }
            match riders.as_slice() {
                [] if saddled => broken_rules.push("saddled without a rider"),
                [] => {}
                [rider] if rider.get_entity().entity_type == &EntityType::ZOMBIFIED_PIGLIN => {
                    piglin_riders += 1;
                    let holds_stick = rider.get_living_entity().is_some_and(|living| {
                        living
                            .entity_equipment
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get(&EquipmentSlot::MAIN_HAND)
                            .item
                            == &Item::WARPED_FUNGUS_ON_A_STICK
                    });
                    if !holds_stick || !saddled {
                        broken_rules.push("piglin rider without its stick or the saddle");
                    }
                }
                [rider] if rider.get_entity().entity_type == &EntityType::STRIDER => {
                    baby_riders += 1;
                    let is_baby = rider
                        .get_mob()
                        .and_then(Mob::as_ageable)
                        .is_some_and(AgeableMob::is_baby);
                    if !is_baby || saddled {
                        broken_rules.push("strider rider that is no baby, or a saddle for it");
                    }
                }
                _ => broken_rules.push("unexpected riders"),
            }
        }
        world.level.shutdown().await;

        assert_eq!(broken_rules, Vec::<&str>::new());
        // 1 in 30 and (29 in 30) x (1 in 10) of 1500: about 50 and 145.
        if monsters {
            assert!((20..=95).contains(&piglin_riders), "{piglin_riders}");
        } else {
            assert_eq!(piglin_riders, 0);
        }
        assert!((90..=210).contains(&baby_riders), "{baby_riders}");
        Ok(())
    }
}
