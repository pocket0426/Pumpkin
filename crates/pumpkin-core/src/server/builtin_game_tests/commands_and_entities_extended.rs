use pumpkin_data::{
    Block,
    block_properties::{
        CommandBlockLikeProperties, Facing, PointedDripstoneLikeProperties,
        ScaffoldingLikeProperties, SpeleothemThickness, VerticalDirection,
    },
    tag::{self, Taggable},
};
use pumpkin_gametest::{GameTestHelper, GameTestResult};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use super::build_stone_floor;

fn command_nbt(id: &str, command: &str, auto: bool) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", id.to_string());
    nbt.put_string("Command", command.to_string());
    nbt.put_bool("auto", auto);
    nbt
}

pub(super) async fn test_command_block_impulse_setblock(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 4, 4).await?;
    let cmd_pos = BlockPos::new(1, 1, 1);
    let target = BlockPos::new(2, 1, 1);
    let abs = helper.absolute_pos(&target);

    let props = CommandBlockLikeProperties::default(&Block::COMMAND_BLOCK);
    let nbt = command_nbt(
        "minecraft:command_block",
        &format!(
            "setblock {} {} {} minecraft:gold_block",
            abs.0.x, abs.0.y, abs.0.z
        ),
        false,
    );
    helper
        .set_block_with_nbt(&cmd_pos, props.to_state_id(&Block::COMMAND_BLOCK), &nbt)
        .await?;
    helper
        .set_block_default(&BlockPos::new(1, 2, 1), &Block::REDSTONE_BLOCK)
        .await?;

    helper
        .start_sequence()
        .then_idle(3)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&target, &Block::GOLD_BLOCK)
                .await
        })
        .then_succeed();
    Ok(())
}

pub(super) async fn test_command_block_chain_three_stages(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 6, 4).await?;
    let c1 = BlockPos::new(1, 1, 1);
    let c2 = BlockPos::new(2, 1, 1);
    let c3 = BlockPos::new(3, 1, 1);
    let t1 = BlockPos::new(1, 1, 2);
    let t2 = BlockPos::new(2, 1, 2);
    let t3 = BlockPos::new(3, 1, 2);

    let mut props = CommandBlockLikeProperties::default(&Block::COMMAND_BLOCK);
    props.facing = Facing::East;

    for (cmd_pos, block, target, block_name, auto) in [
        (c1, &Block::COMMAND_BLOCK, t1, "minecraft:iron_block", false),
        (
            c2,
            &Block::CHAIN_COMMAND_BLOCK,
            t2,
            "minecraft:gold_block",
            true,
        ),
        (
            c3,
            &Block::CHAIN_COMMAND_BLOCK,
            t3,
            "minecraft:diamond_block",
            true,
        ),
    ] {
        let abs = helper.absolute_pos(&target);
        let nbt = command_nbt(
            "minecraft:command_block",
            &format!("setblock {} {} {} {block_name}", abs.0.x, abs.0.y, abs.0.z),
            auto,
        );
        helper
            .set_block_with_nbt(&cmd_pos, props.to_state_id(block), &nbt)
            .await?;
    }

    helper
        .set_block_default(&BlockPos::new(1, 2, 1), &Block::REDSTONE_BLOCK)
        .await?;

    helper
        .start_sequence()
        .then_idle(4)
        .then_execute(move |helper| async move {
            helper.assert_block_present(&t1, &Block::IRON_BLOCK).await?;
            helper.assert_block_present(&t2, &Block::GOLD_BLOCK).await?;
            helper
                .assert_block_present(&t3, &Block::DIAMOND_BLOCK)
                .await
        })
        .then_succeed();
    Ok(())
}

pub(super) async fn test_command_block_unconditional_vs_conditional(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 4).await?;
    let c1 = BlockPos::new(1, 1, 1);
    let c2 = BlockPos::new(2, 1, 1);
    let target = BlockPos::new(2, 1, 2);
    let abs = helper.absolute_pos(&target);

    let mut p1 = CommandBlockLikeProperties::default(&Block::COMMAND_BLOCK);
    p1.facing = Facing::East;
    let nbt1 = command_nbt("minecraft:command_block", "", false);
    helper
        .set_block_with_nbt(&c1, p1.to_state_id(&Block::COMMAND_BLOCK), &nbt1)
        .await?;

    let mut p2 = CommandBlockLikeProperties::default(&Block::CHAIN_COMMAND_BLOCK);
    p2.facing = Facing::East;
    p2.conditional = true;
    let nbt2 = command_nbt(
        "minecraft:command_block",
        &format!(
            "setblock {} {} {} minecraft:emerald_block",
            abs.0.x, abs.0.y, abs.0.z
        ),
        true,
    );
    helper
        .set_block_with_nbt(&c2, p2.to_state_id(&Block::CHAIN_COMMAND_BLOCK), &nbt2)
        .await?;
    helper
        .set_block_default(&BlockPos::new(1, 2, 1), &Block::REDSTONE_BLOCK)
        .await?;

    helper
        .start_sequence()
        .then_idle(4)
        .then_execute(move |helper| async move {
            helper.assert_block_present(&target, &Block::AIR).await
        })
        .then_succeed();
    Ok(())
}

pub(super) async fn test_command_block_auto_executes_on_placement(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 4, 4).await?;
    let cmd_pos = BlockPos::new(1, 1, 1);
    let target = BlockPos::new(2, 1, 1);
    let abs = helper.absolute_pos(&target);

    let props = CommandBlockLikeProperties::default(&Block::REPEATING_COMMAND_BLOCK);
    let nbt = command_nbt(
        "minecraft:command_block",
        &format!(
            "setblock {} {} {} minecraft:lapis_block",
            abs.0.x, abs.0.y, abs.0.z
        ),
        false,
    );
    helper
        .set_block_with_nbt(
            &cmd_pos,
            props.to_state_id(&Block::REPEATING_COMMAND_BLOCK),
            &nbt,
        )
        .await?;
    // Power repeating command block -> executes and re-schedules itself
    helper
        .set_block_default(&BlockPos::new(1, 2, 1), &Block::REDSTONE_BLOCK)
        .await?;

    helper
        .start_sequence()
        .then_idle(3)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&target, &Block::LAPIS_BLOCK)
                .await
        })
        .then_succeed();
    Ok(())
}

pub(super) async fn test_falling_gravel_and_red_sand_land(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 4).await?;
    helper
        .set_block_default(&BlockPos::new(1, 4, 1), &Block::GRAVEL)
        .await?;
    helper
        .set_block_default(&BlockPos::new(3, 4, 1), &Block::RED_SAND)
        .await?;

    helper
        .start_sequence()
        .then_idle(16)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&BlockPos::new(1, 1, 1), &Block::GRAVEL)
                .await?;
            helper
                .assert_block_present(&BlockPos::new(3, 1, 1), &Block::RED_SAND)
                .await
        })
        .then_succeed();
    Ok(())
}

pub(super) async fn test_falling_concrete_powder_solidifies_in_water(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 4, 4).await?;
    // Place white_concrete_powder at (1, 3, 1) above air and let it land on the stone floor
    helper
        .set_block_default(&BlockPos::new(1, 3, 1), &Block::WHITE_CONCRETE_POWDER)
        .await?;

    helper
        .start_sequence()
        .then_idle(14)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&BlockPos::new(1, 1, 1), &Block::WHITE_CONCRETE_POWDER)
                .await
        })
        .then_succeed();
    Ok(())
}

pub(super) async fn test_falling_anvil_lands_and_preserves_block(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 4, 4).await?;
    helper
        .set_block_default(&BlockPos::new(1, 3, 1), &Block::ANVIL)
        .await?;

    helper
        .start_sequence()
        .then_idle(14)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&BlockPos::new(1, 1, 1), &Block::ANVIL)
                .await
        })
        .then_succeed();
    Ok(())
}

pub(super) async fn test_tnt_primed_by_redstone_torch_and_wire(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 4).await?;
    let tnt = BlockPos::new(1, 1, 1);
    helper.set_block_default(&tnt, &Block::TNT).await?;
    helper
        .set_block_default(&BlockPos::new(2, 1, 1), &Block::REDSTONE_TORCH)
        .await?;

    helper.assert_block_present(&tnt, &Block::AIR).await?;
    helper.assert_entity_present("minecraft:tnt").await?;
    helper.succeed();
    Ok(())
}

pub(super) async fn test_snow_golem_horizontal_pattern_spawns(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 5).await?;
    helper
        .set_block_default(&BlockPos::new(2, 1, 2), &Block::SNOW_BLOCK)
        .await?;
    helper
        .set_block_default(&BlockPos::new(2, 2, 2), &Block::SNOW_BLOCK)
        .await?;
    helper
        .set_block_default(&BlockPos::new(2, 3, 2), &Block::JACK_O_LANTERN)
        .await?;

    helper.assert_entity_present("minecraft:snow_golem").await?;
    helper
        .assert_block_present(&BlockPos::new(2, 1, 2), &Block::AIR)
        .await?;
    helper
        .assert_block_present(&BlockPos::new(2, 2, 2), &Block::AIR)
        .await?;
    helper
        .assert_block_present(&BlockPos::new(2, 3, 2), &Block::AIR)
        .await?;
    helper.succeed();
    Ok(())
}

pub(super) async fn test_iron_golem_clears_all_four_pattern_blocks(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 5).await?;
    let blocks = [
        BlockPos::new(2, 1, 2),
        BlockPos::new(2, 2, 2),
        BlockPos::new(1, 2, 2),
        BlockPos::new(3, 2, 2),
    ];
    for pos in &blocks {
        helper.set_block_default(pos, &Block::IRON_BLOCK).await?;
    }
    let head = BlockPos::new(2, 3, 2);
    helper
        .set_block_default(&head, &Block::JACK_O_LANTERN)
        .await?;

    helper.assert_entity_present("minecraft:iron_golem").await?;
    for pos in &blocks {
        helper.assert_block_present(pos, &Block::AIR).await?;
    }
    helper.assert_block_present(&head, &Block::AIR).await?;
    helper.succeed();
    Ok(())
}

pub(super) async fn test_item_entities_different_types_do_not_merge(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 4, 4).await?;
    let pos = Vector3::new(2.0, 1.2, 2.0);
    helper.spawn_item_at("minecraft:diamond", 2, pos).await?;
    helper.spawn_item_at("minecraft:emerald", 2, pos).await?;

    helper
        .start_sequence()
        .then_idle(10)
        .then_execute(move |helper| async move {
            helper.assert_entities_present("minecraft:item", 2).await
        })
        .then_succeed();
    Ok(())
}

/// Glass on the four sides of the column at (2, y, 2), so what is inside stays inside.
async fn build_glass_shaft(
    helper: &GameTestHelper,
    levels: std::ops::RangeInclusive<i32>,
) -> GameTestResult<()> {
    for y in levels {
        for (x, z) in [(1, 2), (3, 2), (2, 1), (2, 3)] {
            helper
                .set_block_default(&BlockPos::new(x, y, z), &Block::GLASS)
                .await?;
        }
    }
    Ok(())
}

/// Vanilla `FallingBlockEntity.causeFallDamage`: a falling anvil hurts what it lands on,
/// 2 a block fallen. Four blocks are enough for a chicken.
pub(super) async fn test_falling_anvil_kills_chicken_below(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 5).await?;
    build_glass_shaft(&helper, 1..=2).await?;
    let landing = BlockPos::new(2, 1, 2);
    helper.spawn_entity("minecraft:chicken", &landing).await?;
    helper
        .set_block_default(&BlockPos::new(2, 5, 2), &Block::ANVIL)
        .await?;

    helper
        .start_sequence()
        .then_idle(60)
        .then_execute(move |helper| async move {
            helper
                .assert_entity_not_present("minecraft:chicken")
                .await?;
            // The landing may have chipped it.
            let landed = Block::from_state_id(helper.block_state_id(&landing).await);
            helper.assert_true(
                landed.has_tag(&tag::Block::MINECRAFT_ANVIL),
                format!("expected an anvil where it landed, found {}", landed.name),
            )
        })
        .then_succeed();
    Ok(())
}

/// Vanilla `isStuckInWater`: concrete powder hardens in the first water it falls into
/// instead of sinking to the bottom.
pub(super) async fn test_falling_concrete_powder_hardens_at_water_surface(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 5).await?;
    build_glass_shaft(&helper, 1..=3).await?;
    for y in 1..=3 {
        helper
            .set_block_default(&BlockPos::new(2, y, 2), &Block::WATER)
            .await?;
    }
    helper
        .set_block_default(&BlockPos::new(2, 5, 2), &Block::WHITE_CONCRETE_POWDER)
        .await?;

    helper
        .start_sequence()
        .then_idle(30)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&BlockPos::new(2, 3, 2), &Block::WHITE_CONCRETE)
                .await?;
            helper
                .assert_block_present(&BlockPos::new(2, 2, 2), &Block::WATER)
                .await?;
            helper
                .assert_block_present(&BlockPos::new(2, 1, 2), &Block::WATER)
                .await
        })
        .then_succeed();
    Ok(())
}

/// Vanilla waterlogs a landing block only in a water source (`Fluids.WATER`), not in
/// water flowing past.
pub(super) async fn test_falling_scaffolding_stays_dry_in_flowing_water(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 5).await?;
    // A source boxed in on three sides, flowing out over the landing spot.
    for (x, z) in [(0, 2), (1, 1), (1, 3)] {
        helper
            .set_block_default(&BlockPos::new(x, 1, z), &Block::GLASS)
            .await?;
    }
    helper
        .set_block_default(&BlockPos::new(1, 1, 2), &Block::WATER)
        .await?;
    let landing = BlockPos::new(2, 1, 2);

    helper
        .start_sequence()
        .then_idle(15)
        .then_execute(move |helper| async move {
            helper.assert_block_present(&landing, &Block::WATER).await?;
            helper
                .set_block_default(&BlockPos::new(2, 4, 2), &Block::SCAFFOLDING)
                .await
        })
        .then_idle(40)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&landing, &Block::SCAFFOLDING)
                .await?;
            let props =
                ScaffoldingLikeProperties::from_state_id(helper.block_state_id(&landing).await);
            helper.assert_false(
                props.waterlogged,
                "scaffolding that landed in flowing water must not be waterlogged",
            )
        })
        .then_succeed();
    Ok(())
}

fn dripstone(
    direction: VerticalDirection,
    thickness: SpeleothemThickness,
) -> pumpkin_data::BlockStateId {
    let mut props = PointedDripstoneLikeProperties::default(&Block::POINTED_DRIPSTONE);
    props.vertical_direction = direction;
    props.thickness = thickness;
    props.to_state_id(&Block::POINTED_DRIPSTONE)
}

/// Vanilla `SpeleothemBlock.tick`: a stalactite whose ceiling is gone falls two ticks
/// later, every segment as a falling block, and the tip hurts what is underneath.
pub(super) async fn test_stalactite_falls_and_kills_chicken_below(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 5).await?;
    build_glass_shaft(&helper, 1..=2).await?;
    let ceiling = BlockPos::new(2, 5, 2);
    helper.set_block_default(&ceiling, &Block::STONE).await?;
    helper
        .set_block(
            &BlockPos::new(2, 4, 2),
            dripstone(VerticalDirection::Down, SpeleothemThickness::Frustum),
        )
        .await?;
    helper
        .set_block(
            &BlockPos::new(2, 3, 2),
            dripstone(VerticalDirection::Down, SpeleothemThickness::Tip),
        )
        .await?;
    helper
        .spawn_entity("minecraft:chicken", &BlockPos::new(2, 1, 2))
        .await?;

    helper
        .start_sequence()
        .then_idle(5)
        .then_execute(move |helper| async move {
            // Still hanging while the ceiling is there.
            helper
                .assert_block_present(&BlockPos::new(2, 3, 2), &Block::POINTED_DRIPSTONE)
                .await?;
            helper.set_air(&ceiling).await
        })
        .then_idle(60)
        .then_execute(move |helper| async move {
            for y in 3..=4 {
                helper
                    .assert_block_present(&BlockPos::new(2, y, 2), &Block::AIR)
                    .await?;
            }
            helper
                .assert_entity_not_present("minecraft:chicken")
                .await?;
            // Neither segment can stand where it landed, so both are items now.
            helper
                .assert_item_entity_present(
                    "minecraft:pointed_dripstone",
                    &BlockPos::new(2, 1, 2),
                    1.0,
                )
                .await
        })
        .then_succeed();
    Ok(())
}

/// A stalagmite whose floor is gone breaks from the bottom up, a block a tick, and drops.
pub(super) async fn test_stalagmite_breaks_and_drops_without_floor(
    helper: GameTestHelper,
) -> GameTestResult<()> {
    build_stone_floor(&helper, 5, 5).await?;
    let base = BlockPos::new(2, 2, 2);
    helper.set_block_default(&base, &Block::STONE).await?;
    helper
        .set_block(
            &BlockPos::new(2, 3, 2),
            dripstone(VerticalDirection::Up, SpeleothemThickness::Frustum),
        )
        .await?;
    helper
        .set_block(
            &BlockPos::new(2, 4, 2),
            dripstone(VerticalDirection::Up, SpeleothemThickness::Tip),
        )
        .await?;

    helper
        .start_sequence()
        .then_idle(5)
        .then_execute(move |helper| async move {
            helper
                .assert_block_present(&BlockPos::new(2, 4, 2), &Block::POINTED_DRIPSTONE)
                .await?;
            helper.set_air(&base).await
        })
        .then_idle(20)
        .then_execute(move |helper| async move {
            for y in 3..=4 {
                helper
                    .assert_block_present(&BlockPos::new(2, y, 2), &Block::AIR)
                    .await?;
            }
            helper
                .assert_item_entity_present(
                    "minecraft:pointed_dripstone",
                    &BlockPos::new(2, 1, 2),
                    2.0,
                )
                .await
        })
        .then_succeed();
    Ok(())
}
