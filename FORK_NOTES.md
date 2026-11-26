# Pumpkin 포크 진행 메모

바닐라 미구현 메커니즘을 쉬운 것부터 순서대로 구현하는 작업의 기록입니다.

- 기준 커밋: `cfc4bff` (업스트림 master, 2026-11-24)
- 작업 브랜치: `fork/vanilla-parity`
- 대상 버전: Minecraft 26.3 (`CURRENT_MC_VERSION`)
- 방침: 개인 포크에서 진행. 업스트림이 같은 기능을 올리면 그때 우리 구현과 대조해서 조정.

## 현재 상태 (2026-11-26)

- 저장소: `pocket0426/Pumpkin`, 브랜치 `fork/vanilla-parity`. `master`는 업스트림과 동일하게 유지.
- 빌드에는 바닐라 26.3 `server.jar`(sha1 `33680f5f…`)가 필요합니다. `PUMPKIN_MINECRAFT_SERVER_JAR`로 지정.
- 완료: 항목 1, 2(2a·2b·2c), 3, 4, 5, 6, 린트 정리, 일광 감지기 패닉 방어, **피글린·피글린 브루트 AI의 바닐라 Brain 전환** (14번 기록, 업스트림 PR #3630을 가져와 맞춤).
- 완료: 항목 7(호퍼 즉시 흡수, 15번 기록).
- 완료: 항목 8(드립스톤, 16번 기록).
- 완료: 항목 9(피스톤, 17번 기록). 처음 정한 쉬운 항목 목록 1~9가 모두 끝났습니다.
- 작업 기준(사용자 요청): 동작은 바닐라와 같아야 하고, 자주 호출되는 경로의 비용을 늘리지 않는다.

## 작업 방법 (이 환경 기준)

```bash
export RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu
export PUMPKIN_MINECRAFT_SERVER_JAR=/path/to/server.jar   # 바닐라 26.3
cargo check --locked -p pumpkin-core
cargo clippy --locked -p pumpkin-util -p pumpkin-world -p pumpkin-core --all-targets --all-features
cargo run --locked -p pumpkin-tester      # 게임 테스트 전체 (약 1분 + 빌드)
```

- 바닐라 대조: 디컴파일러를 따로 받지 않고 JDK의 `javap -p -c`로 jar 안의 클래스 바이트코드를 읽습니다. 26.x는 난독화가 없어 메서드 호출 순서를 그대로 확인할 수 있습니다.
- 게임 테스트를 추가할 때는 `builtin_game_tests/mod.rs`의 `BUILTIN_TEST_IDS` 목록과 `register_function_test` 두 곳 모두에 등록해야 실행됩니다.
- 빌드 캐시의 `target/debug/incremental`이 수십 GB까지 커져 디스크가 가득 찬 적이 있습니다. `CARGO_INCREMENTAL=0`으로 빌드하거나 주기적으로 지웁니다.
- 플레이어가 필요한 동작은 `tools/pumpkin-tester/src/bot_tests/`의 봇 테스트로 확인합니다. 패킷 처리 함수를 직접 호출하면 틱 없이 결과를 볼 수 있어 AI 간섭이 없습니다.
- 서버를 그냥 띄워 콘솔 명령으로 확인하는 방법은 플레이어가 없으면 청크가 로드되지 않아 쓸 수 없었습니다. 게임 테스트를 쓰는 편이 낫습니다.

## 단계별 기록

### 0. 환경 확인 — 완료
- Rust 1.97 설치되어 있음. `rust-toolchain.toml`이 채널 동기화를 시도해 실패하므로 `RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu`를 지정해서 실행.
- crates.io 의존성 다운로드는 정상.
- Mojang 다운로드 차단 → 위 "막힌 곳" 참고.

### 1. 후보 조사 — 완료
`crates/pumpkin-core/src/block`, `item`의 TODO를 조사했습니다(블록 37개, 아이템 0개).
쉬운 순서로 정한 작업 목록:

| 순서 | 항목 | 위치 | 상태 |
|---|---|---|---|
| 1 | 흙으로 바뀔 때 위의 엔티티 밀어 올리기 (`Block.pushEntitiesUp`) | `dirt_path.rs`, `farmland.rs` | **완료** |
| 2 | 셜커 상자 뚜껑 상태와 열기 가능 판정 (원래 TODO는 바닐라와 달라 범위를 다시 잡음, 아래 4번 기록) | `shulker_box.rs` | **완료** (2a·2b·2c) |
| 3 | 셜커 상자 열기/닫기 게임 이벤트 (`CONTAINER_OPEN/CLOSE`) | `entities/shulker_box.rs` | **완료** (이벤트 발생원 플레이어는 미전달) |
| 4 | 엔더 상자·상자를 열 때 피글린 분노 | 상자·엔더 상자·통·셜커 상자, 금 블록 등 파괴 | **완료** (분노 지속 시간 등은 피글린 AI 포팅 필요, 11번 기록) |
| 5 | 침대에서 자는 주민 깨우기 | `bed.rs` | **완료** |
| 6 | 물 공급원 판정에 켈프·해초·거품 기둥 포함 | `fluid/flowing_trait.rs` | **완료** |
| 7 | 호퍼가 위에 닿은 아이템을 즉시 흡수 (`HopperBlock.entityInside`) | `hopper.rs` | **완료** (15번 기록) |
| 8 | 뾰족 드립스톤 성장·액체 전달 | `dripstone.rs` | **완료** (업스트림이 이미 구현, 빠진 한 가지만 보완. 16번 기록) |
| 9 | 피스톤 남은 검사들 | `piston/*.rs` | **완료** (17번 기록) |

보류: `tripwire.rs:108`의 "가위 내구도 1 감소" TODO. 바닐라에서는 철사 블록이 아니라 가위 아이템의 채굴 처리 쪽에서 내구도가 깎이는 것으로 기억하는데, 소스로 확인하기 전에는 건드리지 않습니다(이중 차감 위험).

### 2. 항목 1: 엔티티 밀어 올리기 — 완료
변경 파일 5개 (`01-push-entities-up.patch`):
- `block/mod.rs`: `push_entities_up()` 추가. 새 블록이 기존 블록보다 더 차지하는 충돌 영역을 구하고, 그 안의 엔티티(관전자 제외)를 영역 위로 텔레포트.
- `block/blocks/farmland.rs`: `turn_to_dirt()` 추가. 밀어 올리기 → 흙으로 교체 → `BLOCK_CHANGE` 게임 이벤트. 예약 틱, 무작위 틱, 밟아서 망가질 때 세 곳에서 사용.
- `block/blocks/dirt_path.rs`: 예약 틱에서 `turn_to_dirt()` 호출.
- `server/builtin_game_tests/`: 게임 테스트 `pumpkin:dirt_path_pushes_entity_up_when_turning_to_dirt` 추가.

바닐라 26.3 대조 결과 (바이트코드 기준):
- `Block.pushEntitiesUp`: (새 모양 − 기존 모양)을 구해 `getEntities(null, bounds)`로 엔티티를 찾고, 박스를 1칸 올린 뒤 −1만큼 충돌 계산해 `teleportRelative(0, 1+offset, 0)`. 구현과 일치.
- `getEntities(null, box)`의 기본 필터는 `NO_SPECTATORS`. 구현과 일치.
- 26.3에서는 클래스 이름이 `FarmlandBlock.turnToBaseBlock`, `PathBlock.turnToBaseBlock`이고 둘 다 기반 블록이 `DIRT`. 순서는 pushEntitiesUp → setBlockAndUpdate → `BLOCK_CHANGE`(엔티티 컨텍스트). 구현과 일치.
- 호출 위치: 경작지 `tick`(생존 불가 시), `randomTick`(수분 0, 유지 조건 없음), `fallOn`(밟힘), 흙길 `tick`. 구현과 일치.

검증:
- [x] `cargo fmt --check`
- [x] `cargo check -p pumpkin-core`
- [x] `cargo clippy -p pumpkin-core --no-deps`: 변경 파일에 새 경고 없음
- [x] 게임 테스트 206개 전부 통과 (새 테스트 포함)
- [x] 수정을 끈 상태에서는 새 테스트가 실패함을 확인 (`unexpected entity 'minecraft:armor_stand' at 1, 1, 1`)
- [ ] 인게임 확인(사람): 경작지에서 점프해 밟아 흙으로 바뀔 때, 그리고 흙길 위에 서 있는데 위에 블록이 놓일 때 발이 묻히지 않고 흙 위로 올라오는지. 플레이어는 텔레포트 패킷으로 처리되므로 화면이 튀지 않는지도 확인.

남긴 것:
- 바닐라 `PathBlock.getStateForPlacement`는 흙길을 놓을 수 없는 자리에 놓을 때도 `pushEntitiesUp`을 호출합니다. Pumpkin의 `on_place`는 `Arc<World>`를 받지 않아 이번에는 넣지 않았습니다. 흙길 아이템은 서바이벌에서 얻을 수 없어 영향이 작습니다.

관찰한 기존 문제 (3번 기록에서 처리):
- clippy 1.97에서 `pumpkin-world` 2건, `pumpkin-core` 1건이 원래부터 실패.
- 테스트 실행 중 `air is not a valid block for DaylightDetectorLikeProperties` 패닉이 간헐적으로 발생.
- 서버는 기본으로 익명 텔레메트리를 `telemetry.pumpkinmc.org`로 보냅니다. `pumpkin.toml`의 `[telemetry]`에서 끌 수 있습니다.

### 3. 기존 린트 오류와 일광 감지기 패닉 — 완료
패치: `02-lint-and-detector-panic.patch`

린트 3건 (동작 변화 없음):
- `pumpkin-world/src/chunk/io/file_manager.rs`: `Duration::from_secs(60)` → `from_mins(1)`
- `pumpkin-world/src/cylindrical_chunk_iterator.rs`: `all_chunks_within`에 `#[must_use]`
- `pumpkin-core/src/world/mod.rs`: `and_then(...map(...))` → `Option::zip`. 블록 엔티티 조회가 조건과 무관하게 먼저 실행되도록 바뀌었지만 읽기 전용 조회라 결과는 같습니다.
- 결과: `cargo clippy -p pumpkin-util -p pumpkin-world -p pumpkin-core --all-targets --all-features`가 오류 없이 끝남.

일광 감지기 패닉:
- 원인: 월드 틱이 블록 엔티티 목록을 스냅샷으로 떠서 병렬로 틱합니다. 그 사이 감지기 자리가 다른 블록으로 바뀌면, 스냅샷에 남은 감지기 블록 엔티티가 공기 블록의 상태를 감지기 속성으로 읽다가 패닉이 납니다. 20틱마다 한 번만 갱신하므로 타이밍이 맞을 때만 발생합니다.
- 바닐라: `LevelChunk$BoundTickingBlockEntity`가 틱 직전에 `getType().isValid(state)`를 확인하고 맞지 않으면 건너뜁니다.
- 수정: `DaylightDetectorBlock::update_signal_strength`에서 그 자리의 블록이 일광 감지기가 아니면 바로 반환.
- 검증의 한계: 패닉을 의도적으로 재현하는 테스트는 만들지 못했습니다(경합 타이밍 의존). 수정 후 테스트 하네스를 4번 돌려 패닉 0회였지만, 원래도 5번 중 1번 꼴이라 이것만으로 증명되지는 않습니다. 원인 분석과 코드상 방어가 근거입니다.
- 남은 위험: 같은 스냅샷 구조라 다른 블록 엔티티도 비슷한 문제가 있을 수 있습니다. 틱 루프에서 일괄 확인하면 근본 해결이 되지만 매 틱 조회 비용이 들어 이번에는 넣지 않았습니다.

### 4. 항목 2: 셜커 상자 — 2a 완료
패치: `03-shulker-box-lid.patch`

범위 재설정:
- 원래 코드의 TODO는 "닫히는 중에는 열지 않기"였지만, 바닐라 26.3에는 그런 동작이 없습니다. `triggerEvent`는 열림 수가 0이면 CLOSING, 1이면 OPENING으로 바꿀 뿐이고, `canOpen`은 닫히는 중이면 오히려 항상 열 수 있게 합니다.
- 바닐라에 있고 Pumpkin 서버에 없던 것은 (a) 뚜껑 애니메이션 상태, (b) 뚜껑이 막혀 있으면 열리지 않는 판정, (c) 뚜껑이 엔티티를 미는 처리, (d) 이웃 블록 갱신입니다.

2a에서 한 것:
- `block/entities/shulker_box.rs`: `AnimationStatus`(Closed/Opening/Opened/Closing)와 진행도 추가. `trigger_event`(바닐라 `triggerEvent`), `update_animation`(바닐라 `updateAnimation`, 틱당 0.1).
- `block/blocks/shulker_box.rs`: 블록 이벤트를 블록 엔티티로 전달. `can_open`(바닐라 `ShulkerBoxBlock.canOpen`) 추가: 닫힌 상태에서 뚜껑 앞 반 칸 공간에 블록이나 단단한 엔티티가 있으면 열리지 않음.
- `entity/mob/shulker.rs`: `progress_delta_aabb`(바닐라 `Shulker.getProgressDeltaAabb`).
- `pumpkin-util` `BoundingBox::contract`(바닐라 `AABB.contract`).

검증:
- [x] fmt, clippy(전체 타깃) 통과
- [x] 단위 테스트 2개 추가·통과: 뚜껑이 10틱에 열리고 다시 닫힘, 뚜껑 공간 상자가 방향별로 맞음(값은 바닐라 바이트코드에서 손으로 계산)
- [x] 게임 테스트 206개 통과
- [ ] 인게임 확인(사람): 셜커 상자 뚜껑 쪽에 블록을 붙여 놓으면 우클릭해도 안 열리는지, 막지 않으면 정상적으로 열리는지, 여러 방향으로 놓았을 때도 같은지

바닐라와 다른 점:
- 바닐라 `canOpen`은 충돌 검사에 기준 엔티티를 넘기지 않습니다. Pumpkin의 `no_entity_collision`은 기준 엔티티가 필요해 플레이어를 넘겼습니다. 플레이어가 탄 보트가 뚜껑을 막는 경우에만 결과가 다릅니다.

남은 것:
- 2b: `doNeighborUpdates` (뚜껑이 움직이기 시작·끝날 때 이웃 갱신)
- 2c: `moveCollidedEntities` (열리는 뚜껑이 엔티티를 밀어냄)
- 소리: 바닐라는 음량 0.5, 음높이 0.9~1.0 무작위. 현재 구현 확인 필요.
- 바닐라는 열 때 주변 피글린을 화나게 함(`PiglinAi.angerNearbyPiglins`). 작업 목록 4번에서 처리.

### 5. 항목 2b, 3: 셜커 상자 이웃 갱신, 게임 이벤트, 소리 — 완료
커밋 `83172c1`
- 2b: 뚜껑이 움직이기 시작하는 틱과 끝나는 틱에 바닐라 `doNeighborUpdates`와 같은 이웃 갱신(모양 갱신 + 이웃 알림)을 실행.
- 3: 처음 열릴 때 `CONTAINER_OPEN`, 마지막 사람이 닫을 때 `CONTAINER_CLOSE` 게임 이벤트 발생. 스컬크 센서가 반응합니다.
- 소리를 바닐라 값으로: 블록 중심에서 음량 0.5, 음높이 0.9~1.0.
- 바닐라와 다른 점: 바닐라는 게임 이벤트에 연 플레이어를 발생원으로 넘깁니다. Pumpkin의 열람자 추적기가 플레이어를 전달하지 않아 발생원 없이 보냅니다. 상자·통도 같은 구조라 이벤트 자체가 아직 없습니다.
- 검증: clippy, 단위 테스트(이웃 갱신이 첫 틱과 마지막 틱에만 일어나는지 포함), 게임 테스트 206개 통과.
- 인게임 확인(사람): 셜커 상자 옆에 스컬크 센서를 두고 여닫을 때 반응하는지, 소리가 바닐라와 같은지.

### 6. 항목 2c: 뚜껑이 엔티티를 밀어냄 — 보류
- 바닐라 `moveCollidedEntities`는 뚜껑이 지나가는 공간의 엔티티를 뚜껑 방향으로 옮깁니다. 표지·상호작용·디스플레이 엔티티, 효과 구름, 마커 갑옷 거치대는 제외합니다.
- 바닐라에서는 셜커 상자의 충돌 모양이 뚜껑 진행도에 따라 커져서(`getShape` → `getBoundingBox`) 밀린 엔티티가 그 위에 얹혀 있습니다.
- Pumpkin의 충돌 모양은 블록 상태에서만 나오는 고정값이라, 미는 처리만 넣으면 엔티티가 매 틱 올라갔다 떨어지기를 반복합니다. 그래서 넣지 않았습니다.
- 선행 작업: 블록 엔티티 상태에 따라 달라지는 충돌 모양과 지지 모양. 월드 충돌 판정을 건드리는 작업이라 "쉬운 항목" 범위를 넘습니다.

### 7. 항목 4: 상자를 열 때 피글린 분노 — 보류
- 바닐라 `PiglinAi.angerNearbyPiglins`는 반경 16칸의 한가한 피글린에게 분노 대상을 설정합니다.
- Pumpkin의 피글린에는 분노 대상(`ANGRY_AT` 기억)을 설정하는 AI가 아직 없습니다. 몹 AI 작업이 먼저 필요합니다.

### 8. 항목 5: 침대에서 자는 주민 깨우기 — 완료
- 바닐라 26.3 `AbstractBedBlock.useWithoutItem`: 침대가 사용 중이면 `kickVillagerOutOfBed`를 먼저 시도하고, 깨울 주민이 없을 때만 "사용 중" 메시지를 보냅니다. `kickVillagerOutOfBed`는 침대 머리 블록 안에서 자고 있는 첫 번째 주민의 `stopSleeping`을 부릅니다.
- 구현: `bed.rs`에 `kick_villager_out_of_bed` 추가. Pumpkin의 `VillagerEntity::wake_up`이 바닐라 `Villager.stopSleeping`(기상 + `LAST_WOKEN` 기록)에 해당합니다.
- 검증: 봇 테스트 `bot:bed_kicks_sleeping_villager` 추가. 주민을 침대에 재우고 플레이어가 침대를 사용하면 주민이 깨고 침대가 비는지 확인. 수정을 끄면 실패(`sleeping=true, occupied=true`), 켜면 통과.
- 인게임 확인(사람): 밤에 주민이 자는 침대를 우클릭하면 주민이 일어나고 "사용 중" 메시지가 뜨지 않는지.

### 9. 린트에 대한 정정
- 앞서 "기존 린트 오류"라고 한 것들은 업스트림 코드의 결함이라기보다 **이 작업 환경의 Rust(1.97)가 업스트림이 쓰는 최신 stable보다 오래돼서** 생긴 차이로 보입니다. 근거: 이 환경의 clippy가 저장소가 쓰는 린트 이름 일부를 모릅니다(`unknown lint` 경고). `tools/pumpkin-codegen`은 1.97에서 아직 불안정한 표준 라이브러리 기능을 써서 컴파일 자체가 안 됩니다.
- 고친 4건(`from_mins` 2곳, `#[must_use]`, `Option::zip`)은 양쪽 버전에서 모두 유효하고 동작 변화가 없어 그대로 둡니다.
- `pumpkin-codegen`은 건드리지 않았습니다. 생성 코드를 다시 만들어야 하는 작업(데이터 추가 등)은 더 새 Rust가 있는 환경에서 해야 합니다.

### 10. 항목 2c: 열리는 뚜껑의 충돌과 엔티티 밀기 — 완료
커밋 `521d099`. 6번 기록에서 보류했던 것을 선행 작업과 함께 구현.

바닐라 동작 (26.3 바이트코드):
- 셜커 상자의 충돌 모양은 블록 한 칸에 뚜껑 방향으로 `0.5 × 진행도`만큼 늘어난 상자 (`getShape` → `ShulkerBoxBlockEntity.getBoundingBox`).
- `moveCollidedEntities`: 열리는 동안 매 틱, 뚜껑이 이번 틱에 지나간 공간의 엔티티를 뚜껑 방향으로 `(지나간 두께 + 0.01)`만큼 `Entity.move`. 관전자와 피스톤 반응이 `IGNORE_ENTITY`인 엔티티는 제외.
- `IGNORE_ENTITY`를 돌려주는 엔티티: 효과 구름, 디스플레이 3종, 상호작용, 마커, 불길한 아이템 생성기, 마커 상태의 갑옷 거치대.

구현:
- `world/shulker_lids.rs`: 닫혀 있지 않은 뚜껑이 블록 밖으로 차지하는 공간의 목록. 블록 엔티티가 매 틱 갱신하고, 닫히거나 블록이 교체되면 제거. 청크 언로드 등으로 갱신이 끊긴 항목은 월드 틱에서 정리(최대 2틱 잔존).
- `World::get_block_collisions`, `World::is_space_empty`가 이 목록을 함께 봅니다.
- `EntityBase::get_piston_push_reaction` 추가(기본 `Normal`), 위 엔티티 7종에서 `Ignore`.
- `ShulkerBoxBlockEntity::move_collided_entities`.

성능:
- 충돌 판정은 서버에서 가장 자주 불리는 경로라, 뚜껑이 열린 셜커 상자가 하나도 없을 때는 원자 변수 읽기 한 번만 추가됩니다. 블록마다 블록 엔티티를 조회하는 방식은 쓰지 않았습니다.
- 열린 상자가 있을 때만 잠금을 잡고 그 목록(보통 몇 개)을 훑습니다.
- 닫힌 셜커 상자의 틱은 상태 읽기와 진행도 쓰기뿐입니다.
- 수치로 측정한 벤치마크는 아직 없습니다. 설계상의 비용만 따졌습니다.

검증:
- 봇 테스트 `bot:shulker_lid_pushes_entity`: 셜커 상자 위의 갑옷 거치대가 열릴 때 0.5칸 올라가고 닫히면 제자리로 돌아오는지 확인. 뚜껑 충돌 등록을 끄면 실패(0칸), 켜면 통과.
- 반복 실행에서 이 테스트의 실패 없음.

바닐라와 다른 점:
- 플레이어는 서버가 밀지 않습니다. Pumpkin은 플레이어 이동을 클라이언트에 맡기고, 클라이언트가 같은 뚜껑 애니메이션으로 스스로 밀려납니다(피스톤도 같은 방식).
- 아직 뚜껑 모양을 보지 않는 곳: 지지 모양(`getBlockSupportShape`, 열린 상자에 붙은 블록), 길찾기, 엔티티가 블록 안에 끼었는지 판정.

### 11. 항목 4: 피글린 분노 — 현재 AI 범위 안에서 완료
바닐라 `PiglinAi.angerNearbyPiglins(level, player, requireLineOfSight)`:
- 플레이어 경계 상자를 16칸 넓힌 범위의 피글린(브루트 제외) 중 한가한(`IDLE`) 개체만, 시야 조건이 있으면 플레이어를 볼 수 있는 개체만 대상으로 `setAngerTarget`.
- 호출 위치: 통·상자·엔더 상자·셜커 상자를 열 때(시야 필요), `guarded_by_piglins` 태그 블록을 플레이어가 부술 때(시야 불필요), 상자 광산 수레·상자 보트.
- `setAngerTarget`: 시야를 무시한 공격 가능 판정을 통과하면 `ANGRY_AT` 기억을 600틱 설정. 보편 분노 게임 규칙이 켜져 있으면 추가 처리.

구현:
- `PiglinAi::anger_nearby_piglins`, `set_anger_target` 추가. 통·상자·엔더 상자·셜커 상자의 열기 처리와 `World::break_block`에서 호출.
- 판정 조건(범위, 한가함, 시야, 공격 가능 여부)은 바닐라와 같습니다.

바닐라와 다른 점 (사용자 요청 기준에 못 미치는 부분):
- Pumpkin의 피글린은 바닐라의 Brain(기억·활동) 구조가 아니라 목표(goal) 기반의 간이 구현입니다. 그래서 분노를 `ANGRY_AT` 기억이 아니라 "공격 대상 지정"으로 넣었습니다.
- 그 결과 600틱 뒤 분노가 풀리는 시점, 보편 분노 규칙, 주변 피글린에게 분노가 퍼지는 동작은 바닐라와 다를 수 있습니다.
- 이것까지 맞추려면 피글린 Brain 전체를 포팅해야 합니다. 트래커에서도 피글린은 부분 구현이고, 몹 AI 작업이라 규모가 큽니다.
- 상자 광산 수레·상자 보트를 열 때의 분노는 넣지 않았습니다.

검증:
- 봇 테스트 `bot:piglin_angered_by_opening_barrel`: 피글린 옆에서 통을 열면 크리에이티브 플레이어에게는 화내지 않고 서바이벌 플레이어는 공격 대상이 되는지 확인.
- 인게임 확인(사람): 네더에서 금 갑옷을 입고 피글린 근처의 상자를 열면 공격해 오는지, 금 블록을 캘 때도 그런지, 얼마 뒤에 분노가 풀리는지.

### 12. 항목 6: 켈프·해초·거품 기둥을 물 공급원으로 — 완료
- 바닐라는 `getFluidState()`로 판정하므로 물에 잠긴 블록뿐 아니라 켈프, 해초, 거품 기둥도 물 공급원입니다.
- Pumpkin에는 추출 데이터로 만든 블록 상태 플래그(`is_liquid`: 유체 상태가 비어 있지 않음)가 이미 있어서, 블록을 하나씩 비교하지 않고 그 플래그 한 번으로 판정하게 바꿨습니다. 이전의 물에 잠김 속성 검사보다 가볍습니다.
- 검증: 게임 테스트 `pumpkin:water_source_forms_next_to_kelp` 추가. 물 공급원과 켈프 사이의 빈 칸이 공급원으로 바뀌는지 확인. 수정 전 코드로는 실패, 수정 후 통과.

### 13. 테스트 하네스에서 관찰한 기존 문제 (우리 변경과 무관)
- 시작 직후 멈춤: 약 30회 중 2~3회, 하네스가 첫 테스트 전에 멈췄습니다. 스레드 덤프를 보니 청크 생성 스케줄러(`GenerationSchedule::recompute_dependency_stages`)만 돌고 있고 우리 코드는 스택에 없었습니다. 업스트림 청크 시스템의 문제로 보입니다.
- 지형에 따라 실패하는 기존 봇 테스트: 참나무 다락문 열기, 눈 골렘의 눈 생성 취소. 봇 주변 지형이 시드마다 달라서 생기는 것으로 보입니다.
- 새로 추가한 봇 테스트도 처음에는 같은 이유로 간헐적으로 실패했습니다. 블록을 놓을 자리를 비우고, 봇이 서 있는 청크 안에서만 작업하도록 고친 뒤로는 반복 실행에서 실패가 없습니다.

### 14. 피글린 AI를 바닐라 Brain 구조로 포팅 — 완료 (브랜치 `fork/piglin-brain`)

처음에는 직접 단계별로 포팅하려 했으나, 업스트림에 같은 작업의 PR이 이미 있어 그것을 가져와 맞추는 쪽으로 바꿨습니다.

- 가져온 것: 업스트림 PR #3630 "feat(entity): move piglins and piglin brutes onto the brain" (작성자 wiflow, 커밋 `87fd9e75`). 피글린과 피글린 브루트 둘 다 포함.
- 그 PR은 옛 경로(`crates/pumpkin/src`)와 옛 Brain API 기준이라 그대로는 컴파일되지 않았습니다(오류 203개). 현재 프레임워크에 맞춰 고쳤습니다.

포크에서 추가로 한 일:

| 내용 | 바닐라 근거 |
|---|---|
| 창 행동 3종 `SpearApproach`, `SpearAttack`, `SpearRetreat` 포팅 후 전투 활동에 연결 (PR에 없던 부분) | `PiglinAi.initFightActivity`, 세 클래스 바이트코드 |
| 상자·통·셜커 상자·엔더 상자를 열거나 금 블록류를 캘 때의 분노를 Brain 기준으로 다시 작성 (PR에 없던 부분) | `PiglinAi.angerNearbyPiglins` |
| `Behavior.timedOut`을 재정의하는 행동을 위한 `can_time_out` 훅 | `SpearApproach`, `SpearAttack`은 시간 초과 없음 |
| 원거리 무기 사거리를 반영한 `isWithinAttackRange(mob, target, margin)` | `BehaviorUtils.isWithinAttackRange` |
| UUID 기억에서 플레이어도 찾도록 수정 (분노 대상이 플레이어일 때 필요) | `Level.getEntity(UUID)` |
| 아이템 줍기, 환경음, 발소리, 아기 플래그를 마스터의 공용 훅으로 연결 | 마스터가 PR 기준점 이후 바꾼 구조 |
| Brain 우편함(`post_to_brain`) 최적화: 우편이 없으면 잠금 없이 원자 플래그만 확인, Brain 잠금은 틱당 한 번 | — |
| AI가 꺼진 몹은 우편을 받지 않음 (비워 줄 틱이 없어 쌓이기만 하므로) | 바닐라와 차이: 바닐라는 기억에 쓰지만 AI가 꺼진 동안 관찰되지 않음 |

바닐라와 대조한 것 (26.3 `javap`):
- 활동 7종(`CORE`, `IDLE`, `ADMIRE_ITEM`, `FIGHT`, `CELEBRATE`, `AVOID`, `RIDE`)의 행동 목록, 순서, 상수.
- `updateActivity`의 활동 우선순위와 뒤처리(공격 자세, 아기 내리기, 춤).
- 창 행동 3종의 시작 조건, 지속 조건, 거리 계산, 기억 정리.
- `angerNearbyPiglins`의 범위 16, 한가한 피글린만, 시야 조건, `universalAnger` 분기.

대조하지 않고 PR을 믿은 것: 물물교환 던지기 세부, 브루트 AI 전체, 타기·내리기, 쇠뇌 공격의 틱 단위 동작.

테스트:
- 단위 `spear_holder_approaches_then_charges_then_hands_over_to_retreat`: 접근 → 돌진 → 후퇴 인계, 돌진 길이가 무기의 피해 구간과 같은지.
- 봇 `bot:piglin_angered_by_opening_barrel`: 금 투구를 쓴 플레이어가 통을 열 때 크리에이티브면 무반응, 서바이벌이면 `ANGRY_AT`이 그 플레이어. Brain은 틱에 반응하므로 테스트가 서버 틱을 직접 돌립니다.
- 테스트 작성 중 확인된 정상 동작: 금 갑옷이 없는 서바이벌 플레이어는 보이는 즉시 공격 대상이 됩니다.

게임 안에서 사람이 확인할 것:
1. 네더에서 금 갑옷 없이 접근하면 공격, 금 갑옷을 입으면 무시.
2. 금 갑옷을 입고 상자를 열거나 금 블록을 캐면 주변 피글린이 화냄.
3. 금괴를 던지면 주워서 살펴본 뒤 물물교환 아이템을 던짐.
4. 쇠뇌 피글린이 장전 후 발사하고 가까우면 물러남.
5. 금 창을 든 피글린이 접근 → 돌진 → 물러남을 반복.
6. 영혼불·영혼 랜턴 등 기피 블록 근처를 피함, 좀비화 피글린을 피함.
7. 어른 피글린이 호글린을 사냥하고 잡은 뒤 춤춤. 아기 피글린이 아기 호글린에 탐.
8. 오버월드에서 15초 뒤 좀비화, 인벤토리 아이템을 떨어뜨림.
9. 브루트는 금 갑옷과 무관하게 공격하고 물물교환을 하지 않음.
10. 서버 저장 후 재시작해도 피글린의 인벤토리와 상태가 유지됨.

남은 것:
- 호글린은 아직 목표(Goal) 기반입니다. 피글린이 호글린 수를 세는 쪽은 동작하지만 호글린 자체의 Brain 전환은 별도 작업입니다.
- 업스트림이 #3630을 병합하면 포크와 충돌합니다. 그때 업스트림 판을 기준으로 삼고 위 표의 추가분만 다시 얹으면 됩니다.

### 15. 항목 7: 호퍼가 그릇 안의 아이템을 즉시 흡수 — 완료

바닐라 (`HopperBlock.entityInside` → `HopperBlockEntity.entityInside`, 26.3 바이트코드):
- 호퍼 칸에 걸친 아이템 엔티티가 `SUCK_AABB`(그릇 바닥 11/16부터 위 블록 꼭대기까지)와 겹치면 `tryMoveItems`를 통해 바로 넣습니다.
- `tryMoveItems`를 거치므로 쿨다운 중이거나 꺼진(레드스톤 신호) 호퍼는 받지 않고, 성공하면 쿨다운 8이 걸립니다. 이때 호퍼 안에 있던 아이템의 배출도 같이 일어납니다.
- 호퍼 자체의 끌어오기(`suckInItems`)와 달리 위에 꽉 찬 블록이 있어도 막히지 않습니다. 이전 구현과 눈에 보이는 차이는 이 경우입니다.

구현:
- `HopperBlock::on_entity_collision` → `HopperBlockEntity::entity_inside`.
- `try_move_items`가 바닐라처럼 동작 함수를 받도록 바꿨고, 플러그인 줍기 이벤트(`InventoryPickupItemEvent`)는 두 경로가 같은 함수를 씁니다.
- 호퍼 틱과 엔티티 틱이 다른 작업자에서 동시에 들어와도 한 틱에 두 번 옮기지 않도록 호퍼별 잠금으로 직렬화했습니다.
- 비용: 그릇에 놓인 아이템은 매 틱 묻지만, 쿨다운 중이면 원자값 하나만 읽고 끝납니다.

테스트: 게임테스트 `pumpkin:covered_hopper_takes_item_inside_its_bowl` (위를 돌로 덮은 호퍼의 그릇 안에 아이템 생성 → 호퍼에 들어감). 훅을 끄면 실패하는 것을 확인했습니다.

게임 안에서 확인할 것: 위가 막힌 호퍼 옆에서 물길로 아이템을 그릇 안으로 밀어 넣으면 들어가는지, 레드스톤으로 잠근 호퍼는 받지 않는지.

### 16. 항목 8: 뾰족 드립스톤 — 완료 (대부분 업스트림이 이미 구현)

조사 결과 `dripstone.rs:49`의 TODO는 낡은 주석이었습니다. 무작위 틱, 액체 전달(진흙 → 점토, 가마솥 채우기), 성장(유황 가시 포함)이 이미 들어 있습니다.

바닐라 26.3 바이트코드(`PointedDripstoneBlock.randomTick`, `maybeTransferFluid`, `SpeleothemBlock.randomTick`, `growStalactiteOrStalagmiteIfPossible`)와 대조한 결과:
- 확률(물 0.17578125, 용암 0.05859375, 성장 0.011377778), 탐색 길이 11, 판정 순서, 가마솥 틱 지연(50 + 낙하 거리), 월드 이벤트 1504, 게임 이벤트가 일치합니다.
- 빠진 것 하나: 진흙이 점토로 바뀔 때의 `Block.pushEntitiesUp`. 진흙은 점토보다 2픽셀 낮아서, 위에 서 있던 엔티티를 점토 위로 올려야 합니다. 항목 1에서 만든 함수로 보완했습니다.

참고 (테스트를 쓰다 다시 확인한 규칙): 종유석은 진흙에 직접 달리는 것이 아니라 진흙 **아래 블록**에 달려 있어야 합니다.

테스트: 봇 `bot:dripstone_dries_mud_and_lifts_entity` (진흙 위 갑옷 거치대가 0.125 올라가는지). 밀어 올리기를 빼면 실패하는 것을 확인했습니다.

### 17. 항목 9: 피스톤 — 완료

바닐라 26.3(`PistonBaseBlock`, `PistonMovingBlockEntity`, `PistonMath`, `Entity.limitPistonMovement` 바이트코드)과 대조해 찾은 차이를 모두 처리했습니다.

| 번호 | 내용 | 바닐라 | 상태 |
|---|---|---|---|
| 9a | 밀 수 있는지 판정에 월드 높이·월드 경계 검사 | `PistonBaseBlock.isPushable` | 완료 |
| 9b | 끈끈이 피스톤이 블록을 두고 오는 조건 | `checkIfExtend`, `triggerEvent` | 완료 |
| 9c | 움직이는 블록이 엔티티를 미는 처리 | `moveCollidedEntities`, `moveStuckEntities`, `fixEntityWithinPistonBase` | 완료 |
| 9d | 움직이는 중인 블록의 저장·불러오기 | `loadAdditional`, `saveAdditional` | 완료 |
| 9e | 움직이는 블록의 충돌 판정 (조사 중 추가로 발견) | `PistonMovingBlockEntity.getCollisionShape` | 완료 |
| 9f | 도착 처리: 버틸 수 없는 블록은 부서져 드롭, 물에 잠긴 블록은 마른 채 도착, 자기 자신에게 이웃 갱신 | `PistonMovingBlockEntity.tick`, `finalTick` | 완료 |

9a:
- 위치가 월드 높이 밖이거나 월드 경계 밖이면 공기여도 밀 수 없음. 최상단에서 위를 보는 피스톤은 뻗지 못합니다(이전에는 뻗었음).
- 최하단 블록을 아래로, 최상단 블록을 위로 미는 것은 불가. 월드 경계는 바닐라처럼 블록의 최소 꼭짓점 한 점만 검사.

9b:
- 수축 이벤트를 처리할 때 밀던 블록이 아직 움직이는 중이면, 진행도와 무관하게 그 자리에 내려놓고 끌어오지 않습니다(이전에는 진행도 0.5 미만일 때만). 2틱 펄스로 블록을 "뱉는" 잘 알려진 동작입니다. 3틱 펄스부터는 끌어옵니다.
- 바닐라의 `isHandlingTick` 조건은 넣지 않았습니다. 바닐라는 블록 이벤트를 같은 틱 안에서 처리하고 Pumpkin은 다음 틱 시작에 처리하는데, 펄스 길이별로 따져 보면 나머지 두 조건(`progress < 0.5`, `gameTime == lastTicked`)만으로 같은 결과가 나옵니다. 코드 주석에 이유를 남겼습니다.

9c:
- 밀어내는 영역을 바닐라처럼 "앞면이 이번 틱에 지나가는 얇은 판"으로 계산하고, 블록의 실제 충돌 모양(반블록, 계단, 피스톤 머리와 팔)을 씁니다. 이전에는 블록 전체를 직육면체로 보고 머리는 4픽셀 판으로 근사했습니다.
- 이동은 벽에 막히는 `Entity.move(PISTON)` 방식이고, 한 틱에 축당 0.51까지만 밀립니다(`limitPistonMovement`). 피스톤 여러 개가 같은 엔티티를 동시에 밀 때 두 배로 밀리지 않습니다.
- 슬라임 블록은 미는 방향으로 속도 1을 줍니다. 꿀 블록은 옆으로 움직일 때 위에 선 엔티티를 데려갑니다.
- 밀림 반응이 `IGNORE`인 엔티티는 밀지 않습니다.
- **정정**: 지난 보고에서 "플레이어가 피스톤에 밀리지 않는다"고 했는데 틀렸습니다. 플레이어는 클라이언트가 같은 계산을 직접 하고 위치를 보고하므로 원래부터 밀렸고, 서버가 건드리지 않는 지금 구조가 맞습니다. 슬라임 튕김도 플레이어는 클라이언트 쪽에서 처리됩니다.

9e:
- 움직이는 블록(`moving_piston`)은 블록 상태에 충돌 모양이 없어서, 이전에는 이동하는 2틱 동안 몹과 아이템이 블록을 통과했습니다. 옆으로 밀리는 블록 위에 서 있던 엔티티가 블록 속으로 떨어진 뒤 같이 밀려갔습니다.
- 셜커 상자 뚜껑용으로 만든 등록부를 일반화해(`world/dynamic_collisions.rs`) 움직이는 블록의 현재 위치를 충돌에 포함시켰습니다. 수축 중인 피스톤은 몸통도 포함합니다.
- 움직이는 블록이 없으면 원자값 하나만 읽습니다.
- 바닐라의 `NOCLIP`(밀리는 엔티티가 자기를 미는 블록에는 막히지 않게 하는 장치)은 따로 두지 않았습니다. 충돌 상자가 밀기 **전** 진행도 기준이라 밀리는 엔티티는 항상 그 앞에 있습니다.

9f:
- 밀려간 자리에서 버틸 수 없는 블록(받침 없는 레일 등)은 도착 후 부서져 아이템을 떨어뜨립니다.
- 물에 잠긴 블록은 물 없이 도착합니다.
- 도착한 블록 자신에게 이웃 갱신을 보내서, 밀려간 피스톤이나 조명이 전원을 다시 읽습니다.

테스트 (게임테스트 7개 추가, 단위 1개):
- `piston_pushes_entity_ahead`, `pushed_honey_block_carries_entity`, `pushed_plain_block_leaves_entity_behind`, `pushed_rail_without_support_drops`, `pushed_waterlogged_slab_arrives_dry`, `sticky_piston_short_pulse_leaves_block`, `sticky_piston_longer_pulse_pulls_block_back`, 단위 `nothing_is_pushed_out_of_the_world`.
- 이전 피스톤 코드로 되돌려 돌리면 `pushed_plain_block_leaves_entity_behind`, `pushed_waterlogged_slab_arrives_dry`, `sticky_piston_short_pulse_leaves_block`가 실패합니다. 레일 테스트는 이전 코드에서 실행마다 결과가 달랐습니다. 나머지 셋은 이전 코드에서도 통과하는 회귀 방지용입니다(꿀 블록은 이전 코드에서 엔티티가 블록 속으로 빠진 채 밀려가 같은 칸에 도착했습니다).

게임 안에서 사람이 확인할 것:
1. 옆으로 밀리는 블록 위에 몹을 세워 두면 블록만 빠져나가고 몹은 제자리에서 떨어지는지. 꿀 블록이면 같이 가는지.
2. 슬라임 블록으로 몹·아이템을 밀면 튕겨 나가는지.
3. 관측기 펄스(2틱)로 끈끈이 피스톤이 블록을 뱉는지, 긴 신호에서는 끌어오는지.
4. 플레이어가 피스톤에 밀릴 때 이전과 다르게 덜컹거리지 않는지(서버 쪽 충돌이 새로 생겼으므로).
5. 피스톤이 움직이는 도중 서버를 저장·재시작해도 블록이 사라지지 않는지.

참고: `piston_extension.rs`의 "TODO: use player"는 바닐라에 해당하는 동작이 없습니다(`MovingPistonBlock.destroy`는 플레이어 없이 블록만 제거). 그대로 둡니다.

### 18. 2차 목록 — 진행 중

1~9가 끝나서 `block/`에 남은 TODO 21개를 다시 훑어 다음 목록을 정했습니다.

| 순서 | 항목 | 위치 | 상태 |
|---|---|---|---|
| 10 | 종: 주민에게 들린 시각 기록, 약탈자 발광, 클라이언트 흔들림 | `entities/bell.rs`, `redstone/bell.rs` | **완료** |
| 11 | 철사 가위 내구도 TODO | `redstone/tripwire.rs` | **정리** (바닐라에 없는 동작, 주석만 교체) |
| 12 | 주크박스 `setTheItem` 순서 (아이템 → `HAS_RECORD` → 재생/정지) | `entities/jukebox.rs` | **완료** (순서와 중복 이벤트만 정리) |
| 13 | 스포너·시련 스포너의 `SpawnData` NBT 적용 | `entities/mob_spawner.rs`, `trial_spawner.rs`, 스포너 광산 수레 | **완료** |
| 14 | 용기의 전리품 표 풀기 누락분 (통·셜커 상자·발사기·공급기·호퍼·제작기) | `entities/*.rs`, `player.rs`, `world/mod.rs` | **완료** |
| 15 | 용기 잠금(`lock` 컴포넌트) | `blocks/chests.rs` | **정리** (`Player::open_handled_screen`에 이미 구현돼 있어 낡은 주석만 삭제) |

10번 세부 (`BellBlockEntity` 26.3 바이트코드):
- 종을 치면 블록 이벤트(종류 1, 방향)를 큐에 넣습니다. 클라이언트는 이 이벤트로 종을 흔듭니다(이전에는 보내지 않았음).
- 이벤트 처리에서 반경 48의 생물을 찾고(최대 3초에 한 번), 32 안의 살아 있는 생물의 Brain에 `HEARD_BELL_TIME`을 씁니다. 이 기억을 등록한 것은 주민뿐입니다.
- 흔들림 5틱 뒤 32 안에 약탈자(`raiders` 태그)가 있으면 공명음을 내고, 40틱 뒤 48 안의 약탈자에게 발광 3초를 겁니다.
- 주민은 자기 Brain 틱 안에서 종을 치기 때문에, 탐색을 이벤트로 미루지 않으면 자기 Brain을 다시 잠그게 됩니다. 바닐라도 같은 순서(큐 → 다음에 처리)입니다.
- Brain을 쓰지 않는 몹에 Brain을 새로 만들지 않도록 `with_existing_brain`을 추가했습니다.
- 테스트: 봇 `bot:bell_ring_alerts_villagers_and_reveals_raiders`.

게임 안에서 확인할 것: 종을 치면 흔들리는지, 근처 주민이 집으로 숨는지, 습격 중 종을 치면 약탈자가 발광하는지.

14번 세부 (`RandomizableContainerBlockEntity`):
- 이전에는 상자만 전리품 표를 가졌습니다. 통·발사기·공급기·셜커 상자·호퍼·제작기는 `LootTable` NBT를 읽지 않아, 구조물로 생성된 통(어부 집 등)이나 발사기(정글 사원)가 비어 있었고 저장하면 표 자체가 사라졌습니다.
- 공용 `PendingLoot`(표 이름 + 시드)를 만들어 여섯 블록 엔티티에 넣었습니다. 표가 남아 있는 동안에는 아이템 대신 표를 저장합니다.
- 바닐라가 슬롯을 들여다보는 모든 자리에서 풉니다: 플레이어가 열 때, 호퍼가 접근할 때, 비교기가 읽을 때, 발사기·공급기·제작기가 작동할 때, 블록이 부서지거나 교체될 때.
- 관전자는 풀지 못하고 열리지도 않습니다(`container.spectatorCantOpen`).
- 풀기 함수는 표가 없으면 맨 앞에서 바로 돌아오므로 호퍼·비교기 경로의 비용은 검사 한 번입니다.
- 테스트: 단위 2개(`packed_barrel_saves_its_table_instead_of_items`, `unpacking_fills_the_barrel_once_and_forgets_the_table`), 봇 `bot:barrel_loot_table_unpacks_for_first_player`.

게임 안에서 확인할 것: 마을 어부 집의 통, 정글 사원 발사기에 내용물이 있는지. `/setblock ~ ~ ~ barrel{LootTable:"minecraft:chests/simple_dungeon"}`로 만든 통을 열면 채워지는지, 열기 전에 부수면 전리품이 떨어지는지.

13번 세부 (`BaseSpawner` 26.3 바이트코드):
- 이전에는 스포너가 엔티티 종류 하나만 기억했습니다. `SpawnData`의 나머지(엔티티 NBT, `custom_spawn_rules`, `equipment`)와 `SpawnPotentials`는 읽지 않았고, **저장하면 사라졌습니다**. 명령어나 맵으로 만든 커스텀 스포너가 평범한 스포너로 바뀌던 문제입니다.
- 이제 `SpawnData`와 `SpawnPotentials`를 그대로 보존하고 씁니다.
  - 엔티티 태그가 `id`뿐이면 이전처럼 자연 스폰처럼 마무리(`finalizeSpawn`)하고, 그 외에는 태그대로 불러옵니다(장비·이름·`NoAI` 등).
  - 태그에 `Pos`가 있으면 그 자리에만 스폰합니다.
  - `custom_spawn_rules`가 있으면 몹 고유의 스폰 규칙 대신 밝기 범위만 봅니다. 평화로움에서 적대적 몹은 여전히 안 나옵니다.
  - 대기 시간이 다시 정해질 때마다 `SpawnPotentials`에서 가중치로 다음 것을 고릅니다.
  - `equipment` 전리품 표를 굴려 장비를 입힙니다(시련 스포너에 있던 코드를 공용으로 뺌). 드롭 확률은 아직 반영하지 않습니다(TODO).
  - 게임 규칙 `spawnerBlocksWork`를 따릅니다.
  - 주변 개수 검사 범위를 바닐라대로 스포너 칸을 스폰 범위만큼 키운 상자로 바꿨습니다(이전에는 가로 2배, 세로 4 고정).
  - 스폰 시 `ENTITY_PLACE` 게임 이벤트.
- 스포너 광산 수레도 같은 데이터를 씁니다. 시련 스포너는 아기·슬라임 크기만 따로 처리하던 것을 태그 전체를 불러오도록 바꿨습니다(기존 특수 처리는 남겨 둠).
- 테스트: 단위 7개(`mob_spawner::tests`), 봇 `bot:spawner_spawns_the_entity_its_data_describes`.
- 알게 된 것: 게임테스트 구역에는 플레이어가 없어서 스포너가 돌지 않습니다. 스포너 테스트는 봇 테스트로 써야 합니다. 이를 위해 테스터에 `pumpkin-nbt` 의존성을 추가했습니다(`Cargo.lock` 한 줄).

12번 세부:
- 레코드를 꺼낼 때 바닐라 순서(블록의 `HAS_RECORD` 해제 → 노래 정지)로 맞추고, 정지 이벤트가 두 번 나가던 것과 재생 중이 아닐 때도 나가던 것을 없앴습니다. 눈에 보이는 차이는 거의 없고, 스컬크 센서가 받는 이벤트 순서·횟수가 달라집니다.

게임 안에서 확인할 것: `/setblock ~ ~ ~ spawner{SpawnData:{entity:{id:"minecraft:zombie",CustomName:"테스트",NoAI:1b}}}`로 만든 스포너에서 이름 붙은 좀비가 나오는지, 저장·재시작 뒤에도 그대로인지. 시련의 방 스포너가 이전처럼 동작하는지.

## 규칙 메모 (저장소 `AGENTS.md`, `CONTRIBUTING.md`)
- 바닐라 동작은 디컴파일한 소스 기준으로 맞춘다. 위키나 기억 금지.
- 수치는 하드코딩하지 않고 `pumpkin-data` 생성값 사용.
- 새 구현은 레지스트리 등록 필수 (블록 `block/registry.rs`, 아이템 `item/items/mod.rs`, 몹 `entity/type.rs`).
- 몹 AI는 엔티티 아일랜드 범위 신고(`GoalReach`)까지 필요.
- 검사: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets --all-features`, `cargo nextest run -p <crate>`, 부팅 테스트.
- 업스트림에 PR을 낼 경우 AI 에이전트가 직접 여는 것은 금지. 사람이 올려야 함.
