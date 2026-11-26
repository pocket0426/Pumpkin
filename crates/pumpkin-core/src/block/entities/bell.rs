use crate::block::entities::BlockEntity;
use crate::entity::EntityBase;
use crate::entity::ai::brain::memory::types;
use crate::world::World;
use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::HorizontalFacing;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::any::Any;
use std::sync::{Arc, Mutex};

/// Vanilla `BellBlockEntity.GLOW_DURATION`.
const GLOW_DURATION: i32 = 60;
/// Vanilla `BellBlockEntity.MIN_TICKS_BETWEEN_SEARCHES`.
const MIN_TICKS_BETWEEN_SEARCHES: i64 = 60;
/// Vanilla `BellBlockEntity.SEARCH_RADIUS`.
const SEARCH_RADIUS: f64 = 48.0;
/// Vanilla `BellBlockEntity.HEAR_BELL_RADIUS`.
const HEAR_BELL_RADIUS: f64 = 32.0;
/// Vanilla `BellBlockEntity.HIGHLIGHT_RAIDERS_RADIUS`.
const HIGHLIGHT_RAIDERS_RADIUS: f64 = 48.0;

pub struct BellBlockEntity {
    pub position: BlockPos,
    pub last_side_hit: AtomicCell<Option<HorizontalFacing>>,
    pub ring_ticks: AtomicCell<i32>,
    pub ringing: AtomicCell<bool>,
    resonating: AtomicCell<bool>,
    resonate_time: AtomicCell<i32>,
    /// Vanilla `lastRingTimestamp`.
    last_ring_timestamp: AtomicCell<i64>,
    /// Vanilla `nearbyEntities`: the living entities found by the latest search, `None` before
    /// the first ring.
    nearby_entities: Mutex<Option<Vec<Arc<dyn EntityBase>>>>,
}

impl BellBlockEntity {
    pub const ID: &'static str = "minecraft:bell";
    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            last_side_hit: AtomicCell::new(None),
            ring_ticks: AtomicCell::new(0),
            resonate_time: AtomicCell::new(0),
            resonating: AtomicCell::new(false),
            ringing: AtomicCell::new(false),
            last_ring_timestamp: AtomicCell::new(0),
            nearby_entities: Mutex::new(None),
        }
    }
    pub fn activate(&self, direction: HorizontalFacing) {
        self.last_side_hit.store(Some(direction));
        if self.ringing.load() {
            self.ring_ticks.store(0);
        } else {
            self.ringing.store(true);
        }
    }
    /// Vanilla `triggerEvent(1, direction)`: what the queued ring does once its block event runs.
    pub fn on_ring_event(&self, world: &Arc<World>, direction: Option<HorizontalFacing>) {
        self.update_entities(world);
        self.resonate_time.store(0);
        self.last_side_hit.store(direction);
        self.ring_ticks.store(0);
        self.ringing.store(true);
    }

    fn center(&self) -> Vector3<f64> {
        self.position.to_centered_f64()
    }

    /// Alive, not removed and closer than `radius` to the middle of the bell.
    fn hears(&self, entity: &dyn EntityBase, radius: f64) -> bool {
        let base = entity.get_entity();
        entity
            .get_living_entity()
            .is_some_and(crate::entity::living::LivingEntity::is_alive)
            && !base.is_removed()
            && base.pos.load().squared_distance_to_vec(&self.center()) < radius * radius
    }

    fn is_raider_within(&self, entity: &dyn EntityBase, radius: f64) -> bool {
        self.hears(entity, radius)
            && entity
                .get_entity()
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_RAIDERS)
    }

    /// Vanilla `updateEntities`: looks for listeners at most every three seconds, and tells
    /// every one in earshot when it heard the bell.
    fn update_entities(&self, world: &Arc<World>) {
        let time = world.get_world_age();
        let mut nearby = self
            .nearby_entities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if time > self.last_ring_timestamp.load() + MIN_TICKS_BETWEEN_SEARCHES || nearby.is_none() {
            self.last_ring_timestamp.store(time);
            let area = BoundingBox::from_block(&self.position).expand_all(SEARCH_RADIUS);
            *nearby = Some(
                world
                    .get_all_at_box(&area)
                    .into_iter()
                    .filter(|entity| entity.get_living_entity().is_some())
                    .collect(),
            );
        }
        for entity in nearby.iter().flatten() {
            if !self.hears(entity.as_ref(), HEAR_BELL_RADIUS) {
                continue;
            }
            // Only a brain that knows the memory keeps it, which is the villagers
            if let Some(mob) = entity.get_mob() {
                mob.get_mob_entity()
                    .with_existing_brain(|brain| brain.set(types::HEARD_BELL_TIME, time));
            }
        }
    }

    /// Vanilla `areRaidersNearby`.
    fn raiders_hear_bell(&self) -> bool {
        self.nearby_entities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .flatten()
            .any(|entity| self.is_raider_within(entity.as_ref(), HEAR_BELL_RADIUS))
    }

    /// Vanilla `makeRaidersGlow`.
    fn make_raiders_glow(&self) {
        let nearby = self
            .nearby_entities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entity in nearby.iter().flatten() {
            if !self.is_raider_within(entity.as_ref(), HIGHLIGHT_RAIDERS_RADIUS) {
                continue;
            }
            if let Some(living) = entity.get_living_entity() {
                living.add_effect(Effect {
                    effect_type: &StatusEffect::GLOWING,
                    duration: GLOW_DURATION,
                    amplifier: 0,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }
        }
    }
}

impl BlockEntity for BellBlockEntity {
    fn write_nbt(&self, _nbt: &mut NbtCompound) {}

    fn from_nbt(_nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        Self::new(position)
    }

    fn tick(&self, world: &Arc<World>) {
        if self.ringing.load() {
            self.ring_ticks.fetch_add(1);
        }
        if self.ring_ticks.load() >= 50 {
            self.ringing.store(false);
            self.ring_ticks.store(0);
        }
        if self.ring_ticks.load() >= 5 && self.resonate_time.load() == 0 && self.raiders_hear_bell()
        {
            self.resonating.store(true);
            world.play_sound_fine(
                Sound::BlockBellResonate,
                SoundCategory::Blocks,
                &self.position.to_f64(),
                1.0,
                1.0,
            );
        }

        if self.resonating.load() {
            if self.resonate_time.load() < 40 {
                self.resonate_time.fetch_add(1);
            } else {
                self.make_raiders_glow();
                self.resonating.store(false);
            }
        }
    }

    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn bedrock_block_actor_data(
        &self,
        _state_id: pumpkin_data::BlockStateId,
    ) -> Option<NbtCompound> {
        let direction = match self.last_side_hit.load() {
            Some(HorizontalFacing::South) => 0,
            Some(HorizontalFacing::West) => 1,
            Some(HorizontalFacing::North) => 2,
            Some(HorizontalFacing::East) => 3,
            None => 255,
        };
        Some(
            super::bedrock::BedrockBlockActorBuilder::new("Bell", self.position)
                .movable(false)
                .put_bool("Ringing", self.ringing.load())
                .put_int("Ticks", self.ring_ticks.load())
                .put_int("Direction", direction)
                .finish(),
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
