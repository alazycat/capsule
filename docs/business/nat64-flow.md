# NAT64 网关流程

> 生成时间：2026-11-02
> 证据基线：git commit `d6aca4f`
> 维护方式：代码变更后同步更新，并更新本时间戳

追踪目标：`examples/nat64`——IPv6↔IPv4 TCP NAT64 网关。从入口追到网卡 TX 与全局状态写入。

## 入口

- `main()`（证据：`examples/nat64/main.rs:164-175`）：`load_config()` → `Runtime::build(config)?` → `add_pipeline_to_core(1, install_6to4)` + `add_pipeline_to_core(1, install_4to6)` → `execute()`。
- [明确] 两条 pipeline 都装在**核 1**（`main.rs:167-168`），分别处理 eth1(入)→eth2(出) 与 eth2(入)→eth1(出) 双向。

## 调用链（带证据）

### 安装链（控制面）

```
add_pipeline_to_core                       core/src/runtime/mod.rs:340-365
  → install.send(Box::new(move || ...))    runtime/mod.rs:354-360   [Job 入队，Send 闭包]
  → pump 任务在核 1 的 LocalSet 上消费      core/src/runtime/core_map.rs:163-167
  → tokio::task::spawn_local(job())        core_map.rs:165
  → installer(port_qs) 在目标核上创建 future  runtime/mod.rs:355
```

- [明确] 非 Send 的 pipeline future 严格在目标核线程上创建（Job 队列设计，`core_map.rs:37-40`）。
- [明确] `install_6to4` 组装：`Poll::new(qs["eth1"]).filter_map(nat_6to4).send(qs["eth2"])`（`main.rs:152-156`）；`install_4to6` 对称（`main.rs:145-150`）。

### 执行驱动（运行时循环）

- [明确] `Send` 实现 `Future`：每次 `poll` 调 `run()` 处理一批包，然后 `cx.waker().wake_by_ref()` + `Poll::Pending` 让出调度，形成持续的忙轮询（证据：`core/src/batch/send.rs:124-137`）。两个 pipeline 同核时由 LocalSet 交替调度。

### 单包旅程（v6→v4 方向）

1. **收包**：`Send::run` → `Poll::replenish`（`core/src/batch/poll.rs:47-53`）→ `PortQueue::receive`（`core/src/dpdk/port.rs:149-171`）→ FFI `_rte_eth_rx_burst`，单批最多 32 包（`RX_BURST_MAX`，`port.rs:150`）。
2. **逐包处理**：`Send::run` 的 `while let Some(disp) = self.batch.next()`（`send.rs:94`）→ `FilterMap::next`（`core/src/batch/filter_map.rs:73-81`）→ `nat_6to4`（`main.rs:107-140`）：
   - `parse::<Ethernet>()` → `parse::<Ipv6>()`（Packet trait `parse`，`core/src/packets/mod.rs:178`）。
   - 过滤：仅 `next_header() == Tcp`，其余 `Either::Drop`（`main.rs:109,137`）。
   - 先复制要保留的字段 dscp/ecn/ttl/protocol/src/dst（`main.rs:110-114`），因为 `remove()` 会消耗掉 v6 envelope。
   - 地址转换 `map6to4`：取 v6 dst 后两个 u16 段拼成 v4（`main.rs:84-90`）。
   - 包重写：`v4.remove()`（`packets/mod.rs:242`）→ `ethernet.push::<Ipv4>()`（`packets/mod.rs:218`）→ 设置字段 → `parse::<Tcp4>()`。
   - 端口映射：`tcp.set_src_port(assigned_port(src, port))`（`main.rs:120,131`）。
   - `reconcile_all()` 重算校验和（`packets/mod.rs:297`；逐层 `reconcile` 钩子 `packets/mod.rs:288`）。
   - 返回 `Either::Keep(tcp.reset())`（`packets/mod.rs:274`，回到裸 Mbuf）。
3. **分发**（`send.rs:94-101`）：`Act` → `transmit_q`；`Drop` → `drop_q`；`Emit` 计数；`Abort` 计数。
   - [明确] Abort 包不做 bulk free，等出 scope 时归还 mempool（注释：`core/src/batch/mod.rs:67-70`）。
4. **发包**：`PortQueue::transmit`（`port.rs:174-204`）→ FFI `_rte_eth_tx_burst`。
   - [明确] TX 队列满但有进展：drain 已发送的，循环重试（`port.rs:187-191`）。
   - [明确] TX 队列满且零进展：**整批剩余包直接 bulk 丢弃**（`port.rs:196-202`）。
5. **丢包**：`Drop` 包走 `Mbuf::free_bulk`（`send.rs:115-117`）。

## 数据状态变化

- 包本身：`Mbuf`（裸 DPDK 缓冲）→ Ethernet→Ipv6→(remove/push)→Ipv4→Tcp4 的类型化视图切换；头部字段重写 + 校验和重算。
- [明确] 全局 NAT 映射表：`PORT_MAP: CHashMap<(Ipv6Addr,u16),u16>` 与 `ADDR_MAP: CHashMap<u16,(Ipv6Addr,u16)>`，`Lazy` 静态初始化（`main.rs:34-39`）。

## 事务边界

- [明确] 无事务概念。唯一状态写入是 `PORT_MAP.insert_new` / `ADDR_MAP.insert_new`（`main.rs:51-52`），CHashMap 单操作原子。
- [明确] 解析/处理错误 → `Disposition::Abort` → 该包退出 pipeline，不影响其他包（`filter_map.rs:78`）。

## 锁与并发

- [明确] `CHashMap`（分段锁 map）+ `AtomicU16::fetch_add`（端口分配，`main.rs:44,47`）。
- [推断] 单进程内实际竞争面小：两条 pipeline 都在核 1 的同一 LocalSet 上**串行**交替执行（tokio current_thread + LocalSet，`core_map.rs:135-138`），全局表跨核共享的场景在本配置下不出现；但类型设计支持多核（CHashMap）。

## 幂等性

- [明确] `assigned_port` 先查 `PORT_MAP`，命中直接返回已分配端口（`main.rs:42-48`）——同一 (addr, port) 元组重复请求幂等。

## 异常与补偿

- [明确] 包级错误：parse 失败等 → `Abort` → errors 计数 + 包归池（`send.rs:99,108`；`filter_map.rs:78`）。无重试、无补偿——包处理语境下合理。
- [明确] 系统级降级：TX 拥塞 → 丢包（见调用链第 4 步）。

## 外部依赖与降级

- [明确] 唯一外部依赖：DPDK 的 RX/TX burst FFI（`port.rs:154,180`）。无 DB/MQ/第三方。
- 降级路径即上述丢包。

## 待验证清单

| 编号 | 规则描述 | 分类 | 验证方式 | 状态 |
|---|---|---|---|---|
| Q1 | NAT 映射表只增不删（无 `remove`/超时回收，`main.rs:51-52`），长期运行内存无限增长 | 推断 | 长时间运行探针（观察内存/表大小） | open |
| Q2 | 端口分配 `NEXT_PORT: AtomicU16`（`main.rs:44`）从 1025 递增，回绕后与已分配端口冲突 | **明确**（已定性）：`CHashMap::insert_new` 带 `debug_assert!(!contains_key)`——debug 构建 panic，release 构建静默写入重复条目（证据：chashmap-2.2.2 `src/lib.rs:768-783`） | 探针测试 `duplicate_insert_new_panics_in_debug_builds`（examples/nat64/main.rs）已钉住该机制；**缺陷待修**（如改用 `insert`/环形分配+冲突重试） | resolved（定性）/ 缺陷修复 open |
| Q3 | 双向不对称：v4→v6 侧有分片防护（`fragment_offset`/`more_fragments`，`main.rs:112-113`），v6→v4 侧无对应防护 | 推断（有意 vs 疏漏无法确认） | 问原作者 / 对照 RFC 6146 分片要求 | open |
| Q4 | TX 零进展即整批丢弃（`port.rs:196-202`）的业务容忍度 | 未知（行为明确，容许度未知） | 业务方确认 / 拥塞注入测试 | open |
| Q5 | 同核两 pipeline 均 `wake_by_ref` 忙轮询，LocalSet 调度是否公平（不会饿死对方） | 未知 | 探针测试（双向同时灌包，观察两侧吞吐） | open |
| Q6 | `Poll::replenish` 会丢弃上一批未消费完的包（注释自述，`poll.rs:44-46`）——正常路径不可达；异常路径是否存在可达场景 | 未知（行为已由探针钉住） | 探针测试 `poll_replenish_discards_unconsumed_packets`（core/src/batch/mod.rs tests），CI 运行确认实际行为 | probe written |

**第 4 步新增测试**（基线 `d6aca4f` 之后，全部依赖 DPDK，须在 CI sandbox 运行 `make test`）：

| 用例 | 位置 | 钉住的规则 |
|---|---|---|
| `assigned_port_is_idempotent_for_same_tuple` | `examples/nat64/main.rs` tests | 同元组幂等 [明确] |
| `assigned_port_is_unique_for_different_tuples` | `examples/nat64/main.rs` tests | 不同元组端口唯一 [明确] |
| `duplicate_insert_new_panics_in_debug_builds` | `examples/nat64/main.rs` tests | Q2 机制探针（debug panic） |
| `filter_map_error_aborts` | `core/src/batch/mod.rs` tests | FilterMap Err→Abort [明确] |
| `poll_replenish_discards_unconsumed_packets` | `core/src/batch/mod.rs` tests | Q6 replenish 丢包探针 |
