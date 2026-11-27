use core::f32;
use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicU32, Ordering},
};

use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{damage::DamageType, entity::EntityType};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::{
    boundingbox::BoundingBox,
    vector3::{Axis, Vector3},
};

use crate::{
    block::blocks::bubble_column::SURFACE_UPWARD_ACCELERATION,
    server::Server,
    world::{
        World,
        entity_islands::{self, AuditedReach},
    },
};

use super::{Entity, EntityBase, living::LivingEntity, player::Player, weak_slot::WeakSlot};

pub struct ExperienceOrbEntity {
    entity: Entity,
    /// Experience one pickup gives, NBT `Value`.
    value: AtomicU32,
    /// How many orbs of that value this one stands for, NBT `Count`. Zero only while
    /// another orb is absorbing it.
    count: AtomicI32,
    health: AtomicI32,
    orb_age: AtomicU32,
    following_player: WeakSlot<Player>,
}

impl ExperienceOrbEntity {
    const AIR_DRAG: f32 = 0.98;
    const DEFAULT_HEALTH: i32 = 5;
    /// Orbs merge only when their ids are this far apart, or a multiple of it.
    const MERGE_GROUPS: i32 = 40;
    const LIFETIME: u32 = 6000;

    pub fn new(entity: Entity, amount: u32) -> Self {
        entity.yaw.store(rand::random::<f32>() * 360.0);
        Self {
            entity,
            value: AtomicU32::new(amount),
            count: AtomicI32::new(1),
            health: AtomicI32::new(Self::DEFAULT_HEALTH),
            orb_age: AtomicU32::new(0),
            following_player: WeakSlot::new(),
        }
    }

    pub fn spawn(world: &Arc<World>, position: Vector3<f64>, amount: u32) {
        Self::spawn_with_direction(world, position, Vector3::new(0.0, 0.0, 0.0), amount);
    }

    fn new_with_direction(
        world: &Arc<World>,
        position: Vector3<f64>,
        direction: Vector3<f64>,
        amount: u32,
    ) -> Self {
        let entity = Entity::new(world.clone(), position, &EntityType::EXPERIENCE_ORB);
        let orb = Self::new(entity, amount);
        let mut velocity = Vector3::new(
            (rand::random::<f64>() * 0.2 - 0.1) * 2.0,
            rand::random::<f64>() * 0.2 * 2.0,
            (rand::random::<f64>() * 0.2 - 0.1) * 2.0,
        );
        if direction.length_squared() > 0.0 && direction.dot(&velocity) < 0.0 {
            velocity = velocity.multiply(-1.0, -1.0, -1.0);
        }
        let size = orb.entity.bounding_box.load().get_average_side_length();
        let offset = direction.normalize() * (size * 0.5);
        orb.entity.set_pos(position.add(&offset));
        orb.entity.set_velocity(velocity);
        if !world.is_space_empty(orb.entity.bounding_box.load()) {
            orb.unstuck_if_possible(size);
        }
        orb
    }

    fn unstuck_if_possible(&self, max_distance: f64) {
        let bounding_box = self.entity.bounding_box.load();
        let size = bounding_box.max - bounding_box.min;
        let center = (bounding_box.min + bounding_box.max) * 0.5;
        let half_distance = Vector3::new(max_distance, max_distance, max_distance) * 0.5;
        let allowed_centers = BoundingBox {
            min: center - half_distance,
            max: center + half_distance,
        };
        let world = self.entity.world.load();
        let (mut collisions, _) =
            world.get_block_collisions(allowed_centers.expand(size.x, size.y, size.z), self);
        {
            let border = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let half = border.new_diameter * 0.5;
            let epsilon = f64::from(1.0e-5f32);
            collisions.retain(|shape| {
                shape.min.x >= border.center_x - half
                    && shape.max.x - epsilon < border.center_x + half
                    && shape.min.z >= border.center_z - half
                    && shape.max.z - epsilon < border.center_z + half
            });
        };
        let expanded_collisions = collisions
            .into_iter()
            .map(|shape| shape.expand(size.x * 0.5, size.y * 0.5, size.z * 0.5));
        if let Some(position) = closest_free_center(allowed_centers, center, expanded_collisions) {
            self.entity
                .set_pos(position.add_raw(0.0, -size.y * 0.5, 0.0));
        }
    }

    pub fn spawn_with_direction(
        world: &Arc<World>,
        position: Vector3<f64>,
        direction: Vector3<f64>,
        amount: u32,
    ) {
        let mut amount = amount;
        while amount > 0 {
            let i = Self::round_to_orb_size(amount);
            amount -= i;
            if Self::try_merge_to_existing(world, position, i) {
                continue;
            }
            let orb = Arc::new(Self::new_with_direction(world, position, direction, i));
            world.spawn_entity(orb);
        }
    }

    /// Vanilla `canMerge(orb, id, value)`: the same value, and ids a multiple of 40 apart
    /// so that a pile settles into at most 40 orbs of each value.
    fn can_merge(&self, id: i32, value: u32) -> bool {
        !self.entity.is_removed()
            && (self.entity.entity_id - id) % Self::MERGE_GROUPS == 0
            && self.value() == value
    }

    /// Vanilla `tryMergeToExisting`: new experience joins an orb already lying there.
    fn try_merge_to_existing(world: &World, position: Vector3<f64>, value: u32) -> bool {
        let area = BoundingBox {
            min: position.add_raw(-0.5, -0.5, -0.5),
            max: position.add_raw(0.5, 0.5, 0.5),
        };
        let id = rand::random_range(0..Self::MERGE_GROUPS);
        for candidate in
            world.get_entities_of_type_at_box(&area, |kind| kind == &EntityType::EXPERIENCE_ORB)
        {
            let Some(orb) = candidate.cast_any().downcast_ref::<Self>() else {
                continue;
            };
            // An orb being absorbed right now (count 0) is left alone.
            if orb.can_merge(id, value)
                && orb
                    .count
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                        (count > 0).then_some(count + 1)
                    })
                    .is_ok()
            {
                orb.orb_age.store(0, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    /// Vanilla `scanForMerges`: once a second, swallow matching orbs half a block around.
    fn scan_for_merges(&self) {
        let world = self.entity.world.load();
        let area = self.entity.bounding_box.load().expand(0.5, 0.5, 0.5);
        for candidate in
            world.get_entities_of_type_at_box(&area, |kind| kind == &EntityType::EXPERIENCE_ORB)
        {
            if candidate.get_entity().entity_id == self.entity.entity_id {
                continue;
            }
            if let Some(other) = candidate.cast_any().downcast_ref::<Self>()
                && other.can_merge(self.entity.entity_id, self.value())
            {
                self.absorb(other);
            }
        }
    }

    /// Vanilla `merge`. Entities tick in parallel here, so the other orb's count is taken
    /// first and handed back if this orb was itself absorbed in the meantime.
    fn absorb(&self, other: &Self) {
        let taken = other.count.swap(0, Ordering::Relaxed);
        if taken <= 0 {
            return;
        }
        let added = self
            .count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count > 0).then_some(count + taken)
            });
        if added.is_err() {
            other.count.store(taken, Ordering::Relaxed);
            return;
        }
        self.orb_age
            .fetch_min(other.orb_age.load(Ordering::Relaxed), Ordering::Relaxed);
        other.entity.remove();
    }

    pub fn set_value(&self, value: u32) {
        self.value.store(value, Ordering::Relaxed);
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::experience_orb::DATA_VALUE,
            VarInt(value as i32),
        );
    }

    #[must_use]
    pub fn value(&self) -> u32 {
        self.value.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn count(&self) -> i32 {
        self.count.load(Ordering::Relaxed)
    }

    /// Vanilla `setUnderwaterMovement`: orbs drift up slowly in water.
    fn underwater_velocity(velocity: Vector3<f64>) -> Vector3<f64> {
        const DRAG: f64 = 0.99f32 as f64;
        Vector3::new(
            velocity.x * DRAG,
            (velocity.y + f64::from(5.0e-4f32)).min(f64::from(0.06f32)),
            velocity.z * DRAG,
        )
    }

    /// Vanilla `ExperienceOrb.followNearbyPlayer`
    /// accelerate towards a player within 8 blocks.
    fn follow_nearby_player(&self) {
        let entity = &self.entity;
        let pos = entity.pos.load();
        let lost = self.following_player.get().is_none_or(|player| {
            player.is_spectator()
                || player.get_entity().pos.load().squared_distance_to_vec(&pos) > 64.0
        });
        if lost {
            let nearest = entity.world.load().get_nearest_player(pos, 8.0, |player| {
                !player.is_spectator() && !player.living_entity.is_dead_or_dying()
            });
            match nearest {
                Some(player) => self.following_player.set(&player),
                None => self.following_player.clear(),
            }
        }
        let Some(player) = self.following_player.get() else {
            return;
        };

        let target = player.get_entity();
        let target_pos = target
            .pos
            .load()
            .add_raw(0.0, target.get_eye_height() / 2.0, 0.0);
        let delta = target_pos.sub(&pos);
        let power = 1.0 - delta.length() / 8.0;
        entity
            .velocity
            .store(entity.velocity.load() + delta.normalize() * (power * power * 0.1));
        entity.velocity_dirty.store(true, Ordering::SeqCst);
    }

    const fn round_to_orb_size(value: u32) -> u32 {
        if value >= 2477 {
            2477
        } else if value >= 1237 {
            1237
        } else if value >= 617 {
            617
        } else if value >= 307 {
            307
        } else if value >= 149 {
            149
        } else if value >= 73 {
            73
        } else if value >= 37 {
            37
        } else if value >= 17 {
            17
        } else if value >= 7 {
            7
        } else if value >= 3 {
            3
        } else {
            1
        }
    }
}

fn closest_free_center(
    allowed_centers: BoundingBox,
    center: Vector3<f64>,
    collisions: impl Iterator<Item = BoundingBox>,
) -> Option<Vector3<f64>> {
    // Subtract expanded collision boxes, matching vanilla's ONLY_FIRST shape operation.
    let mut free_boxes = vec![allowed_centers];
    for collision in collisions {
        let mut remaining = Vec::new();
        for mut free in free_boxes {
            if !free.intersects(&collision) {
                remaining.push(free);
                continue;
            }
            for axis in [Axis::X, Axis::Y, Axis::Z] {
                if free.min.get_axis(axis) < collision.min.get_axis(axis) {
                    let mut piece = free;
                    piece.max.set_axis(axis, collision.min.get_axis(axis));
                    remaining.push(piece);
                    free.min.set_axis(axis, collision.min.get_axis(axis));
                }
                if free.max.get_axis(axis) > collision.max.get_axis(axis) {
                    let mut piece = free;
                    piece.min.set_axis(axis, collision.max.get_axis(axis));
                    remaining.push(piece);
                    free.max.set_axis(axis, collision.max.get_axis(axis));
                }
            }
        }
        free_boxes = remaining;
    }
    free_boxes
        .into_iter()
        .map(|free| {
            Vector3::new(
                center.x.clamp(free.min.x, free.max.x),
                center.y.clamp(free.min.y, free.max.y),
                center.z.clamp(free.min.z, free.max.z),
            )
        })
        .min_by(|a, b| {
            a.squared_distance_to_vec(&center)
                .total_cmp(&b.squared_distance_to_vec(&center))
        })
}

impl EntityBase for ExperienceOrbEntity {
    /// Vanilla `ExperienceOrb.doWaterSplashEffect` does nothing.
    fn do_water_splash_effect(&self, _caller: &dyn EntityBase) {}

    fn audited_reach(&self) -> Option<AuditedReach> {
        let entity = &self.entity;
        let world = entity.world.load();
        let fluid_push = super::max_fluid_push_speed(world.dimension.fast_lava);
        let velocity = entity.velocity.load();
        let mut reach = AuditedReach {
            bounding_box: entity.bounding_box.load(),
            motion: (
                velocity.x.hypot(velocity.z) + fluid_push,
                velocity.y.abs() + self.get_gravity() + SURFACE_UPWARD_ACCELERATION + fluid_push,
            ),
            // Merging reaches half a block around the orb.
            entities: 0.5,
            players: 0.0,
            block_reads: 1.0,
            route: None,
            changes_blocks: false,
            global: false,
            links: entity_islands::links_of(self),
        };
        reach.scan_contact_blocks(&world);
        Some(reach)
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_short("Health", self.health.load(Ordering::Relaxed) as i16);
        nbt.put_short("Age", self.orb_age.load(Ordering::Relaxed) as i16);
        nbt.put_short("Value", self.value() as i16);
        nbt.put_int("Count", self.count().max(1));
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.health.store(
            nbt.get_short("Health")
                .map_or(Self::DEFAULT_HEALTH, i32::from),
            Ordering::Relaxed,
        );
        self.orb_age.store(
            nbt.get_short("Age").map_or(0, |age| age.max(0) as u32),
            Ordering::Relaxed,
        );
        self.set_value(
            nbt.get_short("Value")
                .map_or(0, |value| value.max(0) as u32),
        );
        // Vanilla reads a positive int and falls back to 1.
        self.count.store(
            nbt.get_int("Count").filter(|count| *count > 0).unwrap_or(1),
            Ordering::Relaxed,
        );
    }

    fn init_data_tracker(&self) {
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::experience_orb::DATA_VALUE,
            VarInt(self.value() as i32),
        );
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = &self.entity;
        entity.tick(caller, server);
        let world = entity.world.load();
        let colliding = !world.is_space_empty(entity.bounding_box.load());
        if entity.is_submerged_in_water() {
            entity
                .velocity
                .store(Self::underwater_velocity(entity.velocity.load()));
        } else if !colliding {
            let mut velocity = entity.velocity.load();
            velocity.y -= self.get_gravity();
            entity.velocity.store(velocity);
        }
        if world
            .get_fluid(&entity.block_pos.load())
            .has_tag(&tag::Fluid::MINECRAFT_LAVA)
        {
            // Lava spits orbs back out.
            let spread = || f64::from((rand::random::<f32>() - rand::random::<f32>()) * 0.2);
            entity
                .velocity
                .store(Vector3::new(spread(), f64::from(0.2f32), spread()));
        }
        if entity.tick_count.load(Ordering::Relaxed) % 20 == 1 {
            self.scan_for_merges();
        }
        self.follow_nearby_player();
        if colliding && self.following_player.get().is_none() {
            let bounding_box = entity.bounding_box.load();
            let velocity = entity.velocity.load();
            let ahead = BoundingBox {
                min: bounding_box.min + velocity,
                max: bounding_box.max + velocity,
            };
            if !world.is_space_empty(ahead) {
                let pos = entity.pos.load();
                entity.push_out_of_blocks(Vector3::new(
                    pos.x,
                    f64::midpoint(bounding_box.min.y, bounding_box.max.y),
                    pos.z,
                ));
            }
        }
        let velo = entity.velocity.load();

        entity.move_entity(caller, velo);

        entity.tick_block_collisions(caller);

        let on_ground = entity.on_ground.load(Ordering::Relaxed);
        let mut friction = Self::AIR_DRAG;
        if on_ground {
            let pos = entity.get_pos_with_y_offset(f64::from(0.999_999f32)).0;
            let block = entity.world.load().get_block(&pos);
            friction *= block.slipperiness;
        }
        let mut velocity = entity.velocity.load() * f64::from(friction);
        if on_ground && velo.y < -self.get_gravity() {
            velocity.y = -velo.y * 0.4;
        }
        entity.velocity.store(velocity);

        if self.orb_age.fetch_add(1, Ordering::Relaxed) + 1 >= Self::LIFETIME {
            entity.remove();
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn receives_player_touch(&self) -> bool {
        true
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.entity.is_invulnerable_to(&damage_type, cause) {
            return false;
        }

        let mut event = crate::plugin::api::events::entity::entity_damage::EntityDamageEvent::new(
            self.entity.entity_id,
            damage_type,
            amount,
        );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return false;
        }
        if !event.damage.is_finite() || event.damage <= 0.0 {
            return false;
        }

        self.entity.velocity_dirty.store(true, Ordering::Relaxed);
        let mut health = self.health.load(Ordering::Relaxed);
        loop {
            let remaining = (health as f32 - event.damage) as i32;
            match self.health.compare_exchange_weak(
                health,
                remaining,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    if remaining <= 0 {
                        self.entity.remove();
                    }
                    return true;
                }
                Err(current) => health = current,
            }
        }
    }

    fn on_player_collision(&self, player: &Arc<Player>) {
        let can_pickup = if let Ok(mut delay) = player.experience_pick_up_delay.try_lock()
            && *delay == 0
        {
            *delay = 2;
            true
        } else {
            false
        };
        // An orb with no count left is being absorbed by another one this tick.
        if can_pickup && self.count() > 0 {
            player.living_entity.pickup(&self.entity, 1);
            let remaining = player.apply_mending_from_xp(self.value() as i32);
            if remaining > 0 {
                player.add_experience_points(remaining);
            }
            // Vanilla gives one orb's worth per touch and keeps the rest of a merged pile.
            if self.count.fetch_sub(1, Ordering::Relaxed) <= 1 {
                self.entity.remove();
            }
        }
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn get_gravity(&self) -> f64 {
        0.03
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::ai::test_support::world;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn orb(world: &Arc<World>, value: u32) -> Arc<ExperienceOrbEntity> {
        Arc::new(ExperienceOrbEntity::new(
            Entity::new(
                world.clone(),
                Vector3::new(0.5, 64.0, 0.5),
                &EntityType::EXPERIENCE_ORB,
            ),
            value,
        ))
    }

    /// Two orbs whose ids are a multiple of 40 apart, as merging needs. Other tests take
    /// ids too, so the partner is searched for instead of counted to.
    fn mergeable_pair(
        world: &Arc<World>,
        value: u32,
    ) -> (Arc<ExperienceOrbEntity>, Arc<ExperienceOrbEntity>) {
        let first = orb(world, value);
        loop {
            let candidate = orb(world, value);
            if (candidate.entity.entity_id - first.entity.entity_id) % 40 == 0 {
                return (first, candidate);
            }
        }
    }

    #[tokio::test]
    async fn matching_orbs_merge_into_one_and_others_stay() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let (keeper, twin) = mergeable_pair(&world, 7);
        let (_, other_value) = mergeable_pair(&world, 3);
        let wrong_group = loop {
            let candidate = orb(&world, 7);
            if (candidate.entity.entity_id - keeper.entity.entity_id) % 40 != 0 {
                break candidate;
            }
        };
        keeper.orb_age.store(500, Ordering::Relaxed);
        twin.orb_age.store(20, Ordering::Relaxed);
        for spawned in [&keeper, &twin, &other_value, &wrong_group] {
            world.spawn_entity(spawned.clone());
        }

        keeper.scan_for_merges();
        let merged = (
            keeper.count(),
            keeper.orb_age.load(Ordering::Relaxed),
            twin.entity.is_removed(),
        );
        let untouched = (
            other_value.entity.is_removed(),
            wrong_group.entity.is_removed(),
        );
        world.level.shutdown().await;

        assert_eq!(merged, (2, 20, true));
        assert_eq!(untouched, (false, false));
        Ok(())
    }

    #[tokio::test]
    async fn two_orbs_absorbing_each_other_lose_nothing() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let (first, second) = mergeable_pair(&world, 7);
        // `second` is halfway through taking `first`: it holds the count, `first` shows 0.
        let held = first.count.swap(0, Ordering::Relaxed);
        first.absorb(&second);
        let while_claimed = (first.count(), second.count(), second.entity.is_removed());
        first.count.store(held, Ordering::Relaxed);
        world.level.shutdown().await;

        assert_eq!(while_claimed, (0, 1, false));
        Ok(())
    }

    #[tokio::test]
    async fn new_experience_joins_an_orb_already_there() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let lying = orb(&world, 7);
        lying.orb_age.store(900, Ordering::Relaxed);
        world.spawn_entity(lying.clone());
        let position = lying.entity.pos.load();

        // Only one random id group in 40 matches, so it takes a few tries.
        let joined =
            (0..4000).any(|_| ExperienceOrbEntity::try_merge_to_existing(&world, position, 7));
        let after = (lying.count(), lying.orb_age.load(Ordering::Relaxed));
        let other_value_joins =
            (0..400).any(|_| ExperienceOrbEntity::try_merge_to_existing(&world, position, 3));
        world.level.shutdown().await;

        assert!(joined);
        assert_eq!(after, (2, 0));
        assert!(!other_value_joins);
        Ok(())
    }

    #[tokio::test]
    async fn value_count_and_age_survive_saving() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let pile = orb(&world, 37);
        pile.count.store(12, Ordering::Relaxed);
        pile.orb_age.store(345, Ordering::Relaxed);
        let mut saved = NbtCompound::new();
        pile.write_custom_nbt(&mut saved);

        let loaded = orb(&world, 1);
        loaded.read_custom_nbt(&saved);
        let restored = (
            loaded.value(),
            loaded.count(),
            loaded.orb_age.load(Ordering::Relaxed),
        );
        // No count saved, or a bad one: a single orb.
        saved.put_int("Count", 0);
        loaded.read_custom_nbt(&saved);
        let bad_count = loaded.count();
        world.level.shutdown().await;

        assert_eq!(restored, (37, 12, 345));
        assert_eq!(bad_count, 1);
        Ok(())
    }

    #[test]
    fn orbs_rise_slowly_in_water() {
        let sinking = ExperienceOrbEntity::underwater_velocity(Vector3::new(1.0, -0.2, -1.0));
        assert!((sinking.x - f64::from(0.99f32)).abs() < 1e-12);
        assert!((sinking.y - (-0.2 + f64::from(5.0e-4f32))).abs() < 1e-12);
        assert!((sinking.z + f64::from(0.99f32)).abs() < 1e-12);
        // The upward speed is capped.
        let rising = ExperienceOrbEntity::underwater_velocity(Vector3::new(0.0, 0.5, 0.0));
        assert!((rising.y - f64::from(0.06f32)).abs() < 1e-12);
    }

    #[test]
    fn unstuck_selects_nearest_free_center_and_leaves_enclosed_orbs_in_place() {
        let allowed = BoundingBox {
            min: Vector3::new(0.0, 0.0, 0.0),
            max: Vector3::new(1.0, 1.0, 1.0),
        };
        let center = Vector3::new(0.5, 0.5, 0.5);
        let wall = BoundingBox {
            min: Vector3::new(-1.0, -1.0, -1.0),
            max: Vector3::new(0.75, 2.0, 2.0),
        };
        let floor = BoundingBox {
            min: Vector3::new(-1.0, -1.0, -1.0),
            max: Vector3::new(2.0, 0.625, 2.0),
        };
        assert_eq!(
            closest_free_center(allowed, center, [wall, floor].into_iter()),
            Some(Vector3::new(0.75, 0.625, 0.5))
        );
        assert_eq!(
            closest_free_center(allowed, center, [allowed].into_iter()),
            None
        );
    }
}
