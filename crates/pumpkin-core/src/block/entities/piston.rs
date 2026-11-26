use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::{Axis, PistonHeadLikeProperties, PistonType};
use pumpkin_data::block_state::PistonBehavior;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId, Property};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use pumpkin_world::block::BlockStateCodec;

use crate::block::blocks::piston::piston::PistonProps;
use crate::entity::EntityBase;
use crate::world::{BlockFlags, World};

use super::BlockEntity;

pub struct PistonBlockEntity {
    pub position: BlockPos,
    pub pushed_block_state: &'static BlockState,
    pub facing: BlockDirection,
    pub current_progress: AtomicCell<f32>,
    pub last_progress: AtomicCell<f32>,
    pub extending: bool,
    pub source: bool,
    /// Vanilla `lastTicked`: the game time of the latest tick, `i64::MIN` before the first.
    pub last_ticked: std::sync::atomic::AtomicI64,
}

impl PistonBlockEntity {
    pub const ID: &'static str = "minecraft:piston";

    const fn movement_direction(&self) -> BlockDirection {
        if self.extending {
            self.facing
        } else {
            self.facing.opposite()
        }
    }

    /// Vanilla's `getAmountExtended`: how far back from the block's final position
    /// the visual is at a given animation progress. Negative for extending.
    fn amount_extended(&self, progress: f32) -> f32 {
        if self.extending {
            progress - 1.0
        } else {
            1.0 - progress
        }
    }

    fn dir_vec(dir: BlockDirection, scale: f64) -> Vector3<f64> {
        let off = dir.to_offset();
        Vector3::new(
            f64::from(off.x) * scale,
            f64::from(off.y) * scale,
            f64::from(off.z) * scale,
        )
    }

    /// Vanilla `getCollisionRelatedBlockState`: a retracting piston pushes with its head, not
    /// with the base it will turn back into.
    fn collision_related_state(&self) -> &'static BlockState {
        let block = Block::from_state_id(self.pushed_block_state.id);
        if self.extending
            || !self.source
            || !(block == &Block::PISTON || block == &Block::STICKY_PISTON)
        {
            return self.pushed_block_state;
        }
        let base = PistonProps::from_state_id(self.pushed_block_state.id);
        let mut head = PistonHeadLikeProperties::default(&Block::PISTON_HEAD);
        head.short = self.current_progress.load() > 0.25;
        head.r#type = if block == &Block::STICKY_PISTON {
            PistonType::Sticky
        } else {
            PistonType::Normal
        };
        head.facing = base.facing;
        BlockState::from_id(head.to_state_id(&Block::PISTON_HEAD))
    }

    /// Vanilla `PistonMovingBlockEntity.getCollisionShape` at the current progress, in world
    /// coordinates: the moving block where it is now, plus the base of a retracting piston.
    fn collision_boxes(&self) -> Vec<BoundingBox> {
        let progress = self.current_progress.load();
        let moved_block = Block::from_state_id(self.pushed_block_state.id);
        let mut boxes = Vec::new();
        if !self.extending
            && self.source
            && (moved_block == &Block::PISTON || moved_block == &Block::STICKY_PISTON)
        {
            let mut base = PistonProps::from_state_id(self.pushed_block_state.id);
            base.extended = true;
            boxes.extend(
                BlockState::from_id(base.to_state_id(moved_block))
                    .get_block_collision_shapes_at(&self.position)
                    .map(|shape| shape.at_pos(self.position)),
            );
        }
        let state = if self.source {
            let mut head = PistonHeadLikeProperties::default(&Block::PISTON_HEAD);
            head.facing = self.facing.to_facing();
            head.short = self.extending != (1.0 - progress < 0.25);
            BlockState::from_id(head.to_state_id(&Block::PISTON_HEAD))
        } else {
            self.pushed_block_state
        };
        let offset = self.position.to_f64()
            + Self::dir_vec(self.facing, f64::from(self.amount_extended(progress)));
        boxes.extend(
            state
                .get_block_collision_shapes_at(&self.position)
                .map(|shape| shape.shift(offset)),
        );
        boxes
    }

    /// Makes the moving block solid where it currently is. Called when the block starts moving
    /// and after every step.
    pub fn update_collision(&self, world: &World) {
        world.moving_pistons.set_boxes(
            self.position,
            self.collision_boxes(),
            world.get_world_age(),
        );
    }

    /// Vanilla `moveByPositionAndProgress`: a block-local box at where the moving block is now.
    fn move_by_position_and_progress(&self, aabb: BoundingBox) -> BoundingBox {
        let amount = f64::from(self.amount_extended(self.current_progress.load()));
        aabb.shift(self.position.to_f64() + Self::dir_vec(self.facing, amount))
    }

    /// Vanilla `PistonMath.getMovementArea`: the slab the leading face of `aabb` sweeps when
    /// moving `amount` towards `dir`.
    fn movement_area(aabb: BoundingBox, dir: BlockDirection, amount: f64) -> BoundingBox {
        let mut area = aabb;
        match dir {
            BlockDirection::West => {
                area.max.x = aabb.min.x;
                area.min.x = aabb.min.x - amount;
            }
            BlockDirection::East => {
                area.min.x = aabb.max.x;
                area.max.x = aabb.max.x + amount;
            }
            BlockDirection::Down => {
                area.max.y = aabb.min.y;
                area.min.y = aabb.min.y - amount;
            }
            BlockDirection::Up => {
                area.min.y = aabb.max.y;
                area.max.y = aabb.max.y + amount;
            }
            BlockDirection::North => {
                area.max.z = aabb.min.z;
                area.min.z = aabb.min.z - amount;
            }
            BlockDirection::South => {
                area.min.z = aabb.max.z;
                area.max.z = aabb.max.z + amount;
            }
        }
        area
    }

    /// Vanilla `moveCollidedEntities`: pushes what the moving block runs into.
    ///
    /// Players are left to their client, which runs this same push on its own copy of the block
    /// entity and reports where it ended up.
    fn move_collided_entities(&self, world: &Arc<World>, new_progress: f32) {
        let direction = self.movement_direction();
        let delta = f64::from(new_progress - self.current_progress.load());
        let shapes: Vec<BoundingBox> = self
            .collision_related_state()
            .get_block_collision_shapes_at(&self.position)
            .collect();
        let Some(bounds) = shapes.iter().copied().reduce(|a, b| a.minmax(&b)) else {
            return;
        };
        let bounds = self.move_by_position_and_progress(bounds);
        let search = Self::movement_area(bounds, direction, delta).minmax(&bounds);
        let entities = world.get_entities_at_box(&search);
        if entities.is_empty() {
            return;
        }
        let areas: Vec<BoundingBox> = shapes
            .iter()
            .map(|shape| {
                Self::movement_area(self.move_by_position_and_progress(*shape), direction, delta)
            })
            .collect();
        let is_slime = Block::from_state_id(self.pushed_block_state.id) == &Block::SLIME_BLOCK;
        for entity in entities {
            if entity.get_player().is_some()
                || entity.get_piston_push_reaction() == PistonBehavior::Ignore
            {
                continue;
            }
            let base = entity.get_entity();
            if is_slime {
                // A slime block launches what it pushes at one block per tick
                let step = direction.to_offset().to_f64();
                let mut velocity = base.velocity.load();
                match direction.to_axis() {
                    Axis::X => velocity.x = step.x,
                    Axis::Y => velocity.y = step.y,
                    Axis::Z => velocity.z = step.z,
                }
                base.velocity.store(velocity);
                base.velocity_dirty.store(true, Ordering::Relaxed);
            }
            let entity_box = base.bounding_box.load();
            let mut movement: f64 = 0.0;
            for area in &areas {
                if !area.intersects(&entity_box) {
                    continue;
                }
                movement = movement.max(Self::intersection_size(*area, direction, entity_box));
                if movement >= delta {
                    break;
                }
            }
            if movement <= 0.0 {
                continue;
            }
            let movement = movement.min(delta) + 0.01;
            Self::move_entity_by_piston(entity.as_ref(), direction, movement);
            if !self.extending && self.source {
                Self::fix_entity_within_piston_base(
                    entity.as_ref(),
                    &self.position,
                    direction,
                    delta,
                );
            }
        }
    }

    /// Vanilla `moveStuckEntities`: a honey block moving sideways carries what stands on it.
    fn move_stuck_entities(&self, world: &Arc<World>, new_progress: f32) {
        if Block::from_state_id(self.pushed_block_state.id) != &Block::HONEY_BLOCK {
            return;
        }
        let direction = self.movement_direction();
        if direction.to_axis() == Axis::Y {
            return;
        }
        let top = self
            .pushed_block_state
            .get_block_collision_shapes_at(&self.position)
            .map(|shape| shape.max.y)
            .fold(f64::NEG_INFINITY, f64::max);
        if !top.is_finite() {
            return;
        }
        let above = self.move_by_position_and_progress(BoundingBox::new(
            Vector3::new(0.0, top, 0.0),
            Vector3::new(1.0, 1.500_001, 1.0),
        ));
        let delta = f64::from(new_progress - self.current_progress.load());
        for entity in world.get_entities_at_box(&above) {
            if entity.get_player().is_some()
                || !self.matches_sticky_criteria(above, entity.as_ref())
            {
                continue;
            }
            Self::move_entity_by_piston(entity.as_ref(), direction, delta);
        }
    }

    /// Vanilla `matchesStickyCritera`.
    fn matches_sticky_criteria(&self, above: BoundingBox, entity: &dyn EntityBase) -> bool {
        let base = entity.get_entity();
        if entity.get_piston_push_reaction() != PistonBehavior::Normal
            || !base.on_ground.load(Ordering::Relaxed)
        {
            return false;
        }
        let pos = base.pos.load();
        base.supporting_block_pos.load() == Some(self.position)
            || (pos.x >= above.min.x
                && pos.x <= above.max.x
                && pos.z >= above.min.z
                && pos.z <= above.max.z)
    }

    /// Vanilla `getMovement`: how far `area` reaches into `entity` along `motion_dir`.
    fn intersection_size(
        area: BoundingBox,
        motion_dir: BlockDirection,
        entity: BoundingBox,
    ) -> f64 {
        match motion_dir {
            BlockDirection::East => area.max.x - entity.min.x,
            BlockDirection::West => entity.max.x - area.min.x,
            BlockDirection::Up => area.max.y - entity.min.y,
            BlockDirection::Down => entity.max.y - area.min.y,
            BlockDirection::South => area.max.z - entity.min.z,
            BlockDirection::North => entity.max.z - area.min.z,
        }
    }

    /// Vanilla `moveEntityByPiston`. The moving block itself has no collision here, which is what
    /// vanilla's `NOCLIP` arranges for the entity being moved.
    fn move_entity_by_piston(entity: &dyn EntityBase, dir: BlockDirection, distance: f64) {
        entity
            .get_entity()
            .move_by_piston(entity, Self::dir_vec(dir, distance));
    }

    /// Vanilla `fixEntityWithinPistonBase`: an entity a retracting head pulled into the base's
    /// own block is put back out.
    fn fix_entity_within_piston_base(
        entity: &dyn EntityBase,
        piston_pos: &BlockPos,
        motion_dir: BlockDirection,
        delta: f64,
    ) {
        let body = BoundingBox::from_block(piston_pos);
        let entity_box = entity.get_entity().bounding_box.load();
        if !body.intersects(&entity_box) {
            return;
        }
        let back = motion_dir.opposite();
        let whole = Self::intersection_size(body, back, entity_box) + 0.01;
        let overlap =
            Self::intersection_size(body, back, Self::aabb_intersection(body, entity_box)) + 0.01;
        if (whole - overlap).abs() < 0.01 {
            Self::move_entity_by_piston(entity, back, whole.min(delta) + 0.01);
        }
    }

    const fn aabb_intersection(a: BoundingBox, b: BoundingBox) -> BoundingBox {
        BoundingBox::new(
            Vector3::new(
                a.min.x.max(b.min.x),
                a.min.y.max(b.min.y),
                a.min.z.max(b.min.z),
            ),
            Vector3::new(
                a.max.x.min(b.max.x),
                a.max.y.min(b.max.y),
                a.max.z.min(b.max.z),
            ),
        )
    }

    /// The arrival half of vanilla `PistonMovingBlockEntity.tick`.
    fn place_moved_block(&self, world: &Arc<World>) {
        let pos = self.position;
        let moved = self.pushed_block_state;
        let updated = world.update_from_neighbor_shapes(moved.id, &pos);
        if BlockState::from_id(updated).is_air() {
            // The block cannot stay where it was pushed: it arrives silently and breaks,
            // dropping its loot (vanilla `Block.updateOrDestroy`)
            world.set_block_state(
                &pos,
                moved.id,
                BlockFlags::FORCE_STATE | BlockFlags::MOVED | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
            if !moved.is_air() {
                world.break_block(&pos, None, BlockFlags::NOTIFY_ALL);
            }
            return;
        }
        // A waterlogged block arrives dry
        let updated = without_waterlogging(updated);
        world.set_block_state(&pos, updated, BlockFlags::NOTIFY_ALL | BlockFlags::MOVED);
        // Vanilla `neighborChanged` on the block itself, so a moved piston or lamp
        // re-reads its power
        world.update_neighbor(&pos, Block::from_state_id(updated));
    }

    pub fn finish(&self, world: &Arc<World>) {
        if self.last_progress.load() < 1.0 {
            let pos = self.position;
            world.remove_block_entity(&pos);
            world.moving_pistons.remove(&pos);
            if world.get_block(&pos) == &Block::MOVING_PISTON {
                let state = if self.source {
                    Block::AIR.default_state.id
                } else {
                    world.update_from_neighbor_shapes(self.pushed_block_state.id, &pos)
                };
                world.set_block_state(&pos, state, BlockFlags::NOTIFY_ALL);
                world.update_neighbors(&pos, None);
                // Vanilla `finalTick` ends with `neighborChanged` on the block itself
                world.update_neighbor(&pos, Block::from_state_id(state));
            }
        }
    }
}

/// The state with `waterlogged` cleared, when it has that property.
fn without_waterlogging(state_id: BlockStateId) -> BlockStateId {
    let block = Block::from_state_id(state_id);
    let Some(properties) = block.properties(state_id) else {
        return state_id;
    };
    let mut properties = properties.to_props();
    let mut changed = false;
    for property in &mut properties {
        if property.key_str() == "waterlogged"
            && property.value_str() == "true"
            && let Some(dry) = Property::from_str_pair("waterlogged", "false")
        {
            *property = dry;
            changed = true;
        }
    }
    if changed {
        block.from_properties(&properties).to_state_id(block)
    } else {
        state_id
    }
}

const BLOCK_STATE: &str = "blockState";
const FACING: &str = "facing";
const LAST_PROGRESS: &str = "progress";
const EXTENDING: &str = "extending";
const SOURCE: &str = "source";

impl BlockEntity for PistonBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn tick(&self, world: &Arc<World>) {
        self.last_ticked
            .store(world.get_world_age(), Ordering::Relaxed);
        let current_progress = self.current_progress.load();
        self.last_progress.store(current_progress);
        if current_progress >= 1.0 {
            let pos = self.position;
            world.remove_block_entity(&pos);
            world.moving_pistons.remove(&pos);
            if world.get_block(&pos) == &Block::MOVING_PISTON {
                self.place_moved_block(world);
            }
            return;
        }
        let new_progress = current_progress + 0.5;
        self.move_collided_entities(world, new_progress);
        self.move_stuck_entities(world, new_progress);
        self.current_progress.store(new_progress.min(1.0));
        self.update_collision(world);
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        // Vanilla falls back to air for a missing or unknown state
        let pushed_block_state = nbt
            .get_compound(BLOCK_STATE)
            .and_then(BlockStateCodec::state_from_nbt)
            .map_or(Block::AIR.default_state, BlockState::from_id);
        let facing = nbt.get_byte(FACING).unwrap_or(0);
        let last_progress = nbt.get_float(LAST_PROGRESS).unwrap_or(0.0);
        let extending = nbt.get_bool(EXTENDING).unwrap_or(false);
        let source = nbt.get_bool(SOURCE).unwrap_or(false);
        Self {
            pushed_block_state,
            position,
            facing: BlockDirection::from_index(facing as u8).unwrap_or(BlockDirection::Down),
            current_progress: last_progress.into(),
            last_progress: last_progress.into(),
            extending,
            source,
            last_ticked: std::sync::atomic::AtomicI64::new(i64::MIN),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_compound(
            BLOCK_STATE,
            BlockStateCodec::state_to_nbt(self.pushed_block_state.id),
        );
        nbt.put_byte(FACING, self.facing.to_index() as i8);
        nbt.put_float(LAST_PROGRESS, self.last_progress.load());
        nbt.put_bool(EXTENDING, self.extending);
        nbt.put_bool(SOURCE, self.source);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put_compound(
            BLOCK_STATE,
            BlockStateCodec::state_to_nbt(self.pushed_block_state.id),
        );
        nbt.put_byte(FACING, self.facing.to_index() as i8);
        nbt.put_float(LAST_PROGRESS, self.last_progress.load());
        nbt.put_bool(EXTENDING, self.extending);
        nbt.put_bool(SOURCE, self.source);
        // TODO: duplicated code because of async :c
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
