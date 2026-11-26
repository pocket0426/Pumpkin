use std::sync::atomic::{AtomicI32, AtomicI64, AtomicU8, Ordering};

use crate::entity::{Entity, EntityBase, living::LivingEntity};
use crossbeam::atomic::AtomicCell;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::{
    Block,
    damage::DamageType,
    data_component_impl::{EquipmentSlot, EquipmentType},
    entity::{EntityStatus, EntityType},
    item::Item,
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::{
    boundingbox::EntityDimensions, euler_angle::EulerAngle, vector3::Vector3,
};

#[derive(Debug, Clone, Copy)]
pub struct PackedRotation {
    pub head: EulerAngle,
    pub body: EulerAngle,
    pub left_arm: EulerAngle,
    pub right_arm: EulerAngle,
    pub left_leg: EulerAngle,
    pub right_leg: EulerAngle,
}

impl Default for PackedRotation {
    fn default() -> Self {
        Self {
            head: EulerAngle::new(0.0, 0.0, 0.0),
            body: EulerAngle::new(0.0, 0.0, 0.0),
            left_arm: EulerAngle::new(-10.0, 0.0, -10.0),
            right_arm: EulerAngle::new(-15.0, 0.0, 10.0),
            left_leg: EulerAngle::new(-1.0, 0.0, -1.0),
            right_leg: EulerAngle::new(1.0, 0.0, 1.0),
        }
    }
}

impl From<PackedRotation> for NbtTag {
    fn from(val: PackedRotation) -> Self {
        let mut compound = NbtCompound::new();
        compound.put("Head", val.head);
        compound.put("Body", val.body);
        compound.put("LeftArm", val.left_arm);
        compound.put("RightArm", val.right_arm);
        compound.put("LeftLeg", val.left_leg);
        compound.put("RightLeg", val.right_leg);
        Self::Compound(compound)
    }
}

impl From<NbtTag> for PackedRotation {
    #[expect(clippy::unnecessary_fallible_conversions)]
    fn from(tag: NbtTag) -> Self {
        if let NbtTag::Compound(compound) = tag {
            fn get_rotation(
                compound: &NbtCompound,
                key: &'static str,
                default: EulerAngle,
            ) -> EulerAngle {
                compound
                    .get(key)
                    .and_then(|tag| tag.clone().try_into().ok())
                    .unwrap_or(default)
            }

            let default = Self::default();

            Self {
                head: get_rotation(&compound, "Head", default.head),
                body: get_rotation(&compound, "Body", default.body),
                left_arm: get_rotation(&compound, "LeftArm", default.left_arm),
                right_arm: get_rotation(&compound, "RightArm", default.right_arm),
                left_leg: get_rotation(&compound, "LeftLeg", default.left_leg),
                right_leg: get_rotation(&compound, "RightLeg", default.right_leg),
            }
        } else {
            Self::default()
        }
    }
}

pub struct ArmorStandEntity {
    living_entity: LivingEntity,

    armor_stand_flags: AtomicU8,
    last_hit_time: AtomicI64,
    disabled_slots: AtomicI32,

    rotation: AtomicCell<PackedRotation>,
}

impl ArmorStandEntity {
    pub fn new(entity: Entity) -> Self {
        let living_entity = LivingEntity::new(entity);
        let packed_rotation = PackedRotation::default();

        Self {
            living_entity,
            armor_stand_flags: AtomicU8::new(0),
            last_hit_time: AtomicI64::new(0),
            disabled_slots: AtomicI32::new(0),
            rotation: AtomicCell::new(packed_rotation),
        }
    }

    pub fn set_small(&self, small: bool) {
        self.set_bit_field(ArmorStandFlags::Small, small);
    }

    pub fn is_small(&self) -> bool {
        (self.armor_stand_flags.load(Ordering::Relaxed) & ArmorStandFlags::Small as u8) != 0
    }

    pub fn set_show_arms(&self, show_arms: bool) {
        self.set_bit_field(ArmorStandFlags::ShowArms, show_arms);
    }

    pub fn should_show_arms(&self) -> bool {
        (self.armor_stand_flags.load(Ordering::Relaxed) & ArmorStandFlags::ShowArms as u8) != 0
    }

    pub fn set_hide_base_plate(&self, hide_base_plate: bool) {
        self.set_bit_field(ArmorStandFlags::HideBasePlate, hide_base_plate);
    }

    pub fn should_show_base_plate(&self) -> bool {
        (self.armor_stand_flags.load(Ordering::Relaxed) & ArmorStandFlags::HideBasePlate as u8) == 0
    }

    pub fn set_marker(&self, marker: bool) {
        self.set_bit_field(ArmorStandFlags::Marker, marker);
    }

    pub fn is_marker(&self) -> bool {
        (self.armor_stand_flags.load(Ordering::Relaxed) & ArmorStandFlags::Marker as u8) != 0
    }

    fn set_bit_field(&self, bit_field: ArmorStandFlags, set: bool) {
        let current = self.armor_stand_flags.load(Ordering::Relaxed);
        let new_value = if set {
            current | bit_field as u8
        } else {
            current & !(bit_field as u8)
        };
        self.armor_stand_flags.store(new_value, Ordering::Relaxed);
        self.update_dimensions();
    }

    /// Vanilla `ArmorStand.getDefaultDimensions`: a marker has no size, a small stand half.
    fn update_dimensions(&self) {
        let dimensions = if self.is_marker() {
            EntityDimensions::new(0.0, 0.0, 0.0)
        } else if self.is_small() {
            let size = Entity::type_dimensions(&EntityType::ARMOR_STAND).scale(0.5);
            EntityDimensions::new(size.width, size.height, 0.9875)
        } else {
            Entity::type_dimensions(&EntityType::ARMOR_STAND)
        };
        self.living_entity.entity.set_default_dimensions(dimensions);
    }

    pub fn can_use_slot(&self, slot: &EquipmentSlot) -> bool {
        !matches!(slot, EquipmentSlot::Body(_) | EquipmentSlot::Saddle(_))
            && !self.is_slot_disabled(slot)
    }

    pub fn is_slot_disabled(&self, slot: &EquipmentSlot) -> bool {
        let disabled_slots = self.disabled_slots.load(Ordering::Relaxed);
        let slot_bit = 1 << slot.get_offset_entity_slot_id(0);

        (disabled_slots & slot_bit) != 0
            || (slot.slot_type() == EquipmentType::Hand && !self.should_show_arms())
    }

    pub fn set_slot_disabled(&self, slot: &EquipmentSlot, disabled: bool) {
        let slot_bit = 1 << slot.get_offset_entity_slot_id(0);
        let current = self.disabled_slots.load(Ordering::Relaxed);

        let new_val = if disabled {
            current | slot_bit
        } else {
            current & !slot_bit
        };

        self.disabled_slots.store(new_val, Ordering::Relaxed);
    }

    pub fn is_invisible(&self) -> bool {
        self.get_entity().invisible.load(Ordering::Relaxed)
    }

    pub fn pack_rotation(&self) -> PackedRotation {
        self.rotation.load()
    }

    pub fn unpack_rotation(&self, packed: &PackedRotation) {
        self.rotation.store(packed.to_owned());
    }

    /// Vanilla `ArmorStand.kill`: gone, with the death game event.
    fn kill_stand(&self) {
        let entity = self.get_entity();
        entity.remove();
        entity.world.load().emit_game_event(
            pumpkin_data::game_event::GameEvent::EntityDie.name(),
            entity.pos.load(),
        );
    }

    fn play_broken_sound(&self) {
        let entity = self.get_entity();
        entity.world.load().play_sound(
            Sound::EntityArmorStandBreak,
            SoundCategory::Neutral,
            &entity.pos.load(),
        );
    }

    /// Vanilla `brokenByPlayer`: the stand drops as an item that keeps its name, then
    /// everything it wore.
    fn broken_by_player(&self) {
        let entity = self.get_entity();
        let mut stand = ItemStack::new(1, &Item::ARMOR_STAND);
        if let Some(name) = entity.custom_name.load().as_ref() {
            stand.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
                name: name.clone(),
            });
        }
        entity
            .world
            .load_full()
            .drop_stack(&entity.block_pos.load(), stand);
        self.broken_by_anything();
    }

    /// Vanilla `brokenByAnything`: the break sound, and what it wore drops a block higher.
    /// An item with the curse of vanishing is lost.
    fn broken_by_anything(&self) {
        const SLOTS: [EquipmentSlot; 8] = [
            EquipmentSlot::MAIN_HAND,
            EquipmentSlot::OFF_HAND,
            EquipmentSlot::FEET,
            EquipmentSlot::LEGS,
            EquipmentSlot::CHEST,
            EquipmentSlot::HEAD,
            EquipmentSlot::BODY,
            EquipmentSlot::SADDLE,
        ];
        self.play_broken_sound();
        let worn: Vec<ItemStack> = {
            let mut equipment = self
                .living_entity
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            SLOTS
                .iter()
                .map(|slot| equipment.put(slot, ItemStack::EMPTY.clone()))
                .collect()
        };
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let above = entity.block_pos.load().up();
        for item in worn {
            if !item.is_empty() && !crate::entity::living::prevents_equipment_drop(&item) {
                world.drop_stack(&above, item);
            }
        }
    }

    /// Vanilla `causeDamage`: fire wears the stand down until it falls apart.
    fn cause_damage(&self, amount: f32) {
        let health = self.living_entity.health.load() - amount;
        if health <= 0.5 {
            self.broken_by_anything();
            self.kill_stand();
        } else {
            self.living_entity.set_health(health);
            let entity = self.get_entity();
            entity.world.load().emit_game_event(
                pumpkin_data::game_event::GameEvent::EntityDamage.name(),
                entity.pos.load(),
            );
        }
    }

    /// Spawns break particles at the armor stand's position.
    fn spawn_break_particles(entity: &Entity) {
        let world = entity.world.load();
        let pos = entity.pos.load();
        let width = entity.width();
        let height = entity.height();

        // ArmorStand.showBreakingParticles
        world.spawn_block_particle(
            Vector3::new(pos.x, pos.y + f64::from(height) / 1.5, pos.z),
            Vector3::new(width / 4.0, height / 4.0, width / 4.0),
            0.05,
            10,
            Block::OAK_PLANKS.default_state.id.as_u16(),
        );
    }
}

impl EntityBase for ArmorStandEntity {
    fn get_piston_push_reaction(&self) -> pumpkin_data::block_state::PistonBehavior {
        if self.is_marker() {
            pumpkin_data::block_state::PistonBehavior::Ignore
        } else {
            pumpkin_data::block_state::PistonBehavior::Normal
        }
    }

    fn can_hit(&self) -> bool {
        !self.get_entity().is_removed() && !self.is_marker()
    }

    fn can_be_seen_by_anyone(&self) -> bool {
        !self.is_invisible() && !self.is_marker()
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        let disabled_slots = self.disabled_slots.load(Ordering::Relaxed);
        // ...

        nbt.put_bool("Invisible", self.is_invisible());
        nbt.put_bool("Small", self.is_small());
        nbt.put_bool("ShowArms", self.should_show_arms());
        nbt.put_int("DisabledSlots", disabled_slots);
        nbt.put_bool("NoBasePlate", !self.should_show_base_plate());
        if self.is_marker() {
            nbt.put_bool("Marker", true);
        }

        nbt.put("Pose", self.pack_rotation());
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        let mut flags = 0u8;
        // ...

        if let Some(invisible) = nbt.get_bool("Invisible")
            && invisible
        {
            self.get_entity().set_invisible(invisible);
        }

        if let Some(small) = nbt.get_bool("Small")
            && small
        {
            flags |= ArmorStandFlags::Small as u8;
        }

        if let Some(show_arms) = nbt.get_bool("ShowArms")
            && show_arms
        {
            flags |= ArmorStandFlags::ShowArms as u8;
        }

        if let Some(disabled_slots) = nbt.get_int("DisabledSlots") {
            self.disabled_slots.store(disabled_slots, Ordering::Relaxed);
        }

        if let Some(no_base_plate) = nbt.get_bool("NoBasePlate") {
            if !no_base_plate {
                flags |= ArmorStandFlags::HideBasePlate as u8;
            }
        } else {
            flags |= ArmorStandFlags::HideBasePlate as u8;
        }

        if let Some(marker) = nbt.get_bool("Marker")
            && marker
        {
            flags |= ArmorStandFlags::Marker as u8;
        }

        self.armor_stand_flags.store(flags, Ordering::Relaxed);
        self.update_dimensions();

        if let Some(pose_tag) = nbt.get("Pose") {
            let packed: PackedRotation = pose_tag.clone().into();
            self.unpack_rotation(&packed);
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.living_entity.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        Some(&self.living_entity)
    }

    fn kill(&self, _caller: &dyn EntityBase) {
        self.kill_stand();
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        _amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let entity = self.get_entity();
        if entity.is_removed() {
            return false;
        }

        let world = entity.world.load();

        let mob_griefing_gamerule = {
            let game_rules = &world.level_info.load().game_rules;
            game_rules.mob_griefing
        };

        // Vanilla: only mob attackers are stopped by `mobGriefing`.
        if !mob_griefing_gamerule && cause.is_some_and(|cause| cause.get_mob().is_some()) {
            return false;
        }

        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY) {
            self.kill_stand();
            return false;
        }

        if entity.is_invulnerable_to(&damage_type, cause) || self.is_invisible() || self.is_marker()
        {
            return false;
        }

        if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION) {
            self.broken_by_anything();
            self.kill_stand();
            return false;
        }

        // Standing in fire lights the stand, and once alight it is worn down
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_IGNITES_ARMOR_STANDS) {
            if entity.is_on_fire() {
                self.cause_damage(0.15);
            } else {
                self.set_on_fire_for(5.0);
            }
            return false;
        }
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BURNS_ARMOR_STANDS)
            && self.living_entity.health.load() > 0.5
        {
            self.cause_damage(4.0);
            return false;
        }

        let can_break = damage_type.has_tag(&tag::DamageType::MINECRAFT_CAN_BREAK_ARMOR_STAND);
        let always_kills =
            damage_type.has_tag(&tag::DamageType::MINECRAFT_ALWAYS_KILLS_ARMOR_STANDS);

        if !can_break && !always_kills {
            return false;
        }

        let attacker = cause.or(source);
        if let Some(attacker) = attacker
            && let Some(player) = attacker.get_player()
        {
            if !player
                .abilities
                .try_lock()
                .is_ok_and(|a| a.allow_modify_world)
            {
                return false;
            } else if player.is_creative() {
                self.play_broken_sound();
                Self::spawn_break_particles(entity);
                self.kill_stand();
                return true;
            }
        }

        let time = world
            .level_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .query_gametime();

        if time - self.last_hit_time.load(Ordering::Relaxed) > 5 && !always_kills {
            world.send_entity_status(entity, EntityStatus::ArmorstandWobble, ActorEventID::None);
            world.play_sound(
                Sound::EntityArmorStandHit,
                SoundCategory::Neutral,
                &entity.block_pos.load().to_f64(),
            );
            world.emit_game_event(
                pumpkin_data::game_event::GameEvent::EntityDamage.name(),
                entity.pos.load(),
            );
            self.last_hit_time.store(time, Ordering::Relaxed);
        } else {
            self.broken_by_player();
            Self::spawn_break_particles(entity);
            self.kill_stand();
        }

        true
    }

    fn get_gravity(&self) -> f64 {
        0.08
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub enum ArmorStandFlags {
    /// Small armor stand Flag
    Small = 1,
    /// Show arms Flag
    ShowArms = 4,
    /// Hide base plate fLag
    HideBasePlate = 8,
    /// Marker Flag
    Marker = 16,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pumpkin_data::data_component_impl::CustomNameImpl;
    use pumpkin_util::text::TextComponent;

    use super::*;
    use crate::entity::ai::test_support::world;
    use crate::world::World;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn stand(world: &Arc<World>) -> Arc<ArmorStandEntity> {
        Arc::new(ArmorStandEntity::new(Entity::new(
            world.clone(),
            Vector3::new(0.5, 64.0, 0.5),
            &EntityType::ARMOR_STAND,
        )))
    }

    fn hurt(stand: &Arc<ArmorStandEntity>, damage_type: DamageType) -> bool {
        stand.damage_with_context(stand.as_ref(), 1.0, damage_type, None, None, None)
    }

    /// The item stacks lying in the world.
    fn dropped(world: &Arc<World>) -> Vec<ItemStack> {
        world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| {
                item.get_item_stack()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            })
            .collect()
    }

    #[tokio::test]
    async fn broken_stand_drops_itself_with_its_name_and_what_it_wore() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let stand = stand(&world);
        stand
            .get_entity()
            .custom_name
            .store(Arc::new(Some(TextComponent::text("Keeper"))));
        stand
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(
                &EquipmentSlot::HEAD,
                ItemStack::new(1, &Item::DIAMOND_HELMET),
            );

        let hit = hurt(&stand, DamageType::PLAYER_ATTACK);
        let removed = stand.get_entity().is_removed();
        let dropped = dropped(&world);
        world.level.shutdown().await;

        let stand_item = dropped
            .iter()
            .find(|stack| stack.item.id == Item::ARMOR_STAND.id);
        assert!(hit);
        assert!(removed);
        assert_eq!(dropped.len(), 2);
        assert!(
            stand_item
                .is_some_and(|stack| { stack.get_data_component::<CustomNameImpl>().is_some() })
        );
        assert!(
            dropped
                .iter()
                .any(|stack| stack.item.id == Item::DIAMOND_HELMET.id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn fire_lights_the_stand_then_wears_it_down() -> TestResult {
        let folder = tempfile::tempdir()?;
        let world = world(&folder);
        let stand = stand(&world);
        let health = || stand.living_entity.health.load();
        let full = health();

        hurt(&stand, DamageType::IN_FIRE);
        let alight = stand.get_entity().is_on_fire();
        let after_lighting = health();
        hurt(&stand, DamageType::IN_FIRE);
        let after_standing_in_fire = health();
        hurt(&stand, DamageType::ON_FIRE);
        let after_burning = health();
        let mut burns = 1;
        while !stand.get_entity().is_removed() && burns < 20 {
            hurt(&stand, DamageType::ON_FIRE);
            burns += 1;
        }
        let dropped = dropped(&world);
        world.level.shutdown().await;

        assert!(alight);
        assert!((after_lighting - full).abs() < f32::EPSILON);
        assert!((after_standing_in_fire - (full - 0.15)).abs() < 1.0e-4);
        assert!((after_burning - (full - 4.15)).abs() < 1.0e-4);
        // 20 health: 0.15 from the fire, then five burns of 4 bring it to 0.5 or less
        assert_eq!(burns, 5);
        // Burnt down, the stand itself is lost
        assert!(dropped.is_empty());
        Ok(())
    }
}
