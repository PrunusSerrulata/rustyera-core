# AGENTS.md

本文件适用于仓库根目录及其所有子目录。若子目录中存在更具体的
`AGENTS.md`，则以更深层文件的规则为准。

## 项目目标

RustyEra 使用 Rust 复刻 Emuera 的 EraBasic 语言前端、项目数据加载以及未来的
运行相关组件。首要目标是与固定版本的 Emuera 参考实现保持行为兼容，而不是
重新设计语言。除项目明确记录的差异外，边界情况、错误行为、运算符优先级、
格式化字符串和上下文相关语法都应以参考实现为准。

所有输入源码直接按 UTF-8 处理，不需要支持 GBK、Shift-JIS 等传统编码。

## 仓库结构

- `crates/erabasic-ast`：公共 AST、源码位置和诊断结构。
- `crates/erabasic-data`：可序列化的项目 Schema、静态数据及初始化/存档加载契约。
- `crates/erabasic-csv`：只处理前端提交的路径、UTF-8 内容或 I/O 错误的内存 CSV
  加载器；自身不执行文件 I/O。
- `crates/erabasic-lexer`：EraBasic lexer，包括上下文终止规则和格式化字符串。
- `crates/erabasic-parser`：表达式、逻辑行、ERH、ERB、预处理器和块结构 parser。
- `crates/erabasic-repl`：用于人工检查的 Read-Parse-Print Loop。
- `reference/emuera.em`：固定版本的 C# Emuera 参考实现。
- `tools/emuera-reference-cli`：绕过 UI 调用参考实现的 NDJSON 测试工具及平台脚本。

保持各 crate 的职责边界。较大的实现应按语法领域拆分为 module，不要把 lexer
或 parser 重新堆积到单个源文件中。公共类型应尽量由 crate 根模块稳定地重新
导出。

## 当前实现状态与范围

当前已实现的是 AST、lexer、parser、项目数据契约、CSV 加载器和用于人工检查的
REPL。这里的 AST 是语法 AST；`ParserContext` 提供语法解析所需的注册表上下文，
不等同于语义分析。

以下 Rust 组件尚未实现：

- 语义分析器；
- 字节码或其他可执行中间表示；
- 编译器；
- 项目/程序验证器；
- VM；
- runtime。

在这些组件实际落地前，不得在 README、crate 文档、测试结果或交付说明中将其
描述为已实现。CSV 加载期间的格式检查不是未来的验证器；C# reference CLI 能够
调用参考实现的 evaluator、VM 和 runtime，也不代表 Rust 侧已有对应实现。未实现
组件的说明只记录范围和状态，不预先承诺具体内部架构。

本项目不实现具体的应用前端/宿主：GUI、TUI、游戏启动器、文件扫描、渲染、音频
和输入循环都不属于本仓库。本文所说的“应用前端”与已经实现的 EraBasic“语言
前端”（lexer/parser）不是同一概念。

项目边界是 runtime 库及其与外部应用前端之间的公共接口。应用前端负责文件 I/O，
并向 Rust 库提交相对路径、解码后的 UTF-8 内容或对应 I/O 错误；当前只实现了该
边界中的项目加载侧。未来实现 runtime 时，应通过稳定的公共数据/事件接口与前端
交互，不在本仓库加入某一种具体前端实现。

## C# 参考实现边界

`reference/emuera.em` 是行为标准，默认视为只读第三方代码。

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
- 未来新增语义分析或 VM 后，相关修改应覆盖状态变化、输出、输入等待、限制条件
  及错误终止；在当前未实现阶段不得用 C# oracle 测试冒充 Rust 实现测试。
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

## 任务交付

最终说明应简要列出：

1. 实现或修复的行为；
2. 修改和新增的测试；
3. 执行过的 Rust 验证命令；
4. 执行过的 Windows 或 macOS 参考脚本及比较结果；
5. 尚未验证的内容、已知差异或平台限制；
6. 本任务对 `reference/emuera.em` 的全部修改（若无则明确写“无”），包括每个
   文件、headless 隔离条件和正常游戏语义不受影响的依据。
