use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};

use pumpkin_data::{entity::EntityType, world::WorldEvent};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use crate::{
    block::entities::BlockEntity, entity::EntityBase, entity::mob::spawn::finalize_spawn,
    world::World,
};

/// Vanilla `SpawnData`: what the spawner makes next.
pub(crate) struct SpawnData {
    /// The compound as saved: `entity`, plus `custom_spawn_rules` and `equipment` when set.
    raw: NbtCompound,
    entity_type: Option<&'static EntityType>,
    custom_spawn_rules: Option<CustomSpawnRules>,
}

/// Vanilla `SpawnData.CustomSpawnRules`: light ranges that replace the mob's own spawn rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CustomSpawnRules {
    block_light: (i32, i32),
    sky_light: (i32, i32),
}

impl CustomSpawnRules {
    /// Vanilla `LIGHT_RANGE`.
    const ANY_LIGHT: (i32, i32) = (0, 15);

    /// An `InclusiveRange` is a single number, a pair, or `min_inclusive` and `max_inclusive`.
    fn range(tag: Option<&pumpkin_nbt::tag::NbtTag>) -> (i32, i32) {
        let Some(tag) = tag else {
            return Self::ANY_LIGHT;
        };
        if let Some(value) = tag.extract_int() {
            return (value, value);
        }
        if let Some(&[min, max]) = tag.extract_int_array() {
            return (min, max);
        }
        if let Some([min, max]) = tag.extract_list()
            && let (Some(min), Some(max)) = (min.extract_int(), max.extract_int())
        {
            return (min, max);
        }
        tag.extract_compound()
            .and_then(|range| {
                Some((
                    range.get_int("min_inclusive")?,
                    range.get_int("max_inclusive")?,
                ))
            })
            .unwrap_or(Self::ANY_LIGHT)
    }

    fn from_nbt(nbt: &NbtCompound) -> Self {
        Self {
            block_light: Self::range(nbt.get("block_light_limit")),
            sky_light: Self::range(nbt.get("sky_light_limit")),
        }
    }

    /// Vanilla `isValidPosition`.
    fn is_valid_position(&self, world: &World, pos: &BlockPos) -> bool {
        let within = |(min, max): (i32, i32), light: i32| (min..=max).contains(&light);
        within(
            self.block_light,
            i32::from(world.get_block_light_level(pos).unwrap_or(0)),
        ) && within(self.sky_light, i32::from(world.get_sky_light_level(pos)))
    }
}

impl SpawnData {
    fn from_nbt(raw: &NbtCompound) -> Self {
        // Older saves keep the id directly in `SpawnData`
        let raw = if raw.get_compound("entity").is_none()
            && let Some(id) = raw.get_string("id")
        {
            Self::entity_only(id)
        } else {
            raw.clone()
        };
        Self {
            entity_type: raw
                .get_compound("entity")
                .and_then(|entity| entity.get_string("id"))
                .and_then(EntityType::from_name),
            custom_spawn_rules: raw
                .get_compound("custom_spawn_rules")
                .map(CustomSpawnRules::from_nbt),
            raw,
        }
    }

    fn entity_only(id: &str) -> NbtCompound {
        let mut entity = NbtCompound::new();
        entity.put_string("id", id.to_string());
        let mut raw = NbtCompound::new();
        raw.put_compound("entity", entity);
        raw
    }

    fn of_type(entity_type: &'static EntityType) -> Self {
        Self::from_nbt(&Self::entity_only(&format!(
            "minecraft:{}",
            entity_type.resource_name
        )))
    }

    fn entity(&self) -> Option<&NbtCompound> {
        self.raw.get_compound("entity")
    }

    /// Vanilla's test for a plain spawner: the entity tag is nothing but its `id`. Only then
    /// is the mob finalized like a natural spawn.
    fn is_id_only(&self) -> bool {
        self.entity()
            .is_none_or(|entity| entity.child_tags.len() == 1 && entity.get_string("id").is_some())
    }

    /// The `Pos` of the entity tag, which pins every spawn to one spot.
    fn fixed_position(&self) -> Option<Vector3<f64>> {
        let [x, y, z] = self.entity()?.get_list("Pos")? else {
            return None;
        };
        Some(Vector3::new(
            x.extract_double()?,
            y.extract_double()?,
            z.extract_double()?,
        ))
    }
}

/// The part of vanilla `BaseSpawner` that is not a number: `nextSpawnData` and
/// `spawnPotentials`.
/// Weight and data of every potential, and the list as saved.
type SpawnPotentials = (Vec<(i32, Arc<SpawnData>)>, Vec<pumpkin_nbt::tag::NbtTag>);

#[derive(Default)]
pub(crate) struct SpawnerData {
    next: std::sync::Mutex<Option<Arc<SpawnData>>>,
    potentials: std::sync::Mutex<SpawnPotentials>,
}

impl SpawnerData {
    pub(crate) fn of_type(entity_type: Option<&'static EntityType>) -> Self {
        let data = Self::default();
        if let Some(entity_type) = entity_type {
            data.set_entity_type(entity_type);
        }
        data
    }

    fn lock_next(&self) -> std::sync::MutexGuard<'_, Option<Arc<SpawnData>>> {
        self.next
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn read_nbt(&self, nbt: &NbtCompound) {
        let saved = nbt.get_list("SpawnPotentials").unwrap_or_default();
        let potentials: Vec<(i32, Arc<SpawnData>)> = saved
            .iter()
            .filter_map(|tag| tag.extract_compound())
            .filter_map(|entry| {
                // Older saves keep the entity directly in the entry
                let data = entry.get_compound("data").unwrap_or(entry);
                let data = SpawnData::from_nbt(data);
                data.entity_type?;
                Some((entry.get_int("weight").unwrap_or(1).max(1), Arc::new(data)))
            })
            .collect();
        let next = nbt
            .get_compound("SpawnData")
            .map(SpawnData::from_nbt)
            .filter(|data| data.entity_type.is_some())
            .map(Arc::new)
            .or_else(|| {
                nbt.get_string("EntityId")
                    .and_then(EntityType::from_name)
                    .map(|entity_type| Arc::new(SpawnData::of_type(entity_type)))
            })
            // Vanilla `getOrCreateNextSpawnData` falls back to the potentials
            .or_else(|| potentials.first().map(|(_, data)| Arc::clone(data)));
        *self.lock_next() = next;
        *self
            .potentials
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (potentials, saved.to_vec());
    }

    pub(crate) fn write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(next) = self.lock_next().as_ref() {
            nbt.put_compound("SpawnData", next.raw.clone());
        }
        let potentials = self
            .potentials
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !potentials.1.is_empty() {
            nbt.put_list("SpawnPotentials", potentials.1.clone());
        }
    }

    pub(crate) fn next(&self) -> Option<Arc<SpawnData>> {
        self.lock_next().clone()
    }

    pub(crate) fn entity_type(&self) -> Option<&'static EntityType> {
        self.lock_next().as_ref().and_then(|data| data.entity_type)
    }

    /// Vanilla `setEntityId`: only the id changes, whatever else the data holds stays.
    pub(crate) fn set_entity_type(&self, entity_type: &'static EntityType) {
        let mut next = self.lock_next();
        let id = format!("minecraft:{}", entity_type.resource_name);
        let mut raw = next
            .as_ref()
            .map_or_else(NbtCompound::new, |data| data.raw.clone());
        let mut entity = raw.get_compound("entity").cloned().unwrap_or_default();
        entity.put_string("id", id);
        raw.put_compound("entity", entity);
        *next = Some(Arc::new(SpawnData::from_nbt(&raw)));
    }

    /// The potentials half of vanilla `delay`: a weighted pick becomes the next spawn data.
    fn pick_next(&self) {
        let picked = {
            let potentials = self
                .potentials
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let total: i32 = potentials.0.iter().map(|(weight, _)| weight).sum();
            if total <= 0 {
                return;
            }
            let mut roll = rand::random_range(0..total);
            potentials
                .0
                .iter()
                .find(|(weight, _)| {
                    roll -= weight;
                    roll < 0
                })
                .map(|(_, data)| Arc::clone(data))
        };
        if let Some(picked) = picked {
            *self.lock_next() = Some(picked);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SpawnerParams {
    pub delay: i32,
    pub min_delay: i32,
    pub max_delay: i32,
    pub spawn_count: i32,
    pub spawn_range: i32,
    pub max_nearby_entities: i32,
    pub required_player_range: i32,
}

pub(crate) fn write_spawner_params_nbt(params: &SpawnerParams, nbt: &mut NbtCompound) {
    nbt.put_short("Delay", params.delay as i16);
    nbt.put_short("MinSpawnDelay", params.min_delay as i16);
    nbt.put_short("MaxSpawnDelay", params.max_delay as i16);
    nbt.put_short("SpawnCount", params.spawn_count as i16);
    nbt.put_short("SpawnRange", params.spawn_range as i16);
    nbt.put_short("MaxNearbyEntities", params.max_nearby_entities as i16);
    nbt.put_short("RequiredPlayerRange", params.required_player_range as i16);
}

pub(crate) fn parse_spawner_nbt(nbt: &NbtCompound) -> SpawnerParams {
    let get_num = |name: &str| {
        nbt.get_short(name)
            .map(i32::from)
            .or_else(|| nbt.get_int(name))
            .or_else(|| nbt.get_byte(name).map(i32::from))
    };

    let delay = get_num("Delay").unwrap_or(MobSpawnerBlockEntity::DEFAULT_DELAY);
    let min_delay =
        get_num("MinSpawnDelay").unwrap_or(MobSpawnerBlockEntity::DEFAULT_MIN_SPAWN_DELAY);
    let max_delay =
        get_num("MaxSpawnDelay").unwrap_or(MobSpawnerBlockEntity::DEFAULT_MAX_SPAWN_DELAY);
    let spawn_count = get_num("SpawnCount").unwrap_or(MobSpawnerBlockEntity::DEFAULT_SPAWN_COUNT);
    let spawn_range = get_num("SpawnRange").unwrap_or(MobSpawnerBlockEntity::DEFAULT_SPAWN_RANGE);
    let max_nearby_entities =
        get_num("MaxNearbyEntities").unwrap_or(MobSpawnerBlockEntity::DEFAULT_MAX_NEARBY_ENTITIES);
    let required_player_range = get_num("RequiredPlayerRange")
        .unwrap_or(MobSpawnerBlockEntity::DEFAULT_REQUIRED_PLAYER_RANGE);

    SpawnerParams {
        delay,
        min_delay,
        max_delay,
        spawn_count,
        spawn_range,
        max_nearby_entities,
        required_player_range,
    }
}

fn reset_spawner_delay<F: FnOnce()>(
    delay: &AtomicI32,
    min_delay: i32,
    max_delay: i32,
    on_reset: F,
) {
    delay.store(
        if max_delay <= min_delay {
            min_delay
        } else {
            min_delay + rand::random_range(0..max_delay - min_delay)
        },
        Ordering::Relaxed,
    );
    on_reset();
}

/// Vanilla `Mob.equip(EquipmentTable)`: rolls the loot table and puts each item in its slot.
// TODO vanilla also takes the drop chances of the equipment table.
pub(crate) fn equip_from_loot_table(world: &Arc<World>, entity: &Arc<dyn EntityBase>, key: &str) {
    let Some(loot_table) = world.get_loot_table(key) else {
        return;
    };
    let Some(living) = entity.get_living_entity() else {
        return;
    };
    let items = loot_table.generate_loot_with_context(
        rand::random::<i64>(),
        &crate::world::loot::LootContextParameters {
            world: Some(world),
            ..Default::default()
        },
    );
    let mut equipment = living
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut equipment_changes = Vec::new();
    for item in items {
        let slot = item
            .get_data_component::<pumpkin_data::data_component_impl::EquippableImpl>()
            .map_or(
                pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND,
                |equippable| equippable.slot.clone(),
            );
        equipment.put(&slot, item.clone());
        equipment_changes.push((slot, item));
    }
    drop(equipment);
    living.send_equipment_changes(&equipment_changes);
}

enum SpawnAttempt {
    Spawned,
    /// A plugin cancelled it, which still restarts the wait.
    Cancelled,
    NoRoom,
}

/// One pass of the loop in vanilla `BaseSpawner.serverTick`.
fn spawn_one(
    world: &Arc<World>,
    position: BlockPos,
    spawn_range: i32,
    spawn_data: &SpawnData,
    entity_type: &'static EntityType,
) -> SpawnAttempt {
    let pos = position.0;

    let spawn_pos = spawn_data.fixed_position().unwrap_or_else(|| {
        Vector3::new(
            f64::from(pos.x)
                + (rand::random::<f64>() - rand::random::<f64>()) * f64::from(spawn_range)
                + 0.5,
            f64::from(pos.y + rand::random_range(0..3) - 1),
            f64::from(pos.z)
                + (rand::random::<f64>() - rand::random::<f64>()) * f64::from(spawn_range)
                + 0.5,
        )
    });
    if !world.is_space_empty(entity_type.get_spawn_bounding_box(
        spawn_pos.x,
        spawn_pos.y,
        spawn_pos.z,
    )) {
        return SpawnAttempt::NoRoom;
    }
    let spawn_block_pos = BlockPos::floored_v(spawn_pos);
    let allowed = spawn_data.custom_spawn_rules.map_or_else(
        || {
            crate::entity::r#type::check_spawn_rules(
                entity_type,
                world,
                &spawn_block_pos,
                world.is_thundering(),
                crate::entity::mob::spawn::EntitySpawnReason::Spawner,
            )
        },
        // Custom rules replace the mob's own, but hostile mobs still stay out of peaceful
        |rules| {
            (entity_type.category.is_friendly
                || world.level_info.load().difficulty != pumpkin_util::Difficulty::Peaceful)
                && rules.is_valid_position(world, &spawn_block_pos)
        },
    );
    if !allowed {
        return SpawnAttempt::NoRoom;
    }
    let id_only = spawn_data.is_id_only();
    let loaded = if id_only {
        None
    } else {
        spawn_data.entity().and_then(|entity| {
            crate::entity::r#type::load_entity_from_nbt(entity, spawn_pos, world)
        })
    };
    let entity = loaded.unwrap_or_else(|| {
        crate::entity::r#type::from_type(entity_type, spawn_pos, world, uuid::Uuid::new_v4())
    });
    let yaw = rand::random::<f32>() * 360.0;
    entity.get_entity().set_rotation(yaw, 0.0);

    let mut event = crate::plugin::api::events::entity::spawner_spawn::SpawnerSpawnEvent::new(
        entity.get_entity().entity_id,
        position,
    );
    if let Some(server) = world.server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    if event.cancelled {
        return SpawnAttempt::Cancelled;
    }

    // A mob described by more than its id comes as described
    if id_only {
        finalize_spawn(
            &entity,
            world,
            crate::entity::mob::spawn::EntitySpawnReason::Spawner,
            None,
        );
    }
    if let Some(loot_table) = spawn_data
        .raw
        .get_compound("equipment")
        .and_then(|equipment| equipment.get_string("loot_table"))
    {
        equip_from_loot_table(world, &entity, loot_table);
    }
    let entity_pos = entity.get_entity().pos.load();
    world.spawn_entity(entity);
    world.emit_game_event(
        pumpkin_data::game_event::GameEvent::EntityPlace.name(),
        entity_pos,
    );
    world.sync_world_event(WorldEvent::ParticlesMobblockSpawn, position, 0);
    SpawnAttempt::Spawned
}

pub(crate) fn tick_spawner<F: FnOnce()>(
    world: &Arc<World>,
    position: BlockPos,
    delay: &AtomicI32,
    params: &SpawnerParams,
    data: &SpawnerData,
    on_reset: F,
) {
    // Vanilla `isSpawnerBlockEnabled`
    if !world.level_info.load().game_rules.spawner_blocks_work {
        return;
    }
    let Some(spawn_data) = data.next() else {
        return;
    };
    let Some(entity_type) = spawn_data.entity_type else {
        return;
    };
    // Vanilla `delay`: a new wait, and the next of the potentials
    let reset_spawner_delay = |delay: &AtomicI32, min_delay: i32, max_delay: i32, on_reset: F| {
        reset_spawner_delay(delay, min_delay, max_delay, on_reset);
        data.pick_next();
    };

    let center = position.to_centered_f64();
    let max_player_dist_sq = f64::from(params.required_player_range).powi(2);
    let player_nearby =
        world.players.load().iter().any(|p| {
            p.get_entity().pos.load().squared_distance_to_vec(&center) <= max_player_dist_sq
        });

    if !player_nearby {
        return;
    }

    if delay.load(Ordering::Relaxed) < 0 {
        reset_spawner_delay(delay, params.min_delay, params.max_delay, on_reset);
        return;
    }
    if delay.load(Ordering::Relaxed) > 0 {
        delay.fetch_sub(1, Ordering::Relaxed);
        return;
    }

    // Vanilla counts its own kind in the spawner's block grown by the spawn range
    let nearby_area = pumpkin_util::math::boundingbox::BoundingBox::from_block(&position)
        .expand_all(f64::from(params.spawn_range));
    let nearby_count = world
        .get_entities_at_box(&nearby_area)
        .iter()
        .filter(|entity| entity.get_entity().entity_type.id == entity_type.id)
        .count();

    if nearby_count as i32 >= params.max_nearby_entities {
        reset_spawner_delay(delay, params.min_delay, params.max_delay, on_reset);
        return;
    }

    let spawn_range = params.spawn_range;
    let mut spawned_any = false;
    let mut cancelled_any = false;
    for _ in 0..params.spawn_count {
        match spawn_one(world, position, spawn_range, &spawn_data, entity_type) {
            SpawnAttempt::Spawned => spawned_any = true,
            SpawnAttempt::Cancelled => cancelled_any = true,
            SpawnAttempt::NoRoom => {}
        }
    }
    if spawned_any || cancelled_any {
        reset_spawner_delay(delay, params.min_delay, params.max_delay, on_reset);
    }
}

pub struct MobSpawnerBlockEntity {
    pub position: BlockPos,
    pub delay: AtomicI32,
    pub max_delay: i32,
    pub min_delay: i32,
    pub spawn_count: i32,
    pub spawn_range: i32,
    pub max_nearby_entities: i32,
    pub required_player_range: i32,
    pub(crate) data: SpawnerData,
}

impl MobSpawnerBlockEntity {
    pub const ID: &'static str = "minecraft:mob_spawner";
    pub const DEFAULT_DELAY: i32 = 20;
    pub const DEFAULT_MAX_SPAWN_DELAY: i32 = 800;
    pub const DEFAULT_MIN_SPAWN_DELAY: i32 = 200;
    pub const DEFAULT_SPAWN_COUNT: i32 = 4;
    pub const DEFAULT_SPAWN_RANGE: i32 = 4;
    pub const DEFAULT_MAX_NEARBY_ENTITIES: i32 = 6;
    pub const DEFAULT_REQUIRED_PLAYER_RANGE: i32 = 16;

    #[must_use]
    pub fn new(position: BlockPos, entity_type: Option<&'static EntityType>) -> Self {
        Self {
            position,
            delay: AtomicI32::new(Self::DEFAULT_DELAY),
            max_delay: Self::DEFAULT_MAX_SPAWN_DELAY,
            min_delay: Self::DEFAULT_MIN_SPAWN_DELAY,
            spawn_count: Self::DEFAULT_SPAWN_COUNT,
            spawn_range: Self::DEFAULT_SPAWN_RANGE,
            max_nearby_entities: Self::DEFAULT_MAX_NEARBY_ENTITIES,
            required_player_range: Self::DEFAULT_REQUIRED_PLAYER_RANGE,
            data: SpawnerData::of_type(entity_type),
        }
    }

    fn params(&self) -> SpawnerParams {
        SpawnerParams {
            delay: self.delay.load(Ordering::Relaxed),
            min_delay: self.min_delay,
            max_delay: self.max_delay,
            spawn_count: self.spawn_count,
            spawn_range: self.spawn_range,
            max_nearby_entities: self.max_nearby_entities,
            required_player_range: self.required_player_range,
        }
    }

    pub fn write_spawner_nbt(&self, nbt: &mut NbtCompound) {
        write_spawner_params_nbt(&self.params(), nbt);
        self.data.write_nbt(nbt);
    }

    pub fn set_entity_type(&self, entity_type: &'static EntityType) {
        self.data.set_entity_type(entity_type);
    }
}

impl BlockEntity for MobSpawnerBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn tick(&self, world: &Arc<World>) {
        let params = self.params();
        tick_spawner(
            world,
            self.position,
            &self.delay,
            &params,
            &self.data,
            || {
                world.add_synced_block_event(self.position, 1, 0);
            },
        );
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let params = parse_spawner_nbt(nbt);
        let data = SpawnerData::default();
        data.read_nbt(nbt);
        Self {
            position,
            delay: AtomicI32::new(params.delay),
            max_delay: params.max_delay,
            min_delay: params.min_delay,
            spawn_count: params.spawn_count,
            spawn_range: params.spawn_range,
            max_nearby_entities: params.max_nearby_entities,
            required_player_range: params.required_player_range,
            data,
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_spawner_nbt(nbt);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut final_nbt = NbtCompound::new();
        self.write_spawner_nbt(&mut final_nbt);
        Some(final_nbt)
    }

    fn bedrock_block_actor_data(
        &self,
        _state_id: pumpkin_data::BlockStateId,
    ) -> Option<NbtCompound> {
        let entity_id = self.data.entity_type().map_or_else(String::new, |et| {
            super::bedrock::bedrock_item_name(et.resource_name)
        });
        Some(
            super::bedrock::BedrockBlockActorBuilder::new("MobSpawner", self.position)
                .movable(false)
                .put_string("EntityIdentifier", entity_id)
                .put_short("Delay", self.delay.load(Ordering::Relaxed) as i16)
                .put_short("MinSpawnDelay", self.min_delay as i16)
                .put_short("MaxSpawnDelay", self.max_delay as i16)
                .put_short("SpawnCount", self.spawn_count as i16)
                .put_short("MaxNearbyEntities", self.max_nearby_entities as i16)
                .put_short("RequiredPlayerRange", self.required_player_range as i16)
                .put_short("SpawnRange", self.spawn_range as i16)
                .put_float("DisplayEntityWidth", 0.8)
                .put_float("DisplayEntityHeight", 1.8)
                .put_float("DisplayEntityScale", 1.0)
                .finish(),
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_nbt::tag::NbtTag;

    use super::*;

    fn entity(id: &str) -> NbtCompound {
        let mut entity = NbtCompound::new();
        entity.put_string("id", id.to_string());
        entity
    }

    fn spawn_data(entity: NbtCompound) -> NbtCompound {
        let mut data = NbtCompound::new();
        data.put_compound("entity", entity);
        data
    }

    fn spawner_with(data: NbtCompound) -> SpawnerData {
        let mut nbt = NbtCompound::new();
        nbt.put_compound("SpawnData", data);
        let spawner = SpawnerData::default();
        spawner.read_nbt(&nbt);
        spawner
    }

    #[test]
    fn configured_spawn_data_survives_a_save() {
        let mut zombie = entity("minecraft:zombie");
        zombie.put_bool("NoAI", true);
        let mut data = spawn_data(zombie);
        data.put_compound("custom_spawn_rules", NbtCompound::new());
        let spawner = spawner_with(data.clone());

        let mut saved = NbtCompound::new();
        spawner.write_nbt(&mut saved);
        let next = spawner.next();

        assert_eq!(saved.get_compound("SpawnData"), Some(&data));
        assert_eq!(spawner.entity_type(), Some(&EntityType::ZOMBIE));
        assert!(next.as_ref().is_some_and(|next| !next.is_id_only()));
        assert_eq!(
            next.and_then(|next| next.custom_spawn_rules),
            Some(CustomSpawnRules {
                block_light: CustomSpawnRules::ANY_LIGHT,
                sky_light: CustomSpawnRules::ANY_LIGHT,
            })
        );
    }

    #[test]
    fn plain_spawn_data_is_id_only_and_has_no_fixed_position() {
        let spawner = spawner_with(spawn_data(entity("minecraft:pig")));
        let next = spawner.next();

        assert!(next.as_ref().is_some_and(|next| next.is_id_only()));
        assert!(
            next.as_ref()
                .is_some_and(|next| next.fixed_position().is_none())
        );
        assert!(next.is_some_and(|next| next.custom_spawn_rules.is_none()));
    }

    #[test]
    fn entity_pos_pins_the_spawn_position() {
        let mut stand = entity("minecraft:armor_stand");
        stand.put_list(
            "Pos",
            vec![
                NbtTag::Double(1.5),
                NbtTag::Double(64.0),
                NbtTag::Double(-2.5),
            ],
        );
        let spawner = spawner_with(spawn_data(stand));

        assert_eq!(
            spawner.next().and_then(|next| next.fixed_position()),
            Some(Vector3::new(1.5, 64.0, -2.5))
        );
    }

    #[tokio::test]
    async fn described_entity_spawns_where_and_how_its_data_says()
    -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let world = crate::entity::ai::test_support::world(&folder);
        let mut stand = entity("minecraft:armor_stand");
        stand.put_bool("NoGravity", true);
        stand.put_list(
            "Pos",
            vec![
                NbtTag::Double(8.5),
                NbtTag::Double(70.0),
                NbtTag::Double(8.5),
            ],
        );
        let mut data = spawn_data(stand);
        data.put_compound("custom_spawn_rules", NbtCompound::new());
        let data = SpawnData::from_nbt(&data);

        let attempt = spawn_one(
            &world,
            BlockPos::new(0, 64, 0),
            4,
            &data,
            &EntityType::ARMOR_STAND,
        );
        let spawned: Vec<_> = world
            .entities
            .load()
            .iter()
            .map(|entity| {
                let base = entity.get_entity();
                (base.entity_type, base.pos.load(), base.has_no_gravity())
            })
            .collect();
        world.level.shutdown().await;

        assert!(matches!(attempt, SpawnAttempt::Spawned));
        assert_eq!(
            spawned,
            vec![(&EntityType::ARMOR_STAND, Vector3::new(8.5, 70.0, 8.5), true)]
        );
        Ok(())
    }

    #[test]
    fn light_limits_read_every_range_form() {
        let mut rules = NbtCompound::new();
        rules.put("block_light_limit", NbtTag::Int(7));
        rules.put("sky_light_limit", NbtTag::IntArray(vec![2, 9]));
        let mut range = NbtCompound::new();
        range.put_int("min_inclusive", 3);
        range.put_int("max_inclusive", 4);
        let mut compound_rules = NbtCompound::new();
        compound_rules.put_compound("block_light_limit", range);

        let rules = CustomSpawnRules::from_nbt(&rules);
        let compound_rules = CustomSpawnRules::from_nbt(&compound_rules);

        assert_eq!(rules.block_light, (7, 7));
        assert_eq!(rules.sky_light, (2, 9));
        assert_eq!(compound_rules.block_light, (3, 4));
        assert_eq!(compound_rules.sky_light, CustomSpawnRules::ANY_LIGHT);
    }

    #[test]
    fn changing_the_entity_keeps_the_rest_of_the_spawn_data() {
        let mut zombie = entity("minecraft:zombie");
        zombie.put_bool("NoAI", true);
        let spawner = spawner_with(spawn_data(zombie));
        spawner.set_entity_type(&EntityType::SKELETON);
        let next = spawner.next();
        let entity = next.as_ref().and_then(|next| next.entity());

        assert_eq!(spawner.entity_type(), Some(&EntityType::SKELETON));
        assert_eq!(
            entity.and_then(|entity| entity.get_string("id")),
            Some("minecraft:skeleton")
        );
        assert_eq!(
            entity.and_then(|entity| entity.get_bool("NoAI")),
            Some(true)
        );
    }

    #[test]
    fn the_next_spawn_comes_from_the_potentials() {
        let mut potential = NbtCompound::new();
        potential.put_int("weight", 3);
        potential.put_compound("data", spawn_data(entity("minecraft:spider")));
        let mut nbt = NbtCompound::new();
        nbt.put_compound("SpawnData", spawn_data(entity("minecraft:pig")));
        nbt.put_list("SpawnPotentials", vec![NbtTag::Compound(potential)]);
        let spawner = SpawnerData::default();
        spawner.read_nbt(&nbt);
        let before = spawner.entity_type();
        spawner.pick_next();
        let mut saved = NbtCompound::new();
        spawner.write_nbt(&mut saved);

        assert_eq!(before, Some(&EntityType::PIG));
        assert_eq!(spawner.entity_type(), Some(&EntityType::SPIDER));
        assert_eq!(
            saved.get_list("SpawnPotentials").map(<[NbtTag]>::len),
            Some(1)
        );
    }
}
