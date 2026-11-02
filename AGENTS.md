# Capsule

## 背景

Capsule v0.1.5 是一个用 Rust 编写、基于 Intel DPDK 19.11 的网络功能（NFV）开发框架（库 crate，非产品应用）。所有可运行入口在 `examples/` 下（kni/nat64/ping4d/pktdump/signals/skeleton/syn-flood）。workspace 由 core（`capsule`）、ffi（`capsule-ffi`，bindgen 生成的 DPDK 绑定）、macros（proc-macro）、bench 与 7 个 example 组成。

当前状态：tokio 已从 0.2.0-alpha.6 升级到 1.x（执行器模型重写，见 ADR-001）。CI 已完整验证通过（2026-11-02，run 41471384723 @ b8867f9）：check ×3 / test / lint / format / audit 全绿；coverage（tarpaulin ptrace 在 DPDK hugepage 映射下 segfault）与 sanitize（pktdump 在 22.04 runner 上 EAL recvmsg 刷屏）已标记 continue-on-error 非阻塞，待单独排查；lint 的 ~300 处存量 clippy 违规已分三批修复（CI 导出 patch 两批 + 手工修复一批），`make lint` 恢复为准入门槛。

## 常用命令

```bash
make check     # cargo check（core 用 --features full）+ 全 workspace，排除 capsule
make test      # cargo test（core --all-targets --features full）+ 全 workspace
make lint      # clippy -D warnings -D rust-2018-idioms
make fmt       # cargo fmt --all
make compile-failure  # 编译失败断言测试（feature = "compile_failure"）
make bench     # criterion 基准
```

环境约束：core/ffi 只能在 **Linux + DPDK 19.11 + hugepages** 环境编译运行；官方环境是 `getcapsule/sandbox` Docker 镜像（`--privileged`）。Windows 上只能跑 `cargo fmt`、`cargo check -p capsule-macros`、依赖树检查。

## 编码规范

- rustfmt 默认配置；clippy 全量开 `-D warnings`（Makefile:3）。
- crate 级 lint：`missing_docs`/`missing_debug_implementations`/`rust_2018_idioms`/`unreachable_pub` 为 warn（`core/src/lib.rs:19-24`）。
- 每个源文件带 Apache-2.0 许可证头（与现有文件一致）。
- 错误处理：库内用 `thiserror` 定义错误枚举、`anyhow::Result` 向上传播。
- 配置一律走 `RuntimeConfig`（TOML + serde），不引入新的环境变量配置。

## 禁止事项

- **禁止手改 `ffi/src/bindings_rustdoc.rs`**（bindgen 生成物，重新生成于 `ffi/build.rs`）。
- 未获批准不得修改 `core/src/dpdk/`（FFI 安全封装层，unsafe 密集，风险 R1/R2）与 `core/src/runtime/core_map.rs`（执行器模型核心）。
- 不得在 pipeline future 中要求 `Send`——pipeline 按设计是非 Send 的，经 Job 队列在目标核上创建（ADR-001）。
- 不得绕过 `docs/` 知识库直接改业务代码：改前先看 [docs/architecture/](docs/architecture/) 与 [docs/business/](docs/business/)，改后回写文档。

## 文档索引

- 架构体检：[docs/architecture/overview.md](docs/architecture/overview.md)（modules / dependencies / risks 同目录）
- 业务流程：[docs/business/nat64-flow.md](docs/business/nat64-flow.md)（待验证清单 Q1–Q6 在文末）
- 决策记录：[docs/decisions/ADR-001-tokio-1x-migration.md](docs/decisions/ADR-001-tokio-1x-migration.md)

> 所有文档标注生成时间与证据基线（git commit）；代码变更后必须同步更新对应章节与时间戳。
