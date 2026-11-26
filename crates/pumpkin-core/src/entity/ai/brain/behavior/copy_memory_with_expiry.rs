use rand::RngExt;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleType, MemoryStatus, MemoryValue};
use super::OneShot;

/// Inclusive range sampled like `UniformInt`.
pub type Duration = (i32, i32);

/// Copies `source` into the empty `target` for a random `duration` while `copy_if` holds.
#[must_use]
pub fn create<T: MemoryValue + Clone>(
    mut copy_if: impl FnMut(&BrainTick<'_>) -> bool + Send + Sync + 'static,
    source: MemoryModuleType<T>,
    target: MemoryModuleType<T>,
    duration: Duration,
) -> OneShot {
    OneShot::new(
        "CopyMemoryWithExpiry",
        vec![
            (source.id(), MemoryStatus::ValuePresent),
            (target.id(), MemoryStatus::ValueAbsent),
        ],
        move |tick| {
            if !copy_if(tick) {
                return false;
            }
            let Some(value) = tick.state.get(source).cloned() else {
                return false;
            };
            let ttl = tick.mob.get_random().random_range(duration.0..=duration.1);
            tick.state.set_with_expiry(target, value, i64::from(ttl));
            true
        },
    )
}

#[cfg(test)]
mod tests {
    use pumpkin_data::entity::EntityType;
    use pumpkin_data::environment_attribute::Activity;
    use pumpkin_util::math::position::BlockPos;
    use pumpkin_util::math::vector3::Vector3;

    use super::*;
    use crate::entity::Entity;
    use crate::entity::ai::brain::Brain;
    use crate::entity::ai::brain::activity::ActivityData;
    use crate::entity::ai::brain::memory::types;
    use crate::entity::ai::test_support::world;
    use crate::entity::passive::cow::CowEntity;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn copies_only_into_an_empty_target_and_lets_it_expire() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let body = CowEntity::new(Entity::new(
            world.clone(),
            Vector3::new(0.0, 64.0, 0.0),
            &EntityType::COW,
        ));
        let mut brain = Brain::default();
        brain.add_activity(ActivityData::with_priority_start(
            Activity::Core,
            0,
            vec![Box::new(create(
                |_| true,
                types::NEAREST_REPELLENT,
                types::CELEBRATE_LOCATION,
                (2, 2),
            ))],
        ));
        let first = BlockPos::new(1, 64, 0);
        let second = BlockPos::new(9, 64, 0);

        brain.set(types::NEAREST_REPELLENT, first);
        brain.tick(&world, &*body, 1);
        let copied = brain.get(types::CELEBRATE_LOCATION).copied();
        let ttl = brain.time_until_expiry(types::CELEBRATE_LOCATION.id());

        brain.set(types::NEAREST_REPELLENT, second);
        brain.tick(&world, &*body, 2);
        let kept = brain.get(types::CELEBRATE_LOCATION).copied();

        brain.tick(&world, &*body, 3);
        brain.tick(&world, &*body, 4);
        let recopied = brain.get(types::CELEBRATE_LOCATION).copied();
        world.level.shutdown().await;

        assert_eq!(copied, Some(first));
        assert_eq!(ttl, Some(2));
        assert_eq!(kept, Some(first));
        assert_eq!(recopied, Some(second));
        Ok(())
    }
}
