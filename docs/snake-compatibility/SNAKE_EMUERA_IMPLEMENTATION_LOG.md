# 蛇版 Emuera 适配：分批次实施与验收记录

> 文档状态：批次 0 已完成；批次 1 实施中，其他批次仍待登记。批次 0 完成范围是 profile、隔离、基线与门禁，不是完整蛇版语义或蛇版 TW 可玩性。

## 文档职责与填写规则

- [改造思路](SNAKE_EMUERA_MIGRATION_PLAN.md)维护总体架构、批次范围、依赖顺序和验收目标；本文维护每批的具体实施方案、实际改动、证据、未完成项与恢复入口。
- [功能分类](SNAKE_EMUERA_BASELINE_MIGRATION_CLASSIFICATION.md)提供 S/D/C/N 项目编号、兼容语义和替代契约；[兼容性详查](SNAKE_EMUERA_TW_RUSTYERA_COMPATIBILITY_RESEARCH.md)提供历史源码与游戏证据，不能直接作为当前实现状态。
- 开工或续做前，先读改造思路和本批及其上游批次记录，核对代码 revision、环境、接口与已有验证；先填具体方案和依赖证据，再实施。批次编号沿用计划，不得为重置预算或审查次数临时拆分。
- 原版 profile 为 `emuera.em`，蛇版 profile 为 `emuera.skia.snake`；引擎、游戏、profile/codec/service 版本、seed 与资源 hash 分别记录，不混用原版 eraTW 和蛇版 TW。
- 状态可填“未开始 / 进行中 / 阻塞 / 暂停 / 已完成”；初始“待登记”不作事实判断。结论必须区分通过、失败、未执行和不适用，后两者须写原因；计划、命令已提交或 oracle smoke 不能冒充验收通过。
- 若范围、依赖、接口决策或验收目标变化，先在本文记录理由和影响，并同步改造思路；具体结果、提交、未完成项和下一步必须写回本批记录，并更新总览。不要把未验证设想改成历史调研事实。
- 审查、测试顺序、子智能体职责及单次全量限制遵守[工作区规则](../../../AGENTS.md)、[core 规则](../../AGENTS.md)及相关组件规则；用户已明确取消本任务所有批次的测试总时限，单命令限额、卡死检测和磁盘管理仍生效。本模板不增加测试范围。首次全量与修复后定向复验分开记录，不把未重跑全量描述为修复后通过。
- 循环任务在所属批次下按实际轮次追加记录，保留每轮起止时间、分析、改动与结论；同轮续做沿用已用预算。暂停时记录复现命令、工作目录、环境/seed、指标、材料路径、进程释放状态与恢复入口，保留材料直至用户允许清理。
- 路径优先使用仓库/工作区相对路径；证据须足以定位。临时材料若只在某环境可用，注明位置约定与可用性，不提交用户机器绝对路径、游戏副本、原始存档、密钥或临时测试输出。

## 开发工作区

2026-08-27 建立独立的蛇版开发工作区 `.worktrees/snake-compatibility/`（相对于主工作区根）。
各组件在自己的仓库中使用 `codex/snake-compatibility`，下表记录创建起点，不代表各批已实施
或三个组件已通过集成验证。具体位置、共享输入与构建隔离见[本组规范](../../../AGENTS.md)
及各组件 `AGENTS.md`；所有本计划的结果只写回本组 core，不更新 master 副本。

| 组件 | 本组内路径 | 创建起点 SHA |
|---|---|---|
| core | `rustyera-core/` | `372074352f59530956262f1f25ab94331cb4c511` |
| tui | `rustyera-tui/` | `a61e6992c6c5c33f49a1ad155e4da73139f26a12` |
| web | `rustyera-web/` | `a2ae84214151434d1428f866d3f8bcc647306166` |

创建时未带入原工作区未提交内容。两前端的发布绑定仍为
`7ba54e80962b1758e928cc7758429aa1dceebd80`，并未因 worktree 创建而升级；本地联调的实际
core SHA、库/bundle 路径及后续发布绑定变更，须在对应实施批次另行记录与验证。
本次仅建立分支、工作区和开发规范，不将批次 0 或其他适配批次标记为完成。

## 批次总览

| 批次 | 范围 | 状态 | 最近更新 / 负责人 | 当前结论 / 下一步 |
|---|---|---|---|---|
| [0](#batch-0) | 建立基线、profile 与门禁 | 已完成 | 2026-08-27 / Codex | 三端、双 oracle、8 游戏基线与 9 份覆盖报告已验证；后续语义差异逐例保留，磁盘已治理 |
| [1](#batch-1) | 完整摄取与参考能力阻塞项 | 实施中 | 2026-08-27 / Codex | 详细方案已确认；开始 1A 摄取/ERD/ALS，尚未审查或测试 |
| [2](#batch-2) | 确定性 API、输入与兼容差异骨架 | 待登记 | 待填写 | 待填写 |
| [3](#batch-3) | 安全 SQL（蛇版 TW P0） | 待登记 | 待填写 | 待填写 |
| [4](#batch-4) | 主玩法 presentation、图像、scene 与自身存档闭环 | 待登记 | 待填写 | 待填写 |
| [5](#batch-5) | 蛇版存档互操作与音频 | 待登记 | 待填写 | 待填写 |
| [6](#batch-6) | 完整蛇版语言 | 待登记 | 待填写 | 待填写 |
| [7](#batch-7) | 可选 extension 与渲染能力 | 待登记 | 待填写 | 待填写 |

<a id="batch-0"></a>

## 批次 0：建立基线、profile 与门禁

计划入口：[改造思路 / 批次 0](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-0)。状态：已完成；负责人 / 最近更新：Codex / 2026-08-28。

### 具体实施方案

#### 范围、已确认决策与当前基线

本批实现 D01 的 profile、可追溯基线、缓存/存档身份、覆盖报告和双 oracle 证据，并接通 TUI、Browser/WASM、Tauri。
D03/D06/D07/D11/D12 及 PRINTC 的双期望用于锁定差异，不提前实施批次 2/4/6 的语言、输入和布局语义。
本批是一个跨组件的基础设施批次，以下分项共享唯一重构审查、每套全量一次。原定 60 分钟测试预算已被用户明确取消，不通过分项重置其他限制。

用户已确认：

- snake 为实验 profile，允许当前行为但启动、诊断及身份显示实际策略与未兼容项，不宣称饱和算术、蛇版 RNG 或像素布局已实现。
- 项目在 reraconfig.toml 声明 profile，缺省原版；三客户端同步接入，不新增选择界面。
- 保留 emuera.em 裸存档互操作；snake 自身存档使用身份 envelope，外部蛇版存档导入仍在批次 5。
- 授权两个参考仓库添加仅 headless 生效的布局观察和按键注入，不改变正常游戏语义，逐文件审计并分别提交。
- 允许备份 Web 既存 Cargo.lock 差异后重建正确发布锁文件，不把本地 path patch 状态提交为发布绑定。

只读检查起点：core `aa5cd34e9f11346ee6a66e3ab9c4978c92137103`，TUI `da7344e`，Web `14d6c17`。
core/TUI 干净；Web Cargo.lock 原有 18 个 Git source 删除。前端当前发布绑定均为
`7ba54e80962b1758e928cc7758429aa1dceebd80`。没有可据以宣布本批通过的测试结果。
原版语义基准 `26a35dc9334bb67590b96f7b8efbefbf199e391e`，wrapper 起点
`af9886061ba420d530581e7975c4db735c391d03`；蛇版语义基准
`fc4fb21416768c17256d0e82f997e5f99c9bba91`，wrapper 起点
`4a46d7b52280733e8ecb8eeb630a87facdc03a23`。语义基准不移动，wrapper 新提交另记。

#### 0A：基线与资源身份

扩展 core 的 runtime-tester，独立模块提供 `baseline` 子命令及确定性 JSON 清单。
覆盖蛇版 TW 和 eraAkumaMaid、eraMaouEx、原版 eraTW、erafl、erarorona、eratohoK、era魔界牧場1.050_tc8。
逐项记录 Git SHA（可空）、脏状态、排序后的相对路径/长度/BLAKE3，并分别汇总源码、配置、资源。
包含 .als/.erd、数据库种子及后续批次资源，不以当前客户端摄取范围裁剪基线；排除 Git、缓存、日志、
运行存档等产物，数据库种子与运行数据库分开。蛇版 TW 当前配置已有修改，记录实际内容；fixture 覆盖配置
只写专属副本，并另记 hash。提交清单和汇总，不提交游戏、用户存档或本机绝对路径。

#### 0B：统一 profile、配置与公共加载契约

- 新增低层共享 crate `erabasic-compat`，集中定义 CompatibilityProfileId、CompatibilityIdentity 和实际策略描述。
  身份包含 profile/语义版本、实际算术、RNG/状态格式、布局、存档及相关 service 契约；目标与当前实现分开。
  parser/analyzer/HIR/compiler/artifact/VM/runtime 传递同一身份，不分别判断字符串。
- reraconfig schema v4 新增 `[compatibility] profile = "emuera.skia.snake"`；缺省和旧 schema 为 `emuera.em`，
  未知名称/非法值/不支持版本硬失败。profile 不属于客户端偏好；配置迁移、from_values、journal、导出必须保留。
- 公共 RuntimeMessage 新增 ResolveProjectCompatibility 请求/响应：hello 后输入根配置 Option<SubmittedFile>，
  core 唯一解析器返回 identity、规范化配置 digest 和诊断；有错不返回有效 identity，不建 VM、不改活动项目。
  correlation ID 处理取消和过期响应；三端 quick scan 真正读取这一份配置，不各自解析 TOML，也不物化整个项目。
- ProjectManifest、轻量 ProjectIdentity、加载/观察报告携带 identity；完整加载再次核对配置，cache-only 核对缓存身份
  和配置摘要。身份改变要求完整重开；热重载及配置提交在迁移 VM/切换存储前原子拒绝，保留当前会话。

#### 0C：缓存、字节码、snapshot 与存档隔离

- source digest 继续表示文件内容；profile/policy 进入整项目 key、函数 shared dependencies、artifact manifest/execution hash。
  跨 profile cache 作为 miss 重编译，不能作为函数增量种子；同 profile 正常复用。不把客户端名称/可选能力全集写入语言 key。
- 配置解析/请求身份冲突是加载错误；旧缓存/缓存身份差异是可重建 miss，不能被相同 fallback 吞掉。
- runtime、VM、artifact 三层 snapshot 显式核对 identity；损坏或不匹配在写状态前拒绝。旧 snapshot 仅可检查为 legacy，
  不默认赋予原版身份继续执行。
- emuera.em 保留 Text1808/Binary1808/Gzip 裸读写；snake 新 magic/version/checksum identity envelope，inner 复用现有 codec。
  normal/auto/GLOBAL/变量/角色保存、probe/CHECKDATA、导入导出统一校验；snake 拒绝裸存档及未知 profile/codec/RNG。
  不改 Emuera VERSION=1808，不占用 GAMEBASE 描述或脚本可观察扩展槽存 profile。

| 契约 | 计划版本 |
|---|---|
| reraconfig | 3 → 4 |
| runtime protocol | 35.0 → 36.0 |
| HIR | 12 → 13 |
| bytecode container / compiler ABI | 15.0 → 16.0 / 37 → 38 |
| compiled/full-project container | 8 → 9 |
| VM / runtime snapshot | 10 → 11 / 18 → 19 |
| snake save envelope | v1 |
| Browser 私有 manifest | RERMAN01 → RERMAN02 |

不机械升级无变化的 C ABI、ISA、native/host ABI。旧 full project v6–8 可提取源码按 legacy 原版重建，
不能复用旧 executable artifact 或函数 keys。

#### 0D：三个客户端

统一顺序：hello → 根配置读取 → core resolve → 存储绑定 → cache/source load。
TUI 覆盖 ProjectBundle、手写完整 CBOR、项目文件解码和启动状态机；Browser 覆盖私有 manifest 编码、Worker 分块上传头、
cache 轻量 manifest、project transfer 校验；Tauri 覆盖 ProjectHost quick/full scan、materialize、完整 CBOR 和 native transport；
bridge 拆分/重建 manifest 全程保留身份。Web 现有 any 消息类型不能替代边界验证。

原版保留现有路径；snake 使用项目 data root 下 `.rustyera/profiles/emuera.skia.snake/`，隔离 sav/global/data/project/logs/cache、
snapshot 和 sidecar，资源仍从原项目只读获取，不回退原版存档/GLOBAL/cache。storage host 绑定已校验的 session identity，
取消、过期 resolve、失败 reload 不改变当前身份。现有日志/诊断显示 profile、版本、实验状态和缺失能力，不加复杂界面。

冷打开另一个项目沿用客户端原有先结束旧会话的生命周期；本批不增加双会话预加载。
因此冷打开的配置错误会结束于新项目加载失败，不能宣称旧 VM 仍可运行；上述原子保留约束适用于热重载和配置提交。

#### 0E：覆盖报告与诊断

runtime-tester 新增 coverage 独立模块和 JSON/Markdown 输出。每个出现点记录项目 identity、API/语法形态/arity、UTF-8 span、
预处理活动性、analyzer、compiler lowering、VM handler/service、TUI/Browser/Tauri 各自证据、fixture、S/D/C/N 和目标批次。
扫描未调用函数，保留动态目标表达式与未知目标；复用 lexer/parser，不以正则当语义覆盖。
区分 unknown、compiler_trap、unsupported_capability、blocked、unverified；注册、实现、动态验证独立分列。
DT_COLUMN_OPTIONS 的 Native 注册但无 dispatch 作为误报回归，不顺带实现本批以外 API。
全项目分析失败仍输出局部证据和阻塞，不要求本批蛇版 TW 全项目编译通过。
ProtocolDiagnostic、RuntimeFault、CommandRejected 增加 profile/阶段/API/所需 capability operation/version，保留原 code/span。

#### 0F：双 oracle 与七组 fixture

保持 schemaVersion=2 和旧 output，新增版本化 presentationSnapshot:1、headlessInputTrace:1。
布局观察通过 observePresentation/只读 observe 投影现有 DisplayLine/Button/node 的位置、宽度、对齐、space padding；
observer 不刷新、重测量、重排版或隐式 flush，fixture 显式 PRINTL。固定字体文件/hash、实际 family/fallback、字号、
布局参数、Wine/测量库版本，字体未命中即环境失败。本批只证明真实像素度量/布局，不证明 GUI/GPU 或跨客户端像素等价。

按键入口显式启用 headless active、有序事件；原版仅 headless 读取注入 Win32 raw 状态，正常 P/Invoke 不变；
蛇版调用原 SetKeyPressed/Released，不复制 latch。支持在 AWAIT 0 原事件泵位置投递事件，保留 ClearLatches 顺序。
reset/load 清理静态 keytoggle、held/latch、队列、hook，观察不消费输入；请求先完整验证再注入。

| 组 | 最小验收断面 |
|---|---|
| PRINTC | 左右对齐、ASCII/日文、长内容、满列/换行、按钮位置、真实 padding |
| 算术 | 加减乘溢出、负边界、除零/模零；常量折叠及变量执行 |
| RNG | 固定 seed、序列、dump/restore、非法 RANDDATA，记录实际算法 |
| REF | Integer/String 数组、alias 写回、静态/动态调用；元素 REF 单列后续 |
| extra args | 正常、额外值、尾随空项、额外表达式副作用 |
| TOINT | 合法、小数、空串、非法/超范围；值、错误、诊断 |
| GETKEY | inactive、held/edge、交错查询、同泵 down+up、AWAIT、reset 隔离 |

每例分别保存两 oracle、Rust 当前结果和目标处置；未实现蛇版差异逐例归属后续批次，不能批量 skip 或改 golden 掩盖。
2026-08-27 源码复核修正历史 TOINT 分类：两参考版本对空串、普通非数字字符串均返回 0；
明确差异在整数读取异常（例如超范围数值）是否传播。snake 捕获后返回 0，源码本身不产生 warning；
后续 runtime warning 属于待设计的有意差异，不能写成 oracle 的既有输出。
两个参考仓库逐文件补 REFERENCE_CHANGES.md / HEADLESS_CHANGES.md，正常引擎工程也须编译。

#### 调度、测试与交付

先写本方案及基线，再并行 profile 契约、基线/coverage、oracle 扩展；接口稳定后流水线推进持久化和三端。
所有代码、fixture、测试和格式调整完成后，只启动一次独立 refactor-rustyera-code 审查；全部要求落实后才启动第一条测试。
每条测试交由 gpt-5.6-terra low 子智能体，禁止其编辑/格式化/提交。依赖准备不替代验证性构建；构建验收计入预算。

| 范围 | 静态门禁 |
|---|---|
| core | fmt、workspace/all-targets check、Clippy、最小回归 → 一次 workspace 全量 |
| runtime-tester | 独立 workspace fmt/check/Clippy/定向及完整测试 |
| TUI | 最小 pytest → 一次完整 pytest、Ruff、ABI/协议、动态库加载/打包相关检查 |
| Web | 最小 Vitest → 一次完整 Vitest、typecheck/lint/format/build/WASM |
| Web Rust | fmt/check/Clippy/一次 workspace test、check:core-rev |
| 两 oracle | wrapper 与正常 Emuera 工程构建及相关静态检查 |

所有适用静态/共享门禁通过后才运行两 oracle 各一次完整平台 smoke+七组差分、真实 TUI C ABI、Chromium/本机 Firefox/Safari
真实 WASM、真实 Tauri/WebView（先断言 bridgeKind）。使用各组件测试 skill；同套全量最多一次，失败仅最小复验。
原计划整批首条测试起共享 60 分钟墙钟，后由用户明确取消总时限；单条命令仍有超时。Browser/Tauri/oracle 长流程按 5 秒完整状态看门狗，TUI
按稳定等待点。同步 oracle 请求短时有界，不向忙碌 NDJSON 会话并发发送 observe，不用时间戳伪造进展。

重点矩阵：默认/显式原版一致、snake 不污染原版；cold/warm/miss/分段上传/导入导出保留身份；跨 profile cache 不命中且函数
复用为零；配置错误不降级、竞争/取消/旧回复不误提交；跨 profile save/snapshot 原子拒绝且路径不覆盖；原版裸存档回归；
snake envelope 损坏/未知版本拒绝；三类错误准确区分；observer 无副作用、旧 oracle 请求不变。

分项提交：基线工具、profile 契约、cache/artifact、save 隔离、诊断/coverage、fixture；TUI/Web 各自 profile 与存储接入；
两个 oracle 各自布局观察及按键注入。core 契约完成提交后同步前端完整 SHA，Web 全部 Git rev/锁文件一致；记录实际动态库、
WASM/native core SHA。不得自动推送/合并；远端不可获取的本地 SHA 如实说明。
结果只回写本组 core，根 changelog 只记录完成的产品行为。通过全部获准门禁、分项提交和可复现证据后才能完成批次 0；
登记的后续蛇版语义差异不是兼容通过，也不是蛇版 TW 已可玩。用户未另设实施时限。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 0B | core `erabasic-compat`、era-config、runtime-protocol、parser/analyzer、runtime session | 统一身份、schema 4、protocol 36 和纯配置解析入口，严格校验加载/重载；实验警告携带实际策略 | 三端已同步；不改变后续蛇版语义 | `43e589f`、`f09810e` |
| 0C-cache | core HIR/compiler/bytecode/validator、compiled_cache、VM/runtime snapshot | 身份进入缓存与函数 key、跨 profile patch/snapshot 拒绝、v9 manifest 与旧项目源码提取 | HIR 13、container 16/ABI 38、cache 9、snapshot 11/19 | `0be0ce8`、`8d7c2b0`；依赖 0B |
| 0C-save | core era-runtime-save、save_adapter、storage metadata | snake v1 envelope、长度/身份/checksum 校验；原版裸格式保留 | snake 不接受裸存档；外部 snake 导入仍属后续批次 | `65397a5`；依赖 0B |
| 0D | TUI、Web bridge/WASM、Browser Worker、Tauri ProjectHost | 配置先解析、完整/轻量 manifest 透传、存储命名空间隔离 | 三端绑定 core `8862fa9`，实际库/WASM/native 已验证 | TUI `cc1907e`→`7fa8ea0`；Web `63a56de`、`d57c47b`→`56b43b4` |
| 0A/0E/0F | runtime-tester、双 oracle headless 接入、core fixture/driver | 基线、覆盖扫描、实际布局/primitive input、Rust 同输入采集及离线差分 | 观测/差异/阻塞分别报告；不宣称完整兼容 | 基础 `713c492`、`fd00c9a`、`0e0f7b9`、`b125dc3`、`8862fa9`；工具修复 `954e58d`、`5b2c31c` |

下文按时间记录首次验收及定向修复，不能把早期“待验证”当作最终状态。初期依赖准备使用本组新 `.venv`、`node_modules`：Web
`npm ci --ignore-scripts` 已安装；TUI 官方下载超时后由 `uv export --frozen --no-emit-project`
导出含哈希的锁定 requirements，通过清华镜像严格校验哈希安装，再安装本地 editable 包入口。
未修改前端依赖锁定版本。原 Web Cargo.lock 差异备份在本组忽略目录
`batch-0-work/web-lock-before.patch` 与 `web-Cargo.lock.before`，继续保留。

### 审查与验收结果

- 实现与测试输入、环境、最终绑定、游戏/资源 hash、profile/seed 见本节按时间记录及“最终绑定、三端与双 oracle 结果”；基线 hash 以实际字节摘要为准。
- 已完成唯一一次独立 `refactor-rustyera-code` 审查（`batch0_refactor_review`）；审查只读，未测试。
  审查提出 11 项必改要求：①三端 Project/Data 禁止 snake 回退原根目录；②Browser 全加载生命周期取消所有权与配置摘要核对；
  ③bridge 复用 core 规范化配置摘要；④按操作比较实际输出，分离 setup/log/脚本诊断；⑤Rust observation 按 fixture/input/session 拆分；
  ⑥C# 请求先完整验证再施加输入，删除 snake 无效影子状态；⑦布局证据依据实际 provider，区分字体输入 hash 与实际选用证据；
  ⑧共享测试 runner 硬截止和进程组清理；⑨长流程 5 秒完整真实状态监督；⑩修正旧断言并补身份拒绝路径；
  ⑪聚焦真实 Tauri snake 存取验收入口。全部落实后才启动测试，后续不再启动或续跑重构审查。
- 唯一审查的①至⑪全部要求已落实并格式化，输入冻结准备验证；主智能体落实比较器、请求原子性、字体证据、
  外层预算监督和save负例，原实现者落实前端竞争/隔离、core回归/模块拆分及完整状态监督；没有再次启动审查。
- Browser 的operation token覆盖扫描、resolve、缓存、materialize、分段上传与最终字体绑定；保留并复核core配置摘要。
  TUI/Browser/Tauri的Project/Data Read/ReadRange/Stat/List禁止snake回退原版根目录；Resource共享不变。
- Tauri新增真实专用spec；两端save fixture按输入1、2分别观察37→99→37，检查envelope与原版哨兵。
- 三类工具监督已忽略协议外层id/关联序号等纯元数据，仍保留原始快照与脚本id字段；双oracle驱动跨case独立5秒采样。
- ⑦额外定向观察蛇版 PRINTC 的 SKIASHARP 与 TEXTRENDERER；供给字体哈希不再冒充实际安装字节来源，
  证据明确 fontByteSource=unverified-installed-source。正常游戏测量和按键语义不变。
- 首条测试命令于 2026-08-27 19:35:50（Asia/Shanghai）启动，原硬截止为20:35:50。
  用户在本轮明确要求“本次测试时间预算不限时，务必测试和修复到位”，已取消本批总时限；
  保留首次启动时间、原截止和覆盖授权，不重置全量次数/审查次数。每条命令仍有独立超时、进程清理和卡死监督。
  本组 `batch-0-work/validation/budget.json` 为预算证据，命令的 JSON/log 保存在同目录。
- 用户补充：全部现成测试环境、依赖和工具可直接复用，仅缺失或版本差异才下载；Chromium 不新下载。
  已停止新 Chromium 安装，后续复用现有版本 1234 工具，并保持浏览器配置、项目副本、存档和输出隔离。
- 本批同时涉及原版与蛇版身份/观察，因此分别运行两个 oracle。语义基准保持原版
  `26a35dc9334bb67590b96f7b8efbefbf199e391e`、蛇版 `fc4fb21416768c17256d0e82f997e5f99c9bba91`；
  验证开始 wrapper HEAD 分别为 `af9886061ba420d530581e7975c4db735c391d03`、
  `4a46d7b52280733e8ecb8eeb630a87facdc03a23`，均带本批 headless 改动，非干净基准。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| Core workspace 静态 | fmt/check/clippy、最小兼容测试、workspace test | 契约/隔离通过 | 首次全量在 era-config 旧 schema=3 断言失败；后续首次覆盖未执行的修改包时 era-runtime 5项失败，其余通过 | schema/cache/snapshot 旧断言修正后，6个失败节点分别通过；受影响 fmt/clippy 通过 | 首次全量失败与定向通过分开记录，未重跑全量 |
| runtime-tester 静态 | standalone fmt/check/clippy/test/build | 审计工具正确 | 唯一全量26/27，FORM赋值 RHS 的函数出现项漏计 | 复用表达式 parser 并标记待类型确认；scan模块2/2、fmt/check/clippy/build通过 | 不把候选调用标为可执行代码 |
| TUI 静态 | Ruff、最小102用例、worker回归、pytest | 协议/存储通过 | 唯一全量415通过/1失败（真实 staging 旧manifest） | 修复manifest后失败节点通过，Ruff通过；此前worker最小失败节点亦通过 | 使用本组新CAPI，不是主工作区旧库 |
| Web 静态 | Vitest/typecheck/lint/format/build；Rust fmt/check/clippy/test；WASM build | 两host契约可构建 | 唯一Vitest92文件1005/1005；Rust bridge28、Tauri63通过/1既有ignored、WASM10；构建通过 | 早期最小测试/Clippy失败均定向恢复 | 动态发现bigint版本投影错误后，相关18项回归及build定向通过 |
| Python/C# 静态 | driver8、supervisor2+2、local runner5；正常引擎build、CLI publish | 双oracle可构建且headless隔离 | 通过；正常原版219警告、蛇版917警告、0错误 | 取消预算后的runner单测/语法定向通过 | 已有编译警告保留，不声称零警告 |
| TUI 动态 | snake-profile、snake-profile-save；seed123456 | 启动、37→99→37 | 两场景退出0、断言通过 | 无 | 早期traces位于本组batch-0-work/traces；最终pin后save另行复验通过 |
| 双oracle smoke | 本组prefix，预发布CLI，skip-build | 两引擎可观察 | 原版/蛇版首次smoke均退出0；蛇版8组全部通过 | 无 | smoke只证明可用；七组同输入差分最终已完成，见下方汇总 |
| Chromium 启动 | snake-profile，seed123456，clock2026-01-01 | 无错误启动 | 目标断言通过，但日志出现兼容版本无效/profile invalid，整体不能验收 | WASM bigint解析及配置/诊断回归、三项Chromium定向复验通过 | trace在Web .rustyera/test-runs/snake-profile-20260827123106402-90238/ |
| 8游戏基线 | runtime-tester baseline | 实际内容哈希 | 首次因输出父目录不存在退出2；创建专用目录后采集退出0 | 9,118,924字节JSON已生成 | batch-0-work/results/baseline-8-games.json；起始core SHA+dirty |


#### 后续定向验证与修复（2026-08-27 21:05 更新）

- Chromium bigint版本修复后，startup、save、reload-rejected均通过：日志恢复
  `profile=emuera.skia.snake@1/1`，无配置解析错误；拒绝切换后仍可输入并达到`FLAG:0=44`。
  证据为Web `.rustyera/test-runs/` 的 `snake-profile-20260827123657777-95410`、
  `snake-profile-save-20260827123714639-95681`、`snake-profile-reload-rejected-20260827123731618-95806`。
- 原生Firefox首次命令遗漏`--project tests/fixtures/snake-profile-project --startup-only`，
  误跑默认完整交互，在提示通知遮挡按钮时失败。记为执行范围错误，不修改产品来迎合该错误命令；
  计划内Firefox/Safari启动与聚焦Tauri场景待正确命令验证。
- 两profile实际Rust observations已采集；双oracle首次差分各自完成PRINTC和3个常量算术case，
  在`arithmetic-variable`发现裸`RETURN`将`RESULT:0`覆盖为0。已将涉及结果的fixture改为
  `RETURN RESULT:0`，未改期望值；实际runtime回归验证保留i64溢出值。后续仅跑失败/受影响与尚未执行case，
  不重跑完整smoke/差分。旧observations的fixture hash已过期，不伪造新哈希复用。
- 覆盖报告第一次停止于Parsing8200/8200；已加入全局声明、函数索引和局部声明的真实进度，
  定向重试越过原阻塞点，但在DeclaringLocals111600/112727因百分比汇报过粗再次触发看门狗。
  已增加最多64个实际完成单位的汇报阈值，并在锁内读取最新计数防并行进度回退。
  两次故障均真实退出2、未生成报告；未将卡死算成coverage通过。新增进度等价性/细粒度回归、
  analyzer/runtime/tool静态检查、CAPI加载回归均通过；报告仍待定向采集。
- 当前统一前端pin为core `8862fa957f67bae553cb8e30fa349b113745aa3f`，所有产品源码已提交；
  Web19个core依赖锁定同一Git SHA，离线macOS/WASM依赖获取通过。最终绑定/打包门禁执行中，
  未推送远端，不能声称此本地SHA已可由远端CI获取。
- Core分项提交：`43e589f`契约、`0be0ce8`artifact/VM、`f09810e`runtime解析/诊断、
  `8d7c2b0`runtime缓存/snapshot、`65397a5`save envelope、`7f00ba1`声明进度、
  `05742e6`大项目进度间隔、`713c492`基线/监督、`fd00c9a`实际observations、
  `0e0f7b9`fixture RETURN与定向选择、`b125dc3`coverage、`8862fa9`双oracle driver。
  TUI功能提交`cc1907e`；Web Browser/共享桥接提交`63a56de`。这些是依赖链提交，
  验证针对集成源码状态，不将中间契约提交当作独立可发布版本。

#### 最终绑定、三端与双 oracle 结果

- Core 产品契约完整 SHA 为 `8862fa957f67bae553cb8e30fa349b113745aa3f`。其后
  `954e58d`、`5b2c31c` 只修复审计/对照工具及工具文档，不改 `crates/`、产品 Cargo.toml/lock。
  TUI pin、Web 五处直接 Git rev 和 19 个锁定 core source 均为该完整 SHA；没有发布 path dependency。
  这是本地提交，未推送、未合并回主线，不保证远端 CI 可获取。
- TUI 最终 `7fa8ea07b19886da547ce23e4b03b0922268f76a`：本组
  `target/tui-static/debug/libera_runtime_capi.dylib` 重建通过，源库及 PyInstaller 包装库真实
  load/create/release 通过。C ABI 3.9 满足 TUI 请求的 3.8，runtime protocol 36.0。
  复用已安装 PyInstaller 6.21.0，生成本组 `rustyera-tui/dist/rustyera_tui/`。
  最终 pin 的真实 37→99→37 存取通过：`batch-0-work/traces/snake-profile-save-pinned.ndjson`。
  原启动和 save trace 也保留。Ruff、单一失败 pytest 节点复验通过，未重跑全量 415/1。
- Web 最终 `56b43b445ae72f932bcfce34618fbd6993d410bb`：pin 检查、fmt/check/Clippy、
  bridge/WASM 聚焦回归、WASM build 全部通过。首次 Vitest 92 文件 1005/1005；首次 Rust
  bridge 28、Tauri 63 通过/1 既有 ignored、WASM 10。bigint 修复后相关 18 个回归和 build 通过。
  原生 Firefox 154.0.1、Safari 26.6.2 使用正确命令：
  `npm run test:browser-compat -- --browser firefox --project tests/fixtures/snake-profile-project --startup-only`
  （Safari 仅替换 browser），均通过。快照为 `browser-compat-firefox-1787836083157/`、
  `browser-compat-safari-1787836100485/` 的 `snapshots.ndjson`。
- 真实 Tauri 命令：
  `npm run test:tauri -- --project tests/fixtures/snake-profile-project --spec tests/tauri/snake-profile.spec.mjs`。
  native WebDriver 实际断言 `bridgeKind=tauri`，37→99→37、envelope、原版哨兵均通过，1 case passed。
  快照为 Web `.rustyera/test-runs/tauri-snapshots/2026-08-27T13-09-02.314Z-snake-profile.spec.mjs.jsonl`。
  既有 projectionStaleRequest debug 消息未导致 fault；不报告为新错误。
- Chromium 复用主工作区现有 `.playwright-browsers` 的 chromium/headless 1234、ffmpeg 1011；
  未重新下载。浏览器配置、端口、fixture/存档、输出仍隔离；TUI/Web 不冒用主工作区旧 core 产物。
- 两 oracle 正常工程与发布 CLI 静态通过，原版 219、蛇版 917 个既有 warning，0 error；
  首次完整 macOS/Wine smoke 分别通过，未重跑完整 smoke。两个专属 prefix 位于本组 `.wine-prefix/`。
  后续部分 Wine 请求受沙箱启动/清理权限影响，失败记录保留，按相同定向请求获批后恢复；
  driver 同时修复清理异常覆盖主错误/丢失 raw requests 的问题。
- 七组、每引擎 27 case 的有效 Rust/原版、Rust/蛇版对照已完成，并额外观察蛇版 PRINTC
  TEXTRENDERER。完整逐例状态、后续归属、最终来源 hash 见
  [双 oracle 比较汇总](BATCH_0_ORACLE_RESULTS.md)。setup 输出、配置诊断及请求成功/执行失败
  的比较器错误均已有回归并定向通过；重算读取原始记录，不重跑引擎或覆盖旧证据。
  原版 14 matched / 6 incomparable / 4 blocked / 3 different；蛇版 8 matched / 4 blocked / 15 different。
  `matched` 仅是共同观测，不是蛇版目标语义已实现。
- 蛇版 RNG `DumpRanddata` 把 `GetRand` 写到临时 `RANDDATA.ToArray` 副本，导致
  `INITRAND` 恢复零；实际 192905、520548、0、0 作为固定基准缺陷 observed，未修改参考算法。
  原版 roundtrip 仍严格检查。GETKEY 尚缺完整 AWAIT trace 消费/reset 等价原语，PRINTC 仍为
  像素布局与列布局差异；各项归批次 2/4/6，不能靠 oracle smoke 抹平。
- 字体输入仍是 `BIZ UDGothic`，SHA-256
  `e267830408f04daf92858d89477f2df8539c05ee4fe597d13ffdcaa7565b519e`。
  实际 provider/family/fallback 已观察，但实际安装字节来源仍为 `unverified-installed-source`。
  不把输入文件 hash 当作安装验证，不宣称 GUI/GPU 或跨客户端像素一致。

参考仓库分项提交（仅 wrapper/headless；语义基准不移动）：

| 仓库 | 输入原语/隔离 | 只读布局观察 | 共享入口/审计 | 测试监督 | 最终 wrapper SHA |
|---|---|---|---|---|---|
| emuera.em | `68e3894` | `866d528` | `ffe560d` | 随入口记录 | `ffe560dad2fe480c8babddcae0122137350bf021` |
| 蛇版 emuera | `366a76d` | `ad3b25e` | `beda156` | `2c67518` | `2c67518c594a638c2fbdef3e780341eb66ace294` |

发布 CLI 的实际构建来源仍为初始 wrapper SHA + 本批 dirty 源码；最终提交仅整理相同源码，
没有在产物构建后改 C# 字节。原始 evidence 的 wrapperSha 不回填成新 HEAD。两个审计文档
逐文件包含所有引擎、CLI、测试修改；正常入口不变，观察不重排版/消费输入，事件泵不改 latch 顺序。

#### 覆盖率工具修复与磁盘管理

- 第三次失败项目采集越过声明阶段后暴露报告复杂度：3,310,758 个出现点逐项扫描所有诊断、
  复制静态 API 证据并收集巨大 Value；62,848 行时 RSS 约 1.84 GiB，估计需要超过一小时。
  主任务主动停止并清理被沙箱阻挡的遗留进程，不通过扩大超时掩盖工具问题。
- 修复 `954e58d`：按 path/stage 索引诊断；每个 API 共享 registry/VM/runtime/service/frontend
  证据，每行保留原始 appearance/arity/UTF-8 span/activity、diagnostic ID、动态目标与声明覆盖。
  schema 2 流式写入，Markdown 汇总所有 API，不删行/采样。每秒最多一次序列化完整诊断状态，
  只在真实写入后推进；独立五秒完整状态看门狗不变。无效 parser span 原样保留并显式 unverified，
  不伪造有效位置或可执行结论。fmt/check/Clippy、coverage 模块 13/13、build 通过。
- 修复后 snake TW 原版 profile 报告约每分钟 220 万行，3,310,758 行写入成功；8,680 条诊断、
  3,216 个 API、4,196 个 invalid_parser_span。eraAkumaMaid 首次目标 775,680 行、186 条诊断、
  592 个 API、10,086 个 invalid_parser_span。真实流式结构验证逐行校验 API/诊断引用、计数、
  原始 UTF-8 span 和完整 EOF，不再把字符串计数冒充结构验证。
- 用户追加“硬盘空间消耗过大，要做管理”后，暂停新采集并确认本批无在跑的 Cargo/coverage。
  清理本组五个 `target/.../debug/incremental` 共约 12 GiB，保留 deps、可执行文件、动态库、
  WASM、工具及现有依赖环境。未删除主工作区构建、浏览器、用户游戏或存档。
- 使用已有 gzip 将两份 2.0 GiB/431 MiB 报告压缩为 115 MiB/23 MiB，校验解压字节 SHA-256
  与原件一致后删除未压缩副本。证据为 `batch-0-work/validation/coverage-initial-stream-validation.json`；
  原 command 记录中的 `.json` 现对应同路径 `.json.gz`，不是丢失报告。可用空间约 20→35 GiB。
  后续大报告通过 `pipefail`/`noclobber` 管道直接 gzip 写入，串行采集且每项检查剩余空间。
- 八游戏真实内容基线的可提交摘要见 [基线摘要](BATCH_0_BASELINE_SUMMARY.json)，原始
  9,118,924-byte JSON 保留本组 results；不把游戏内容或机器绝对路径提交为文档。

全部九份最终报告和结构校验均退出 0；共 **11,054,509** 个出现点，gzip 总大小
**381,507,167 bytes**（约 364 MiB）。完整压缩/解压 hash、工具 revision、输入身份与数量见
[覆盖摘要](BATCH_0_COVERAGE_SUMMARY.json)。各 `.json.gz` 同名 `.md` 可直接阅读。

| 项目 | profile | 出现点 | 诊断 | API | 无效 parser span |
|---|---|---:|---:|---:|---:|
| eratw-sub-modding（蛇版 TW） | emuera.em | 3,310,758 | 8,680 | 3,216 | 4,196 |
| eratw-sub-modding（蛇版 TW） | emuera.skia.snake | 3,310,758 | 8,680 | 3,216 | 4,196 |
| eraAkumaMaid | emuera.em | 775,680 | 186 | 592 | 10,086 |
| eraMaouEx | emuera.em | 324,990 | 407 | 223 | 3 |
| eraTW（原版 TW） | emuera.em | 1,673,134 | 2,173 | 1,568 | 582 |
| erafl | emuera.em | 390,266 | 5,375 | 1,171 | 2,531 |
| erarorona | emuera.em | 77,215 | 40 | 644 | 333 |
| eratohoK | emuera.em | 1,073,769 | 3,381 | 731 | 778 |
| era魔界牧場1.050_tc8 | emuera.em | 117,939 | 15 | 218 | 5,988 |

所有项目在显式 audit options 下仍有 CSV/analyzer 错误并阻止编译；这是保留的实际覆盖结果，
不是九个游戏全项目编译通过。eraMaouEx 的 `ERB/COMF90_ニプルファック.ERB` 和
`ERB/LIST_APPEND.ERH` 为 unsupported_encoding，保留原始 hash，不转换或修改用户游戏。
除蛇版 TW 失败目标定向重试外，其余七个项目和 snake profile 各首次采集，没有重跑 `--all-games` 全量。

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 批次 0 必需项 | 无未完成项 | 代码门禁、三端、双 oracle、基线/覆盖及证据校验完成 | 进入后续批次前读取本记录 | 本批范围未扩大为完整兼容 |
| 全项目编译及无效 parser span | 真实游戏尚有 CSV/语义/位置映射阻塞，按现有 parser/context 明确 unverified | 全部出现点与原始诊断保留，没有虚假 compiler pass | 批次 1 分项处理实际阻塞与输入摄取，不静默忽略未调用代码 | 沿用批次 1 |
| 算术/RNG/TOINT/extra args/GETKEY/REF/布局 | 后续批次 2/4/6 的语义未实施 | 双 oracle 每例已有 Rust 结果、差异或明确 blocker | 按比较汇总选择最小 fixture；RNG 缺陷须先决策 policy | 已补 RNG 实测与 TOINT 分类修正 |
| GUI/GPU、字体安装字节、跨平台像素一致 | 本批仅真实布局度量和实际 provider/family 观察 | 不把 Windows/Wine 无窗口观察解释为原生 GUI 或像素兼容 | 后续布局验收另设计环境与像素门禁 | 无新增承诺 |
| 总时限/磁盘控制 | 用户明确取消测试总时限并要求管理硬盘 | 保留单次全量/审查和五秒 watchdog；释放约 15 GiB、压缩直写 | 后续沿用隔离产物与按需缓存 | 只调整执行约束 |

### 交付与续做入口

- 本批 0A–0F 验收完成。首次 core/TUI/runtime-tester 全量中的失败及修复后的定向通过分别保留；
  没有重跑失败全量，也不描述为“修复后全量通过”。唯一重构审查只执行一次，11 项要求在首次测试前落实。
- Core 产品 SHA、TUI/Web 最终 SHA、两个 reference wrapper SHA 与分项依赖见上表；本批后续工具
  修复不改产品 crate，因此前端 pin 无需再移动。未推送、未合并主线、未改变产品版本号。
  根 `CHANGELOG_PENDING.md` 已按产品行为追加 profile/缓存/存档/三端隔离两条，根提交 `4d1f94d`；
  文档、测试、覆盖/磁盘管理不额外写成产品功能条目。
- 首条测试为 2026-08-27 19:35:50（Asia/Shanghai）；截至 22:12 已持续约 156 分钟，用户授权不限时。
  所有命令、退出码、时间与首次/定向范围在本组 `batch-0-work/validation/`；原 `budget.json`
  保留首次起点与被取消的原 deadline，不重置次数。静态验证记录与动态原始快照、差分结果均保留。
- 复现从 `tools/runtime-tester/COMPATIBILITY_AUDIT.md`、
  `tools/snake-compatibility-oracle/README.md` 开始；最终报告路径及 hash 已写入两个 JSON 摘要。
  本组 `batch-0-work/validate_coverage.py` 是标准库流式证据校验入口，不将多 GB JSON 一次读入内存。
  后续批次先检查实际 Git SHA、profile 和 fixture/source hash，再选失败 case 定向验证。
- 已删除 20 份 oracle 临时游戏副本和 1 份 Tauri 临时游戏副本（清单
  `batch-0-work/cleanup-game-copies.txt`），保留固定 fixture、原始 evidence、traces、报告、工具和运行库。
  两专属 Wine prefix 的有界 `wineserver -w` 均退出 0，进程核验未发现 Emuera.ReferenceCli；
  Browser/Tauri/TUI/coverage 测试均结束。主工作区游戏、存档、参考语义及用户修改未被清理。

<a id="batch-1"></a>

## 批次 1：完整摄取与参考能力阻塞项

计划入口：[改造思路 / 批次 1](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-1)。状态：实施中；负责人 / 最近更新：Codex / 2026-08-28。

### 具体实施方案

- 采用用户确认的[详细实施方案](BATCH_1_IMPLEMENTATION_PLAN.md)，1A/1B/1C/1D 为四个独立实施子批次。
- 上游批次 0 已完成，来源/门禁/后续缺口见上节；开工 core/TUI/Web 专用分支均干净。
- 1A 开发归属：主智能体 core 数据/协议/文档，TUI 与 Web 分别独立执行者；共享版本和最终绑定由主智能体整合。
- 所有子批次测试总时限由用户取消；单套全量一次、测试前唯一重构审查、静态先于动态和五秒看门狗不变。
- 开工磁盘可用约 35 GiB；本组 target 约 15 GiB，批次 0 证据约 2.1 GiB。新结果使用本组 batch-1-work/，不覆盖批次 0。

| 子批次 | 状态 | 重构审查启动次数 | 首次测试 | 全量启动记录 | 当前边界 |
|---|---|---|---|---|---|
| 1A | 已完成，保留明确差异与观察限制 | 1 | 2026-08-27 23:25:42 +08:00 | core、runtime-tester、TUI、Web Vitest/Rust、oracle Python、supervisor unit 各启动一次 | S01/S02、三端摄取和只读资源清单；动态已启动，首次失败与定向复验分别记录 |
| 1B | 已完成本子批范围，保留明确差分边界 | 1 | 2026-08-28 01:47:57 +08:00 | core workspace / runtime-tester 首次通过 | 双 oracle 各 23 可比项匹配；错误观察限制与批次 2 实参差异见下文 |
| 1C | 已完成并分项提交；差异及首次失败保留 | 1 | 2026-08-28 03:17:34 +08:00 | core/tool/TUI/Web Vitest/Web Rust、双 smoke/列 Oracle、Python driver、四项 BBAS 各一次 | S12、GLOBAL、安全读取；三端实际数据断面通过 |
| 1D | 隔离实现完成，唯一独立重构审查中 | 1 | 未启动 | 无 | S04；活动仓未合入，尚未测试 |

#### 1A 实施进度（静态已执行，动态验收未完成）

- 已写入 S01 三端类别 6/7、完整/快速扫描、源索引、物化、manifest/Worker 传输和增量重载；
  ERD 经数据加载器处理，不再作为 ERB 声明源码。ALS/ERD 采用严格 UTF-8，原始字节摘要与提交文本摘要分列记录。
- 已写入 S02 compatibility 透传、ERD→CSV→ALS 顺序、同目录同 root 关联、signed alias 与显式反向名称；
  snake semantic/policy 变为 2/2，其他蛇版策略保持实验状态及原值。动态字符串索引读取用户表，GETNUM 范围不扩大。
- 资源清单纳入 XML/TXT/DB/SQLite，排除 `.git/.rustyera/sav/save/saves/data/log/logs` 根目录，
  DLL 不作为扩展加载。新输入限制在授权根内，旧源码的既有外链行为保留；Data→Resource 回退尚未实现，属于 1C。
- 当前格式调整：runtime protocol 37.0、project data 3、HIR 14、bytecode container 17、compiler ABI 39、native ABI 16、
  compiled cache/project 10。ISA、VM ABI 和 C ABI 未改；原版 v9 完整项目可恢复源码重建，
  旧 snake 1/1 identity 明确拒绝，所有旧编译缓存拒绝。
- 代码已格式化，core 产品与 fixture 已分项提交；前端 pin 与发布锁文件已同步至
  `f8239986c1d7da69432b2b16bac98e38d71b881f`，重建/动态门禁仍在进行，不代表 1A 完成。
- 1A 测试记录入口为本组 `batch-1-work/1A/`。监督脚本复用批次 0 的进程清理逻辑，默认无总 deadline，
  每命令有独立限额；全量启动标记按子批次独立保存。构建优先复用本组已核验的 `target/core-static`、
  `target/runtime-tester-static`、`target/tui-static`、`target/web-static`，不复用主工作区产物。

#### 1A 唯一重构审查

审查者 `review_batch_1a` 使用 `$refactor-rustyera-code` 完成一次只读跨组件审查，
结论为需要有限重构和修正，不需要大规模拆分。未运行任何测试或格式化，不再启动或恢复该审查者。

| 要求 | 落实方式 | 首次测试前状态 |
|---|---|---|
| R1 完整来源路径 | CSV 输入分别保存 root-relative lookup path 和原始 `source_path`；初次加载与 deferred 诊断保留 provenance，新增双根和 UTF-8 span 回归 | 已写入，待验证 |
| R2 类别主导错误 | runtime 按 Als/Erd 类别拒绝 IoError/Bytes/ExternalResource，不依赖扩展名；保留旧 CSV NotFound，增加非标准后缀及双根错误测试 | 已写入，待验证 |
| R3 工具摄取一致 | `project_inputs` 统一 main/extractor/coverage 分类，修复 resources 下数据资源过滤及 root 丢失，新增五个 canonical/root/UTF-8 回归 | 已写入，待验证 |
| R4 读取时授权复查 | Tauri 扫描、读取、prefix 和流式导出共用授权路径检查；测试同 inode 移至 Data/private 后链接替换 | 已写入，待验证 |
| R5 原版既有缺口 | 增加双 profile 静态主名执行；保留原版动态主名 oracle 成功期望并登记 Rust 既有差异，不伪装拒绝 | 已写入，待验证 |
| R6 v9 profile 边界 | 原版 v9 允许源码恢复；历史 snake 1/1 非流式和流式明确拒绝，文档不再概括为全部可重建 | 已写入，待验证 |
| R7 锁定依赖 | 主智能体机械补 core/runtime-tester 锁文件的 CSV→compat 依赖边，不改变包版本；前端绑定等 core 提交 | 已写入，待验证 |

七项要求均在首条测试前落实。后续格式化和测试不再启动或恢复重构审查；
测试失败由主智能体定位修复，只执行受影响的定向复验。

#### 1A 静态门禁进展

测试由 `gpt-5.6-terra / low` 执行者运行；详细命令、时间、退出码与进程清理记录保存于
本组 `batch-1-work/1A/validation/`。`budget.json` 的总 deadline 为 null。

| 检查 | 首次结果 | 修复及定向复验 | 当前结论 |
|---|---|---|---|
| core fmt / workspace check | fmt 通过；check 因 replay 类别映射缺 ALS/ERD 失败 | 补 replay 两类别与已有 hot reload 测试；单文件格式及 runtime 定向 check 通过 | 后续 lint/最小/full 见下行 |
| core lint / 最小 / 全量 | Clippy 先后发现文档标记、显式 Default、expect_err 与 checked cast 要求 | 主智能体修正；相关文件格式及各受影响 crate Clippy 均通过 | 最小 VM 25、CSV 16、protocol 30、compat 1、extractor 7、runtime replay 6/project 29/cache 28 通过；唯一 workspace full 退出 0，约 87.7 秒 |
| 独立 runtime-tester | 首次 check 缺 `has_direct_child_directory` 导入 | 补导入，定向格式/check 恢复；Clippy 通过 | 最小 inputs 2/extractor 3/fixture 5 通过；唯一全量 39/39 通过 |
| Web 定向 Vitest | 101 通过、3 失败；旧 TXT 排除期望及跨 realm hash 数组比较 | 更新 TXT 资源期望，显式比对完整 hash 字节；仅两受影响文件 80/80 通过 | 其余三文件首次结果保留 |
| Web 完整 Vitest | 92 文件、1017 用例全部通过，唯一一次 | 无 | JS 全量通过 |
| Web typecheck / lint / format / build | 全部退出 0 | 无 | 最终 pin 下 WASM 发布构建及生产 build 通过 |
| Web Rust 最小 / 全量 | fmt/check/clippy 通过；project 最小 36 通过、1 失败、1 既有忽略（TXT 排除旧期望） | 修正后只复验失败具名用例及受影响格式/Clippy；WASM 类别回归通过 | 随后唯一全量：bridge 28、Tauri 69、WASM 11 通过；Tauri 1 个既有 handoff 用例忽略，doctest 通过 |
| oracle Python 工具 | 本批定向四例通过；唯一完整 19/19 通过 | 无 | 仅驱动/比较单元测试，不是实际 oracle 运行 |
| TUI 最小 pytest / Ruff | project 81、wire 3 全部通过；Ruff 通过 | 无 | 完整 pytest 与 pin 定向结果见下行 |
| TUI 完整 pytest | 首次 425 通过、3 失败，44.05 秒；失败均为旧 `version.py` 的 `7ba54e80` 与旧 pin `8862fa95` 已不一致 | 同步最终 pin 与显示元数据后只复验失败三个节点，3/3 通过；version.py Ruff 通过 | 不重跑全量；7ba54e80 是旧 TUI 文本，不是本批 C ABI 库的实际 SHA |
| TUI 静态打包 | 首次 COLLECT 因 sandbox 禁止写用户 bincache 失败 | 最小权限批准后重试构建成功；dist 93 MiB/work 23 MiB | 产物位于本组 batch-1-work/1A/tui-dist；后续 --help 与包内库真实加载冒烟均通过 |
| supervisor 单元测试 | 五例中四通过；孤儿进程用例因 sandbox 的 ps 权限失败 | 沙箱外只复验该一例通过；一次错误模块导入未加载实际用例，单独保留记录 | 首次全量与定向结果分列，不重跑全量 |

动态在相关静态与共享 core 门禁全部恢复后启动；后续失败仅定向复验，不重跑全量。
Web 本地 Rust 封装执行前后发布锁文件 SHA-256 一致（`88491d…8c69c9e`），
此值是同步前基线；现已把原本无 source 的十九个 core 锁包改为完整 Git source/rev/commit，
并补 CSV→compat 依赖边；最终 pin 下 check/clippy/WASM/build 已通过，本地封装执行前后新发布锁摘要保持一致。未以本地 patch 代替最终绑定。
Rust 构建后磁盘剩余约 25 GiB；最多两个构建并行，尚未触发 20 GiB 阈值。

#### 1A 契约提交与当前绑定

| 项目 | core commit | 范围 / 依赖 |
|---|---|---|
| 公共输入基础 | `bb4b04c` | 可选 source_path 与旧调用方机械适配 |
| S02 | `99e3621` | ERD/ALS 语义、signed lookup、反向名称、profile/数据格式；依赖公共基础 |
| S01 | `8c08eb4` | 协议类别、runtime 摄取/缓存/诊断、工具分类；依赖公共基础和 S02 |
| 执行 fixture / 比较工具 | `f823998` | 24 个双 oracle index case；保留原版动态主名既有差异 |

上述提交对应已通过静态门禁的最终代码，不将尚待执行的双 oracle / 动态客户端描述为通过。
当前 TUI/Web pin 均为 `f8239986c1d7da69432b2b16bac98e38d71b881f`，core 产品 crate 无未提交差异。
C ABI 由本组 `target/core-static/debug/libera_runtime_capi.dylib` 重建，SHA-256 为
`5b1d280c2a3b48d77bc7807dd641a665acba20368a1193b0afe0041a0f77f670`；TUI 复验显式使用
`ERA_RUNTIME_LIBRARY`，不使用主工作区产物。初次 C ABI 构建与 pin 后重建内容摘要一致。
一次错误地从 core cwd 调用 TUI 构建脚本在构建前即退出 2，已在 TUI cwd 正确重建；
该基础设施调用错误不作为产品编译失败或成功证据。

#### 1A 动态与真实输入证据（2026-08-28）

- TUI 首次场景因 fixture 缺 binary saves 配置导致 CHARADATA 声明失败；移除 SAVEDATA
  并不足以解除 CHARADATA 的配置要求。补 `[save] binary_format=true` 后，新增真实 C ABI
  fixture 回归单节点、Ruff、diff check 通过。原场景定向复验通过：seed 123456、snake 2/2、
  integer 等待点、`INGEST_FLAG=10,11,300` / `INGEST_BUFF=50,60` /
  `INGEST_ERD=70,80,90` / `SNAKE_INGESTION_READY` 四项均满足。
- TUI PyInstaller 可执行文件 `--help` 与包内 dylib 同场景加载均通过。包内库经 macOS
  处理后的 SHA-256 为 `8252125b1e279e58ba4281796ecaec8987f33e0c2586c3b5422ddf9a1025d851`，
  与未打包库摘要不同，不冒称字节相同。证据 labels `tui-package-help-smoke-1a`、
  `tui-package-embedded-cabi-ingestion-1a`。
- Chromium 私有完整包下载零进展到单命令限额，headless 包下载也未完成；用户明确要求
  不再下载、复用已有浏览器，已停止下载。实际采用主工作区 `.playwright-browsers` 的匹配
  v1234 程序；仅浏览器程序复用，profile、session、项目、WASM 与输出仍保持本组隔离。
  首次成功启动后冷加载输出正确；ALS scoped reload 后仍输出旧值，正在核对旧调用栈
  generation 保留语义与 fixture 的新入口验收，不把旧栈结果误认为冷加载成功证明。
- 原生 Firefox 154.0.1 已观察正确启动输出和 OPFS 导入，但清理会话卡住且中断清理失败；
  保留为基础设施失败，不能把中间正确输出写成完整通过。Safari/Tauri 尚待执行。
- 原版及蛇版 oracle 平台 smoke 各唯一一次均退出 0。Rust 两 profile 的 24-case 原始观察
  已生成。原版完整差分运行到 `index-builtin-alias-same-index` 时失败：固定源
  `ConstantData.cs:1812` 的警告只传一个格式化参数，而 `Lang.cs:655` 模板需要两个，
  重复 index 会使 ALS 后续行停止读取。首次中途 watchdog 中出现的 canonical case 不是
  实际失败 case；以 evidence.json 的最后请求为准。该原版 oracle 缺陷不据此修改 Rust 原版既有语义。
- 蛇版首个差分 load 的 presentation 校验失败：fallback 字体 `SimHei` 不符合 BIZ UDGothic。
  原失败保留，不能称字体验收通过；正在为只比较值/逻辑文本的数据用例增加显式
  `--logical-output-only`，该模式拒绝 presentation 用例且证据注明不请求像素/字体观察。
- 蛇版 TW 唯一静态覆盖流程退出 0：15761 inputs（ALS 20、ERD 2、CSV 206、ERB 3931、
  ERH 169、Resource 11337、ResourceManifest 96），读取错误 0；4328 文本输入同时记录
  原始与 UTF-8 payload 的独立 BLAKE3/长度。CSV accepted，analyzer diagnostic_errors，
  compiler blocked_by_load_diagnostics，未生成 artifact；3310758 appearances、8602 diagnostics。
  这是审计默认选项下的静态覆盖，不是完整游戏编译或标题成功。
- 覆盖 JSON 直接写 gzip（压缩 89067678 bytes），原始 2169108268 bytes、SHA-256
  `5eb81b249e4d4ed058ca6d8075e840716225817499b587081b9f7aa148b3f47e`；没有落盘展开副本。压缩 SHA-256 `c26a23899e5e56b094b4aca1a3f2393ce00d0260cc03c73376313ae41cb52c17`。
  路径本组 `batch-1-work/1A/snake-ingestion-coverage.json.gz` 及同名 Markdown，命令/五秒
  观察见 `validation/core-snake-coverage-1a.*`。磁盘剩余约 24 GiB，未复制真实游戏。

#### 1A 续做记录（2026-08-28，仍未完成）

- TUI 已分项提交：`a4c77dc` 同步 core pin，`e0f743e` 实现摄取；当前工作树干净。
- Chromium 修正了热重载场景的新入口预期后通过：旧调用帧仍输出 42，真实“返回标题”
  确认后输出 84。返回标题复用当前 artifact，没有重新扫描或编译；对应 core 执行回归、
  两个既有返回标题回归、runtime Clippy/格式均通过。浏览器 trace：
  `.rustyera/test-runs/snake-ingestion-20260827164444088-25059/trace.ndjson`。
- Firefox 首次清理失败保留；定向重试通过（154.0.1），Safari 首次通过（26.6.2）；
  两者验证真实 WASM、OPFS cold import、目录 fallback picker 与预期输出。
  snapshots 位于 Web `.rustyera/test-runs/browser-compat-firefox-1787849102559/`
  和 `browser-compat-safari-1787849117245/`。
- Tauri 首次退出 7：新 binary 构建完成，但 runner 硬编码 `../target/debug`，实际启动旧 binary，
  其内嵌旧 fixture 路径不存在。故不能把该次描述为新 host 已通过真实加载。
  主智能体改为 Cargo metadata 的 target_directory，并通过 cargo-local 封装构建，
  避免本地 patch 改写发布锁文件；定向 Vitest 4 文件 51 例、typecheck、ESLint、Prettier、
  core-rev/锁审计、diff check 均通过，原生动态定向复验待执行。
- Web 最终发布锁 SHA-256 为 `4e46531f7ad1d1dc64f2e123a3212f0e944f4b295d1b4bf2f5f43dbbf1c08d17`，
  十九个 core Git source 全部指向 f823。首次 Tauri 绕过封装导致锁漂移，已恢复，不掩盖该故障。
- 蛇版 `--logical-output-only` 的最小恢复 case `index-static-primary-names` 已通过同输入
  比较；它不验证字体/像素，原字体失败保留。随后只选剩余 23 个未执行 case 的尝试在
  capabilities 响应前卡住，五秒相同状态看门狗终止；没有产生新的语义通过结果。
  已确认无活跃 Emuera client，并仅停止蛇版专用 prefix 的孤立 wineserver，等待定向恢复。
- 原版最后三个 case 的 oracle 断言元数据改为实际 warning 格式异常所致的错误；
  Rust 继续读取后续 alias 的既有行为保持，新增执行回归验证 500/210，未将差异豁免为通过。
  旧 fixture 冻结于 `batch-1-work/1A/index-fixture-before-oracle-metadata-fix`，供旧 Rust
  evidence 对应的剩余蛇版 case 使用；原版三例会以新 fixture 身份单独生成定向证据。
- 磁盘曾降至约 18–19 GiB。确认无构建使用后，仅清理本批首条测试后新建的 core/Web
  incremental session（约 4.0/8.6 GiB 逻辑大小）；逐目录来源清单位于
  `regenerable-incremental-cleanup.json`、`regenerable-web-incremental-cleanup.json`。
  未删除 binary、批次 0 证据或其他任务产物；当前可用约 24 GiB，后续大构建串行。

#### 1A 收尾结论与分项提交

- 1A 已完成其摄取/索引范围的实施、唯一审查、静态与三客户端验收；不代表批次 1 完成。
  Tauri 定向复验 1/1 通过：实际 binary 为本组 `target/web-static/debug/era-web-tauri`，
  cold/cached/reload/return-title 均验证，锁 hash 未漂移，完整快照
  `Web/.rustyera/test-runs/tauri-snapshots/2026-08-27T16-58-49.062Z-snake-ingestion.spec.mjs.jsonl`。
  构建时 core HEAD `0531090`，与发布 pin f823 的产品内容一致，差异为已验证的 cfg(test) 回归。
- 蛇版 24-case 证据已收齐：18 matched_observables、6 incomparable；后六项均为预期错误，
  原始错误/边界结果保留，Rust/C# 诊断 schema 不同，不声称错误文本等价。
  原版 24-case：9 matched、10 incomparable、5 different；三个动态用户索引既有差异，
  两个固定原版 warning 格式缺陷差异，均未修改为伪通过。原版既有行为保持的执行回归通过。
- 首次失败和定向恢复分开：原版首次完成 21 项后遇固定 warning 缺陷，最后三项仅定向恢复；
  蛇版首次字体 setup 失败，逻辑输出恢复按 1+1+22 项完成，中间一次 capabilities stall 失败
  未产生语义结果。不重跑 full 或 smoke；仅数据语义验收，不承诺 font/pixel 相同。
  逐例状态/原因、fixture/evidence 来源见本组 `batch-1-work/1A/compact-oracle-results.json`
  和 `.md`。逻辑恢复不豁免 1D 的真实 HTML/pointer/canvas 服务验证。
- Core 产品提交仍为 `bb4b04c` → `99e3621` → `8c08eb4`，fixture `f823998`；
  后续 `0531090` 为 reload generation 执行回归，`791fa7c` 为 oracle 证据/工具修正。
- TUI：pin `a4c77dc` → 摄取 `e0f743e`。Web：pin `617f65a` → 摄取 `92780ed` →
  Tauri harness 修复 `214abd9`。两前端仍完整绑定 f823，不因 core 文档/测试提交移动 pin。
- 测试无总截止；首次 full 结果和受影响定向复验见上表及 validation/*.json。磁盘约 24 GiB。
  证据和复现材料继续保留供 1D 集成；无游戏/参考实现改动，没有推送或合并主线。

#### 1B 实施入口与静态验证进展（唯一审查已结束）

- 主智能体负责公共 method operand/四 opcode、compiler 惰性 expression/method-statement
  lowering、执行分类、格式版本和 fixture/harness 集成；独立执行者分别拥有 VM/STRFORM
  与 analyzer/validator，编辑边界按 crate 隔离。此处设计分析不是重构审查。
- `MethodCallSpec` 保留 omitted/value/variable；先解析 target/kind/type/signature，再按 formal
  捕获实际值或 whole-array REF，fallback 只在不存在时求值。支持合法 i64::MIN，禁止新路径
  使用旧 sentinel。EXISTMETH 采用零实参解析，绑定调用者 generation，不执行 body。
- ISA 7→8、compiler ABI 39→40、VM ABI 14→15、VM snapshot 11→12；不改产品版本、
  runtime protocol、native ABI、C ABI，也不提前改变 profile 算术/RNG/variadic 策略。
- Validator 使用私有 opaque token/slot 栈，检查 operand、origin、连续 slot、分支合流和结果类型；
  VM 校验实际 REF 身份/rank、generation 和 snapshot/STRFORM continuation。不将动态调用送入 memo。
- 新 fixture 35 项已准备，仍未加载/编译/执行；已加入 method-statement 和双 profile VM
  error-side-effect 断言。runtime fault 后 debugger watch 不可用，不能把该缺失说成跨引擎副作用已比较。
  本批全部实质代码完成后才启动唯一 `$refactor-rustyera-code` 审查，再由 terra/low 执行测试。
  1A 的 oracle 使用固定已验证 binary 和 fixture，未因并行 1B 改动重建。
- 实现冻结基线为 core `44ffcbc` 加当前 1B 未提交改动。`review_batch_1b` 已启动本批唯一
  独立审查，使用 `$refactor-rustyera-code`，覆盖全部 analyzer/compiler/bytecode/validator/VM
  以及 runtime-tester fixture/执行断言。审查者不得测试或修改产品；全部要求落实前不启动门禁。
  详细调度计划位于本组 `batch-1-work/1B/validation-plan.md`。
- 唯一审查完整报告已返回本组 `batch-1-work/1B/review.md`；无需架构重做，有四项必需修复：
  R1 活动 REF frame 的 snapshot alias 校验；R2 validator 的先行 resolve、missing 首条 Pop
  和 FunctionLocal owner；R3 STRFORM 无副作用的运算类型预检；R4 暖 memo、持久参数深度限制、
  immutable/Character REF 以及前三项回归。审查未测试、未改产品，也不再启动或恢复。
- R1–R4 已全部落实并由主智能体核对；未启动第二次审查。R1 增加活动 Integer/String REF、
  caller-local/forwarding 与损坏 alias 恢复拒绝；R2 覆盖绕跳至后部 resolve、非法 missing
  入口和不同 owner 的 local；R3 共用无副作用运算类型检查并拒绝损坏 pending AST；R4 增加
  同 VM 暖 memo/debug 对照、真实持久 ARG 深度失败及 immutable/Character REF 拒绝断言。
  完成上述要求后统一格式化；测试尚未启动。冻结 core 输入，由 terra/low 串行执行 1B 门禁，
  使用本组 target/core-static、CARGO_INCREMENTAL=0；磁盘约 24 GiB，不下载 Chromium。
- 首次 fmt 与 workspace all-target check 通过；首次 Clippy 在 VM 报五项警告后停止，
  没有启动 focused/full/oracle。主将方法 dispatch 拆为 resolve、consumer 校验与执行阶段，
  拆出 supplied argument binding，并修正借用/分号/方法引用；仅做相关定向复验后继续尚未
  完成的门禁。原始命令、退出码与修复后结果分别保留在 `batch-1-work/1B/validation/`。


- 后续 Clippy 暴露测试代码的长函数、字符串追加、转换等警告，已逐处修复；少量保持单一
  corruption matrix/fixture 上下文的测试使用带原因的局部 `expect(too_many_lines)`，没有全局压制。
  受影响 VM 定向 lint 和 workspace Clippy 均通过。
- Analyzer 定向测试发现数字赋值 RHS 在类型重解析前仍为 FORM 文本，动态可达性会漏掉
  GETMETH。补数字 RHS 的无诊断预解析，考虑未注册局部声明与全局字符串被局部整数遮蔽；
  已知纯字符串文字不会因此保留无关函数。新增及修复后的 method 七例通过。
- Compiler expression method、bytecode operand、validator method 定向通过；VM 惰性 fallback、
  签名错误副作用、STRFORM、挂起 snapshot、活动 REF snapshot、暖 memo/debug 对照与持久 ARG
  深度限制定向通过。初次失败中还修正 fixture 的裸 RETURN 清空 RESULT、默认 ARG 被误当
  必填、TOSTR 依赖测试 Host、DATA 名称撞专用语法等问题；没有改变相关既有产品语义。
- core workspace **唯一实际全量** `core-workspace-full-run` 退出 0，约 120.2 秒。此前
  `full-core-workspace` label 与 supervisor marker 同名，创建记录失败且未启动 Cargo；保留
  `dispatch-core-workspace-label-conflict.json`，不把该调度失败算作执行成功或第二次全量。
- runtime-tester fmt/check/Clippy 通过；首次 `method_fixture` 定向失败，因为 debug watch
  对 `#DIM` 一维变量未写 `:0`，观察状态变为 blocked。已明确四个计数器的索引并保持预期值，
  仅复验工具相关门禁；工具全量/build、两参考 smoke/差分尚未执行。磁盘约 21 GiB，串行构建。


#### 1B 收尾结论与提交

- runtime-tester 四计数器改为明确 `:0` 后，fmt/check/Clippy 与 method fixture 定向复验
  通过；工具唯一全量 **41/41** 通过，observation build 退出 0。未重跑 core 全量。
- 原版、蛇版平台 smoke 均通过；原版首次 sandbox 禁止 Wine socket，未进入用例，审批后
  同一命令实际执行一次成功；蛇版八组通过。两参考源码改动：**无**。
- 两 profile 35-case Rust 观察均退出 0；原版差分为 23 matched / 12 incomparable，
  蛇版为 23 matched / 11 incomparable / 1 different。全部 oracle case 自身断言通过，
  不把 runner 退出 0 解释为全部差分相等。不可比项是 runtime fault 后 debugger watch
  与错误诊断形状限制；对应副作用已由 VM 直接执行断言覆盖，但未声称跨引擎错误状态完全相同。
- 蛇版 `method-extra-argument-policy` 差异保留：当前非 variadic 签名严格拒绝多余实参，
  蛇版参考仅执行固定形参，按既定边界批次 2 统一。没有为通过而删除 case 或改参考语义。
- 本轮显式 logical-output-only，只验证结构、逻辑输出、返回/变量及可比诊断；未验证字体
  或平台像素。STRFORM observation-only case 保持原始定义，不倒填预先通过声明。
- source fixture SHA-256：`ee38b6a112e3c10e7ec320d5da35c291b87a74090b223fa6ad26bed503f4d779`。
  frozen binary SHA-256：`e11b716adbf5e8750af11f35599dfb82516caaddd4216a8fde98767b0350bedc`。
  Rust original evidence：`36f97c2413edaf756a6527bf43b0a050196887ae799ad49a2f4df063caf81bee`；
  snake evidence：`e2267777acdf1c8b77348b8fcb251b8456e2558903898eb1fc986a4870d1b745`。
  完整清单、双 oracle 请求/响应及结果位于 `batch-1-work/1B/`，源码与二进制冻结在 `frozen/`。
- 固定语义原版 `26a35dc9334bb67590b96f7b8efbefbf199e391e`、蛇版
  `fc4fb21416768c17256d0e82f997e5f99c9bba91`；wrapper 原版
  `ffe560dad2fe480c8babddcae0122137350bf021`、蛇版
  `2c67518c594a638c2fbdef3e780341eb66ace294`。使用本组已有发布程序与 Wine prefix。
- core 分项提交：`46d8bbd` typed bytecode 基础、`9ba6fb5` S03 执行链、`14c8533`
  fixture/观察工具。TUI/Web 本子批无提交，后续绑定随 1C core 契约同步；不把旧 pin 的
  动态库/WASM 当作已验证 S03。三端组合执行仍属 1D，批次 1 整体未完成。
- 所有测试由 terra/low 执行，无批次 deadline；单命令限制、卡死观察与 full-once 保持。
  收尾可用磁盘约 22 GiB。续做保留必要证据，没有推送、合并或产品版本调整。

#### 1C 资源层实施、唯一审查及验证（进行中）

- 1A/1B 前置门禁已完成；1C core 与前端分模块实施。1B binary/fixture 与源码证据已冻结，
  未将 1C 改动混入 1B 验证。具体执行设计见本组 `batch-1-work/1C/`，不属于重构审查。
- 前端执行者拥有 TUI storage/resource 接线与测试、Browser project storage/manifest 读取与测试、
  Tauri StorageHost/ProjectHost 授权接线与测试；不修改 core 协议、版本、pin 或锁文件。
- Resource Read/ReadRange/Stat/List 仅面向当前提交清单，校验原始 hash/长度并执行读取、枚举限额；
  写入/删除在任何文件系统变更前拒绝。Data 的原版根回退策略保持，snake 的 Data→Resource
  顺序由后续 core pending storage 明确表达，前端不得自行回退。
- DT DEFAULT 的专用解析、逐项类型检查/求值/提交、默认值/XML/persistence，以及 GLOBAL
  失败不丢 VM 的事务修复正在实施。真实缺失地图资源不创建替代文件。
- Web 的 21 个 D-only 文件已逐字节保存并移出当前输入，位于 `1D/isolated-pointer-canvas/`；
  1C 验证不会夹带未审查的服务改动。Data 路径逐段 NFC/大小写身份一致性也归 1C，
  避免枚举合并认为 overlay 覆盖但文本查找命中 Resource 的矛盾；原版 profile 保持既有行为。
- 列默认值已具备独立 DEFAULT 关键字解析、column→table 求值、每项类型检查先于值求值、
  稳定列身份（删除/重建不重新绑定）、有类型默认值、XML schema/data 和 GLOBAL 扩展保存。
  structured bundle 2→3、compiler ABI 40→41、native ABI 16→17、VM ABI 15→16、VM snapshot
  12→13、runtime snapshot 19→20；ISA 8、runtime protocol 37、C ABI 和产品版本不变。
- 内部列 ticket 使用普通 String 载体，不是安全 capability。structured 身份与默认值在 snapshot
  恢复时严格校验；损坏的活动 ticket 在下一次内部操作、状态变更之前拒绝，不承诺扫描全部脚本
  字符串并在恢复瞬间识别 ticket。STRFORM 禁止调用内部 native，但仍允许同名用户方法。
- GLOBAL 失败修复保留 Session 中的 VM，先准备及验证，再提交内存/structured/host completion。
  损坏内容或 profile 不符不安装 replay，也不清已有 global structured；这是沿用 Rust 原有原子
  恢复策略，与参考“先清 global structured、捕获异常后返回 0”有意不同，不能报告为差分相同。
- `fixture-snake-data` 的 27 项同输入用例及内存存储 responder 已编写：仅 COLUMNS 开启，
  记录真实请求/回复、namespace、revision 和字节 hash。Resource 只读，snake Data 不自行回退；
  观察工具接纳原始 Resource bytes。内存 responder 不能替代三端实际存储验收。
  XML 显式 Null 的 xsi:nil 本地保真路径待双 oracle 观察，尚未登记参考 golden。
- 1C 唯一独立 `$refactor-rustyera-code` 审查已完成（本组 `1C/review.md`），次数 1、
  测试次数 0。审查冻结输入：core 78、TUI 10、Web 18 文件；不得再次启动/恢复审查者。
  七项必需整改分别为重复 lock 依赖、有界统一模式、遍历错误分类、实际 basename 校验、
  safe link 逻辑前缀、原版不新增 link 子树、memory host 原版目录来源选择。七项已落实并补齐最小回归，
  已格式化并重新冻结输入；下一步由指定测试者启动首条验证。当前可用约 22 GiB，构建串行。
- 蛇版枚举模式明确采用 NFC→Unicode lowercase、scalar `?`、`*`、字面 `[]`，空/省略
  不筛选；4096 UTF-8 字节和 1,048,576 步限额。core/Tauri/tester 共享 Rust helper，
  Python/Browser 读取相同 JSON 向量。参考源码直接调用平台 `Directory.EnumerateFiles`，
  新增 observation-only oracle case 记录大小写、non-BMP、方括号、空模式与 NFC/NFD；
  不将平台语义差异或未执行观察描述为一致。原版匹配策略保持。
- memory host 原版已有 Data 目录时只枚举该目录，删除最后文件也保留目录存在性；原版
  overlay fixture 当前 Rust 递归数量 1，蛇版 2，固定 reference expectation 2 不修改。
  这是一项原版 host 已有差异，工具不得替产品合并命名空间。无测试总 deadline；不下载 Chromium。
- 主智能体进一步落实 R3 的 Tauri 存在性竞态：私有 ResolvedReadPath 保留首次存在性，
  snake normalized lookup 选中目标后不再因后续 NotFound 降级为空列表；两 profile 从 lookup
  到 walker 消失均 Conflict，原版初始缺失仍可项目回退。新增三个精确回归；不追加审查。
- 格式化为源码编辑，不记作测试通过；core/tool 全部改动 Rust（含 include 文件）及 Web
  改动 Rust/TS/JS/MJS/JSON 已格式化。测试前输入见 `1C/post-review-inputs.json`；
  该清单记录首测前状态，以下另行记录实际验证及修复。实际产品版本 core/TUI 0.8.0、Web package 0.9.0，均未修改。


#### 1C 首轮静态验证及修复记录（持续更新）

- 已启动验证，唯一审查次数仍为 1，禁止再启动或恢复审查者。所有命令由指定 terra/low
  测试执行者运行；原始冻结输入、各次修复文件 hash 和压缩日志位于 `batch-1-work/1C/`。
  测试没有批次总时限，单命令超时、进程清理、磁盘阈值及全量一次规则仍生效。
- 监督器首次完整 unittest 为 9 通过、1 个 sandbox `ps` 权限错误；获准后只对孤儿进程
  清理用例定向复验，1 通过，不再运行完整监督器套件。
- core workspace format/check/Clippy 已通过。此前 check 的 validator 输入转换与测试模块
  import、Clippy 的 6 个等价写法/文档问题均已修正，并定向恢复相关静态门禁；原始失败
  与修复结果分开留存，不冒充首次全通过。
- 最小回归已通过：模式 1、parser 2、analyzer 1、compiler 1、VM structured 单元 24、
  执行集成 15、私有 STRFORM 1、列身份 candidate/事务/热重载 3、isolated candidate 1、
  runtime resource 10。完整 workspace 尚未启动时，GLOBAL 最小集出现 2 通过、1 失败。
- GLOBAL 失败根因是新 fixture 未启用二进制存档，却期待保存结构化 VAREXT；固定原版
  与蛇版 `VariableEvaluator.SaveGlobal` 均仅在 binary 分支输出这类扩展。现测试显式覆盖
  两种 profile × text/binary：binary 恢复 row/default=12，text 保持清数据/留现有 schema
  default=99 的既有行为；不修改产品格式来迎合错误预期。oracle/client fixture 原已启用
  binary，无需更改。修复清单 `repair-global-fixture-format.json`，定向复验进行中。
- Python/Browser 共用 JSON 向量各保存一份相同内容到组件 `tests/fixtures/`，避免独立 CI
  checkout 依赖 sibling core；源与复制摘要见 `shared-vectors-provenance.json`。前端尚未首测。
- 此时 core 全量、runtime-tester、前端静态及全部动态门禁尚未完成，1C 不能标为完成；
  可用磁盘约 22 GiB，单构建，无 Chromium 下载。


- core 首次且唯一 workspace 全量退出 0（90.38 秒），其后发现的 XML fixture/重载修复只做
  定向验证，未再次运行全量。runtime-tester 首次且唯一全量 51 通过（6.29 秒）；其格式、
  check/Clippy、data fixture 与 8 项 storage 最小回归均通过。
- tool 最小回归先发现 named XML 应调用 `XML_GET_BYNAME`，并发现 `XML_DOCUMENT` 对已存在
  名称不替换；fixture 改为 `XML_REPLACE` 且断言实际发生修改，避免恢复假阳性。进一步暴露
  analyzer 将 `XML_REPLACE(key, xml)` 两参数重载误判为 inline 写回；现表达式/METHOD
  共用按 arity 选择的约束，两参数 key 为值，三参数以上保持 inline mutable 规则。
  新 VM fixture 的未加引号字符串也已修正。每次原始失败保留，修复记录分别见
  `repair-xml-fixture.json`、`repair-xml-replace-overload.json`、`repair-xml-key-literal.json`。
- 修复后 workspace check/Clippy、analyzer 重载/列选项、VM 重载执行及 structured 16、
  runtime GLOBAL 3、tool 最小及唯一全量均通过。core 产品/fixture 已按功能提交：

  | 提交 | 范围 |
  |---|---|
  | `34d1d08cb19c0df6059a9c29037cd230d4921218` | Unicode 枚举规则、公共限额与共享依赖 |
  | `9928ba46bd15c73e8cc18e4685469b06ac471950` | DEFAULT、稳定列身份、XML/snapshot 默认值 |
  | `e41b6858ddadca7f0d5271ebdfdfdd6130ba1a24` | snake Data→Resource 与有界 cache decode |
  | `35ae660983ff04c393968317578f1a71124eeb15` | GLOBAL 拒绝时保留 VM/replay |
  | `b1481da7842a6a7e997bd517a81a4127d6406264` | XML_REPLACE 存储名称重载 |
  | `0538206ce74e7a1e97c634f2ce86c178932bd54c` | 初始化/资源/GLOBAL fixture 与观察工具 |

- TUI/Web 已机械绑定完整 `0538206ce74e7a1e97c634f2ce86c178932bd54c`，Web Git rev/lock
  及新增依赖边同步，TUI 展示短 SHA 同步。pin 更新尚未替代前端验证；下一步重建本组
  tester/C ABI/WASM/Tauri 并完成前端静态，全部通过后才授权动态/oracle。提交未修改已验证
  源码字节，不推送或合并；1C 尚未完成，根 CHANGELOG_PENDING 尚未追加本子批功能。

- post-commit tester/C ABI 已在本组重建。冻结 tester SHA-256 为
  `245a339e819258b553342f8b93505f0f8383d37099fe19ed3c8379de392413fa`，TUI C ABI 为
  `1583b8344aacb2f0c5a6cc9664b6e2d896910186d90f4db6987d28a40ffe5c72`。
  TUI 最小 7、资源相关 139、Ruff 通过；首次且唯一完整 pytest 475 通过（43.86 秒）。
  PyInstaller 首次遇到用户级 bincache 的 sandbox 删除权限错误，批准限定重试后构建与
  `--help` 通过；记录为构建权限重试，不是第二次全量。打包 C ABI 动态加载仍待验收。
- Web 首次最小 Vitest 94 通过、4 失败。修复资源读取以活动 manifest 授权并保留文件名
  大小写，补足内嵌资源读取；随后发现释放 payload 时丢失长度，改保留 external 长度。
  测试同时修正 jsdom 字节 realm、Vite fixture URL 与缺 Data 目录的空枚举断言。最终最小
  98、目录 32 通过。两次 iterator lint/类型故障均保留日志；改为复用 fixture 的真实
  AsyncGenerator，仅 mock next() 失败后，定向 2、typecheck、lint 恢复通过。
- Web 客户端测试工具 86、format、core revision 检查通过；发布 lock 的完整 SHA 和
  unicode-normalization 依赖边已核对。首次且唯一完整 Vitest 92 files / 1058 tests 通过，
  证据 `1C/validation/runs/web-vitest-first`。Web Rust 静态与构建尚在执行，所有动态及
  oracle 未授权。磁盘约 21 GiB，单构建，未下载 Chromium。
- Web Rust check 通过后，首次 Clippy 报 9 项代码/测试规范问题；已拆出资源枚举与原版链接
  检查，显式导入、checked chunk 索引、64 KiB heap 缓冲区及八进制权限写法，并恢复相关
  check/Clippy。storage 最小 33 通过、1 个旧错误分类断言失败；逃逸路径仍被拒绝，现断言
  统一的 PermissionDenied，失败单例及相关静态复验通过。project 最小 40 通过，首次且
  唯一 Web Rust workspace 全量 130 通过（112.61 秒），1 个既有跨前端 cache handoff 驱动
  保留 ignored，需其专用场景调用。本轮没有重跑已通过的完整 Vitest。
- Web build 与 WASM 重建通过；WASM 更新后再次定向 build，确保 dist 使用本次产物。
  Tauri webdriver 构建亦通过；全部静态门禁通过后，已授权真实客户端及双 oracle 验证，
  当前输入冻结，动态结果待登记。磁盘约 17.39 GiB；跌破 20 GiB 后已获准清理仅本组
  停用的 `target/web-static/debug/incremental`（703,855,752 原始字节），不删二进制或证据；
  计划/结果见 `cleanup-web-incremental-{plan,result}.json`。继续 `CARGO_INCREMENTAL=0`
  及单构建，低于 10 GiB 停止新增高写入任务。

#### 1C 动态验证及定向修复（进行中）

- 真实 TUI RuntimeWorker/C ABI 和打包 C ABI 场景均通过，七个阶段标记及 watches 符合。
- Chromium 首次仅 Resource 标记失败（`0/0/0`），无 runtime fault，其余阶段通过。
  原因为测试 remote filesystem 给不存在的 Data 目录返回虚假句柄，后续安全遍历将
  ENOENT 正确归为 conflict，故未回退。已修驱动先验证目录存在/类型；最小单例及
  webTestLib 60 通过，新增 fixture 的 DOMException lint 问题修正后单例/lint/format 通过。
  生产代码和 WASM 无变化；Chromium 定向复验全部通过。原生 Firefox 随后通过同场景。
- Safari 首次停在已有值 `1` 的真实 prompt，提交点击未推进；相同快照看门狗停止会话后
  才出现 invalid session / ECONNREFUSED，后者不是首因。Safari helper 改走聚焦 prompt
  的原生 WebDriver Enter，三路径单例/lint/format 通过；Safari 定向复验通过，随后真实
  Tauri 同场景通过。两者均验证全部七个阶段标记，未以启动成功代替数据断面。
  所有首次失败和监督证据保留，不重跑全量、不放宽看门狗。
- 证据入口为 `1C/validation/runs/` 及 Web `.rustyera/test-runs/`，修复记录为
  `repair-chromium-remote-directory.json`、`repair-safari-visible-input.json`。没有下载浏览器。
- 原版首次完整 smoke 的脚本退出 0，58 条响应全部写完且通过脚本断言，但 Wine 后台
  进程持有合并输出管道，外层监督器以 `stdoutClosed:false` 退出 1。保留首次全量失败，
  不重跑 smoke；响应 SHA-256 为 `cd3f8e3e6624c6f0f56a510ea0408810d1ee63f79a0c5f0d19996ed139262fd5`。
  本组 `1C/run_wine_command.py` 将后台输出隔离至自有文件、转发实际命令输出、按五秒
  观察进展并等待专属 wineserver 退出；单条 capabilities 和 stdout 边界定向恢复均通过。
  该恢复只证明连接与监督收尾可用，不改写首次完整 smoke 结果。
- 原版 Rust 列观察首次运行通过，27 个 case 的 JSON SHA-256 为
  `7bbcfb2ac5f1666d4aec7aaf1c5f90d41e264b219172ecd07f8e0f5c6bd70eaa`。
  原版列 Oracle 首次在第四项 `column-empty-string-and-explicit-null` 失败；前三项
  DEFAULT/Int64 最小值/饱和转换已逐项匹配。原因是 fixture 的末尾省略值未被参考
  parser 保留，`DT_ROW_ADD(table, column, )` 成为两参数并被拒绝；不是 DEFAULT handler
  返回错误。原始 `1C/diff-columns-original/evidence.json` 保留。当前仅继续尚未执行的
  精确 case；两项同类 Null fixture 待修正并定向复验，不再次启动完整列 Oracle。
- 后续原版 22 项未执行集合首次调度误加不存在的 `--repeat`，退出 2、未执行 case；
  去掉旗标后原版继续完成 16 项（10 项可比观测匹配、6 项错误诊断 schema 不可比），
  在 `column-global-missing` 失败。根因是 driver 多 case 复用目录，前一 GLOBAL 往返
  留下文件；现在每 case 从 pristine template 复制独立目录，记录起始 hash，只有同 case
  内请求共享文件。新增隔离单例通过，Python driver 本子批首次完整 21 项通过；工具
  fmt/check/Clippy 及 defaults 最小回归通过，没有再次运行 core/tool Rust 全量。
- Null fixture 已改为内部省略值加后续 String pair，保留旧输入于
  `1C/frozen/fixture-snake-data-before-null-repair/`。首次定向 Oracle 证明 Null watches
  `0/1/0` 后，又在 fixture 误写的 `STRLEN(...)` 失败（参考函数为 `STRLENS`）；已仅改
  正该函数名，继续最小复验。定向矩阵为 `targeted-oracle-matrix-2.json`：新 fixture 两项
  重新观察，原版存储失败/未执行六项沿用旧冻结 fixture 与原 Rust 证据；不改写旧 hash。

#### 1C 最终定向观察与初始化资源结论

- `STRLENS` 修正后两项 Null 定向执行完成：空值用例匹配；XML 显式 Null 往返前后
  两端均为 `1/1`，差异仅为原始 XML 的 namespace/schema/ID 序列化拼写，保留为
  `different`，不把字符串差异抹除。原版存储六项恢复期间又暴露 XML 扩展未获 fixture
  配置允许；已显式增加 `Valid extensions for LOADTEXT and SAVETEXT:txt,xml`，随后
  resource-map/XML 定向匹配。该配置同样用于 BBAS 副本，不改参考实现。
- 蛇版首次完整 smoke 通过；Rust 列观察首次运行收集 27 项，蛇版列 Oracle 首次在第
  23 项 resource-read 失败，已完成的前 22 项为 15 matched、6 incomparable、1 different。
  蛇版路径解析返回相对路径，CLI 的 load 不切换进程目录。driver 现为每项独立启动进程，
  CWD 指向该项游戏副本，stderr 与 capabilities 分项保存，累计请求不覆盖前项记录。
  最小 Python 三项通过；旧失败证据和全量启动记录不变。
- 首次 CWD 定向恢复无最终证据：短 case 均未触及内层五秒周期，外层未收到输出而按
  看门狗失败。现在每项完成后立即输出该项完整实际 snapshot，周期规则不变；不以
  case 目录已创建或 Wine 进程正常推定执行成功。
- 观察工具还漏交了 fixture 中五个 `patterns/` 资源。此前全零结果撤回，不能据此
  声称匹配或引擎差异。已补充显式资源摄取、目录/符号链接校验及现有 manifest 回归。
  tool fmt/check/Clippy、两项 data fixture 最小测试、构建通过；未重跑 Rust 全量。
  新冻结 tester SHA-256 为
  `fcb7ca6246d4e2a4057bd556e4d0acbf57312d1bd0b4b360079df138c182871b`；
  旧 `245a339e…` 二进制及其证据继续保留，新产物只包含观察工具修复，产品 core 仍为
  `0538206ce74e7a1e97c634f2ce86c178932bd54c`。
- 最终资源定向恢复：蛇版五项中四项 matched、一项 pattern different；原版 pattern
  一项 different。实际 Rust watches 为 `[5,3,1,5,5,1,1]`，两参考为
  `[5,2,1,5,5,1,0]`，对应已声明的有界 Unicode scalar/NFC 模式与平台匹配差异。
  原版 memory responder 的模式结果不代表改写了原版产品文件匹配策略。
  命令、各次输入与 hash 见 `targeted-oracle-resource-input-matrix.json` 和
  `repair-oracle-{xml-config,process-cwd,resource-inputs}.json`。
- BBAS 的 Rust/original、Oracle/original、Rust/snake、Oracle/snake 四项首次运行均
  退出 0、输出正常关闭；两套同输入比较均为一项 `matched_observables`。实际值为
  `RESULT:10..13 = 1/161/4531748/1`，`RESULTS:10..11 = 靈夢/真面目`。
  输入 schema.xml 1,532 字节，SHA-256
  `83b7abf02eda889d85f6d094d26b2069c5483ad0668173de8941aef14ae279ce`；
  bbas_dataset.xml 36,420 字节，SHA-256
  `d17b4ec540698f707e37c4a9f0b2b4b0093ff5b664196ae331f254a62abe4054`。
  原始游戏和参考源码未变。仍缺 `bbas_map_schema.xml`、`bbas_map.xml`，后续初始化
  被资源与 SQL/后置语义阻塞；本结果不代表真实标题或 GRAPH_DB_INIT 已执行。
- 三端实际组合已覆盖 ALS/ERD→GETMETH→资源 overlay→MAP/XML/DT→GLOBAL；图形服务
  等待 1D。本子批完整套件未重跑，全部修复仅定向复验；命令日志、首测/首次全量 claim、
  gzip/原始摘要及失败现场均留在本组 `1C/`。可用磁盘约 16.7 GiB，继续单构建与
  `CARGO_INCREMENTAL=0`，没有下载 Chromium。
- 按每项最新有效执行聚合 27 项列用例：原版为 18 matched、6 incomparable、3 different；
  蛇版为 19 matched、6 incomparable、2 different。六项错误保留原始异常/诊断，尚无
  跨引擎统一诊断 schema，不能以两端都失败声称错误等价。差异为 XML 字符串序列化、
  平台文件模式；原版额外保留原有 Data 目录覆盖资源目录的枚举行为。完整首次与定向
  结果分别记录，没有重新运行全量，也未将原版 smoke 监督器首败改成通过。
- 1C 后续测试修复提交为 core `ca34ce2b7f4e0a402face5b29e4515f5342f64a7`
  （fixture/观察输入）、`28de387976c1cd4caf3fd732d3f9fface789a76e`（Oracle 隔离/监督）。
  产品 crate 相对前端 pin `0538206ce74e7a1e97c634f2ce86c178932bd54c` 未变。

  | 组件 | 共用模式 | 资源接线 | 执行 fixture/驱动 | 完整 core 绑定 |
  |---|---|---|---|---|
  | TUI | `0131f7434dd0a5eeb503022db3c906ddf7bf54d4` | `fb6eff3e22fc1cd766390fcba28d034fc9ec5f26` | `cb58571e886288265a6965afe566dba91b3f950e` | `487b49a123d25f07f99978d556ea36386603611f` |
  | Web | `543aba3064303929ced18d4dbf9133edced478ff` | `68a8318aa7977d25a46417ead70ac2e1ea03edbe` | `0b2cd2630e70dc4bde13c486b183706be1ead77b` | `37dc8e14b6a089866b782daf45c4586a3d4189df` |

- 根 `CHANGELOG_PENDING.md` 仅追加 DEFAULT、安全资源、GLOBAL 拒绝原子性及
  XML_REPLACE 重载四项实际行为；测试工具、文档与流程不列为功能。1C 完成不等于
  批次 1 完成：1D 的真实 HTML/pointer/canvas 服务、覆盖报告及最终组合验收仍待执行。

#### 1D 服务层实施入口（唯一审查中，未测试）

- Web 独立执行者拥有 runtime service 生命周期、pointer 规范按钮值观察及独立 canvas
  replay sampling；不修改正在验证的 core，也不修改 1C 的 browserProject/resource/storage 文件。
- pointer/canvas 保持 v1 payload，使用实际 viewport、三 projection revision、session epoch
  和 request ID；MOUSEY 按固定参考的 clientY-clientHeight 映射。HTML v2 留待 core 契约与
  规范树测量实现，不提前宣告能力；Rust bridge、共享版本、pin/锁文件由主串行整合。
- 当前无测试/构建/格式化/审查/提交；1D 统一集成仍等待 1A–1C 门禁及本批唯一审查。
- 隔离实现已包含保留源码切分、完整参考 length layout 计划、编译器惰性 HTML 参数路径、
  runtime v2 多轮续接/flow/snapshot 身份约束；Web CBOR/provider 接线、TUI 明确拒绝
  的五项 C ABI 断面及覆盖报告 schema 3 已交付隔离源码。共享格式草案为 compiler ABI 42、host ABI 13、runtime snapshot 21，其余
  ABI/产品版本不变；尚未改活动仓。真实服务及组合 fixture 源已准备，未运行，不作为
  参考像素或运行成功证据。各隔离目录包含基线/输出 hash，集成时逐项合并而非覆盖 1C。
- 全部实质源码已机械整合为 `1D/integration-source/` 的 180 个变化文件（core 109、
  Web 59、TUI 12），活动源 hash 与隔离输出逐项登记。包含真实服务捕获/typed watch/
  CBOR 证据工具、局部 int32 HTML 长度单位换算修正及 17-case Oracle fixture；捕获历史
  只作为证据保留，不允许消息计数增长掩盖相同 DOM/runtime 的卡死。
- 当前在隔离源码上启动本子批唯一独立 `$refactor-rustyera-code` 审查，次数 1，入口
  `1D/review-start.json`。未运行任何 D 测试/构建；待落实全部要求、1C 收尾及活动仓
  整合后，才能启动 D 的首条静态命令。该审查不是 1C 的二次审查。


### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 审查与验收结果

- 实现/测试输入 revision、工作目录、环境、游戏/资源 hash、profile/seed：待填写。
- 重构审查是否触发、唯一审查记录、结论及要求落实情况：待填写；未触发须说明。
- 首条测试命令时间、已用/剩余墙钟预算、首次全量启动记录：待填写。
- Oracle 选择与理由、语义基准和 wrapper revision：待填写；按范围分别记录原版与蛇版，或说明不适用。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 交付与续做入口

- 本批结论、已完成与未验证范围、是否满足整批验收：待填写。
- 各组件提交、分项对应关系、发布/迁移注意事项、CHANGELOG_PENDING 更新情况：待填写。
- 当前轮次/起止时间、最近观察状态或指标、材料与复现命令、下一步恢复入口：待填写。
- 临时材料保留/清理、相关进程停止与资源释放情况：待填写。

<a id="batch-2"></a>

## 批次 2：确定性 API、输入与兼容差异骨架

计划入口：[改造思路 / 批次 2](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-2)。状态：待登记；负责人 / 最近更新：待填写。

### 具体实施方案

- 目标、S/D/C/N 编号、范围与明确不做项：待填写。
- 前置批次/子项、已通过门禁和对应证据：待填写；区分可并行实现与必须汇合的集成验收。
- 受影响仓库/模块、接口与数据格式、profile/cache/save/service 版本变化：待填写。
- 分项步骤、文件/hunk 归属、共享基础依赖、资源隔离与提交划分：待填写。
- 验收目标、最小 fixture、获准测试范围、风险/回退方案与用户时限：待填写。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 审查与验收结果

- 实现/测试输入 revision、工作目录、环境、游戏/资源 hash、profile/seed：待填写。
- 重构审查是否触发、唯一审查记录、结论及要求落实情况：待填写；未触发须说明。
- 首条测试命令时间、已用/剩余墙钟预算、首次全量启动记录：待填写。
- Oracle 选择与理由、语义基准和 wrapper revision：待填写；按范围分别记录原版与蛇版，或说明不适用。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 交付与续做入口

- 本批结论、已完成与未验证范围、是否满足整批验收：待填写。
- 各组件提交、分项对应关系、发布/迁移注意事项、CHANGELOG_PENDING 更新情况：待填写。
- 当前轮次/起止时间、最近观察状态或指标、材料与复现命令、下一步恢复入口：待填写。
- 临时材料保留/清理、相关进程停止与资源释放情况：待填写。

<a id="batch-3"></a>

## 批次 3：安全 SQL（蛇版 TW P0）

计划入口：[改造思路 / 批次 3](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-3)。状态：待登记；负责人 / 最近更新：待填写。

### 具体实施方案

- 目标、S/D/C/N 编号、范围与明确不做项：待填写。
- 前置批次/子项、已通过门禁和对应证据：待填写；区分可并行实现与必须汇合的集成验收。
- 受影响仓库/模块、接口与数据格式、profile/cache/save/service 版本变化：待填写。
- 分项步骤、文件/hunk 归属、共享基础依赖、资源隔离与提交划分：待填写。
- 验收目标、最小 fixture、获准测试范围、风险/回退方案与用户时限：待填写。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 审查与验收结果

- 实现/测试输入 revision、工作目录、环境、游戏/资源 hash、profile/seed：待填写。
- 重构审查是否触发、唯一审查记录、结论及要求落实情况：待填写；未触发须说明。
- 首条测试命令时间、已用/剩余墙钟预算、首次全量启动记录：待填写。
- Oracle 选择与理由、语义基准和 wrapper revision：待填写；按范围分别记录原版与蛇版，或说明不适用。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 交付与续做入口

- 本批结论、已完成与未验证范围、是否满足整批验收：待填写。
- 各组件提交、分项对应关系、发布/迁移注意事项、CHANGELOG_PENDING 更新情况：待填写。
- 当前轮次/起止时间、最近观察状态或指标、材料与复现命令、下一步恢复入口：待填写。
- 临时材料保留/清理、相关进程停止与资源释放情况：待填写。

<a id="batch-4"></a>

## 批次 4：主玩法 presentation、图像、scene 与自身存档闭环

计划入口：[改造思路 / 批次 4](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-4)。状态：待登记；负责人 / 最近更新：待填写。

### 具体实施方案

- 目标、S/D/C/N 编号、范围与明确不做项：待填写。
- 前置批次/子项、已通过门禁和对应证据：待填写；区分可并行实现与必须汇合的集成验收。
- 受影响仓库/模块、接口与数据格式、profile/cache/save/service 版本变化：待填写。
- 分项步骤、文件/hunk 归属、共享基础依赖、资源隔离与提交划分：待填写。
- 验收目标、最小 fixture、获准测试范围、风险/回退方案与用户时限：待填写。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 审查与验收结果

- 实现/测试输入 revision、工作目录、环境、游戏/资源 hash、profile/seed：待填写。
- 重构审查是否触发、唯一审查记录、结论及要求落实情况：待填写；未触发须说明。
- 首条测试命令时间、已用/剩余墙钟预算、首次全量启动记录：待填写。
- Oracle 选择与理由、语义基准和 wrapper revision：待填写；按范围分别记录原版与蛇版，或说明不适用。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 交付与续做入口

- 本批结论、已完成与未验证范围、是否满足整批验收：待填写。
- 各组件提交、分项对应关系、发布/迁移注意事项、CHANGELOG_PENDING 更新情况：待填写。
- 当前轮次/起止时间、最近观察状态或指标、材料与复现命令、下一步恢复入口：待填写。
- 临时材料保留/清理、相关进程停止与资源释放情况：待填写。

<a id="batch-5"></a>

## 批次 5：蛇版存档互操作与音频

计划入口：[改造思路 / 批次 5](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-5)。状态：待登记；负责人 / 最近更新：待填写。

### 具体实施方案

- 目标、S/D/C/N 编号、范围与明确不做项：待填写。
- 前置批次/子项、已通过门禁和对应证据：待填写；区分可并行实现与必须汇合的集成验收。
- 受影响仓库/模块、接口与数据格式、profile/cache/save/service 版本变化：待填写。
- 分项步骤、文件/hunk 归属、共享基础依赖、资源隔离与提交划分：待填写。
- 验收目标、最小 fixture、获准测试范围、风险/回退方案与用户时限：待填写。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 审查与验收结果

- 实现/测试输入 revision、工作目录、环境、游戏/资源 hash、profile/seed：待填写。
- 重构审查是否触发、唯一审查记录、结论及要求落实情况：待填写；未触发须说明。
- 首条测试命令时间、已用/剩余墙钟预算、首次全量启动记录：待填写。
- Oracle 选择与理由、语义基准和 wrapper revision：待填写；按范围分别记录原版与蛇版，或说明不适用。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 交付与续做入口

- 本批结论、已完成与未验证范围、是否满足整批验收：待填写。
- 各组件提交、分项对应关系、发布/迁移注意事项、CHANGELOG_PENDING 更新情况：待填写。
- 当前轮次/起止时间、最近观察状态或指标、材料与复现命令、下一步恢复入口：待填写。
- 临时材料保留/清理、相关进程停止与资源释放情况：待填写。

<a id="batch-6"></a>

## 批次 6：完整蛇版语言

计划入口：[改造思路 / 批次 6](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-6)。状态：待登记；负责人 / 最近更新：待填写。

### 具体实施方案

- 目标、S/D/C/N 编号、范围与明确不做项：待填写。
- 前置批次/子项、已通过门禁和对应证据：待填写；区分可并行实现与必须汇合的集成验收。
- 受影响仓库/模块、接口与数据格式、profile/cache/save/service 版本变化：待填写。
- 分项步骤、文件/hunk 归属、共享基础依赖、资源隔离与提交划分：待填写。
- 验收目标、最小 fixture、获准测试范围、风险/回退方案与用户时限：待填写。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 审查与验收结果

- 实现/测试输入 revision、工作目录、环境、游戏/资源 hash、profile/seed：待填写。
- 重构审查是否触发、唯一审查记录、结论及要求落实情况：待填写；未触发须说明。
- 首条测试命令时间、已用/剩余墙钟预算、首次全量启动记录：待填写。
- Oracle 选择与理由、语义基准和 wrapper revision：待填写；按范围分别记录原版与蛇版，或说明不适用。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 交付与续做入口

- 本批结论、已完成与未验证范围、是否满足整批验收：待填写。
- 各组件提交、分项对应关系、发布/迁移注意事项、CHANGELOG_PENDING 更新情况：待填写。
- 当前轮次/起止时间、最近观察状态或指标、材料与复现命令、下一步恢复入口：待填写。
- 临时材料保留/清理、相关进程停止与资源释放情况：待填写。

<a id="batch-7"></a>

## 批次 7：可选 extension 与渲染能力

计划入口：[改造思路 / 批次 7](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-7)。状态：待登记；负责人 / 最近更新：待填写。

### 具体实施方案

- 目标、S/D/C/N 编号、范围与明确不做项：待填写。
- 前置批次/子项、已通过门禁和对应证据：待填写；区分可并行实现与必须汇合的集成验收。
- 受影响仓库/模块、接口与数据格式、profile/cache/save/service 版本变化：待填写。
- 分项步骤、文件/hunk 归属、共享基础依赖、资源隔离与提交划分：待填写。
- 验收目标、最小 fixture、获准测试范围、风险/回退方案与用户时限：待填写。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 审查与验收结果

- 实现/测试输入 revision、工作目录、环境、游戏/资源 hash、profile/seed：待填写。
- 重构审查是否触发、唯一审查记录、结论及要求落实情况：待填写；未触发须说明。
- 首条测试命令时间、已用/剩余墙钟预算、首次全量启动记录：待填写。
- Oracle 选择与理由、语义基准和 wrapper revision：待填写；按范围分别记录原版与蛇版，或说明不适用。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 待填写 | 待填写 | 待填写 | 待填写 | 待填写 |

### 交付与续做入口

- 本批结论、已完成与未验证范围、是否满足整批验收：待填写。
- 各组件提交、分项对应关系、发布/迁移注意事项、CHANGELOG_PENDING 更新情况：待填写。
- 当前轮次/起止时间、最近观察状态或指标、材料与复现命令、下一步恢复入口：待填写。
- 临时材料保留/清理、相关进程停止与资源释放情况：待填写。
