use pumpkin_data::tag::Taggable;
use std::sync::{Arc, Weak};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::Difficulty;

use crate::entity::{
    Entity,
    ai::behavior::fleeing::{self, Flee},
    ai::behavior::hunting::{self, Hunt},
    ai::goal::{
        bow_attack::BowAttackGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal, revenge::RevengeGoal,
        swim::SwimGoal, wander_around::WanderAroundGoal,
    },
    ai::target_match::TargetMatch,
    mob::{Mob, MobEntity, equipment::RegionalDifficulty},
};
use crate::world::World;

pub mod bogged;
pub mod parched;
#[allow(clippy::module_inception)]
pub mod skeleton;
pub mod stray;
pub mod wither;

const SKELETON_FLEES: &[Flee] = &[Flee::new(
    3,
    TargetMatch::Type(&EntityType::WOLF),
    6.0,
    1.0,
    1.2,
)];

const SKELETON_HUNTS: &[Hunt] = &[
    Hunt::new(2, TargetMatch::Type(&EntityType::PLAYER), true),
    Hunt::new(3, TargetMatch::Type(&EntityType::IRON_GOLEM), true),
    hunting::baby_turtles(3, true),
];

pub struct SkeletonEntityBase {
    pub mob_entity: MobEntity,
}

impl SkeletonEntityBase {
    pub fn new(entity: Entity) -> Arc<Self> {
        Self::with_hunts(entity, &[])
    }

    /// `extra` goes first, like vanilla subclasses adding targets before `super`.
    pub fn with_hunts(entity: Entity, extra: &[Hunt]) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let mob = Self { mob_entity };
        let mob_arc = Arc::new(mob);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };
        {
            let goal_selector = &mob_arc.mob_entity.goals_selector;
            let target_selector = &mob_arc.mob_entity.target_selector;

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            // TODO: RestrictSunGoal(2) and FleeSunGoal(3), once both match vanilla.
            fleeing::apply(goal_selector, SKELETON_FLEES);
            // Vanilla `reassessWeaponGoal`: the bow or the melee goal at 4, by the held weapon.
            goal_selector.add_goal(
                4,
                Box::new(BowAttackGoal::new(1.0, 20, 15.0).with_mob_interval()),
            );
            goal_selector.add_goal(
                4,
                Box::new(
                    MeleeAttackGoal::new(1.2, false)
                        .gated_by(|mob| !BowAttackGoal::is_holding_bow(mob)),
                ),
            );
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::water_avoiding(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            hunting::apply(target_selector, &mob_arc.mob_entity, extra);
            hunting::apply(target_selector, &mob_arc.mob_entity, SKELETON_HUNTS);
        };

        mob_arc
    }
}

pub fn wants_to_pick_up<M: Mob + ?Sized>(mob: &M, stack: &ItemStack) -> bool {
    !stack
        .item
        .has_tag(&pumpkin_data::tag::Item::MINECRAFT_SPEARS)
        && mob.can_hold_item(stack)
}

impl Mob for SkeletonEntityBase {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        // Default armor slots (super.populateDefaultEquipmentSlots)
        if rand::random::<f32>()
            < MobEntity::MAX_WEARING_ARMOR_CHANCE * difficulty.special_multiplier
        {
            let mut armor_type = rand::random_range(0..3);
            for _ in 1..=3 {
                if rand::random::<f32>() < MobEntity::WEARING_ARMOR_UPGRADE_MATERIAL_CHANCE {
                    armor_type += 1;
                }
            }

            let partial_chance = if difficulty.base_difficulty == Difficulty::Hard {
                0.1f32
            } else {
                0.25f32
            };

            let living = &self.mob_entity.living_entity;
            let mut equipment = living
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut first = true;

            for slot in &MobEntity::EQUIPMENT_POPULATION_ORDER {
                let current = equipment.get(slot);
                if !first && rand::random::<f32>() < partial_chance {
                    break;
                }
                first = false;
                if current.is_empty()
                    && let Some(item) = MobEntity::get_equipment_for_slot(slot, armor_type)
                {
                    equipment.put(slot, ItemStack::new(1, item));
                }
            }
        }

        // AbstractSkeleton sets BOW on MAIN_HAND
        let living = &self.mob_entity.living_entity;
        let mut equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        equipment.put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, &Item::BOW));
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::effect::StatusEffect;
    use pumpkin_data::potion::Effect;
    use pumpkin_util::math::vector3::Vector3;

    use super::bogged::BoggedSkeletonEntity;
    use super::parched::ParchedSkeletonEntity;
    use super::stray::StraySkeletonEntity;
    use super::*;
    use crate::entity::ai::test_support::world;
    use crate::entity::projectile::arrow::ArrowEntity;
    use crate::item::potion::PotionContents;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// The effects an arrow shot by `mob` carries, as (effect, duration).
    fn arrow_effects(world: &Arc<World>, mob: &dyn Mob) -> Vec<(&'static StatusEffect, i32)> {
        let arrow = ArrowEntity::new(
            Entity::new(
                world.clone(),
                Vector3::new(0.0, 64.0, 0.0),
                &EntityType::ARROW,
            ),
            Some(mob.get_entity()),
        );
        let arrow = mob.get_arrow(arrow);
        let stack = arrow
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        PotionContents::read_potion_effects(&stack)
            .into_iter()
            .map(|(effect, duration, ..)| (effect, duration))
            .collect()
    }

    fn entity(world: &Arc<World>, entity_type: &'static EntityType) -> Entity {
        Entity::new(world.clone(), Vector3::new(0.0, 64.0, 0.0), entity_type)
    }

    #[tokio::test]
    async fn each_skeleton_kind_tips_its_arrows_and_keeps_its_pace() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let skeleton = SkeletonEntityBase::new(entity(&world, &EntityType::SKELETON));
        let stray = StraySkeletonEntity::new(entity(&world, &EntityType::STRAY));
        let bogged = BoggedSkeletonEntity::new(entity(&world, &EntityType::BOGGED));
        let parched = ParchedSkeletonEntity::new(entity(&world, &EntityType::PARCHED));

        let plain = arrow_effects(&world, &*skeleton);
        let slowing = arrow_effects(&world, &*stray);
        let poisoning = arrow_effects(&world, &*bogged);
        let weakening = arrow_effects(&world, &*parched);
        let pace = |mob: &dyn Mob| {
            (
                mob.bow_attack_interval(false),
                mob.bow_attack_interval(true),
            )
        };
        let paces = [
            pace(&*skeleton),
            pace(&*stray),
            pace(&*bogged),
            pace(&*parched),
        ];
        world.level.shutdown().await;

        assert_eq!(plain, vec![]);
        assert_eq!(slowing, vec![(&StatusEffect::SLOWNESS, 600)]);
        assert_eq!(poisoning, vec![(&StatusEffect::POISON, 100)]);
        assert_eq!(weakening, vec![(&StatusEffect::WEAKNESS, 600)]);
        assert_eq!(paces, [(40, 20), (40, 20), (70, 50), (70, 50)]);
        Ok(())
    }

    #[tokio::test]
    async fn parched_shrugs_off_weakness_only() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let parched = ParchedSkeletonEntity::new(entity(&world, &EntityType::PARCHED));
        let living = &parched.get_mob_entity().living_entity;
        let effect = |effect_type| Effect {
            effect_type,
            duration: 200,
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        };
        living.add_effect(effect(&StatusEffect::WEAKNESS));
        living.add_effect(effect(&StatusEffect::SLOWNESS));
        let weak = living.has_effect(&StatusEffect::WEAKNESS);
        let slow = living.has_effect(&StatusEffect::SLOWNESS);
        world.level.shutdown().await;

        assert!(!weak);
        assert!(slow);
        Ok(())
    }
}
