use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::ai::behavior::fleeing::{self, Flee};
use crate::entity::ai::behavior::hunting::{self, Hunt};
use crate::entity::ai::target_match::TargetMatch;
use crate::entity::living::LivingEntity;
use crate::entity::{
    Entity,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal, open_door::OpenDoorGoal,
        swim::SwimGoal, wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity,
        patrol::{LongDistancePatrolGoal, PatrolData, PatrollingMonster},
        raider::{
            self, HoldGroundAttackGoal, ObtainRaidLeaderBannerGoal, PathfindToRaidGoal, Raider,
            RaiderCelebrationGoal, RaiderData, RaiderMoveThroughVillageGoal,
        },
    },
};
use crate::world::World;

const VINDICATOR_FLEES: &[Flee] = &[raider::flee_creaking(1, 1.0, 1.2)];

const VINDICATOR_HUNTS: &[Hunt] = &[
    Hunt::new(2, TargetMatch::Type(&EntityType::PLAYER), true),
    Hunt::new(3, TargetMatch::VILLAGER_LIKE, true),
    Hunt::new(3, TargetMatch::Type(&EntityType::IRON_GOLEM), true),
];

/// The custom name that turns a vindicator on every living thing.
const JOHNNY: &str = "Johnny";

pub struct VindicatorEntity {
    pub mob_entity: MobEntity,
    pub raider_data: RaiderData,
    /// Vanilla `isJohnny`: set by the name or the `Johnny` tag, never cleared.
    johnny: AtomicBool,
    /// The name came from saved data whose `Johnny` tag said no. Vanilla reads the tag
    /// after the name, so `/summon` with only `CustomName:"Johnny"` is not a Johnny.
    loaded_name_overruled: AtomicBool,
}

impl VindicatorEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let vindicator = Self {
            mob_entity,
            raider_data: RaiderData::default(),
            johnny: AtomicBool::new(false),
            loaded_name_overruled: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(vindicator);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let goal_selector = &mob_arc.mob_entity.goals_selector;

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            goal_selector.add_goal(1, Box::new(OpenDoorGoal::new(false)));
            goal_selector.add_goal(1, Box::new(ObtainRaidLeaderBannerGoal));
            fleeing::apply(goal_selector, VINDICATOR_FLEES);
            goal_selector.add_goal(2, Box::new(HoldGroundAttackGoal::new(10.0)));
            goal_selector.add_goal(3, Box::new(MeleeAttackGoal::new(1.0, true)));
            goal_selector.add_goal(4, Box::new(LongDistancePatrolGoal::new(0.7, 0.595)));
            goal_selector.add_goal(4, Box::new(RaiderMoveThroughVillageGoal::new(1.05)));
            goal_selector.add_goal(4, Box::new(PathfindToRaidGoal::default()));
            goal_selector.add_goal(5, Box::new(RaiderCelebrationGoal));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));

            let target_selector = &mob_arc.mob_entity.target_selector;
            target_selector.add_goal(1, raider::revenge_goal());
            hunting::apply(target_selector, &mob_arc.mob_entity, VINDICATOR_HUNTS);
            // Vanilla `VindicatorJohnnyAttackGoal`: every tick, sight and reach checked,
            // `LivingEntity::attackable` as the selector (only armor stands fail it).
            target_selector.add_goal(
                4,
                Box::new(ActiveTargetGoal::new(
                    &mob_arc.mob_entity,
                    TargetMatch::Any,
                    0,
                    true,
                    true,
                    Some(|target: &LivingEntity, _: &World| {
                        target.entity.entity_type != &EntityType::ARMOR_STAND
                    }),
                ))
                .gated_by(|mob| {
                    mob.cast_any()
                        .downcast_ref::<Self>()
                        .is_some_and(Self::is_johnny)
                }),
            );
        };

        mob_arc
    }

    fn is_named_johnny(&self) -> bool {
        self.mob_entity
            .living_entity
            .entity
            .custom_name
            .load()
            .as_ref()
            .as_ref()
            .is_some_and(|name| name.clone().get_text() == JOHNNY)
    }

    /// Vanilla sets the flag in `setCustomName`. Naming has no per-mob hook here, so the
    /// name is read on the goal's every-tick check and on save instead.
    pub fn is_johnny(&self) -> bool {
        if self.johnny.load(Ordering::Relaxed) {
            return true;
        }
        if !self.is_named_johnny() {
            self.loaded_name_overruled.store(false, Ordering::Relaxed);
            return false;
        }
        if self.loaded_name_overruled.load(Ordering::Relaxed) {
            return false;
        }
        self.johnny.store(true, Ordering::Relaxed);
        true
    }
}

impl Mob for VindicatorEntity {
    fn mob_considers_entity_as_ally(
        &self,
        other: &dyn crate::entity::EntityBase,
        _brain: Option<&crate::entity::ai::brain::BrainState>,
    ) -> Option<bool> {
        Some(raider::illager_considers_ally(self, other))
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_patrolling_monster(&self) -> Option<&dyn PatrollingMonster> {
        Some(self)
    }

    fn as_raider(&self) -> Option<&dyn Raider> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_raider_nbt(nbt);
        if self.is_johnny() {
            nbt.put_bool("Johnny", true);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_raider_nbt(nbt);
        let tagged = nbt.get_bool("Johnny").unwrap_or(false);
        self.loaded_name_overruled
            .store(!tagged && self.is_named_johnny(), Ordering::Relaxed);
        self.johnny.store(tagged, Ordering::Relaxed);
    }
}

impl PatrollingMonster for VindicatorEntity {
    fn get_patrol_data(&self) -> &PatrolData {
        &self.raider_data.patrol_data
    }
}

impl Raider for VindicatorEntity {
    fn get_raider_data(&self) -> &RaiderData {
        &self.raider_data
    }

    fn get_celebrate_sound(&self) -> Sound {
        Sound::EntityVindicatorCelebrate
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::vector3::Vector3;
    use pumpkin_util::text::TextComponent;

    use super::*;
    use crate::entity::EntityBase;
    use crate::entity::ai::goal::goal_selector::GoalSelector;
    use crate::entity::ai::test_support::world;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn vindicator(world: &Arc<World>) -> Arc<VindicatorEntity> {
        VindicatorEntity::new(Entity::new(
            world.clone(),
            Vector3::new(0.5, 64.0, 0.5),
            &EntityType::VINDICATOR,
        ))
    }

    fn name(vindicator: &VindicatorEntity, name: &str) {
        vindicator
            .get_entity()
            .set_custom_name(TextComponent::text(name.to_string()));
    }

    fn saved_tag(vindicator: &VindicatorEntity) -> Option<bool> {
        let mut nbt = NbtCompound::new();
        vindicator.mob_write_nbt(&mut nbt);
        nbt.get_bool("Johnny")
    }

    #[tokio::test]
    async fn the_name_johnny_sticks_and_is_saved() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let plain = vindicator(&world);
        let named = vindicator(&world);
        name(&plain, "Jonny");
        name(&named, "Johnny");
        let before = (plain.is_johnny(), named.is_johnny());
        name(&named, "Bob");
        let after_rename = named.is_johnny();
        let tags = (saved_tag(&plain), saved_tag(&named));
        world.level.shutdown().await;

        assert_eq!(before, (false, true));
        assert!(after_rename);
        assert_eq!(tags, (None, Some(true)));
        Ok(())
    }

    #[tokio::test]
    async fn saved_tag_overrules_the_saved_name() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        // `/summon vindicator ~ ~ ~ {CustomName:"Johnny"}`: name first, then no tag.
        let summoned = vindicator(&world);
        name(&summoned, "Johnny");
        summoned.mob_read_nbt(&NbtCompound::new());
        let summoned_is = summoned.is_johnny();
        // Renamed by hand afterwards.
        name(&summoned, "Bob");
        let renamed_away = summoned.is_johnny();
        name(&summoned, "Johnny");
        let renamed_back = summoned.is_johnny();

        let tagged = vindicator(&world);
        let mut nbt = NbtCompound::new();
        nbt.put_bool("Johnny", true);
        tagged.mob_read_nbt(&nbt);
        let tagged_is = tagged.is_johnny();
        world.level.shutdown().await;

        assert!(!summoned_is);
        assert!(!renamed_away);
        assert!(renamed_back);
        assert!(tagged_is);
        Ok(())
    }

    #[tokio::test]
    async fn only_johnny_goes_for_a_cow_and_spares_illagers() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let cow = crate::entity::r#type::from_type(
            &EntityType::COW,
            Vector3::new(3.5, 64.0, 0.5),
            &world,
            uuid::Uuid::new_v4(),
        );
        world.spawn_entity(cow.clone());
        // Closer than the cow, but a fellow illager.
        world.spawn_entity(crate::entity::r#type::from_type(
            &EntityType::PILLAGER,
            Vector3::new(1.5, 64.0, 0.5),
            &world,
            uuid::Uuid::new_v4(),
        ));
        let hunter = vindicator(&world);
        let targets = |hunter: &Arc<VindicatorEntity>| {
            for _ in 0..4 {
                GoalSelector::tick(&hunter.mob_entity.target_selector, &**hunter);
            }
            hunter
                .mob_entity
                .target
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .and_then(Weak::upgrade)
                .map(|target| target.get_entity().entity_id)
        };
        let plain_target = targets(&hunter);
        name(&hunter, "Johnny");
        let johnny_target = targets(&hunter);
        world.level.shutdown().await;

        assert_eq!(plain_target, None);
        assert_eq!(johnny_target, Some(cow.get_entity().entity_id));
        Ok(())
    }
}
