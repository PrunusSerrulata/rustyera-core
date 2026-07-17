# AGENTS.md

本文件适用于仓库根目录及其所有子目录。若子目录中存在更具体的
`AGENTS.md`，则以更深层文件的规则为准。

## 项目目标

RustyEra 使用 Rust 复刻 Emuera 的 EraBasic 语言和运行环境。发生目标冲突时，
最高设计准则依次为：**跨客户端/跨平台支持 > 架构纯净 > 与固定 Emuera 参考实现
严格行为一致**。该顺序是冲突裁决规则，不代表可以忽略参考实现；游戏规则、输入
判定、状态变化及其他脚本可观察行为，在不违反更高优先级原则时仍应严格兼容。
依赖 WinForms、GDI、设备或平台状态的行为应提炼为 runtime 持有的可移植语义，
由不同前端投影。有意差异、稳定不支持项和遗漏功能必须分别记录，详见
`docs/design-principles.md` 和 `docs/runtime-compatibility-status.zh-CN.md`。

所有输入源码直接按 UTF-8 处理，不需要支持 GBK、Shift-JIS 等传统编码。

## 仓库结构

- `crates/erabasic-ast`：公共 AST、源码位置和诊断结构。
- `crates/erabasic-data`：可序列化的项目 Schema、静态数据及初始化/存档加载契约。
- `crates/erabasic-csv`：只处理前端提交的路径、UTF-8 内容或 I/O 错误的内存 CSV
  加载器；自身不执行文件 I/O。
- `crates/erabasic-lexer`：EraBasic lexer，包括上下文终止规则和格式化字符串。
- `crates/erabasic-parser`：表达式、逻辑行、ERH、ERB、预处理器和块结构 parser。
- `crates/erabasic-hir`：稳定、可序列化的类型化高级中间表示。
- `crates/erabasic-analyzer`：项目级声明、符号、类型、指令参数和控制流语义分析。
- `crates/erabasic-bytecode`：版本化 VM 指令、Host ABI、自包含容器、源码映射和补丁。
- `crates/erabasic-compiler`：确定性、可并行并支持函数级缓存的 HIR 到字节码编译器。
- `crates/erabasic-validator`：HIR 与不可信字节码的结构、类型、控制流和 ABI 验证。
- `crates/erabasic-vm`：确定性解释器、协作式多 fiber 调度、Host/Native 边界、双轨
  状态保存与多代热替换。
- `crates/erabasic-repl`：用于人工检查的 Read-Parse-Print Loop。
- `crates/era-protocol`、`crates/era-runtime-protocol`、`crates/era-debug-protocol`：
  版本化公共消息信封、正常运行协议和独立调试协议。
- `crates/era-runtime-save`：不执行文件 I/O 的传统存档编解码、迁移和恢复契约。
- `crates/era-runtime`：驱动 VM 并持有权威游戏、展示、交互、存档和协议状态的
  caller-pumped runtime。
- `crates/era-runtime-ffi`、`crates/era-runtime-capi`：安全 Rust FFI 契约及唯一包含
  `unsafe` 指针边界的 C ABI 动态库实现。
- `reference/emuera.em`：固定版本的 C# Emuera 参考实现。
- `reference/real-erb`：真实游戏eraTW中使用的完整脚本集，包含csv、erh和erb。由于文本量过大（80余MB），一般不要全量引用。
- `reference/eraTW-minimal`：用于测试游戏功能的最小脚本子集。
- `tools/emuera-reference-cli`：绕过 UI 调用参考实现的 NDJSON 测试工具及平台脚本。

保持各 crate 的职责边界。较大的实现应按语法领域拆分为 module，不要把 lexer
或 parser 重新堆积到单个源文件中。公共类型应尽量由 crate 根模块稳定地重新
导出。

## 当前实现状态与范围

当前已实现的是 AST、lexer、parser、项目数据契约、CSV 加载器、类型化 HIR、
项目级语义分析器、字节码、编译器、验证器、VM、用于人工检查的 REPL、
caller-pumped runtime 和 C ABI 动态库。这里的 AST 是语法 AST；`ParserContext`
提供语法解析所需的注册表上下文，项目级符号解析和类型检查由 analyzer 完成。

当前 runtime 已实现握手、内存项目加载与编译、新游戏启动、受限 VM 驱动、文本与
输入等待、时钟/按键/熵服务、当前格式传统存档、稳定等待 VM snapshot、增量热替换、
部分系统流程、独立调试通道与 VM 调试端口，以及 Map、可变 XML、固定 XPath 子集、
参考形状 DataTable XSD/XML Native、VAREXT、资源图、canvas replay、规范化展示状态、
typed image/audio/update/open-URL 边界、候选 `SAVEINFO` 事务、runtime 自有存档菜单、
独立调试通道和持久化 operation contract。

当前尚未实现或仅部分实现的范围包括若干数据列表/动态调用/专用输出指令、部分完整
系统流程、客户端物理文本历史与 WinForms/GDI/CBG 相关能力，以及兼容性状态文档中
列出的 Host 调用。不得仅因协议中存在类型、参考 CLI 存在端点或源码中存在占位分支，
就将能力描述为已实现。CSV 加载期间的格式检查不是字节码验证器；C#
reference CLI 能够调用参考实现的 evaluator、VM 和 runtime，也不代表 Rust 侧已实现
参考 runtime 的全部能力。未实现组件的说明只记录范围和状态，不预先承诺具体内部架构。

本项目不实现具体的应用前端/宿主：GUI、TUI、游戏启动器、文件扫描、渲染、音频
和输入循环都不属于本仓库。本文所说的“应用前端”与已经实现的 EraBasic“语言
前端”（lexer/parser）不是同一概念。

项目边界是 runtime 库及其与外部应用前端之间的公共接口。应用前端负责文件 I/O，
并向 Rust 库提交相对路径、解码后的 UTF-8 内容或对应 I/O 错误。runtime 通过公共
数据/事件接口与前端交互，不在本仓库加入某一种具体前端实现。

## 接口兼容性策略

- Runtime、VM 及其以下层级的组件将作为单一实体发布，因此这些组件之间的内部
  Rust 接口、消息和数据契约在更新时默认无需保持向下兼容。尤其是 runtime 与 VM
  的接口可以随二者同步演进，不需要为尚未独立发布的旧内部接口保留适配层。
- 上述规则只适用于同一发布实体内部的接口演进，不放宽可移植游戏规则和脚本可观察
  行为的兼容要求；因更高优先级设计准则产生的差异仍须明确记录。该规则也不取消
  字节码、存档或 VM snapshot 等持久化格式应有的版本标识、兼容性检查和不兼容时
  拒绝加载的要求。
- 当前开发版本的 runtime—应用前端公共接口同样默认不保证向下兼容。进行破坏性
  更新时，应同步更新协议版本、Schema/C 头文件、接口文档和测试，不能留下互相
  矛盾的新旧定义。
- 如果用户在具体任务中明确要求 runtime—前端接口或内部接口保持向下兼容，则该
  要求优先于上述默认策略。此时必须保留旧调用方可用的行为，并通过版本协商、
  additive 变更、兼容适配层或迁移逻辑实现，同时添加覆盖旧版与新版调用方的测试；
  不得以“当前处于开发阶段”为由进行静默破坏。

## C# 参考实现边界

`reference/emuera.em` 是可移植语言与运行行为的兼容性标准，默认视为只读第三方代码。
若其行为依赖特定客户端或平台，应按最高设计准则提炼语义并记录有意差异，而不是把
WinForms/GDI 的实现细节引入 runtime。

- 除非用户明确要求修改 C# 参考实现，否则不允许修改、格式化、重构或自动修复
  `reference/emuera.em` 中的任何代码。
- 不要为了让 Rust 实现更容易而改变参考实现的语义。
- 可以只读搜索和调试参考源码，以确认行为、错误条件和内部执行顺序。
- 若任务明确授权修改参考实现，改动必须最小化，并使用仅在 headless/reference
  模式启用的隔离入口；同时单独报告所有参考目录内的改动。
- 若 `emuera-reference-cli` 无法启动、提前退出或卡住，不得跳过 oracle 测试并把
  任务描述为已验证。应先定位并修复 reference CLI。为恢复 CLI 而确有必要时，
  可以修改 `reference/emuera.em` 中仅供 reference/headless 路径使用的接入点，
  无需改动正常游戏入口。
- reference CLI 修复绝不得改变正常游戏链路的后端执行语义，也不得为迁就 Rust
  结果而改变 parser、数据加载、验证、执行或状态转移规则。正常模式必须继续调用
  原有逻辑；headless 分支只能隔离 UI、暴露只读状态或施加测试安全限制。
- 所有 `reference/emuera.em` 内的 oracle 相关修改都必须逐文件、逐目的追加到
  `tools/emuera-reference-cli/REFERENCE_CHANGES.md`，并在最终交付中另设清单报告。
  不得只用“修复了 reference CLI”概括参考目录改动。
- 不要更新参考实现版本或 commit，除非用户明确要求。兼容基准固定为项目文档中
  记录的 commit。

## 实现规范

- Rust 使用当前 workspace 的 edition、格式和 lint 约定。
- 源码中的实现思路、兼容性原因和非显然算法应使用英文注释说明。
- 优先复用成熟库，但上下文相关 lexer/parser 行为无法由库准确表达时，应选择
  清晰、可测试的手写实现。
- 当 runtime 难以精确复刻参考实现中某条指令的行为时，应先检查该指令在真实游戏
  脚本中的实际使用方式，并从脚本开发者的角度推断其意图。对于与具体界面和排版
  强相关的行为，应优先围绕该意图设计 Runtime 持有的规范化展示语义和前端投影，
  以支持不同前端和跨平台运行，无需强行进行像素级复刻。此原则不放宽游戏规则、
  输入判定或脚本可观察副作用的兼容要求；有意差异仍须记录并测试。
- 源码位置统一使用 UTF-8 byte offset；涉及 C# 输出时要明确区分 UTF-8 byte
  offset 与 UTF-16 code-unit offset。
- 诊断必须保留足以定位问题的 span 和稳定类别。不要仅依赖本地化错误文本。
- 避免无关重构、批量格式化和跨 crate 的非必要 API 变化。
- 保持确定性：测试用 JSON、诊断顺序、集合遍历和 AST 输出不得依赖随机哈希顺序。

提交修改前运行：

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## 测试要求

每一个开发任务都必须包含与改动对应的测试用例。任务不能仅以“可以编译”作为
完成标准。

- lexer 修改应覆盖 token 类型、内容、终止位置、UTF-8 span 和错误路径。
- parser 修改应覆盖 AST 形状、优先级、恢复行为和诊断。
- analyzer、compiler、VM 或 runtime 修改应覆盖状态变化、输出、输入等待、限制条件
  及错误终止；不得用 C# oracle 测试冒充 Rust 实现测试。
- 修复 bug 时先添加能够稳定复现问题的回归用例。
- 测试数据应尽可能小，并明确体现所验证的 Emuera 行为。

测试流程依次为：为改动添加最小 Rust 单元/集成测试；运行 Rust 全量格式、测试和
lint；运行对应平台的参考实现脚本确认 oracle 可用；最后使用同一输入比较 Rust 与
C# 输出。每次开发任务都必须完成与本次改动相关的步骤，不能用平台 smoke test
代替真正的差分比较。

### Windows

在 Windows 上运行：

```powershell
tools/emuera-reference-cli/tests/protocol-smoke.ps1
```

### macOS

在 macOS 上运行：

```sh
tools/emuera-reference-cli/test-macos-wine.sh
```

macOS 脚本使用项目内固定的 `.wine-prefix/emuera-reference-cli`，并将临时请求、
日志和 NDJSON 输出写入 `.wine-tmp/emuera-reference-cli`。这些本地工具和产物已被
`.gitignore` 忽略。

平台脚本成功只表示参考 oracle 能正常工作；还必须把参考 NDJSON 与本次修改对应
的 Rust 结果进行比较。新增语法或执行路径时，应同时扩充相关 fixture、请求集合
和 Rust 测试，使双方接收相同输入。比较时可以忽略请求 ID、绝对路径等明确的环境
元数据，但 token、AST/语义结构、诊断、输出、变量值和终止原因必须一致。任何有意
差异都需要在测试和交付说明中明确记录。

如果平台脚本超时、无输出、进程提前退出或返回协议错误，应将其视为
`emuera-reference-cli` 缺陷并优先修复。修复后必须重新运行导致故障的请求以及完整
平台 smoke test；若触碰参考目录，还要验证普通 Emuera 项目仍可编译，并在
`REFERENCE_CHANGES.md` 记录隔离方式和正常游戏链路为何不受影响。

如果当前机器无法运行目标平台脚本，不得把它描述为已验证；应说明阻塞原因，并给
出需要在对应平台执行的准确命令。

## 工作区与 Git 安全

- 工作区可能包含用户尚未提交的修改；不要覆盖、回滚或格式化无关文件。
- 不使用 `git reset --hard`、`git checkout --` 等破坏性命令。
- 修改前检查相关文件和现有测试，修改后检查 `git diff --check`。
- 构建产物、Wine prefix、测试日志和临时 NDJSON 不应加入版本控制。
- 不要提交密钥、用户机器绝对路径或本地游戏数据。
- 完成一次开发任务并执行完上述检查后，生成合适的commit message（标题和简要内容）并创建commit。

## 任务交付

最终说明应简要列出：

1. 实现或修复的行为；
2. 修改和新增的测试；
3. 执行过的 Rust 验证命令；
4. 执行过的 Windows 或 macOS 参考脚本及比较结果；
5. 尚未验证的内容、已知差异或平台限制；
6. 本任务对 `reference/emuera.em` 的全部修改（若无则明确写“无”），包括每个
   文件、headless 隔离条件和正常游戏语义不受影响的依据。
7. 本次任务的commit message。
