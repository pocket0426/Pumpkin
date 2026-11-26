use pumpkin_data::entity::EntityType;
use rand::RngExt;

use super::super::memory::{MemoryStatus, types};
use super::{OneShot, utils};

/// Vanilla `SetEntityLookTargetSometimes.Ticker`: true once per random interval.
pub struct Ticker {
    min_interval: i32,
    max_interval: i32,
    ticks_until_next_start: i32,
}

impl Ticker {
    #[must_use]
    pub const fn new(min_interval: i32, max_interval: i32) -> Self {
        Self {
            min_interval,
            max_interval,
            ticks_until_next_start: 0,
        }
    }

    pub fn tick_down_and_check(&mut self, rng: &mut impl RngExt) -> bool {
        if self.ticks_until_next_start == 0 {
            self.ticks_until_next_start =
                rng.random_range(self.min_interval..=self.max_interval) - 1;
            return false;
        }
        self.ticks_until_next_start -= 1;
        self.ticks_until_next_start == 0
    }
}

#[must_use]
pub fn create(max_dist: f32, min_interval: i32, max_interval: i32) -> OneShot {
    create_matching(None, max_dist, min_interval, max_interval)
}

/// Only looks at entities of `entity_type`, or at anything when it is `None`.
#[must_use]
pub fn create_matching(
    entity_type: Option<&'static EntityType>,
    max_dist: f32,
    min_interval: i32,
    max_interval: i32,
) -> OneShot {
    let max_dist_sqr = f64::from(max_dist * max_dist);
    let mut ticker = Ticker::new(min_interval, max_interval);
    OneShot::new(
        "SetEntityLookTargetSometimes",
        vec![
            (types::LOOK_TARGET.id(), MemoryStatus::ValueAbsent),
            (
                types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
                MemoryStatus::ValuePresent,
            ),
        ],
        move |tick| {
            let position = tick.mob.get_entity().pos.load();
            let target = tick
                .state
                .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)
                .and_then(|visible| {
                    visible.find_closest(&tick.visibility(), |target| {
                        let target = target.get_entity();
                        entity_type.is_none_or(|entity_type| target.entity_type == entity_type)
                            && target.pos.load().squared_distance_to_vec(&position) <= max_dist_sqr
                    })
                });
            let Some(target) = target else {
                return false;
            };
            if !ticker.tick_down_and_check(&mut tick.mob.get_random()) {
                return false;
            }
            utils::look_at_entity(tick.state, target);
            true
        },
    )
}
