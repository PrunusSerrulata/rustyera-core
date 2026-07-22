# Runtime Playability Audit Tool

本目录是 runtime、C ABI 和 Textual frontend 的人工/长流程测试工具，不属于产品 runtime，
也不向游戏或前端提供稳定 API。本文件覆盖本目录及其子目录。

## 边界与约束

- 工具可以读取前端提交的游戏目录并驱动 runtime，但不得把文件 I/O 移入
  `era-runtime` 或 VM。
- `fixture-declaration/` 是可提交的最小 UTF-8 项目夹具；修改它时必须说明预期行为，
  不得写入本机绝对路径。
- `reference/eraTW` 是本地真实游戏输入，受仓库根 `.gitignore` 管理，不得复制或提交到
  本目录。
- `artifacts/`、`target/`、日志、存档、snapshot 和 oracle 输出都是生成物，不得提交。
- 测试不得修改 `reference/emuera.em`。若 oracle 本身失效，按仓库根 `AGENTS.md` 的
  reference CLI 规则处理。

## Rust audit harness

从仓库根执行：

```sh
cargo run --manifest-path tools/runtime-playability-audit/Cargo.toml -- COMMAND [PROJECT] [FILTER]
```

支持的命令与输入输出：

- `registry`：无输入；stdout 输出仍被 analyzer 识别但 compiler registry 标记为不支持的
  builtin 名称。
- `parse-file FILE`：输入单个 UTF-8 ERB 文件；stdout 输出 parser 诊断及 UTF-8 byte span。
- `csv [PROJECT]`：输入项目目录，默认 `reference/eraTW`；stdout 输出 CSV 诊断和若干
  eraTW schema 探针。
- `analyzer [PROJECT]`、`compile [PROJECT]`：默认 `reference/eraTW`；stdout 输出各阶段
  耗时、诊断、产物规模与确定性 ID。`compile` 额外生成并验证 bytecode。
- `minimal [PROJECT] [FILTER]`、`minimal-root-paths [PROJECT] [FILTER]`：默认使用
  `fixture-declaration/`；输出 runtime 协议事件和最终 phase。`FILTER` 仅缩小诊断展示。
  内置夹具只用于验证声明加载和首次进入 `WaitingInput`；它没有完整系统流程，自动提交
  后续输入后的 fault 不应被解释为完整游戏生命周期测试失败。
- `benchmark [PROJECT]`：默认 `reference/eraTW`；输出加载/执行耗时、RSS、展示事件数量、
  snapshot 资格和最终 phase。macOS 上 RSS 探针调用 `/bin/ps`，受 sandbox 限制时需要批准。
- `restore-saved [PROJECT] [SAVE]`：默认项目为 `reference/eraTW`，默认存档为
  `artifacts/save99.sav`；stdout 输出导入、恢复和首个稳定等待的结果。

`minimal*` 在收到 runtime 生成的传统存档时会写入
`$ERA_AUDIT_OUTPUT_DIR/save99.sav`；未设置时输出目录为本目录的 `artifacts/`。所有命令以
退出码 0 表示 harness 正常完成；具体 runtime 成败仍应读取结构化 stdout 字段，panic 或
非零退出表示工具、输入或运行流程失败。

## Textual/C ABI scripts

先构建 release C ABI，并通过 frontend 的 uv 环境运行：

```sh
cargo build --release -p era-runtime-capi
UV_CACHE_DIR=/tmp/rustyera-uv-cache \
  uv --project frontends/era-tui run python tools/runtime-playability-audit/tui_day1.py
```

公共输入环境变量：

- `ERA_AUDIT_PROJECT`：游戏目录，默认 `reference/eraTW`。
- `ERA_RUNTIME_CAPI`：动态库路径；默认按当前平台选择 `target/release` 下的
  `.dylib`、`.so` 或 `.dll`。
- `ERA_TUI_DATA_DIR`：前端持久数据目录；测试应指向 `/tmp` 下的独立目录。

脚本约定：

- `tui_day1.py` 驱动真实前端 worker 至日 1 菜单。`ERA_AUDIT_ANSWERS` 是逗号分隔的整数
  输入序列；`ERA_AUDIT_STDIN=1` 允许序列耗尽后从 stdin 读取；
  `ERA_AUDIT_FALLBACK_ANSWER` 提供非交互 fallback。stdout 输出每次 wait、答案、进度和
  `DAY1_MILESTONE`。退出码 0 表示到达里程碑或成功导出 snapshot，1 表示 runtime/前端错误，
  2 表示输入耗尽，3 表示超时，4 表示扫描完等待点仍无合格 snapshot。
- 设置 `ERA_AUDIT_SNAPSHOT_PATH=/tmp/name.snapshot` 可在目标等待点导出；再设置
  `ERA_AUDIT_SNAPSHOT_EVERY_WAIT=1` 会从每个无 deadline 的稳定等待开始寻找首个合格点。
  成功时 stdout 输出 `VM_SNAPSHOT_BYTES`。
- `tui_snapshot_restore.py SNAPSHOT` 输入已导出的 snapshot；stdout 输出传输状态和
  `RESTORE_OK phase=... wait=...`，退出码 0 表示恢复到稳定等待，1 表示错误，2 表示超时。
- `tui_fixture_compare.py` 输入固定的 `tools/emuera-reference-cli/tests/fixture`，stdout
  输出单行 UTF-8 JSON，包含 `termination`、展示文本、wait kind 和 system-input 标志；
  用于与 reference CLI 对同一 fixture 的 NDJSON 字段比较。

Python 脚本不得直接解释游戏数据；它们只能复用正式 frontend 的扫描、C ABI、投影与输入
路径。新增输出字段应保持确定性，并同步更新本文件。
