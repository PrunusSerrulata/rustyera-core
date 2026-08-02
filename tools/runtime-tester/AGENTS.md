# Runtime tester instructions

本目录的 Rust 工具用于人工检查 runtime、VM、编译缓存和 C ABI。TUI 专用 Python
审计脚本已经迁移至独立 `rustyera-tui/tools/runtime-tester`。

- 默认真实项目为外层 `../eraTW`；可通过命令参数覆盖。
- `project-extractor-all` 的完整项目根目录仍由第三个参数指定，默认读取本地忽略的
  `reference/`，不得提交游戏内容。
- `ERA_TARGET_DIR` 可覆盖构建产物目录；四仓布局默认使用外层共享 `../target`，独立
  checkout 则使用仓库内 `target`。
- 参考 oracle 位于兄弟仓库 `../emuera.em`。本工具不得修改参考实现；平台脚本接受
  `EMUERA_REFERENCE_ROOT` 覆盖。
- fixture 必须保持最小、确定性并纳入版本控制。`fixture-reference` 是从固定参考 CLI
  fixture 复制的 Rust 自有测试输入，core 测试不得在运行时依赖兄弟仓库。
- 同一套全量流程每任务最多运行一次；修复后只能重跑受影响的子命令或用例。所有长流程/
  端到端运行必须每 5 秒输出完整可观察状态，连续两次内容相同则立即按卡死退出；
  任务的全部测试共享 60 分钟墙钟预算，超时必须停止并报告具体阻塞阶段。

常用命令：

```sh
cargo run --manifest-path tools/runtime-tester/Cargo.toml -- COMMAND [PROJECT] [FILTER]
```

支持 `registry`、`minimal`、`minimal-root-paths`、`csv`、`analyzer`、`compile`、
`benchmark`、`restore-saved`、`parse-file` 和 `project-extractor-all`。修改本目录 Rust
代码时继续遵守仓库根 `AGENTS.md` 的格式、Clippy、测试顺序和测试子 agent 要求。
