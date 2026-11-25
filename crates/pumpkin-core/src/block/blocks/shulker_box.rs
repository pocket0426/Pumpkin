use std::sync::Arc;
use std::sync::Mutex;

use crate::block::{
    GetComparatorOutputArgs, GetScreenHandlerFactoryArgs, OnPlaceArgs, OnSyncedBlockEventArgs,
    PlacedArgs,
};
use crate::block::{
    registry::BlockActionResult,
    {BlockBehaviour, NormalUseArgs},
};
use crate::entity::mob::piglin_ai::PiglinAi;

use crate::block::entities::shulker_box::{AnimationStatus, ShulkerBoxBlockEntity};
use crate::entity::mob::shulker::ShulkerEntity;
use pumpkin_data::BlockStateId;
use pumpkin_data::FacingExt;
use pumpkin_data::translation;
use pumpkin_inventory::Inventory;
use pumpkin_inventory::generic_container_screen_handler::create_generic_9x3;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::text::TextComponent;

struct ShulkerBoxScreenFactory(Arc<dyn Inventory>);

impl ScreenHandlerFactory for ShulkerBoxScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let handler = create_generic_9x3(sync_id, player_inventory, self.0.clone(), player);
        let screen_handler_arc = Arc::new(Mutex::new(handler));

        Some(screen_handler_arc as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_SHULKERBOX,
            translation::bedrock::CONTAINER_SHULKERBOX
        )
    }
}

#[pumpkin_block_from_tag("minecraft:shulker_boxes")]
pub struct ShulkerBoxBlock;

type EndRodLikeProperties = pumpkin_data::block_properties::EndRodLikeProperties;

impl BlockBehaviour for ShulkerBoxBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = EndRodLikeProperties::default(args.block);
        props.facing = args.direction.to_facing().opposite();
        props.to_state_id(args.block)
    }

    fn on_synced_block_event(&self, args: OnSyncedBlockEventArgs<'_>) -> bool {
        args.world
            .get_block_entity(args.position)
            .and_then(|block_entity| {
                block_entity
                    .as_any()
                    .downcast_ref::<ShulkerBoxBlockEntity>()
                    .map(|shulker_box| shulker_box.trigger_event(args.r#type, args.data))
            })
            .unwrap_or(false)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        {
            let barrel_block_entity = ShulkerBoxBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(barrel_block_entity));
        }
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if !Self::can_open(&args) {
            return BlockActionResult::Success;
        }
        if let Some(factory) = self.get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: args.server,
            world: args.world,
            block: args.block,
            position: args.position,
            player: args.player,
        }) {
            args.player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                pumpkin_data::statistic::CustomStatistic::OpenShulkerBox as i32,
                1,
            );
            args.player
                .open_handled_screen(factory.as_ref(), Some(*args.position));
            PiglinAi::anger_nearby_piglins(args.world, args.player, true);
        }

        BlockActionResult::Success
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        let block_entity = args.world.get_block_entity(args.position)?;
        let inventory = block_entity.get_inventory()?;
        Some(Box::new(ShulkerBoxScreenFactory(inventory)))
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        crate::block::container_comparator_output(&args)
    }
}

impl ShulkerBoxBlock {
    pub const OPEN_ANIMATION_EVENT_TYPE: u8 = 1;

    /// Vanilla `ShulkerBoxBlock.canOpen`: a closed box needs free space for its lid.
    fn can_open(args: &NormalUseArgs<'_>) -> bool {
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return false;
        };
        let Some(shulker_box) = block_entity
            .as_any()
            .downcast_ref::<ShulkerBoxBlockEntity>()
        else {
            return false;
        };
        if shulker_box.animation_status() != AnimationStatus::Closed {
            return true;
        }
        let state_id = args.world.get_block_state_id(args.position);
        let facing = EndRodLikeProperties::from_state_id(state_id).facing;
        let lid_space = ShulkerEntity::progress_delta_aabb(
            1.0,
            facing.to_block_direction(),
            0.0,
            0.5,
            args.position.to_f64(),
        )
        .contract_all(1.0E-6);
        // Vanilla passes no source entity; ours only matters for a vehicle the player shares.
        args.world.is_space_empty(lid_space)
            && args
                .world
                .no_entity_collision(args.player.as_ref(), lid_space)
    }
}
