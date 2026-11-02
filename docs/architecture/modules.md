# 模块明细

> 生成时间：2026-11-02
> 证据基线：git commit `d6aca4f`
> 维护方式：代码变更后同步更新，并更新本时间戳

## core crate（`capsule`）

顶层模块声明（证据：`core/src/lib.rs:124-147`）：

| 模块 | 可见性 | 职责 | 关键证据 |
|---|---|---|---|
| `batch` | pub | 声明式包处理组合子（Batch trait + 组合子） | `core/src/batch/mod.rs:121-221` |
| `config` | pub | TOML 运行时配置 + EAL 参数生成 | `core/src/config.rs:177-256` |
| `metrics` | pub（feature = "metrics"，默认开启） | metrics-runtime 封装 | `core/src/lib.rs:129-131`、`core/Cargo.toml:45` |
| `net` | pub | MacAddr/Cidr 等网络基础类型 | `core/src/config.rs:21` 引用 |
| `packets` | pub | 包类型系统（Packet trait + 各协议） | `core/src/packets/mod.rs:53-80` |
| `testils` | pub（test/testils feature） | 测试工具与固定 pcap 字节流 | `core/src/lib.rs:138-140` |
| `dpdk` | **私有** | DPDK 安全封装（Mbuf/Mempool/Port/KNI/CoreId 等） | `core/src/lib.rs:126`、`core/src/dpdk/mod.rs:19-34` |
| `ffi` | 私有 | 对 capsule-ffi 的再封装入口 | `core/src/lib.rs:127` |
| `runtime` | 私有 | Runtime、执行器模型、信号/超时主循环 | `core/src/lib.rs:137` |
| `pcap` | 私有（feature = "pcap-dump"） | 端口流量抓包落盘 | `core/src/lib.rs:134-136` |

顶层导出：`KniRx, KniTxQueue, Mbuf, PortQueue, SizeOf, Runtime, UnixSignal`（证据：`core/src/lib.rs:142-144`）。

### dpdk 子模块（私有，FFI 封装层）

| 文件 | 职责 | 证据 |
|---|---|---|
| `dpdk/mod.rs` | eal_init/eal_cleanup、CoreId、CURRENT_CORE_ID thread_local | `core/src/dpdk/mod.rs:176-177` |
| `dpdk/mbuf.rs` | Mbuf 包缓冲区（unsafe 密集：25 处） | grep 统计 |
| `dpdk/mempool.rs` | Mempool + 每线程 `MEMPOOL: Cell<*mut rte_mempool>` | `core/src/dpdk/mempool.rs:122-127` |
| `dpdk/port.rs` | PortBuilder/Port/PortQueue（unsafe 14 处） | grep 统计 |
| `dpdk/kni.rs` | KNI 内核网卡接口 rx/tx（unsafe 16 处） | `core/src/dpdk/kni.rs:48-119,181-220` |
| `dpdk/queue.rs` 等 | 收发队列 | `core/src/dpdk/mod.rs:19-34` |

### batch 子模块（组合子）

- `Batch` trait：`replenish/next` 为基础方法，链式组合子 `emit/filter/filter_map/map/for_each/...`（证据：`core/src/batch/mod.rs:121-221`）。
- 组合子文件：`emit/filter/filter_map/for_each/group_by/inspect/map/poll/replace/rxtx/send`（证据：`core/src/batch/mod.rs:21-43`）。
- `Send` batch：把 batch 变成 Future/Pipeline 接入运行时，处理 Act/Drop/Emit/Abort 分发（证据：`core/src/batch/send.rs:41-149`）。
- `Pipeline` trait：runtime 安装 pipeline 的类型约束（证据：`core/src/runtime/mod.rs:23`）。

### packets 子模块（类型系统）

- `Packet` trait 定义：Envelope/mbuf/offset/header_len 等抽象（证据：`core/src/packets/mod.rs:53-80`）。
- 公开子模块 `arp/checksum/icmp/ip/types`；`ethernet/tcp/udp` 私有实现经 `pub use` 暴露（证据：`core/src/packets/mod.rs:21-32`）。

### runtime 子模块（执行器模型，tokio 1.x）

- 每核一个 `current_thread` tokio Runtime + `LocalSet`；pipeline 以 `Job` 队列（`Box<dyn FnOnce() -> LocalTask + Send>`）投递到目标核，由常驻 pump 任务在其上 `spawn_local`，保证非 Send future 在目标核上创建（证据：`core/src/runtime/core_map.rs:35-40,163-167,316-349,351-391`）。
- 两阶段启动：背景线程先 park（`Park/Unpark`，std sync_channel(0)，`core_map.rs:47-94`），`execute()` 时统一 unpark（`core/src/runtime/mod.rs:553-560`）。
- 关机：tokio oneshot `Shutdown`，`run_until(shutdown)` 终止各核主循环（`core_map.rs:96-127`、`core_map.rs:293`）。
- 主循环三种退出：`duration` 超时（`runtime/mod.rs:472-480`）、unix 信号 `tokio::select!`（`runtime/mod.rs:483-514`）、`execute()` 流程编排（`runtime/mod.rs:585-601`）。

## ffi crate（`capsule-ffi`）

- `links = "dpdk"`，build.rs 用 bindgen 0.57 生成 `bindings.rs`；`rustdoc` feature 下改用仓库内预生成快照 `bindings_rustdoc.rs`（证据：`ffi/Cargo.toml:8-9,18-21,23-27`、`ffi/src/lib.rs:30-34`）。
- 全文件 `#![cfg(target_os = "linux")]`，关闭全部 clippy（证据：`ffi/src/lib.rs:23-28`）。
- `bindings_rustdoc.rs` 为生成物（约 2.5 万行，unsafe 1759 处），不应手改。

## macros crate（`capsule-macros`）

- proc-macro crate（证据：`macros/Cargo.toml:2-20`）。
- `derive_packet`：Packet derive 实现，含 unsafe（证据：grep `macros/src/derive_packet.rs` unsafe 2 处）。
- `bench`/`test` 宏经 core 再导出（证据：`core/src/lib.rs:145-147`）。

## examples（7 个 bin）

每个 example 一个 `src/main.rs`，统一模式：`load_config()` → `Runtime::build(config)?` → 链式 `add_pipeline_to_port/core(...)` → `execute()`。以 nat64 为例（证据：`examples/nat64/main.rs:164-175`）。入口行号清单见 [overview.md](overview.md)。

## bench crate

- 3 个 benchmark target（证据：`bench/Cargo.toml:2-31`）。
- CI 在 PR 与 push 时分别跑基准对比（critcmp）（证据：`.github/workflows/main.yml:29-103`）。
