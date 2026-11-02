# ADR-001: tokio 0.2.0-alpha.6 → 1.x 执行器模型迁移

- 状态：已接受（CI 完整验证待运行，见「后果」）
- 日期：2026-11-02
- 证据：`core/src/runtime/core_map.rs:35-40,135-167,316-391`；`core/src/batch/send.rs:124-137`；`core/Cargo.toml:36`；scratch 工程实测（tokio 1.54.1）

## 背景

原实现绑定 tokio 0.2.0-alpha.6 生态（`tokio-executor`/`tokio-net`/`tokio-timer`/`futures-preview`），其 `CurrentThread<Timer<Reactor>>`、`new_with_park`、`timer::set_default`、`driver::set_default` 在 tokio 1.x 均无公开等价物。核心矛盾：pipeline future 是非 `Send` 的（持有 `Mbuf` 等），必须在其运行的核上创建与执行；而 tokio 1.x 的调度模型对此提出了新的约束。

## 决策

1. **Job 队列模式**：每核一个 `current_thread` Runtime + `LocalSet`（`enable_all()`）。pipeline 安装通道为 `tokio::sync::mpsc::UnboundedSender<Job>`，`Job = Box<dyn FnOnce() -> LocalTask + Send>`——闭包本体 Send 可跨线程投递，它创建的 `LocalTask`（非 Send future）在目标核上由常驻 pump 任务 `spawn_local`（`core_map.rs:35-40,163-167`）。
2. **两阶段启动保留自研实现**：`Park`/`Unpark`（std `sync_channel(0)`）不动（`core_map.rs:47-94`）；关机通道换成 tokio oneshot，各核 `block_on(local.run_until(shutdown))` 终止（`core_map.rs:96-127,293`）。
3. **信号**：监听器在 `block_on` 上下文内创建以注册到 master 核 IO driver，`tokio::select!` 三路 `Signal::recv()`（cancel-safe），经 `on_signal` 闭包过滤（`core/src/runtime/mod.rs:483-514`）。
4. **定时任务**：`tokio::time::interval`（首 tick 立即触发，与旧 `Interval::new_interval` 语义一致）（`runtime/mod.rs:408-418,452-461`）。
5. **`Send` batch 忙轮询**：删除对 `tokio-executor::current_thread` waker 内部 API 的依赖，改 `cx.waker().wake_by_ref()` + `Poll::Pending`（`send.rs:124-137`）。
6. 依赖收敛：tokio 仅 `features = ["macros","rt","signal","sync","time"]`；futures/futures-preview/tokio-executor/tokio-net/tokio-timer 全部移除（`core/Cargo.toml:22-38`）。

## 备选方案（已否决）

- **`Arc<LocalSet>` 跨线程共享**：不可行——`LocalSet` 含 `Cell`/`Rc`/`PhantomData<*const ()>`，是 `!Send`（tokio 1.54.1 实测编译失败）。
- **`Handle::spawn` 的任务内调 `spawn_local`**：不可行——panic「`spawn_local` called from outside of a `task::LocalSet`」（tokio 1.54.1 `local.rs:445` 实测）。pump 任务必须由 LocalSet 自己 spawn。
- **重写 pipeline 为 Send**：侵入面太大，违背框架类型安全设计（非 Send 是 Mbuf 所有权模型的自然结果）。

## 后果

- 正向：脱离 tokio alpha 生态；依赖树干净（tokio 1.54.1/mio 1.2.4）；API 与 tokio 1.x 长期支持路线对齐。
- 负向/待办：本机（Windows）无法编译 DPDK 代码，`wait_for_signal` 的 unix signal 路径仅经 docs.rs 核对，未实机运行；完整验证依赖 CI（`make check && make test`，运行于 getcapsule/sandbox 容器）。新增端到端集成测试 `core/tests/runtime.rs`（net_null0 + duration 超时）作为回归锚点。
- 后续改动约束：向某核安装任何任务必须走 `CoreExecutor.install`（Job 队列），不得绕过直接 `spawn_local`。
