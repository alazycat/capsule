# 业务流程索引

> 生成时间：2026-11-02
> 证据基线：git commit `d6aca4f`
> 维护方式：代码变更后同步更新，并更新本时间戳

本框架的「业务」单位是**包处理 pipeline**。各流程详细追踪文档在第二步生成，本文先建立框架。

## 框架级主流程（已确认，第一步体检结论）

**应用生命周期**（所有 example 共用，证据：`core/src/runtime/mod.rs:585-601`、`examples/nat64/main.rs:164-175`）：

```
load_config()                  core/src/config.rs:415
  → Runtime::build(config)     core/src/runtime/mod.rs:76-139
      eal_init → mempools → cores(每核 Runtime+LocalSet, park) → KNI → ports
  → add_pipeline_to_port/core(...)   runtime/mod.rs:228-261,340-365
      （Job 入队，目标核 pump 任务 spawn_local）
  → execute()                  runtime/mod.rs:585-601
      add_kni_tx_pipelines → start_ports → unpark_cores
      → wait_for_signal / wait_for_timeout
      → shutdown_cores → stop_ports → Drop(eal_cleanup, runtime/mod.rs:612-629)
```

**包处理数据面**（单个包的旅程）：

```
NIC RX queue → PortQueue::receive (dpdk/port.rs)
  → Poll batch (batch/poll.rs) → 组合子链 filter/map/...（batch/mod.rs:121-221）
  → Send batch（batch/send.rs:41-149，run_once 驱动，Act/Drop/Emit 分发）
  → PortQueue::transmit → NIC TX queue
```

## 待追踪业务清单（第二步逐条展开）

| # | 业务域 | 入口 example | 状态 |
|---|---|---|---|
| 1 | NAT64 网关：v6→v4 / v4→v6 双向转换与端口映射 | `examples/nat64/main.rs` | ✅ 已追踪 → [nat64-flow.md](../business/nat64-flow.md) |
| 2 | KNI：内核态↔用户态包交换（rx/tx pipeline + 内核请求处理） | `examples/kni/main.rs` | 待追踪 |
| 3 | 信号驱动关机：on_signal 过滤 → select! → 优雅退出 | `examples/signals/main.rs` | 待追踪 |
| 4 | 包解析类型系统：Mbuf → Ethernet → IP → L4 的 parse/peek/remove/push | `examples/pktdump/main.rs` | 待追踪（部分内容已随 nat64-flow 覆盖） |

> 第二步将按 [REFERENCE.md] 的三分类规范（[明确]/[推断]/[未知]）为每个业务产出 `docs/business/{domain}-flow.md`，并回填本索引。
