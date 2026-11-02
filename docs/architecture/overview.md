# 项目全貌

> 生成时间：2026-11-02
> 证据基线：git commit `d6aca4f`（upgrade tokio to 1.x and remove futures-preview）
> 维护方式：代码变更后同步更新，并更新本时间戳

## 项目定位

Capsule v0.1.5：一个用 Rust 编写、基于 Intel DPDK 的网络功能（NFV）开发框架，灵感来自 NetBricks（证据：`core/src/lib.rs:29-30`，`core/Cargo.toml:12-15`）。它是一个**库**（framework），自身不提供可执行产品；所有可运行入口都在 `examples/` 下。

## 技术栈与版本

| 组件 | 版本 | 证据 |
|---|---|---|
| Rust edition | 2018 | `core/Cargo.toml:6` |
| capsule (core) | 0.1.5 | `core/Cargo.toml:3` |
| tokio（异步运行时） | 1.x（features: macros/rt/signal/sync/time） | `core/Cargo.toml:36` |
| DPDK | 19.11，经 bindgen 0.57 生成 FFI 绑定 | `core/src/lib.rs:50-52`、`ffi/Cargo.toml:9,19` |
| 配置 | serde + toml 0.5，clap 2.33 命令行 | `core/Cargo.toml:26,34,37` |
| 可观测 | tracing 0.1 + metrics-runtime 0.13（可选 feature） | `core/Cargo.toml:29-30,38` |
| 错误处理 | thiserror 1.0 + anyhow 1.0 | `core/Cargo.toml:23,35` |

## 启动入口

本仓库有 **7 个二进制入口**（全部在 examples 下，均为 `fn main -> Result<()>`，经 `Runtime::build(...).execute()` 驱动）：

| 入口 | 位置 |
|---|---|
| kni（内核网卡接口） | `examples/kni/main.rs:35` |
| nat64（IPv6→IPv4 NAT 网关） | `examples/nat64/main.rs:164` |
| ping4d（ping 守护进程） | `examples/ping4d/main.rs:60` |
| pktdump（抓包打印） | `examples/pktdump/main.rs:93` |
| signals（信号处理演示） | `examples/signals/main.rs:34` |
| skeleton（骨架模板） | `examples/skeleton/main.rs:25` |
| syn-flood（TCP SYN flood 演示） | `examples/syn-flood/main.rs:79` |

workspace 成员清单证据：`Cargo.toml:2-14`（bench、core、7 个 example、ffi、macros）。

命令行配置入口：`load_config()` 用 clap 解析 `-f <config.toml>`（证据：`core/src/config.rs:415-416`，clap 导入 `core/src/config.rs:51`）。

## 模块划分（crate 级）

- `core`（crate 名 `capsule`）：框架本体。公开模块 `batch/config/metrics/net/packets/testils`，私有模块 `dpdk/ffi/runtime/pcap`（证据：`core/src/lib.rs:124-140`）。
- `ffi`（crate 名 `capsule-ffi`）：DPDK 的 bindgen 绑定，`links = "dpdk"`，build.rs 生成 `bindings.rs`；仅 Linux（证据：`ffi/Cargo.toml:8-9,28-31`、`ffi/src/lib.rs:28-34`）。
- `macros`（crate 名 `capsule-macros`）：proc-macro，`derive_packet` 与 bench/test 宏（证据：`core/Cargo.toml:25`、`core/src/lib.rs:144-147`）。
- `bench`：criterion 基准测试（证据：`bench/Cargo.toml:2-31`）。
- `examples/*`：7 个示例应用。

## 构建与本地运行

- `make check`：先在 core 内 `cargo check --all-targets --features full`，再全 workspace 检查（排除 capsule 自身）（证据：`Makefile:14-16`）。
- `make test`：core 内 `cargo test --all-targets --features full` + 全 workspace（证据：`Makefile:40-42`）。
- `make lint`：clippy `-D warnings` + rust-2018-idioms（证据：`Makefile:3,36-38`）。
- `make compile-failure`：跑「预期编译失败」测试（证据：`Makefile:44-45`；测试本体如 `core/src/packets/mod.rs:413-421`）。
- **本地运行依赖 DPDK 19.11 + Linux + hugepages**；Windows/macOS 无法编译 core/ffi。官方开发环境为 getcapsule/sandbox Docker 镜像（证据：`core/src/lib.rs:45-56`、CI 容器配置 `.github/workflows/main.yml:33-35`）。

## 外部依赖

无数据库 / Redis / MQ（workspace 全量依赖中无此类客户端，证据：`core/Cargo.toml:22-38`）。真实运行时外部依赖：

1. **DPDK 19.11**（C 库，FFI 链接）：EAL 初始化、mempool、端口、KNI（证据：`core/src/runtime/mod.rs:78` 调 `dpdk::eal_init`；`ffi/build.rs` + bindgen）。
2. **Linux 环境**：unix signal 处理（`core/src/runtime/mod.rs:35,492-494`）、`libc`、KNI 内核模块。
3. **hugepages**：DPDK 大页内存（CI 中显式设置 `vm.nr_hugepages=1024`，证据：`.github/workflows/main.yml:224-227`）。
4. **配置文件**：TOML（`core/src/config.rs:1-44` 文档示例）+ 命令行参数。

## 关键风险（索引）

详见 [risks.md](risks.md)。最高优先级三条：

1. FFI 边界 + `unsafe impl Send for SendablePtr` 跨线程裸指针（`core/src/runtime/core_map.rs:191-193`）。
2. 线程局部 mempool 裸指针 `MEMPOOL: Cell<*mut rte_mempool>`（`core/src/dpdk/mempool.rs:122-127`）。
3. `Runtime` 的 `Drop` 手动控制释放顺序，EAL 清理失败即 unwrap panic（`core/src/runtime/mod.rs:612-629`）。
