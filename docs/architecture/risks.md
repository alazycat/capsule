# 风险清单

> 生成时间：2026-11-02
> 证据基线：git commit `d6aca4f`
> 维护方式：代码变更后同步更新，并更新本时间戳
> 原则：体检阶段只记录，不顺手修复。

## 高风险

### R1 FFI 裸指针跨线程传递
`SendablePtr(*mut ffi::rte_mempool)` 用 `unsafe impl Send` 强行跨线程（证据：`core/src/runtime/core_map.rs:191-193`，发送点 `core_map.rs:261`）。安全性完全依赖「每个 DPDK worker 线程只用自己 socket 的 mempool」这一隐含约定。
分类：[推断]——约定本身有注释佐证（`core_map.rs:187-190`），但无运行时校验。

### R2 thread_local mempool 裸指针
`MEMPOOL: Cell<*mut rte_mempool>`（证据：`core/src/dpdk/mempool.rs:122-127`）。在新线程上创建 Mbuf 前若未初始化，拿到的是 null 指针；生命周期依赖 `Runtime.ports/mempools` 的 `ManuallyDrop` 手动释放顺序（证据：`core/src/runtime/mod.rs:612-629`）。若释放顺序错乱，thread_local 悬垂指针可能被后续代码触达。
分类：[推断]。

### R3 EAL 清理失败即 panic
`Runtime::drop` 中 `dpdk::eal_cleanup().unwrap()`（证据：`core/src/runtime/mod.rs:628`）。清理路径任何异常都会在 drop 中 panic，可能掩盖原始错误。
分类：[明确]。

## 中风险

### R4 与 tokio 1.x 迁移相关的未回归验证
执行器模型已重写为「每核 current_thread Runtime + LocalSet + Job 队列」（证据：`core/src/runtime/core_map.rs:135-167,316-391`）。本机（Windows）无法编译 DPDK 代码，完整验证依赖 CI 的 `make check && make test`。信号处理 `Signal::recv` 语义经 docs.rs 确认但未在真实 Linux 环境运行。
分类：[明确]（状态事实）/ 影响待 CI 确认。

### R5 KNI 核心选择的随机性
`add_kni_rx_pipeline_to_port` 选「最后一个核」、`add_kni_tx_pipelines` 选「第一个核」（证据：`core/src/runtime/mod.rs:304`、`runtime/mod.rs:524`）。多核端口上 rx/tx pipeline 可能落在不同核，属有意设计但注释自述「randomly」——实际是顺序选择，命名与行为有偏差。
分类：[明确]（行为）/ [推断]（设计意图）。

### R6 Job 队列无背压
pipeline 安装通道为 `tokio::sync::mpsc::UnboundedSender<Job>`（证据：`core/src/runtime/core_map.rs:153,336,371`）。当前用法（启动期少量安装）无实际风险，但 API 上不设限。
分类：[明确]（现状）/ 风险等级低。

### R7 CI 基础设施老化
CI 运行于 `ubuntu-18.04`（2025 年后 GitHub 已停用该 runner 镜像）与 actions v1/v2 版本（actions/checkout@v2、actions-rs/* 已归档）（证据：`.github/workflows/main.yml:14,18,146,168,194` 等多处）。
分类：[明确]（版本事实）/ [推断]（可用性影响随时间恶化）。

## 低风险 / 记录在案

### R8 依赖版本偏旧
toml 0.5、clap 2.33、criterion 0.3、bindgen 0.57 均为老主版本（证据：`core/Cargo.toml:26-27,37`、`ffi/Cargo.toml:19`）。升级属维护项，非缺陷。
分类：[明确]。

### R9 docs.rs 文档链接指向 0.1.4
`core/Cargo.toml:11` 的 documentation URL 写死 0.1.4 而包版本为 0.1.5。
分类：[明确]。

### R10 `Runtime::build` 文档示例中的 API 名有误
`runtime/mod.rs:222,277-279` 的 doc example 写作 `add_add_pipeline_to_port`（多一个 add_），doctest 关闭（`core/Cargo.toml:20`）所以未被发现。
分类：[明确]。

## 体检阶段「未知」汇总（转第二步/测试固化输入）

- [未知] core 私有 `ffi` 模块（`core/src/lib.rs:127`）的完整封装面，及其是否引入隐藏依赖环。
- [未知] `pcap-dump` feature 的运行时行为与性能影响（`core/src/pcap.rs`，unsafe 19 处，未逐行审读）。
- [未知] DPDK 19.11 绑定中 KNI 的内核模块依赖在 CI sandbox 内的实际可用性（KNI 相关测试缺失）。
