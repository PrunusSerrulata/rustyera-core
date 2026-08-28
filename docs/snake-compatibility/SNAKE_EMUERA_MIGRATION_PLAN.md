# 蛇版 Emuera 适配：RustyEra 改造思路

> 来源：从[功能分类文档](SNAKE_EMUERA_BASELINE_MIGRATION_CLASSIFICATION.md)原第 7 章独立抽取，保留已核对的批次 0–7 范围与依赖关系。本文是实施计划，不是完成状态或运行通过证明。

本文维护总体架构、批次范围、前置依赖和验收目标；具体实施方案、实际改动、验收结果和未完成项统一写入[分批次实施与验收记录](SNAKE_EMUERA_IMPLEMENTATION_LOG.md)。调整范围或依赖时须同步关联记录，不能仅修改计划就宣称完成。

- 背景与证据：[蛇版兼容性详查](SNAKE_EMUERA_TW_RUSTYERA_COMPATIBILITY_RESEARCH.md)。历史审计不代表当前实现状态。
- S/D/C/N 编号及第 1–4 类定义：[功能分类与替代契约](SNAKE_EMUERA_BASELINE_MIGRATION_CLASSIFICATION.md)。通用验收原则亦见该文档第 9.2 节。
- 原版 profile：`emuera.em`；蛇版 profile：`emuera.skia.snake`。profile 名称与其语义/格式版本分开记录，不混用两个 oracle。
- 源码/资源路径以多组件工作区根目录（`rustyera-core` 的上一级）为基准；文档链接相对于本文目录。执行时同时遵守工作区及各组件的 `AGENTS.md`。

<a id="architecture"></a>

## 1. 总体架构

建议把兼容工作分成四条相互约束的流水线：

1. **Dialect/semantic policy**：parser、type checker、compiler 和 VM 从同一 `CompatibilityProfile` 取得 arity、算术、错误、RNG、负索引等规则，禁止各层各自判断“是不是蛇版”。
2. **Canonical runtime model**：Float、MAP、input、AudioState、Presentation、SceneLayer、存档等只保存规范化、可序列化状态；同一行为产生稳定 event/delta。
3. **Versioned services**：SQL、字体/布局、指针、图像、音频观察、环境和 extension 都通过 capability 协商及 request/reply；reply 带 revision/epoch，顺序进入 runtime。
4. **Compatibility evidence**：每个功能绑定最小 ERB fixture、参考/蛇版期望和目标游戏调用链；缓存、存档和错误日志能显示采用的 profile 与 service 版本。

建议的 profile identity 至少包含：

```text
language = emuera.em | emuera.skia.snake
arithmetic = reference-wrap-or-current | snake-saturating
rng = <algorithm-id, state-format-version>
layout = unicode-column | snake-pixel-intent
save = <codec-version>
services = <sql, presentation, audio, extension capability versions>
```

实际字段名可以不同，但不能把所有差异压进一个无版本布尔值。

<a id="batches"></a>

## 2. 分批次实施方案

批次编号表示主线交付顺序；允许并行的工作及其汇合门禁见[批次依赖](#dependencies)。每批先完成前置契约与实现，再按仓库规则完成重构审查（如触发）、静态门禁和动态验收。早期批次只验收已具备依赖的最小 fixture/调用断面，不把后续批次的真实标题、地图或存档闭环提前列为通过条件。

<a id="batch-0"></a>

### 批次 0：建立基线、profile 与门禁

- 固定蛇版 Emuera、参考实现、蛇版 TW 和 7 个回归游戏的 revision/资源 hash。
- 引入 `emuera.em`、`emuera.skia.snake` profile，并纳入 project manifest、compiled cache、save/snapshot identity。
- 自动生成“脚本出现 API → analyzer → compiler → VM/service → frontend”覆盖表，区分 unknown、trap、unsupported capability。
- 为第 2 类建立双期望 fixture；先锁定 `PRINTC`、算术、RNG、REF、extra args、`TOINT`、`GETKEY`。

2026-08-27 细化：三个客户端同时接入项目配置显式选择，缺省原版。批次 0 的 snake profile
为实验状态，身份记录当前有效策略并明确告警，不将后续语义提前标为兼容。原版裸存档互操作
保留，snake 自身存档采用独立身份容器及存储目录。用户授权两个 oracle 增加仅 headless 生效
的真实布局观察和设备原语注入，以补足 PRINTC/GETKEY 的动态证据；不改变正常游戏语义，
不把布局度量等同于 GUI/GPU 或跨客户端像素等价。详细契约、版本、分项与门禁见本批实施记录。

批次 0 实测进一步记录固定蛇版基准的 RNG dump/restore 状态丢失：`DumpRanddata`
向临时 `ToArray` 副本写入，随后 `INITRAND` 恢复零。批次 2 须明确决定复刻该可观察行为
还是采用有意修复并升级 policy；算法名称相同不代表状态兼容。原始向量及两引擎逐例结果见
[实测比较汇总](BATCH_0_ORACLE_RESULTS.md)，不得先改参考实现或以新 golden 隐藏差异。

验收：选择 `emuera.skia.snake` profile 不改变 `emuera.em` profile 的既有 fixture；错误中能显示 profile 和缺失 capability。

<a id="batch-1"></a>

### 批次 1：完整摄取与参考能力阻塞项

2026-08-28 已完成本批约定范围，详见[验收汇总](BATCH_1_ACCEPTANCE_SUMMARY.md)；
参考/像素差异、SQL和缺失地图资源仍明确保留，不代表真实蛇版TW可玩。

前置：批次 0 的 profile、identity 与诊断契约。

2026-08-27 用户确认的[详细实施方案](BATCH_1_IMPLEMENTATION_PLAN.md)将本批划为独立的
1A（S01/S02 与资源清单）、1B（动态方法）、1C（列选项/GLOBAL/安全读取）、
1D（服务与集成）。1C 依赖 1A，1D 的最终集成等待 1A–1C；各自唯一审查、独立验收，
功能仍分别提交。用户取消本任务各子批次测试总时限，但未取消单次全量、静态门禁或看门狗。
2026-08-28 用户追加规则：本组 core 审计工具的明确项目加载阶段改为连续4次5秒完整采样
相同才退出；数据摄取、解析/分析/编译和符号准备属于加载，报告处理、执行与输入等待仍为
连续2次相同即退出。真实进展或阶段变化重置计数，不将采样计数视作进度；Browser/Tauri
看门狗未因此修改。用户单独授权的TW全量重跑次数与结果继续逐次记录，不自动重试。
Browser/Tauri 完成真实服务，TUI 本批对像素测量/pointer/canvas 明确缺能力，不新增终端投影。
执行需管理磁盘：20 GiB 以下减少并行，10 GiB 以下暂停新增高写入任务并清理本任务可再生产物。

- 实施 S01/S02：三个客户端都提交 `.als/.erd`，core 建立用户 ERD/ALS。
- 实施 S03/S12：`GETMETH/GETMETHS/EXISTMETH` 与 `DT_COLUMN_OPTIONS` 从注册到 runtime 全链路可用。
- 核验标题必需的 `LOADGLOBAL/SAVEGLOBAL`，以及初始化必需的 `LOADTEXT`、MAP/XML/DT、递归资源清单与安全路径映射；发现参考能力缺口先补齐，供批次 3 的 SQL seed/XML 读取和批次 4 的启动闭环复用。
- 实施 S04 的已有服务接线：先协商并验证现有 HTML 测量、pointer、canvas pixel operation；蛇版新增标签、scene 与投影坐标的一致性留到批次 4。
- 建立蛇版 TW 静态全项目覆盖报告，逐项记录尚未支持的语法/API 及其目标批次；此时不承诺全项目编译通过，但不允许已承诺功能落到 trap。未调用函数中的后置 API 也不能直接忽略，须明确后续实现、显式 unsupported capability 或可证明安全的裁剪策略。

验收：文件数量/hash 无静默遗漏；GLOBAL、数据读取和动态方法最小 fixture 可执行，标题与 `GRAPH_DB_INIT` 的符号/动态目标可解析；所有 unsupported 都有明确层级和 capability 名。不以真实标题或图数据库初始化已运行作为本批结论。

<a id="batch-2"></a>

### 批次 2：确定性 API、输入与兼容差异骨架

前置：批次 1 的完整符号/数据摄取、动态方法和已有 service 接线。

- 实施 S05-S11、S13：EXISTVAR storage 重载、CSV/数组、bit、MAP、STRFORMCHECK、BGC、unchecked、动画查询；S12 已在批次 1 完成，S14 明确留到批次 4 的 scene 模型之后，S05 的 Float bit 留到批次 6。
- 实施 D04、D06-D08、D10-D13、D17 的 profile 分支和输入状态机。先统一 D10 的逻辑计时器再接 S13；先定 D13 的输入顺序再验 D12 的键/鼠标 latch。D04/D06 固定动态调用、实参处理和 call-frame 扩展边界，作为批次 6 新 ABI 的基础。
- 为 C03 提供 capability-based environment；兼容 `GETPLATFORM` 但发 portability diagnostic。
- D11 同步定义 RNG algorithm/state-format identity 与当前已支持状态的保存/恢复契约，不能等到批次 5 才补；外部蛇版存档导入后置。
- 对真实 176 MiB 项目先记录摄取、符号分析和编译缺口/内存基线；用已可编译 fixture 验证 compiled cache 与函数缓存。完整项目缓存收益等批次 4 编译闭环就绪后再验收；优化 N01 的替代路径，不接入 lazy 二进制索引。

验收：独立 fixture 中环境分支、NF 输入、计时器、动态调用和 RNG 状态可重复；7 个非蛇版项目的关键语义 fixture 仍按 `emuera.em` profile；可编译 fixture 有可量化缓存结果。真实标题在 NF 输入前已调用 SQL 并使用扩展 HTML，因此必须等批次 3/4，不能在本批提前宣布标题可交互。

<a id="batch-3"></a>

### 批次 3：安全 SQL（蛇版 TW P0）

前置：批次 1 的动态方法、MAP/XML/DT 与安全资源读取，批次 2 的语义/RNG/调用策略。

- 先定稿 C01 `Sql` service、存储命名空间、epoch/handle、资源限额、错误与 snapshot 规则。
- 同时定稿数据库与存档关系：衍生缓存可重建，用户数据必须进入导出/迁移策略；活动 reader/transaction 阻止 stable snapshot/reload。此契约先于批次 4 的自身存档闭环，不能留到批次 5 再决定。
- 实现蛇版 TW 实际用到的 connect、nonquery、Integer/String scalar、parameter、reader、MAP XML import。
- Tauri、Browser、TUI 使用一致 SQLite 版本/fixture；Resource seed 到 `Data/sql` 采用 copy-on-write 或显式初始化。
- 用具备明确初始变量/资源的调用断面验证 `QOL_DB_INIT`、`GRAPH_DB_INIT`、事务重建、BFS/跨地图边和 reader close；测试项目切换、异常事务、断连和配额。
- 核验同属 `INIT_NG_OR_LOAD` 的 `CREATE_BBAS_DATABASE` 数据前置；对前置报告指出缺失的 `bbas_map_*.xml`，须确认参考容错行为或报告资源阻塞，不能因 SQL 通过而假定整个初始化成功。

验收：上述 SQL 初始化断面可完成，路径不能逃逸命名空间，三个客户端相同 SQL fixture 得到相同 typed rows，数据库保存/重建策略明确。无法支持持久化的客户端在启动前明确拒绝。真实标题与完整新游戏/自身读档初始化在批次 4 汇合；外部蛇版存档读入属于批次 5/6。

<a id="batch-4"></a>

### 批次 4：主玩法 presentation、图像、scene 与自身存档闭环

前置：批次 2 的输入/计时器/RNG 契约，以及批次 3 的 SQL 与数据库保存策略。图形实现可在批次 2 后独立推进，但完整游戏验收必须等批次 3 就绪。

- 实施 D14/D15 和 C04/C05/C08：扩展 canonical HTML AST、SceneLayer、CanvasReplay、line anchor 和资源 service。
- 上述模型就绪后实施 D09 的 sprite/CBG 新重载和 S14 `EXISTSIMAGELAYER`；保留旧 arity，查询必须读取实际 scene，不能提前返回伪造值。
- 补蛇版 TW 活动使用的 `HTML_PRINTC/LC`、font/img/div 属性、CBG、sprite、动画和 pointer 坐标；在本批复验 S04 对新增标签和 viewport/scene revision 的测量、命中与像素采样。
- Web/Tauri 先达到主地图可玩；TUI 明确文本降级和 unsupported 边界，不伪造像素等价。
- 对指定字体/viewport 建结构布局和视觉 fixture；同时保留 `ColumnCell` 的跨客户端参考模式。
- 提前完成 RustyEra 自身 Integer/String、用户 SAVEDATA/CHARADATA/ERD 数组、GLOBAL 与 RNG 的保存/恢复闭环，落实批次 3 的数据库策略；不要求此时导入外部 ERAZIP 或 Float 存档。
- 闭合首个可玩范围的编译门禁：后置数学、音频、渲染等 API 若仍出现在全项目源码中，必须可正确编译并给出明确 capability 处置，或有覆盖动态调用的安全裁剪证据；实际游玩路径不得触发 unsupported/trap。无法满足时按依赖阻塞处理，不能用删游戏代码或虚假成功跳过。编译闭环通过后再验真实项目的重复启动缓存、峰值内存和口上规模。

验收：真实标题、新游戏与自身存档读入的公共初始化、QOL 菜单、地图悬停/点击、状态 UI、depth/scroll 顺序可重复；自身存读档保留变量 shape、GLOBAL、RNG 及数据库策略；重放 scene delta 不依赖前端私有对象。满足这些条件才达到首个可玩里程碑，外部蛇版存档兼容不在此结论内。

<a id="batch-5"></a>

### 批次 5：蛇版存档互操作与音频

前置：批次 4 的自身存档闭环和批次 3 的 SQL 外部状态契约。

- 实施 D18 的非 Float 子集：先读 ERAZIP 和蛇版 Integer/String、自定义数组及已支持的 RNG 状态，复用自身存档闭环；明确单向或双向兼容。未知 RNG/codec 与 Float tags 必须显式拒绝，不得丢弃、转成 Integer 或声称已兼容任意蛇版存档。
- 实施 D16/C07：规范化音频期望状态和 revision-bound 实际查询；缺能力客户端给稳定诊断。
- 按批次 3 的数据库策略验证外部存档导入/迁移，不把外部数据库假装包含在普通 save 内。

验收：已支持类型/codec/RNG 的真实蛇版存档 fixture 保留变量 shape、RNG、GLOBAL 并落实数据库策略，未支持类型有明确拒绝结果；音频查询不支持时不会悄悄返回误导值。Float 存档互操作在批次 6 补验。

<a id="batch-6"></a>

### 批次 6：完整蛇版语言

前置分层：语言主线依赖批次 2 的 profile、动态调用、实参/算术/RNG 策略；SQL Float 接入另需批次 3，Float 自身存档另需批次 4，Float 外部存档导入另需批次 5。接口与文件隔离时可并行推进语言主线，不得把尚未就绪的集成项算作通过。

- 先定 D02/C09 的 Float 类型、bit-exact wire、确定数学/格式化规则，再接 D03 的 variadic、元素 REF、OUT、`ARGLEN`；保持批次 2 的 reference/非 variadic 实参规则。
- 类型与调用 ABI 就绪后完成 D05/D19 的 EVAL/EVALS/EVALF 和 Float 动态 API，同时补 S05 的 Float bit。
- 在批次 4 的自身存档模型上增加 Float save tags；在批次 5 的 importer 上增加外部 Float 存档支持。SQL Float 仅在批次 3 的 service 与本批 Float 契约均就绪后开放。
- 对递归 call frame、alias 生命周期、null OUT、动态表达式错误位置和 cache invalidation 做模型测试。

验收：23/83 API 的语言层 inventory 全部有实现或明确的 service/diagnostic disposition；Float 自身存档 round-trip bit exact，外部 Float 导入与 SQL Float 分别完成集成验证。仅语言主线完成不能宣布本批全部完成。

<a id="batch-7"></a>

### 批次 7：可选 extension 与渲染能力

前置分层：C02 接入依赖批次 0/1 的 profile、capability 与既有 Extension protocol；C06/C10 渲染与 viewport 接入依赖批次 4 的 presentation/scene 契约。只有使用 Float 值/schema 的扩展才额外等待批次 6，不把完整语言作为所有可选能力的统一前置。

- 用 C02 的 Extension protocol 承载明确声明的宿主扩展；不实现任意 CLR 反射。
- 用 C06 的 renderer-neutral hints 表达 strict fallback、quality、text drawing；前端独立选择 renderer。
- C10 的缩放/窗口偏好与 pointer 逆映射复用批次 4 的逻辑坐标和 viewport revision，不新增本机窗口语言对象。
- 改进统一 debug protocol 和各前端 UI，不移植 WinForms debugger、内存窗口或 Skia backend。

验收：同一项目在不具备可选能力的客户端启动前即可得到准确 capability 报告；可选后端不改变 core save/snapshot identity 之外的语义。

<a id="dependencies"></a>

## 3. 批次依赖与首个“可玩”里程碑

```text
主线交付：0 → 1 → 2 → 3 → 4（真实启动/地图/自身存档，首个可玩）→ 5

可并行实现与必须汇合的门禁：
2 → 4 的图形实现；{3, 4 的图形与自身存档实现} → 4 的完整游戏验收
2 → 6 的语言主线（Float/ABI → EVAL/动态 API）
{3, 6 的 Float 契约} → 6 的 SQL Float 集成
{4, 6 的 Float 契约} → 6 的自身 Float 存档集成
{5, 6 的 Float 契约} → 6 的外部 Float 存档集成
1 → 7 的 Extension 接入；4 → 7 的 renderer/viewport 接入
6 的 Float 契约 → 使用 Float schema 的可选扩展（若有）
```

图中花括号表示所有前置均须满足；箭头区分子项实现与整批验收，不表示可在上游共享文件仍变动时验证旧产物。并行工作必须隔离文件、构建产物、游戏副本与会话，依赖契约变化后重验受影响的最小集合；各批仍遵守独立审查、测试预算与静态先于动态的门禁。

“首个可玩”不要求先实现完整 Float、CALLSHARP、外部 ERAZIP 导入或蛇版 lazyloading；但必须完成 SQL、动态方法、文件/资源摄取、GLOBAL、NF 输入、HTML 测量、pointer、地图实际使用的图形路径，以及自身存档和数据库保存/重建闭环。未调用的 ImageLayer 测试函数等后置功能仍须满足批次 4 的编译/capability 处置，不能仅凭“运行时大概不会调用”绕过静态阻塞。
