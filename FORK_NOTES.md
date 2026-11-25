# Pumpkin 포크 진행 메모

바닐라 미구현 메커니즘을 쉬운 것부터 순서대로 구현하는 작업의 기록입니다.

- 기준 커밋: `cfc4bff` (업스트림 master, 2026-11-24)
- 작업 브랜치: `fork/vanilla-parity`
- 대상 버전: Minecraft 26.3 (`CURRENT_MC_VERSION`)
- 방침: 개인 포크에서 진행. 업스트림이 같은 기능을 올리면 그때 우리 구현과 대조해서 조정.

## 현재 상태 (2026-11-25)

- 빌드·검증 환경 확보됨. 사용자가 제공한 바닐라 26.3 `server.jar`(sha1 `33680f5f…`)를 `PUMPKIN_MINECRAFT_SERVER_JAR`로 지정해 빌드합니다.
- 완료: 항목 1, 기존 린트 오류 3건, 일광 감지기 패닉 방어.
- 진행 중: 항목 2(셜커 상자). 2a 완료, 2b·2c 남음.
- 아직 커밋·푸시하지 않음. 변경분은 패치 파일로 보관 (`01-…`, `02-…`, `03-…`, 전체는 `all-changes.patch`).

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
| 2 | 셜커 상자 뚜껑 상태와 열기 가능 판정 (원래 TODO는 바닐라와 달라 범위를 다시 잡음, 아래 4번 기록) | `shulker_box.rs` | **2a 완료**, 2b·2c 대기 |
| 3 | 셜커 상자 열기/닫기 게임 이벤트 (`CONTAINER_OPEN/CLOSE`) | `entities/shulker_box.rs:176,181` | 대기 |
| 4 | 엔더 상자·상자를 열 때 피글린 분노 | `ender_chest.rs:109`, `chests.rs:398` | 대기 (피글린 AI 쪽 함수 확인 필요) |
| 5 | 침대에서 자는 주민 깨우기 | `bed.rs:269` | 대기 |
| 6 | 물 공급원 판정에 켈프·해초·거품 기둥 포함 | `fluid/flowing_trait.rs:67` | 대기 |
| 7 | 호퍼가 위에 닿은 아이템을 즉시 흡수 (`HopperBlock.entityInside`) | `hopper.rs:57` | 대기 |
| 8 | 뾰족 드립스톤 성장·액체 전달 | `dripstone.rs:49` | 대기 (규모 큼) |
| 9 | 피스톤 남은 검사들 | `piston/*.rs` | 대기 (규모 큼) |

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

## 규칙 메모 (저장소 `AGENTS.md`, `CONTRIBUTING.md`)
- 바닐라 동작은 디컴파일한 소스 기준으로 맞춘다. 위키나 기억 금지.
- 수치는 하드코딩하지 않고 `pumpkin-data` 생성값 사용.
- 새 구현은 레지스트리 등록 필수 (블록 `block/registry.rs`, 아이템 `item/items/mod.rs`, 몹 `entity/type.rs`).
- 몹 AI는 엔티티 아일랜드 범위 신고(`GoalReach`)까지 필요.
- 검사: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets --all-features`, `cargo nextest run -p <crate>`, 부팅 테스트.
- 업스트림에 PR을 낼 경우 AI 에이전트가 직접 여는 것은 금지. 사람이 올려야 함.
