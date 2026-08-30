# 蛇版兼容批次 3：安全 SQL 分批实施方案

## 总结

批次 3 聚焦蛇版 TW 当前真实使用的安全 SQLite 子集，不实现任意连接字符串或通用数据库能力。采用“仅派生缓存”策略：

- `SQL_CONNECT name` 创建会话内存库。
- `SQL_CONNECT name, "Data Source=plugins/qol_data.db"` 只允许从项目 `Resource` 读取安全相对路径，以 COW 方式生成 `Data/sql` 下的可重建派生数据库。
- 传统存档不嵌入数据库；VM 快照记录不可变数据库修订。活动 reader、SQL transaction 或未完成请求阻止快照和热重载。
- 用户数据库、任意路径、Float、DT/custom XML、XML 导出和 `SQL_ESCAPE` 后移至批次 5/6。

实施依赖顺序：

```text
3.0 参考语义固化
  → 3.1 协议与编译契约
  → 3.2 Core 运行时
  → ┬ 3.3 Web/Tauri Provider
    └ 3.4 TUI Provider
  → 3.5 三端契约收敛
  → 3.6 蛇版 TW 实际流程验收与收尾
```

3.3 与 3.4 可在 3.2 的 core 契约提交后并行；3.5 必须等待两者完成。

## 公共接口与行为契约

### 脚本 API

本批次实现并注册：

- `SQL_CONNECT`、`SQL_DISCONNECT`
- `SQL_EXECUTE_NONQUERY`、`SQL_P_EXECUTE_NONQUERY`
- `SQL_EXECUTE_SCALAR_LONG/STRING`
- `SQL_P_EXECUTE_SCALAR_LONG/STRING`
- `SQL_EXECUTE_READER`、`SQL_P_EXECUTE_READER`
- `SQL_READER_READ`
- `SQL_READER_GET_LONG/STRING`
- `SQL_READER_ISNULL`
- `SQL_READER_CLOSE`
- `SQL_IMPORT_MAP_XML`

参数化调用继续使用现有 variadic host ABI；前两个参数为连接名和 SQL，余下实参按顺序绑定到 `@0…@N`。省略参数映射为 SQL `NULL`，其余参数均按字符串绑定，不进行 SQL 拼接。

其余蛇版 SQL API 进入编译目录，但明确返回“能力未实现”诊断，不得落入未知函数或运行时 trap：

- `SQL_CONNECTION_OPEN`
- Float scalar/reader
- `SQL_ESCAPE`
- DT/custom XML 导入
- MAP/DT/custom XML 导出

### Core 服务协议

- 新增 `HostCapability::Sql`、`ServiceKind::Sql = 11` 和 `rustyera.sql@1`。
- 定义带 serde round-trip 的 `SqlRequestV1`/`SqlResponseV1`：
  - `Open`：内存或已校验 Resource seed。
  - `Execute`：NonQuery、ScalarInteger、ScalarString、Reader。
  - `ReaderRead/Get/IsNull/Close`。
  - `ImportMapRows`：接收 core 已解析的键值行，不接收文件路径。
  - `Disconnect`。
- 值类型仅为 `Null | Integer(i64) | String`；provider handle、连接 handle、reader handle 均绑定 `service_epoch`，脚本不可见原生句柄。
- 响应必须返回 authoritative `transaction_active`、数据库修订、reader 状态及结构化错误。SQLite 文本只作为诊断上下文，不作为测试判定依据。
- SQL 操作统一视为可能写入的外部异步操作：SAVEINFO candidate、VM clone transaction 和调试求值中禁止；请求 pending 时阻止快照/热重载。

### 生命周期与持久化

- 连接名限制为 ASCII `[A-Za-z0-9_.-]`、1–64 字节，大小写不敏感。
- 只接受无选项的 `Data Source=<安全项目相对路径>`；拒绝绝对路径、URI、盘符、父级跳转、附加 SQLite 参数和非 Resource 来源。
- 同名同配置重复连接为成功的幂等操作；同名不同配置返回稳定冲突错误。
- 派生数据库采用不可变 revision blob 加原子 current pointer：
  - 身份包含规范化 seed 路径、seed SHA-256、SQLite 版本和格式版本。
  - 每次成功 autocommit 写操作或 `COMMIT` 发布新修订；`ROLLBACK` 不发布。
  - 发布使用 expected-revision 比较，冲突时拒绝覆盖。
  - VM 快照记录确切修订；修订缺失时明确拒绝恢复，不能静默切换到较新数据库。
  - 批次 3 不自动删除旧修订；达到配额后拒绝新提交，避免破坏已有快照。
- 项目关闭/切换会回滚未提交 transaction、关闭 reader、递增 epoch；旧句柄统一失效。
- `SQL_DISCONNECT` 同样回滚活动 transaction 并关闭其 reader。
- 稳定快照和热重载只允许无 pending 请求、无 reader、无 SQL transaction 的连接。
- v1 固定限制：8 个连接、32 个 reader、256 KiB SQL、64 个参数、8 MiB 参数总量、1 MiB 单元格、64 MiB 单数据库、100,000 行/8 MiB MAP XML、1,000,000 行 reader 和 5 秒 provider 执行预算。三端必须报告并使用相同限制。

### 兼容语义

- scalar 无行或 `NULL`：LONG 返回 `0`，STRING 返回空串。
- reader 列为 `NULL`：GET_LONG 返回 `0`，GET_STRING 返回空串，ISNULL 返回 `1`。
- reader EOF 或无效 reader 的 `READ` 返回 `0`；无效 reader 的 `CLOSE` 幂等成功；无效 getter、列越界或类型错误产生稳定脚本错误。
- transaction 状态由 SQLite provider 返回，不在 core 中用 SQL 文本猜测。
- `SQL_IMPORT_MAP_XML db, table, path` 先通过现有安全 Resource 服务读取文件，再用现有 `quick-xml` 能力解析 `/map/p`；`k` 为键，`v` 使用与蛇版参考一致的 inner XML。core 校验表名和配额后，将规范化行传给 provider，并在一个 transaction 中创建/替换 `k TEXT PRIMARY KEY, v TEXT`。
- intentional differences 仅限安全边界、稳定错误和生命周期；其余返回值以固定蛇版参考 CLI 为准。

## 子批次实施

### 3.0：参考语义与输入基线固化

- 用固定蛇版 Emuera reference CLI 建立最小 SQL oracle：
  - 重复连接、断开、事务提交/回滚。
  - NULL、无行、EOF、无效 reader、类型转换和 SQL 错误。
  - variadic 参数省略与 `@N` 绑定。
  - MAP XML 的 inner XML、重复键及空值行为。
- 固化 `plugins/qol_data.db` 的摘要、SQLite schema、`_meta` 版本及关键表行数；固定两个翻译 MAP XML 的摘要。
- 单独执行 `CREATE_BBAS_DATABASE` 的资源预检并记录参考行为。当前缺少 `plugins/bbas_map_schema.xml` 和 `plugins/bbas_map.xml`：
  - 参考实现若失败，登记为游戏资源阻塞。
  - 参考实现若允许缺失，捕获其确切 fallback 并仅复现该行为。
  - 不创建、补写或修改蛇版 TW 文件。
- 产出机器可复用的短 SQL fixture 和 oracle 记录，供后续所有组件使用。

验收：每项待实现语义都有输入、返回值/错误、数据库副作用和 reference 证据；不存在靠日志全文推断的规则。

### 3.1：Core 协议、能力与编译契约

- 增加 SQL capability、service kind、请求/响应、限制、错误码和兼容身份。
- 注册本批次脚本 API 的签名、variadic 物理 arity 和 `rustyera.sql@1` operation contract。
- 为后移 API 注册确定性的 unsupported capability。
- 将 SQL service version、limits policy 和 profile semantic identity 纳入缓存/握手身份，旧前端在项目加载前即得到版本拒绝。
- 提交边界至少分为：
  - SQL v1 协议与能力身份。
  - SQL 脚本目录和 operation contract。

测试：协议 round-trip/未知版本、artifact 校验、catalog 签名、variadic arity、缓存身份隔离、缺失 capability 的加载前诊断；随后执行本子批次 core 静态门禁与一次完整 workspace suite。

验收：无 SQLite provider 时能完成编译并在启动前给出明确 capability 错误；协议内不存在原始文件路径或原生句柄泄露。

### 3.2：Core SQL 调度、句柄与 MAP XML

- 增加 runtime-owned 的连接描述符、脚本 reader ID、service epoch、transaction 状态和 durable revision。
- 实现 host 参数转换、异步 service continuation、结果回填、结构化 SQL fault 和 stale-handle 检查。
- 将活动 reader/transaction/inflight SQL 接入现有 snapshot、reload、project switch 和 stop 门禁。
- 将现有 XML 解析能力抽出窄接口供 `SQL_IMPORT_MAP_XML` 复用；Resource 读取、大小限制和行规范化在 core 完成。
- fake provider 单元测试覆盖乱序 completion、旧 epoch completion、断开期间 pending、SQL error 后 transaction 状态及持久化失败。
- 提交边界至少分为：
  - SQL 连接、执行与 reader 生命周期。
  - MAP XML 安全导入。
  - SQL 快照、重载和项目切换门禁。

测试：先做 targeted compiler/runtime/protocol 测试，再执行 core fmt、check、clippy、workspace suite；用 3.0 fixture 对蛇版参考做最小同输入差分，对原版参考做非 SQL 回归。

验收：fake provider 下所有脚本语义成立；pending/reader/transaction 不可能被保存为伪稳定状态；项目切换后旧 completion 不影响新项目。

### 3.3：Web 与 Tauri 共用 SQLite Provider

- 固定 `@sqlite.org/sqlite-wasm` `3.53.0-build1`，Browser 与 Tauri 均使用同一 Worker provider；不增加第二套 Tauri 原生 SQL 实现。
- Worker 内执行 SQLite；主线程只负责：
  - Resource seed 读取。
  - `Data/sql` revision blob/current pointer 的原子读写。
  - service request/response 和取消消息转发。
- 扩展共享 frontend bridge 的 typed SQL storage helper；Browser 使用项目私有 Data 后端，Tauri 使用其项目 data root，二者遵守同一命名、revision conflict 和失败语义。
- autocommit/COMMIT 成功前完成 revision 发布；持久化失败不得向 core 报告成功。
- 为 worker 崩溃、project close、request cancellation 和配额错误实现确定性清理。
- 绑定 3.2 已提交的完整 core SHA，并记录实际 WASM/core 产物身份。

测试：worker/provider focused Vitest、路径与 revision 单元测试、完整前端静态门禁；之后用真实 WASM Worker 做 Chromium SQL smoke，并用真实 Tauri WebView 验证同一 fixture、真实 bridge 类型和项目切换。所有浏览器/Tauri 动态测试执行 5 秒完整 DOM/runtime 快照看门狗。

验收：Browser 与 Tauri 使用相同 SQLite 引擎和同一 provider 代码；无主线程同步 SQL；崩溃或写盘失败不产生已确认但未持久化的提交。

### 3.4：TUI SQLite Provider

- 固定 `apsw==3.53.0.0`，与 Web/Tauri 的 SQLite 3.53.0 对齐，并同步依赖锁及 PyInstaller 收集配置。
- 在现有 `RuntimeWorker` 服务路由内实现 SQL provider，不占用 Textual 主线程。
- 使用与 Web 相同的连接规则、limits、typed values、revision blob/current pointer、原子发布和 epoch 清理。
- 不允许 Python DB-API 隐式 transaction 改写协议语义；以 APSW/SQLite 实际 autocommit 状态作为权威结果。
- 绑定 3.2 已提交的完整 core SHA，并记录实际动态库与 SQLite 版本。

测试：focused pytest 覆盖 provider、C ABI service routing、持久化失败和 bundle import；完成 Ruff、完整 pytest 及打包检查；随后通过真实 RuntimeWorker/C ABI 运行 3.0 fixture 和项目切换场景。

验收：TUI 返回的类型、错误、transaction 状态及 revision 与 Web/Tauri 一致；打包产物不依赖开发机隐式 sqlite 安装。

### 3.5：三端 SQL 契约收敛

使用同一固定 fixture 在 TUI、Browser、Tauri 上验证：

- 内存库建表、参数化写入、NULL、scalar 和 reader 全生命周期。
- Resource seed 首开、重复连接、断开重连及已提交修订重用。
- `BEGIN/COMMIT/ROLLBACK`，提交后重启可见、回滚后不可见。
- MAP XML 导入、重复键、Unicode 和 inner XML。
- reader/transaction/pending 对 snapshot 与 reload 的拒绝；inactive connection 快照恢复到精确 revision。
- 项目切换后的 epoch 失效、旧 completion 丢弃和不同项目 Data/sql 隔离。
- 绝对路径、`..`、URI、附加连接参数、非法表名、超限 SQL/参数/XML/数据库及 revision conflict。
- 三端返回同一 typed rows、错误 code/context、SQLite 版本、limits 和数据库摘要。

测试顺序：各组件 focused/static gates 全部通过后，再启动真实 Chromium、Firefox、Safari（macOS）、Tauri 和 TUI 场景；SQL 语义对蛇版 reference 差分，非 SQL 生命周期对原版 reference 回归。每端首个未登记差异即停止。

验收：三端 fixture 的结构化结果和最终数据库摘要完全一致；仅已登记的安全差异允许不同于蛇版参考。

### 3.6：蛇版 TW 实际流程与批次收尾

- 在隔离的蛇版 TW 副本中依次验证：
  1. `QOL_DB_INIT` 的 item、pharmacy、dish、mushi、wood 初始化。
  2. `TR_DB` 和两个翻译 MAP XML 导入。
  3. `GRAPH_DB_INIT` 的 schema 检查与 transaction rebuild。
  4. BFS distance、cross-map edge、node attributes、查询结果和 reader close。
  5. 重启后派生 revision 复用；seed 摘要变化后建立新派生链。
  6. transaction 中断、项目切换、disconnect、配额及异常关闭。
- 对 `CREATE_BBAS_DATABASE` 只执行 3.0 已锁定的资源前提和参考行为；缺失 bbas MAP 文件不阻碍安全 SQL 子集验收，但必须列为批次 4 前的外部资源阻塞。
- 不把完整标题/新游戏/存档初始化宣称为批次 3 成果；这些仍属于批次 4。
- 全部验收结束后，才统一更新 core 的批次 3 实施日志和总览，记录：
  - 实际改动与审查结论。
  - 首次全量与定向复验结果。
  - 三端 core SHA、SQLite 版本、fixture/trace 入口。
  - reference 差异、缺失资源和未完成 API。
- 按组件分别提交；产品行为条目追加到根 `CHANGELOG_PENDING.md`，文档、测试和重构本身不记 changelog。

验收：QOL 与 GRAPH SQL 切片在三端真实客户端完成，结果与蛇版参考或固定数据库断言一致；日志只在整个批次最终结束时写入。

## 测试、审查与提交门禁

- 每个修改代码的子批次先完成该组件要求的唯一一次 `$refactor-rustyera-code` 审查，落实全部要求后才能启动任何测试。
- 每条测试命令交给规定的 test-only agent；每个子批次共享 60 分钟测试预算，每套完整 suite 最多启动一次，失败后只定向复验。
- Core 顺序：fmt → workspace/all-target check → clippy `-D warnings` → focused tests → 一次 workspace suite → oracle smoke/diff。
- Web 顺序：focused Vitest、typecheck/lint/format/build/WASM build → 一次完整 Vitest → 真实浏览器/Tauri；动态测试遵守 5 秒相同快照即失败。
- TUI 顺序：focused pytest、Ruff/静态门禁 → 一次完整 pytest/打包检查 → 真实 RuntimeWorker/C ABI 场景。
- 3.3 与 3.4 使用隔离的 target、依赖、端口、项目副本、`Data/sql` 和证据目录，不复用其他工作区产物。
- 每个功能点独立 commit；core、Web、TUI 和根 changelog 分属各自仓库，前端 pin 只能绑定已提交的完整 core SHA。

## 假设与当前基线

- 开始实施时重新确认专用 worktree 仍在 `codex/snake-compatibility` 且工作区干净；规划时基线为 core `35dd5a99…`、TUI `69ed1249…`、Web `2158972c…`。
- 蛇版 TW 规划时为 `667b9cd0…`，已有用户修改的 `emuera.config` 全程保留，不进入提交。
- `plugins/qol_data.db` 是批次 3 唯一允许的预置数据库来源；当前约 1.3 MiB，实施前复核其已记录摘要和 schema。
- SQLite 统一固定为 3.53.0：Web/Tauri 使用 `@sqlite.org/sqlite-wasm@3.53.0-build1`，TUI 使用 `apsw==3.53.0.0`。
- 批次 3 不修改蛇版 Emuera、蛇版 TW 或游戏资源；reference 与游戏仓库保持只读。
- “批次 3 完成”不要求补齐缺失的 bbas MAP 文件，也不包含完整标题、新游戏、传统存档或外部蛇版存档兼容。
