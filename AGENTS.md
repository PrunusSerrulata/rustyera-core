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
`docs/design-principles.zh-CN.md` 及对应实现测试。

所有输入源码直接按 UTF-8 处理，不需要支持 GBK、Shift-JIS 等传统编码。

## 仓库结构

- `crates/erabasic-ast`：公共 AST、源码位置和诊断结构。
- `crates/erabasic-data`：可序列化的项目 Schema、静态数据及初始化/存档加载契约。
- `crates/erabasic-csv`：只处理前端提交的路径、UTF-8 内容或 I/O 错误的内存 CSV
  加载器；自身不执行文件 I/O。
- `crates/erabasic-lexer`：EraBasic lexer，包括上下文终止规则和格式化字符串。
- `crates/erabasic-parser`：表达式、逻辑行、ERH、ERB、预处理器和块结构 parser。
- `crates/era-config`：Era 配置项的可序列化模型和规范化处理。
- `crates/erabasic-hir`：稳定、可序列化的类型化高级中间表示。
- `crates/erabasic-html`：EraBasic HTML 子集的规范化与安全文本处理。
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
- `../emuera.em`：独立 Git 仓库中的固定版本原版 C# Emuera 参考实现。
- `../emuera_lazyloading_selfmodified_version`：独立 Git 仓库中的蛇版 emuera。
- `../games/eraTW`：原版 TW（eraTW）脚本集；`../games/eratw-sub-modding`：蛇版TW。
  两者均为本地游戏，不纳入版本控制，不得混用。
- `../emuera.em/emuera-reference-cli`：绕过 UI 调用参考实现的 NDJSON 测试工具；
  平台测试脚本位于 `tools/`。
- `../emuera_lazyloading_selfmodified_version/emuera-reference-cli`：蛇版 emuera 的
  NDJSON 测试工具；平台测试脚本和固定 fixture 位于其 `tests/`。
- `rustyera-tui` 与 `rustyera-web` 是独立前端仓库；本仓库不得重新引入具体应用前端。
- `tools/runtime-tester`：runtime 与 C ABI 的人工/长流程测试工具。

保持各 crate 的职责边界。较大的实现应合理拆分为 module，不要堆积至单个源文件中。
公共类型应尽量由 crate 根模块稳定地重新导出。

## 术语与指代

“蛇版emuera”专指 `../emuera_lazyloading_selfmodified_version`；“蛇版TW”专指
`../games/eratw-sub-modding`；“eraTW”专指原版 TW `../games/eraTW`。单独的“蛇版”
应根据上下文判断是引擎还是游戏；无法可靠推测时必须询问用户，不得默认映射。
“原版参考实现”指 `../emuera.em`，不指任一 TW 游戏目录。

## 蛇版适配计划与实施记录

本仓库的蛇版开发使用 `codex/snake-compatibility` 分支及专用 worktree；其兄弟
`../rustyera-tui`、`../rustyera-web` 必须是同组 worktree，不得误用原 master 工作区。
开工先核对分支、工作树和实际 Cargo patch/target 来源；具体位置与隔离要求见主工作区
规范。此分支仍同时维护 `emuera.em` 与 `emuera.skia.snake` 的明确兼容边界，不能用
蛇版行为全局覆盖原版行为。

core 负责语言、VM、规范化状态和公共协议，前端 I/O、数据库后端、像素/音频观察仍通过
版本化 service 表达。跨组件改动先明确协议/Schema、capability、profile/cache/save
identity 及所需前端适配，完成 core 契约门禁后再做集成验收；分别记录各组件 SHA。
不得以切换分支、协议占位或 oracle smoke 代替已实现和通过验证的证据。

按计划开发或续做蛇版适配时，开工前必须读取以下两个文档：

1. [RustyEra 改造思路](docs/snake-compatibility/SNAKE_EMUERA_MIGRATION_PLAN.md)：
   确认本批目标、架构约束、前置依赖、允许并行的子项及验收门槛。
2. [分批次实施与验收记录](docs/snake-compatibility/SNAKE_EMUERA_IMPLEMENTATION_LOG.md)：
   读取本批及所依赖批次的具体方案、实际改动、已有验证、未完成项和恢复入口；核对当前
   revision、环境与证据是否仍有效，不把“待登记”解释为已完成或尚未实现。

涉及具体语义和兼容判断时，还应读取同目录的
[功能分类与替代契约](docs/snake-compatibility/SNAKE_EMUERA_BASELINE_MIGRATION_CLASSIFICATION.md)
及[历史兼容性详查](docs/snake-compatibility/SNAKE_EMUERA_TW_RUSTYERA_COMPATIBILITY_RESEARCH.md)。
历史调研只提供当时的证据，不能替代当前实现检查和测试结果。

- 实施前先在本批记录中细化方案、涉及组件/文件与契约、依赖证据、验证范围和分项提交。
  前置未就绪时记录阻塞；并行实施不能越过集成验收依赖。原版 profile 使用 `emuera.em`，
  蛇版 profile 使用 `emuera.skia.snake`，不得混用对应引擎、游戏或验证结果。
- 每批/每轮收尾、暂停或交付前，必须把所作改动、审查要求落实情况、验收命令与结果、
  证据、各组件 commit、未完成项、已用测试预算和下一步/恢复入口写回对应批次记录，
  并更新总览。首次全量与修复后的定向复验必须分开，未执行/不适用须说明原因。
- 架构、批次范围、依赖顺序或验收目标发生变化时，必须同步更新改造思路，并在本批记录
  写明原因和影响；实际进度与验证结果以实施记录为准，不在多份文档维护相互矛盾的状态。
- 文档写回不能替代实现与验收；只有获准范围内的要求均满足、且无未处理必需项时才能
  标为完成。上述记录要求不改变现有审查次数、测试预算、测试子智能体及分项提交规则。

## 当前实现状态与范围

当前尚未实现或仅部分实现的范围包括若干数据列表/动态调用/专用输出指令、部分完整
系统流程、客户端物理文本历史与 WinForms/GDI/CBG 相关能力，以及兼容性状态文档中
列出的 Host 调用。不得仅因协议中存在类型、参考 CLI 存在端点或源码中存在占位分支，
就将能力描述为已实现。CSV 加载期间的格式检查不是字节码验证器；C#
reference CLI 能够调用参考实现的 evaluator、VM 和 runtime，也不代表 Rust 侧已实现
参考 runtime 的全部能力。未实现组件的说明只记录范围和状态，不预先承诺具体内部架构。

具体应用前端位于独立的 `rustyera-tui` 与 `rustyera-web` 仓库。它们只通过公共协议、
C ABI 或固定 Git revision 使用本仓库，不得让 runtime 反向依赖具体前端。本文所说的
“应用前端”与 EraBasic“语言前端”（lexer/parser）不是同一概念。

项目边界是 runtime 库及其与外部应用前端之间的公共接口。应用前端负责文件 I/O，
并向 Rust 库提交相对路径、解码后的 UTF-8 内容或对应 I/O 错误。runtime 通过公共
数据/事件接口与 TUI 或其他前端交互；runtime 和 VM 不依赖仓库中的具体 TUI 实现。

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

兄弟仓库 `../emuera.em` 是默认的可移植语言与运行行为兼容性基准；涉及蛇版 emuera
的功能以 `../emuera_lazyloading_selfmodified_version` 为对应 oracle。两者均默认视为
只读第三方代码，测试选择遵循下文与 `$test-rustyera-core`，不得将两者的结果混为一谈。
若其行为依赖特定客户端或平台，应按最高设计准则提炼语义并记录有意差异，而不是把
WinForms/GDI 的实现细节引入 runtime。

- 除非用户明确要求修改 C# 参考实现，否则不允许修改、格式化、重构或自动修复
  两个参考仓库中的任何代码（下述 reference CLI 故障修复例外除外）。
- 不要为了让 Rust 实现更容易而改变参考实现的语义。
- 可以只读搜索和调试参考源码，以确认行为、错误条件和内部执行顺序。
- 若任务明确授权修改参考实现，改动必须最小化，并使用仅在 headless/reference
  模式启用的隔离入口；同时单独报告所有参考目录内的改动。
- 若 `emuera-reference-cli` 无法启动、提前退出或卡住，不得跳过 oracle 测试并把
  任务描述为已验证。应先定位并修复 reference CLI。为恢复 CLI 而确有必要时，
  可以修改本次选中的参考仓库中仅供 reference/headless 路径使用的接入点，
  无需改动正常游戏入口。
- reference CLI 修复绝不得改变正常游戏链路的后端执行语义，也不得为迁就 Rust
  结果而改变 parser、数据加载、验证、执行或状态转移规则。正常模式必须继续调用
  原有逻辑；headless 分支只能隔离 UI、暴露只读状态或施加测试安全限制。
- 所有 oracle 相关修改都必须逐文件、逐目的追加到对应审计文档：原版为
  `../emuera.em/emuera-reference-cli/REFERENCE_CHANGES.md`，蛇版为
  `../emuera_lazyloading_selfmodified_version/emuera-reference-cli/HEADLESS_CHANGES.md`，
  并在最终交付中按参考仓库另设清单报告。
  不得只用“修复了 reference CLI”概括参考目录改动。
- 不要更新参考实现版本或 commit，除非用户明确要求。兼容基准固定为项目文档中
  记录的 commit：原版为 `26a35dc9334bb67590b96f7b8efbefbf199e391e`，蛇版为
  `fc4fb21416768c17256d0e82f997e5f99c9bba91`；wrapper 当前 commit 与语义基准分别记录。

## 实现规范

- Rust 使用当前 workspace 的 edition、格式和 lint 约定。
- 源码中的实现思路、兼容性原因和非显然算法应使用英文注释说明。
- 优先复用成熟库以降低工作量。
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

## 测试要求

用户提出多个开发/修改/修复点时，按工作区根 `AGENTS.md` 先评估规模：小项目合并实现、
重构和测试，共享一个批次；大项目各自独立实现、重构、测试及预算。无论是否合并执行，
每个功能点必须分开提交；跨组件分别提交，共用基础改动单独记录依赖。以下审查次数、
全量次数、静态门禁与 60 分钟预算均以当前批次为范围。循环迭代任务中，此范围进一步限定
为当前批次的当前轮次：各轮独立审查和计时，不能把单轮预算当成整个任务时限。未达用户
目标且未到用户时限时应继续迭代；用户要求暂停或确有阻塞时按根规则处理。有时限须预留
收尾并在截止前完成验证、分项提交或撤回本轮未完成改动，不留半成品。暂停时保留测试流程、
脚本、fixture 和必要临时材料及恢复记录，仅在用户明确表示任务完成或中止时才允许清理。

遵循工作区根 `AGENTS.md` 的并行与依赖调度规则：互不干扰的分析、开发、重构、测试尽可能
并行，依赖步骤流水线推进；共享可变资源须隔离。测试命令列表表达门禁依赖，不要求将无依赖
的检查全部串行；重构完成、最小回归通过、静态门禁通过等前置条件仍必须满足。

每个开发任务都必须包含与改动对应的最小测试，不能仅以“可以编译”作为完成标准。
修复 bug 时先添加稳定复现问题的回归用例；测试数据应尽可能小，并明确体现所验证
的 Emuera 行为。

在运行 C# reference 测试前，必须按本次开发、修改或修复涉及的行为选择 oracle：

- 不涉及蛇版 emuera：沿用原版 `emuera.em` reference CLI。
- 涉及蛇版 emuera：C# reference 测试改为运行蛇版 emuera 的 reference CLI 和对应
  fixture，并执行 Rust/蛇版同输入差分，不能仍只跑原版。
- 涉及蛇版 emuera 且涉及兼容行为，或还需要与原版参考实现对照：在蛇版测试之外，
  还必须运行原版 reference CLI 并分别进行 Rust/原版对照，记录两套基准各自的结果和差异。

上述选择不改变 Rust 静态门禁、回归测试和全量测试的顺序，也不扩大纯文档任务的验证
范围。每套 oracle 的 smoke 只证明自身可用，不能替代 Rust 测试或同输入差分。
蛇版TW与 eraTW 的真实游戏测试也不能替代对应引擎的 reference 测试。

同一套全量测试每个批次只能启动一次；发现问题并修复后，只重跑直接受影响的最小
测试集，不得重跑全量。端到端、长流程及 reference oracle 流程必须每 5 秒报告完整
可观察状态；如果存在 HTML 界面，同时报告全部 HTML 元素。连续两次报告内容相同时必须
立即判定卡死并退出。每个批次的测试流程从本批次首条测试命令开始共享 60 分钟墙钟预算；
该批次/轮次测试超时立即停止其测试进程，并报告命令、用例/阶段、最后状态、已用时间及未验证项。

## 重构审查要求

涉及功能开发或修改、问题修复，或当前批次新增与改动的代码合计超过 100 行时，在最终
测试验收前必须委派独立的子智能体使用 `$refactor-rustyera-code` skill 审查当前批次涉及
的全部代码文件，尤其是新增和修改的部分。该子智能体须报告是否有重构必要；如有，须
提供可执行的重构方案。审查认为有必要重构时，必须先按该方案完成重构，再进行最终测试
验收；不得以时间、预算或“改动已能工作”为由跳过。最终交付必须说明审查结论，以及在
需要时已落实的方案。

每个触发上述条件的批次，重构子智能体必须且只能运行一次。它必须在该批次任何测试启动前
完成对本批次全部代码的完整审查；主智能体必须在本批次首条测试命令前解决其提出的所有重构
要求。该批次测试开始后不得为其新建、恢复、追加轮次或再次启动重构子智能体；也不得以二次
审查替代主智能体对首次审查要求的完整落实。

- lexer 修改应覆盖 token 类型、内容、终止位置、UTF-8 span 和错误路径。
- parser 修改应覆盖 AST 形状、优先级、恢复行为和诊断。
- analyzer、compiler、VM 或 runtime 修改应覆盖状态变化、输出、输入等待、限制条件
  及错误终止；不得用 C# oracle 测试冒充 Rust 实现测试。

所有验证必须使用仓库 skill `$test-rustyera-core`（位于
`.agents/skills/test-rustyera-core/`）。该 skill 规定命令顺序、按改动范围选择测试、
reference oracle 与差分测试，以及结果报告要求；不得绕过门禁或颠倒依赖顺序。

每条测试命令必须委派给运行 **gpt-5.6-terra low** 的子智能体。该子智能体只能执行测试
并返回各命令、退出码和相关输出，不得编辑、格式化或提交代码、fixture、文档及配置；
测试生成文件只能写入临时目录或已忽略目录。实现、格式化、测试编写、失败诊断和修复仍
由主智能体负责，不得用主智能体亲自运行测试替代测试子智能体。相关测试开始后若实现、
测试、fixture、依赖或构建输入发生变化，必须立即告知测试子智能体，要求其按需重建并
重跑所有受影响检查；旧结果一律作废。

## 工作区与 Git 安全

- 工作区可能包含用户尚未提交的修改；不要覆盖、回滚或格式化无关文件。
- 不使用 `git reset --hard`、`git checkout --` 等破坏性命令。
- 修改前检查相关文件和现有测试，修改后检查 `git diff --check`。
- 构建产物、Wine prefix、测试日志和临时 NDJSON 不应加入版本控制。
- 不要提交密钥、用户机器绝对路径或本地游戏数据。
- 完成一次开发任务并执行完上述检查后，生成合适的commit message（标题和简要内容）并创建commit。

## 任务交付

最终说明应简要列出实现行为、测试变更、`$test-rustyera-core` 要求的验证结果、尚未
验证的内容或已知差异、commit message，以及选用的 oracle、选择理由、两套参考仓库
各自的测试结果与全部修改。若未修改参考仓库，应明确写“无”；若有修改，应逐文件说明
目的、headless 隔离条件及正常游戏语义不受影响的依据。
