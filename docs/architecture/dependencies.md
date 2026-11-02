# 依赖关系

> 生成时间：2026-11-02
> 证据基线：git commit `d6aca4f`
> 维护方式：代码变更后同步更新，并更新本时间戳

## Crate 级依赖图（箭头 = 依赖方向）

```
examples/* ──→ core(capsule) ──→ capsule-ffi ──→ DPDK 19.11 (C, links="dpdk")
     │              │
     └──→ 间接       └──→ capsule-macros (proc-macro)
bench ──→ core
```

证据：
- workspace 成员：`Cargo.toml:2-14`
- core 依赖 ffi/macros：`core/Cargo.toml:24-25`
- ffi `links = "dpdk"` + bindgen：`ffi/Cargo.toml:9,18-21`

## core 内部模块依赖方向

```
runtime ──→ batch ──→ dpdk ──→ ffi(CDPDK 绑定)
   │           │        ↑
   │           └──→ packets ─┘
   ├──→ config ──→ dpdk(CoreId), net
   └──→ metrics（feature）
```

逐条证据：

| 依赖边 | 证据 |
|---|---|
| runtime → batch | `core/src/runtime/mod.rs:23` `use crate::batch::Pipeline` |
| runtime → config | `core/src/runtime/mod.rs:24` |
| runtime → dpdk | `core/src/runtime/mod.rs:25-27` |
| config → dpdk | `core/src/config.rs:20` `use crate::dpdk::CoreId` |
| config → net | `core/src/config.rs:21` |
| batch → dpdk | `core/src/batch/send.rs`、`rxtx.rs:22-23` `use crate::{KniRx, KniTxQueue, Mbuf, PortQueue}` |
| packets → dpdk | `core/src/packets/mod.rs:22`（Mbuf） |
| dpdk → ffi | `core/src/runtime/core_map.rs:191` `*mut ffi::rte_mempool` 等 |

## 反向依赖与循环依赖检查

- **batch 不依赖 runtime**：batch 只被 runtime 依赖（`Pipeline` trait 由 runtime 消费，`core/src/runtime/mod.rs:23`）。方向干净。✅ [明确]
- **config 依赖 dpdk（CoreId）而非相反**；dpdk 不依赖 config/batch/runtime。dpdk 是唯一的底层叶子。✅ [明确]
- **未见模块级循环依赖** [明确]：上述 8 条边构成 DAG。已检查的潜在嫌疑点（batch↔runtime、packets↔batch）均为单向。
- [未知] `core/src/ffi.rs`（core 内私有 ffi 模块，`core/src/lib.rs:127`）对 capsule-ffi 的再封装范围未逐一盘点，不排除个别间接环；待第 2 步顺带确认。

## 外部 crate 依赖清单（core）

全部声明于 `core/Cargo.toml:22-50`：

- 运行时：`tokio 1.x`（macros/rt/signal/sync/time）、`libc`、`once_cell`、`thiserror`、`anyhow`
- 序列化/配置：`serde`、`toml 0.5`、`clap 2.33`、`regex`
- 可观测：`tracing`、`metrics-core 0.5`、`metrics-runtime 0.13`（optional，metrics feature）
- 测试/基准：`criterion 0.3`、`proptest 1.0`（optional，testils feature）
- 内部：`capsule-ffi`、`capsule-macros`

feature 门控（`core/Cargo.toml:44-50`）：`default = ["metrics"]`；`full = ["metrics","pcap-dump","testils"]`；`compile_failure` 仅用于编译失败测试。

## 隐式全局状态（跨模块共享点）

| 状态 | 位置 | 说明 |
|---|---|---|
| `MEMPOOL: Cell<*mut rte_mempool>` | `core/src/dpdk/mempool.rs:122-127` | thread_local 裸指针，每核初始化时设置（`core/src/runtime/core_map.rs:324,359`） |
| `CURRENT_CORE_ID: Cell<CoreId>` | `core/src/dpdk/mod.rs:176-177` | thread_local 当前核标识 |
| `RECEIVER: OnceCell` / `SINK: Lazy` | `core/src/metrics.rs:86,140` | metrics 全局接收器/汇聚点 |
| `TEST_INIT: Once` | `core/src/testils/mod.rs:35` | 测试环境一次性初始化 |

## 对 CI/构建链的依赖

- `getcapsule/sandbox` 容器 + `--privileged`：check/lint/test/coverage/bench/sanitize 六类 job（证据：`.github/workflows/main.yml:33-35,139-141,208-210,158-160,109-111,231-241,246-248`）。
- check job 三 toolchain 矩阵 stable/beta/nightly，nightly 允许失败（`.github/workflows/main.yml:127-137`）。
- format/audit job 无需容器（`main.yml:189-203,13-26`）。
