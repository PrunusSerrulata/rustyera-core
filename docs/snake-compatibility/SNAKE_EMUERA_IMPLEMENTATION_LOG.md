# 蛇版 Emuera 适配：分批次实施与验收记录

> 文档状态：批次 0、批次 1 已完成；批次 2 产品实施与功能行为验收完成，保留一项规模采集基础设施缺口；后续批次仍待登记。各批明确差异继续保留，已完成批次均不代表完整蛇版语义或蛇版 TW 可玩性。
>
> 2026-08-30 按用户明确要求完成全历史本地材料清理：已删除可再生构建产物、运行 payload、
> 原始日志、DOM/runtime 快照和缓存；保留测试脚本、fixture/config、工具、环境、review、索引及
> 精简 summary。下文历史证据路径仍用于说明当时的验收绑定，但被清理的原始 payload 不再承诺
> 本机可用，不能据此重新宣称动态测试仍可 cache-only 复跑。

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
| [1](#batch-1) | 完整摄取与参考能力阻塞项 | 已完成 | 2026-08-28 / Codex | 1A–1D分项提交及必要验收完成；参考/像素差异与后置资源阻塞见验收汇总，不代表蛇版TW完整可玩 |
| [2](#batch-2) | 确定性 API、输入与兼容差异骨架 | 功能验收完成；规模证据有缺口 | 2026-08-30 / Codex | 2A–2F 产品与三端行为已交付；峰值 RSS 因沙箱权限未取得，后续性能批次补采 |
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

计划入口：[改造思路 / 批次 1](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-1)。状态：已完成（保留明确差异）；负责人 / 最近更新：Codex / 2026-08-28。

### 具体实施方案

- 范围按[详细实施方案](BATCH_1_IMPLEMENTATION_PLAN.md)划为四个独立子批次：1A 完整摄取与 ERD/ALS（S01/S02），1B 动态方法（S03），1C 列选项、GLOBAL 与安全读取（S12），1D 已有服务接线与统一集成（S04）。四个子批次均已完成约定范围。
- 前置为已完成的批次 0 profile、identity 与诊断契约；1C 依赖 1A，1D 集成汇合 1A–1C。core 持有语言、数据与服务语义，TUI/Web 分别实现摄取和存储；Browser/Tauri 提供真实图形服务，TUI 明确诊断未实现能力。
- 三个专用 worktree 分别提交，共享协议、版本和发布绑定统一整合。游戏与参考仓库只读；测试数据、存储、Wine、会话和构建产物在本组隔离，Chromium 使用已有可执行文件。
- 验收范围为最小执行 fixture、三端组合、四个图形客户端服务及蛇版 TW 完整摄取/静态覆盖。不包含 SQL、蛇版算术/RNG、Float、variadic、元素 REF、OUT、新 HTML 标签、scene、外部蛇版存档或真实游戏完整可玩性。
- 用户取消所有子批次测试总时限；各子批次唯一重构审查、静态先于动态、单命令限制与看门狗仍生效。TW 全量重跑有单独授权；加载阶段连续四次 5 秒完整采样不变才退出，其他 core 阶段为连续两次，Browser/Tauri 仍为相邻完整 DOM/runtime 快照相同即失败。磁盘按 20/10 GiB 阈值管理，最多两个并行构建。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| S01 公共摄取与三端接线 | core `era-runtime-protocol`、runtime project/cache、tester `project_inputs`；TUI/Web 项目扫描、manifest、Worker | 增加 ALS/ERD 完整/快速扫描、延迟读取、缓存 hydration 和增量重载；严格 UTF-8，读取错误不静默跳过，分别记录原始字节和 UTF-8 payload hash | `Als=6`、`Erd=7`；ERD 经数据加载器而非 ERB analyzer；XML/TXT/数据库 seed 纳入只读资源清单，DLL 不成为扩展 | core 公共基础 `bb4b04c`、摄取 `8c08eb4`；TUI `e0f743e`；Web `92780ed`；依赖 S02 |
| S02 ERD/ALS | core `erabasic-csv`、analyzer、data、compatibility | 初次加载与 deferred resolution 共用 policy；snake 按 ERD→CSV→ALS 稳定路径排序；关联同目录同 stem，尊重 UseERD；trim/空名/重复 alias、主表优先及显式反向名称映射 | signed alias 可保存负数或超维度值，实际访问仍检查边界；CHARADATA 角色维不计入 ERD；原版既有行为不改 | core `99e3621`；执行 fixture `f823998`，reload 回归 `0531090` |
| S03 动态方法 | core analyzer/compiler/bytecode/validator/VM/STRFORM | GETMETH/GETMETHS 先解析目标、类型及签名；仅目标不存在时求 fallback；显式 omitted/value/variable slot，支持 Integer/String 返回和数组 REF；EXISTMETH 按零实参规则解析且不执行方法体 | 合法 i64::MIN 不作省略值；验证 token/栈/REF/返回类型，绑定 program generation；包含格式化字符串动态可达性，不误入 memo | core typed 基础 `46d8bbd`、执行 `9ba6fb5`、fixture `14c8533` |
| S12 列 DEFAULT | core parser/analyzer/compiler/VM structured 数据 | 专用 DEFAULT 语法、逐项类型检查与求值、稳定列身份、类型化默认值；新增行使用默认值，修改默认值不回填；XML/schema/snapshot/GLOBAL 保留默认值 | 缺表、缺列、非法选项及类型错误明确失败；损坏 structured 身份拒绝，不返回伪成功 | core `9928ba4` |
| 安全资源读取与枚举 | core storage/compatibility；TUI storage；Browser/Tauri storage/project host | snake 字符串读取先 Data，仅 NotFound 回退清单授权 Resource；EXISTFILE/递归 ENUMFILES 共用命名空间，Data 覆盖同名资源；归一化、链接逃逸/循环、冲突、hash/长度与读取限额检查 | SAVETEXT 只写 Data，整数文本编号仍用 Save，Resource 禁写删；NFC/Unicode scalar 有界匹配与平台模式差异显式保留，原版策略不变 | core 规则 `34d1d08`、读取 `e41b685`；TUI `0131f74`、`fb6eff3`；Web `543aba3`、`68a8318`；依赖 1A |
| GLOBAL 与 XML_REPLACE | core runtime GLOBAL、analyzer XML 重载 | GLOBAL 验证后原子提交，损坏或 profile 不符时保留 VM、replay 和现有 structured 数据；修复 XML_REPLACE 两参数存储名称重载 | binary 恢复 structured 数据/default；text 保留既有行为；拒绝时原子性与参考异常路径有意不同 | core GLOBAL `35ae660`、XML `b1481da`；数据 fixture `0538206` |
| S04 HTML 服务 | core `erabasic-html`、compiler、runtime HTML query；Web DOM projection | 以 core 规范树和源映射测量，core 保留原字符串切分、实体、标签闭合及 RESULTS；支持首显示行、半角/像素单位、空串、多行和 Unicode 安全边界 | 三个 HTML operation 升至 v2；无法推进明确 NoProgress；不新增标签、不承诺跨平台像素一致 | core 契约 `e3e8e42`、执行 `935e746`；Web `ec25261`、`70f03a7`、`83e187e` |
| S04 pointer | core service 协商；Web viewport/pointer | 实际 viewport 逻辑坐标、MOUSEY 客户区底边映射、MOUSEB 悬停按钮脚本值；覆盖 resize、滚动、离开及失焦，拒绝后台/过期事件 | pointer_state v1；按钮观察不依赖输入提交资格；TUI 不宣告支持 | core `3f85f47`；Web `6d04994`、`ce58c9a`、`aa417ae` |
| S04 canvas 与服务生命周期 | core graphics/interaction；Web replay renderer/service lifecycle | 独立受限画布重放指定 revision 后采样 ARGB；查询前发布待输出和绘图；处理单像素替换、取消、真实异步解码、项目切换和旧回复 | 校验 request/session/projection/canvas 身份，区分错误类别；不依赖显示 canvas 挂载；坏回包结束等待，解码配额有界 | core `a6dd644`、`b88e8b6`、`25805d2`；Web `011e5ca`、`da4e562` |
| 调试与投影修复 | core postmortem；Web debug/store/grid/export | 故障后允许只读观察，禁止写入/步进/继续执行；修复早到响应登记、显式拒绝后的投影候选恢复、窄窗口网格被 prompt 撑宽及转移 buffer 后证据丢失 | 保留故障终态、实际视口和严格来源检查；不改变 C 函数表布局 | core `b8b5bee`；Web `86c8072`、`319f960`、`ab7367b`、`23a721f`、`380e97e` |
| 执行证据与覆盖报告 | core runtime-tester/oracle；TUI/Web fixture/驱动 | API 分层状态、文件/符号/动态候选及未解析原因、typed capture、实际输入与生命周期观察、流式 gzip；TUI 验证准确缺能力 | 无效 span 不作证据，动态目标保守保留；工具退出成功不等于行为匹配 | core `fa6082e`、`4537449`；TUI `4f63c8b`、`d4266c7`；Web `44bd04a`；收尾分项见[交付提交](BATCH_1_DELIVERY_COMMITS.md) |

最终格式与兼容身份：runtime protocol **37.0**，project data **3**，HIR **14**，bytecode container **17**，
ISA **8**，compiler/native/host/VM ABI **42/17/13/16**，VM snapshot **13**，runtime snapshot **21**，
structured bundle **3**，compiled cache/project **10**；snake semantic/policy **2/2**，仍为实验状态。
旧编译缓存及不兼容 snapshot 明确拒绝；原版 v9 完整项目可恢复源码重建，旧 snake 1/1 identity 拒绝。
C ABI 布局、产品版本和其他 snake 策略未调整。

### 审查与验收结果

- 实现与测试均在本组专用 worktree 执行；最终发布 core pin 为 `b8b5bee45d1a7d3fc31f4df42dcbe0048422794a`。各 fixture、实际源码/dirty 状态、产物 hash 和命令/退出码保存在本组 `batch-1-work/1A/` 至 `1D/`；最终提交不追溯替换旧捕获的输入身份。三端数据组合 seed 为 `123456`。
- 1A–1D 各完成一次 `$refactor-rustyera-code` 独立审查，分别提出 7/4/7/6 项要求，全部在对应首条测试前落实；没有追加审查。1A 结论为有限来源/授权/格式边界修正，1B 为 REF snapshot/validator/STRFORM/memo 修正，1C 为模式/遍历/路径/原版隔离修正，1D 为像素/取消/限额/坏回复/真实生命周期/NoProgress 修正。
- 首测时间（+08:00）：1A `2026-08-27 23:25:42`；1B `2026-08-28 01:47:57`；1C `2026-08-28 03:17:34`；1D `2026-08-28 05:52:36`。测试由 **gpt-5.6-terra / low** 只读执行，无批次总预算；各套首次全量与定向复验分别保存，TW 仅按用户额外授权重跑。
- 同时选择两套 oracle 核对 profile 边界：原版语义基准 `26a35dc9334bb67590b96f7b8efbefbf199e391e`、wrapper `ffe560dad2fe480c8babddcae0122137350bf021`；蛇版语义基准 `fc4fb21416768c17256d0e82f997e5f99c9bba91`、wrapper `2c67518c594a638c2fbdef3e780341eb66ace294`。本批两参考仓库改动均为**无**。

#### 静态门禁与首次全量

下表合并各子批次的最终门禁结论，首次全量中的失败或跳过仍原样保留。详细命令见对应 `validation/` 记录。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 1A core/tool | workspace 与 runtime-tester 最小/全量；index fixture | 摄取、索引及缓存正确 | core 全量 exit 0 / 87.7s；tool 39/39 | 来源、类别、reload 等受影响最小集及 fmt/check/Clippy 通过 | `1A/validation/`；静态门禁完成 |
| 1A TUI/Web | pytest、Vitest、Web Rust 及前端静态门禁 | 三端协议和发布绑定一致 | TUI 425 通过、3 失败；Vitest 1017 通过；Web Rust 108 通过、1 ignored | TUI 版本元数据三例 3/3；Web 受影响最小集通过；Ruff/typecheck/lint/format/build/WASM/pin/打包检查通过 | 首次 TUI 全量不改写为修复后全量通过 |
| 1B core/tool | method fixture 35 项；workspace/tool 全量 | 惰性、类型/签名、省略、REF、递归/深度、generation、snapshot、优化一致 | core 全量 exit 0 / 120.2s；tool 41/41 | method/STRFORM/活动 REF/暖 memo/深度等定向与 fmt/check/Clippy 通过 | `1B/validation/`；本子批无前端改动，最终组合在 1D 验收 |
| 1C core/tool | data fixture、storage/GLOBAL/XML；workspace/tool 全量 | DEFAULT 与安全存储可执行 | core 全量 exit 0 / 90.38s；tool 51/51 | XML_REPLACE、GLOBAL、资源输入等受影响最小集及 fmt/check/Clippy 通过 | `1C/acceptance-summary.json`；未重跑全量 |
| 1C TUI/Web | pytest、Vitest、Web Rust 及前端静态门禁 | 真实存储契约一致 | TUI 475 通过；Vitest 1058 通过；Web Rust 130 通过、1 ignored | 资源读取/枚举/驱动受影响最小集通过；Ruff/typecheck/lint/format/build/WASM/pin/打包检查通过 | `1C/validation/`；静态门禁完成 |
| 1D core/tool | HTML/pointer/canvas/生命周期；workspace/tool 全量 | 规范服务、错误与状态顺序正确 | core 全量 exit 0 / 约 96s；tool 57/57 | postmortem 7 项、HTML/绘图屏障/坏回复及覆盖工具定向通过；fmt/check/Clippy 通过 | `1D/validation/`；未重跑全量 |
| 1D TUI/Web | pytest、Vitest、Web Rust 及前端静态门禁 | 图形服务与缺能力边界准确 | TUI 480 通过、5 跳过；Vitest 1209 通过、1 失败；Web Rust 130 通过、1 ignored | 缺能力 opt-in 5/5；mediaImage 20/20；其余受影响最小集与类型/lint/格式/build/WASM/pin/打包门禁通过 | Vitest 不称修复后全量通过；Web Rust ignored 为既有专用 handoff 用例 |
| Python 工具/监督器 | oracle driver、capture、supervisor unit | 证据与进程监督可靠 | oracle Python：1A 19/19、1C 21/21、1D 34/34；supervisor：1A 4 通过/1 权限错误，1C 9 通过/1 权限错误 | 两次 supervisor 均仅对权限失败用例定向通过；oracle/capture 后续最小集通过 | 属于工具验证，不替代真实 oracle 或客户端 |

#### 实际客户端与双 oracle

`matched / incomparable / different` 分别表示可比观测匹配、不可比和差异；不可比不计作匹配。
以下为每项最新有效证据的聚合，不代表对修复后代码重跑全矩阵。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 双 oracle smoke | 各子批次固定 reference CLI smoke | 两套参考可执行 | 1A/1B/1D 两套通过；1C 蛇版通过，原版脚本 exit 0、58 响应断言通过但外层监督 exit 1 | 1C 原版仅 capabilities/stdout 边界定向通过 | 不把监督首败改写为 smoke 全量通过；没有重跑该 smoke |
| 1A 索引双期望 | 24-case index fixture | ERD/ALS 与原版边界可追溯 | 首次完整结果未全部通过 | 最终原版 **9/10/5**，蛇版 **18/6/0** | `1A/compact-oracle-results.json`；原版动态用户索引既有差异和 warning 格式缺陷差异保留 |
| 1B 方法双期望 | 35-case method fixture | 惰性、签名、REF 与副作用可验证 | 两 profile Rust 观察 exit 0；oracle case 自身断言通过 | 最终原版 **23/12/0**，蛇版 **23/11/1** | 诊断/fault 后观察限制单列，VM 有直接副作用断言；蛇版 extra-argument 差异留批次 2 |
| 1C 列与存储双期望 | 27-case data fixture | DEFAULT、GLOBAL、XML、资源读取与枚举 | 首次 Oracle 未完整通过 | 最终原版 **18/6/3**，蛇版 **19/6/2** | `1C/acceptance-summary.json`；XML 拼写、文件模式、原版 Data 覆盖资源目录差异保留 |
| BBAS 初始化数据 | 现有 schema.xml / bbas_dataset.xml 最小断面 | 真实 XML/DT 数据可读可转换 | 两套 Rust/Oracle 共四项均 exit 0 | 无需修复；两 profile 各 1 matched | `RESULT:10..13=1/161/4531748/1`，`RESULTS:10..11=靈夢/真面目`；不含缺失地图文件或 SQL |
| TUI 实际 C ABI | RuntimeWorker、源码/打包库；batch1 组合及缺能力 fixture | 数据链执行、图形服务准确拒绝 | 最终源码与打包场景均通过 | 当前 core pin 下组合、debug 与缺能力定向通过 | `1D/tui-postmortem-dynamic-commands.json`、`tui-package-library-repair36.json`；GLOBAL=7、FLAG=55、method=42 |
| Chromium | batch1/services/lifecycle 与 service-oracle fixture | 真实 DOM、pointer、canvas、取消与切项目 | 最终基础/组合通过，生命周期 exit 0 / 17.62s | 普通服务 34 项最终 **22/6/6** | 组合/生命周期证据分别为 `1D` 的 repair62/58；仅复用已有浏览器程序 |
| 原生 Firefox | 同上，实际安装的 Firefox | 同上 | 最终基础/组合通过，生命周期 exit 0 / 22.01s | 普通服务 34 项最终 **22/6/6** | 生命周期 repair57，普通对照 repair51/52 |
| 原生 Safari | 同上，实际安装的 Safari | 同上 | 最终基础/组合通过，生命周期 exit 0 / 10.58s | 普通服务 34 项最终 **22/6/6** | 生命周期 repair57，普通对照 repair53 |
| Tauri | 同上，实际原生 host | 同上，不依赖显示 canvas 挂载 | 最终基础/组合通过，生命周期 exit 0 / 11.13s | 普通服务 34 项最终 **7/21/6**；18 份修复后与 16 份有效旧捕获分别绑定 | `1D/repair76-tauri-offline-result.json`；生命周期 repair56，未挂载显示 canvas=0、blocked=[] |
| 无进展危险断面 | 四 host × 两 profile；S04_CASE_NO_PROGRESS | runtime 有界报错，不无限等待 | 两参考均 load 成功后 run 停滞，看门狗终止、exit 1，无可比最终返回/watch | Rust **8/8**：ok=false、faulted、RESULT:10=777，vm_fault/context.api=html__lines_step，消息前缀 html.query.NoProgress | `1D/repair71-hazard-result.json`；明确安全差异，不称 oracle 通过 |

四个图形客户端的生命周期均覆盖 **6 组独立 pointer 观察、2 组真实图片解码竞态**，包括滚动、resize、
失焦、离开、重启取消与项目切换。TUI 不宣告 HTML 像素测量、pointer、canvas pixel 能力，诊断包含 profile/service/version。
实际服务契约为 `presentation_query/html_string_len/2.0`、`html_substring/2.0`、`html_string_lines/2.0`，
`input_state/pointer_state/1.0`，`canvas/sample_canvas_pixel/1.0`。

普通服务的 12 项非匹配逐项检查了实际值、副作用、终态和错误；包括错误观察限制、资源上限、实体/标签、字体像素及 Unicode 边界。
Tauri 额外不可比来自 `info runtime.compiled_cache_ready`，未过滤或改写原 verdict。
混合样式像素为 Tauri **85**、浏览器 **84**、原版 **104**、蛇版 **88**；其余 RESULT:11–15 为 **16/32/0/0/0**，
错误保持 `services.erb:76 InvalidMarkup`。不承诺不同字体/平台像素一致。

执行 fixture 入口及单文件 SHA-256（完整输入身份以 capture/trace 为准）：

| 入口 | 覆盖范围 | SHA-256 |
|---|---|---|
| `tools/runtime-tester/fixture-snake-batch1-clients/ERB/main.erb` | ALS/ERD→GETMETH→Resource/overlay→MAP/XML/DT→GLOBAL | `22fcde4c6014a6c3b7cc25905ffc944dc89fd14b2dccb5de40f968388a056eeb` |
| 同 fixture 的 `ERB/services.erb` | HTML、pointer、canvas | `16c6f1ef87f8378706aa5018e426423454b67bf205a7688127d0aa82741de831` |
| `tools/runtime-tester/fixture-snake-service-lifecycle/ERB/main.erb` | pointer 与异步生命周期 | `86c1de8601a4a1ee69d1b237bdfb950a414dd8813135661a7db2adce3b242274` |
| BBAS `schema.xml`（1,532 bytes） | 实际 schema 输入 | `83b7abf02eda889d85f6d094d26b2069c5483ad0668173de8941aef14ae279ce` |
| BBAS `bbas_dataset.xml`（36,420 bytes） | 实际 dataset 输入 | `d17b4ec540698f707e37c4a9f0b2b4b0093ff5b664196ae331f254a62abe4054` |

#### 蛇版 TW 最终覆盖结果

最新有效 v3 审计为用户授权的 repeat4；审计 **2012.55s**、流式完整性核验 **89.49s**。
共 **15,761 输入、0 读取失败、20 ALS、2 ERD**；报告 **5,069,815 行、112,727 函数、44,431 引用**，
并保留 901 项显式排除、7,983 个用户变量及原始字节/UTF-8 payload 各自的长度和 hash。

| 结果 | 最终值 / 边界 |
|---|---|
| 标题静态切片 | 321 个闭包函数、41,885 引用、611 目标解析记录 |
| GRAPH_DB_INIT 静态切片 | 11 个闭包函数、475 引用、21 目标解析记录 |
| 可达性结论 | 两者均为 `static_slice_not_execution`；保留动态候选及未解析原因，无效 parser span 不作有效证据 |
| 编译边界 | 默认 binary=false 审计保留 8,198 个 CHARADATA 错误；正确 binary=true 的最小 fixture 编译 0 错误、别名执行通过；不代表完整游戏编译通过 |
| 原始报告 | 4,668,806,077 bytes；SHA-256 `b4dd4441e7f0e731fd1434fecfb54a24f3f706640b4a54ab6960f362a57700db` |
| gzip 报告 | 150,730,066 bytes；SHA-256 `d365704edff41c8e47b3179b52db8a7b0239d469d001765efa337f9d9d45c88b` |
| 证据入口 | `batch-1-work/1D/tw-coverage-authorized-repeat4/summary-verified.json`；报告直接流式压缩，无多份展开副本 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 本批必要实施项 | 无未处理项 | 1A–1D 约定范围已验收，保留本节明确差异 | 进入后续批次，不重复本批全量 | 否，已登记完成 |
| 参考差异与观察限制 | 原版动态用户索引/warning 缺陷，错误诊断 schema，XML 序列化、平台匹配与像素差异 | 原始 matched/incomparable/different 不改写；GLOBAL 拒绝原子性、Unicode 安全与资源限额为明确边界 | 按实际语义范围复验，不以工具退出 0 替代等价 | 否，边界已登记 |
| 蛇版多余实参 | 非 variadic policy 仍严格检查 | method-extra-argument-policy 与参考不同 | 批次 2 统一实参策略 | 否，原计划范围 |
| TUI 图形服务 | 本批未实现终端像素/pointer/canvas 投影 | 只验证数据链及准确缺能力，不伪造坐标或测量 | 后续按终端能力单独规划 | 否，已明确边界 |
| NoProgress 参考停滞 | 固定两参考在相同 run 输入均无进展 | Rust 有界错误已验证；参考失败，无可比最终 watch/返回值 | 保留安全差异和原始失败证据，不延长等待或改参考 | 否，本批要求明确报错 |
| SQL 与地图资源 | 未实施 SQL，且缺少 bbas_map_schema.xml、bbas_map.xml | 现有 schema/dataset 断面通过，不代表 GRAPH_DB_INIT 或完整初始化执行 | 批次 3 实施 SQL，并确认缺失资源或参考容错边界 | 否，已登记阻塞 |
| 后置语义与真实可玩性 | 批次 2/4/5/6 的算术/RNG、标签/scene、存档及完整语言尚未完成 | 不承诺真实蛇版 TW 全项目编译、标题、新游戏或完整可玩 | 按依赖完成后续批次并独立验收 | 否，原计划范围 |
| TW 测试耗时与输出优化 | 尚未实施独立性能优化 | 基线见上节；约八成五秒采样位于报告生成，尚不能认定具体热点 | 后续先测分阶段耗时/内存/磁盘，再评估序列化、压缩、缓冲与同步摘要；保留完整证据及语义等价检查，不新增本批全量 | 否，仅保留用户要求的后续待办 |

### 交付与续做入口

- **结论：批次 1 已完成约定实施与验收，已知差异和后续阻塞如上；不代表完整蛇版兼容或真实游戏可玩。** 最终功能/证据索引见[验收汇总](BATCH_1_ACCEPTANCE_SUMMARY.md)，收尾分项提交与验证绑定见[交付提交](BATCH_1_DELIVERY_COMMITS.md)。
- 实施与验收时间为 2026-08-27 至 2026-08-28；本批测试结束，相关原生/Wine 会话已释放。下一批入口为[批次 2](#batch-2)，本次收尾未启动下一批或新的性能测试。

| 组件 | 最终交付提交 / 绑定 | 说明 |
|---|---|---|
| core | 产品契约 `b8b5bee45d1a7d3fc31f4df42dcbe0048422794a`；工具收尾 `e919d3719a2b0f5394c545783caa27289dcd7f7d`；验收记录 `30c25beb8060ef53f200ac31709eee56abc8649c` | 工具及后续文档提交不改产品 crate，不因此移动前端 pin |
| TUI | `ad5c018b7c73bac441a9064d3339a174eff7dcfa` | 完整绑定 b8b5bee；源码/打包 C ABI 数据与缺能力场景通过 |
| Web | `e3633311233df4a502faa41b32d8807c8c38de33` | rev、Git 依赖和发布锁完整绑定 b8b5bee；Browser/WASM 与 Tauri 分别验收 |
| 根更新日志 | `40fdea805c2fac3065a69fe541b8dd6265046efb`（本批最终追加） | CHANGELOG_PENDING 仅记录已完成的产品功能/修复，不写文档、工具或流程；无推送、主线合并或产品版本调整 |

最终产物 SHA-256：

- 本组 C ABI 与 TUI 打包输入：`a190f4957816d7ed973179efeb03e7f19ddfa9a986f17310d9237183d462014c`；包内处理后的实际身份见 `1D` 打包证据，不声称与输入字节相同。
- WASM：`bbe455923aca722a49c8f4dde3cc35498393455eae7f3a301b2f9d9d439205bf`。
- 最新 Tauri：`e6355b5425d1e25926871b1f4f9cecea4a3d80147dd81a966c3b3f58e77bc6be`；旧捕获和生命周期的原产物绑定仍单独保留，不改写为最新 binary 执行。
- Web 发布 Cargo.lock：`f164ec73c5e0846673d42fdf55cd536e1861ce935e9ac2b269019a1021b16002`。

本地命令、fixture/hash、首次全量与定向结果、审查结论、DOM/runtime 快照、差分及压缩覆盖报告
保存在同组 `batch-1-work/1A/`–`1D/`；参考失败及不可比证据仍保留，本文不再展开中间调试过程。
用户授权清理后，已删除可再生 codegen 中间文件、结束的 Tauri 临时项目副本和少量 Python 字节码，
实际释放 **14.34 GiB**，清理完成时可用 **33.09 GiB**；测试脚本、流程、工具、固定 fixture、
批次 0/1 验收证据及必要产物保留，受保护文件 hash 不变。清理记录为本组
`batch-cleanup-2026-08-28/README.md`；已清理的项目副本路径不再作为可用输入，复现从保留 fixture 与归档恢复。

<a id="batch-2"></a>

## 批次 2：确定性 API、输入与兼容差异骨架

计划入口：[详细实施方案](BATCH_2_IMPLEMENTATION_PLAN.md)；总体入口：
[改造思路 / 批次 2](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-2)。状态：**产品实施与功能行为验收完成；
保留一项规模采集基础设施缺口**。负责人：Codex；最终更新：2026-08-30。

### 具体实施方案

- 实施固定的 2A–2F：D07/D11/S11/TOINT，D04/D06/S05/S09/N01，S06–S08，D17，
  D08/D10/S10/S13，以及 D13/D12/C03 和最终三端汇合。依赖顺序、测试 agent 粒度、
  单次全量和唯一重构审查均按[详细方案](BATCH_2_IMPLEMENTATION_PLAN.md)执行。
- 明确不做 SQL、Float、variadic、元素 REF/OUT、扩展 HTML/scene、外部蛇版存档和真实游戏
  可玩性；TUI 不宣称完整 GETKEY。固定 RNG 为 RustyEra 的统一 SFMT 权威状态，不实现
  `UseNewRandom` 的 `.NET Random` 双路径，也不复制蛇版 dump 临时副本缺陷。
- 最终身份为 snake semantic/policy **9/9**，arithmetic `snake_saturating_i64_v1`，
  RNG `sfmt19937/state1`；runtime protocol **40.0**、HIR **18**、container **21.0**、ISA **10.0**、
  compiler/native/host/VM ABI **46/21/16/21**、VM snapshot **20**、runtime snapshot **26**。
  旧 snake cache 与不兼容状态明确拒绝，不做静默迁移；原版 profile 行为保持隔离。
- 规划基线为 core `35275f8`、TUI `ad5c018`、Web `e363331`；最终产品绑定为 core
  `68d0f208ce4c8d7cb2c95b0c2d894e1a4c0c72a4`、TUI
  `69ed1249ed5859ef6f486b43aec0ea7983e5863d`、Web
  `2158972c6041e3855021d3c45411a39ecc019329`。三仓均在 `codex/snake-compatibility`，收尾时干净。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 2A / D07、D11、S11、D12 | core compat、analyzer/compiler、VM、runtime、oracle fixture | 集中逐操作整数策略；普通 snake 算术按固定边界处理，UNCHECKED 始终 wrapping；TOINT 仅捕获整数 reader；SFMT dump/restore、snapshot/replay 统一且非法状态原子拒绝 | snake identity 3/3；保留原版及固定 RNG 有意差异 | `d6cbf820`–`6e730606` |
| 2B / D04、D06、S05、S09、N01 | parser/HIR/compiler/validator/VM/runtime/cache | CALLSTR 六变体解析完整调用文本；多余用户实参不求值；EXISTVAR 模式解析表达式；STRFORMCHECK 实际展开并捕获限定脚本故障；动态调用依赖保守失效 | identity 4/4；新增 continuation、typed failure、cache 依赖与严格实参配置 | `5bda5424`–`277f1929` |
| 2C / S06–S08 | analyzer、VM structured/data、runtime-tester | MATCHALL/EX、CSV 名称反向索引、事务 bit API 和有序 MAP 扩展；MAP string 格式无转义且逐条写入 | identity 5/5；structured snapshot/GLOBAL 走既有状态契约 | `aaffe753`–`1e869fa6` |
| 2D / D17 | compat、VM fault lifecycle、protocol | snake BEFORE_THROW/BEFORE_ERROR 在最终 fault 前运行；保留原错误并附 secondary fault，禁重入和恢复执行 | identity 6/6；协议发布结构化 fault chain | `cc4be698`–`194d5e3d` |
| 2E / D08、D10、S10、S13 | core runtime/presentation；TUI/Web renderer | 负历史索引、全局整行背景、逻辑动画计时器和 bitmap-cache 诊断；状态进入 snapshot/delta，TUI 合成透明色 | identity 7/7；前端不从 DOM/终端私有状态重建语义 | core `7988576c`–`cb2ef2fa`；TUI `81dc782`–`73d1dff`；Web `2457df2`–`21945d0` |
| 2F / D13、D12、C03 | core protocol/runtime/input；TUI；Web/WASM/Tauri | sequence 单槽、宏开关、NF viewport、设备事件/latch、真实 AWAIT 0 泵、Environment capability、GETPLATFORM 映射；按 runtime 收件顺序裁决 | identity 9/9、runtime protocol 40；TUI 明确缺按键 latch 能力，Browser/Tauri 提交真实事件 | core `5fafb61c`–`68d0f208`；TUI `05ed41c`–`69ed124`；Web `c6c191f`–`c30d202` |
| 2F 汇合修复 | Web store 与 Tauri/cache harness | 串行原生 IPC；缓存 handoff mtime/variadic sentinel 可移植；结构化验证合法 snake profile warning；按 correlation owner 路由拒绝；修复偏好 applied 早于 message ID 的竞态 | 不吞合法诊断，也不把 stale projection context 投成独立 warning；cache hit 与热设置可继续 | Web `38bdc36`、`738fd83`–`2158972`；core `68d0f208` |

### 审查与验收结果

- 六个子批次均在各自首条测试前完成且只完成一次独立重构审查，审查要求先落实后测试。
  2A/2B 的完整落盘审查分别为 `batch-2-work/2A/review/review.md` 与
  `batch-2-work/2B/refactor-review.md`；2C–2F 的结论与落实记录保留在任务审查回执及各目录
  冻结输入/定向验证证据中，没有启动第二次审查。
- 原版 oracle 为 `emuera.em` wrapper `ffe560d`、exe SHA
  `0361383d31daf9931f2cd4cde214190e71a09788d83a05fb0038d5cb78886132`；蛇版 oracle wrapper
  `2c67518`、exe SHA `098ed2fbed4100b732f182a90ebda7b99f4b16b484af4ac133b279ec57089dc3`。
  每例依次完成 Rust 执行、adapter、返回值/副作用/终态/诊断断言和差分；原始
  `different`/`incomparable` 与已登记差异均保留，未改写成相等。
- 首次全量各只启动一次。core 最终汇合的 `cargo test --workspace` 首次在两个 VM 数组用例失败；
  TUI 首次完整 pytest 为 **474 passed / 9 failed / 5 skipped**，失败均为旧 C ABI protocol mismatch；
  Web 首次完整 Vitest 为 **1360 passed / 7 failed**。修复后只跑受影响定向集合：core 两用例及
  相关 VM 集恢复；TUI 9 个 real-CABI 用例恢复；Web runtimeStore 定向 **197 passed**。
  因规则禁止重跑全量，这些结果不得描述为“修复后全量通过”。
- 最终 Tauri 断面只允许一条可见 warning：core 发布的结构化
  `runtime.experimental_compatibility_profile`，severity `warning`、stage `configuration`，并携带
  snake semantic/policy identity 9/9。测试按 code、stage、identity 和 correlation 验证来源，
  不按显示文案放行；无结构化来源、重复或额外 warning 均失败。偏好回复早到和拒绝误归属已在
  Web 事务路由修复，合法 profile warning 本身未被隐藏或降级。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 2A 算术/RNG/TOINT | policy fixture，双 profile Rust + 双 oracle，35 个 case | 数值、告警、状态与固定差异逐例可判 | 原始 matched/different/incomparable 均保留 | acceptance index 离线绑定全部最终 receipt | `batch-2-work/2A/acceptance-index-v1.json`；通过，固定原版 MIN 负号历史差异和 UseNewRandom 差异 |
| 2B 动态调用 | call fixture 46 个 profile ordinal | 完整文本、lazy args、REF、TRY/JUMP、EXISTVAR、STRFORMCHECK、cache | raw：27 matched、16 incomparable、2 different、1 load rejection | 后续定向修复及静态覆盖完成 | `batch-2-work/2B/acceptance-index-v1.json`、`validation/`；登记拒绝阶段和诊断不可比 |
| 2C 数据 API | 8-case snake matrix + original profile rejection | CSV/MATCH/bit/MAP 返回、副作用和顺序一致 | 8/8 matched observables | 不需动态重跑 | `batch-2-work/2C/dynamic/audit-index.json`；通过 |
| 2D fault hook | 5 个 snake hook + 1 个 original no-hook | hook 分流、secondary fault、禁用和原错误保留 | 6 个断面完成；诊断文字不可比单列 | 不需动态重跑 | `batch-2-work/2D/dynamic/audit-index.json`；通过 |
| 2E 展示/计时器 | core 双 oracle + TUI/Web 三端 fixture | 历史、背景 alpha、timer 边界、snapshot/delta | 静态首次全量记录在 `2E/static/phase2/` | 定向修复负历史 pending 行和 timer fault publication | `batch-2-work/2E/`；Browser/Tauri 全宽投影和 TUI 合成降级通过 |
| 2F 静态门禁 | core fmt/check/Clippy/minimal/一次 workspace；TUI minimal/full/Ruff；Web minimal/full/type/lint/format/build/WASM/Rust | 输入冻结后全部适用门禁可判 | 三套首次全量均保留失败结果 | 只跑最小受影响集合，最终静态门禁全绿 | `batch-2-work/2F/static/`；不得解读为第二次全量 |
| 2F 真实客户端输入 | Chromium、Firefox、SafariDriver、Tauri、TUI | NF、sequence/macro、设备泵、down/up、blur、缺能力路径 | harness 差异逐个停止并修复 | Chromium/Firefox/Safari/Tauri/TUI 最终断面通过 | `batch-2-work/2F/dynamic/results/`；Safari 仅 SafariDriver API，未请求 Accessibility |
| 编译缓存汇合 | TUI↔Browser/Tauri source-index handoff、warm/hot setting | 跨端实际保存/加载、identity 拒绝、零错误命中 | 初次暴露 mtime 与 sentinel 可移植性问题 | 双向 handoff 和两个 Tauri source-index 通过；Tauri cache hit + FontSize 18→17 通过 | `cache-handoff-mtime-retest/summary.txt`、`tauri-compiled-cache-owner-routing-final/summary.txt` |
| 蛇版 TW 规模断面 | frozen runtime-tester coverage，profile snake | 一次流式压缩报告，保留阶段/诊断/缺口 | child exit 0；1576.74s；5,069,815 appearances、8,571 diagnostics | 按规则不重跑 | `snake-tw-final-scale/`；静态覆盖，不是编译通过或可玩性证据；峰值 RSS 未取得 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| 蛇版 TW 峰值 RSS | `/usr/bin/time -l` 在任务沙箱结束时因 `sysctl kern.clockrate: Operation not permitted` 返回 1；coverage 子进程已 exit 0 | wall/user/sys、压缩报告及阶段完整；只有 peak RSS 缺失。按证据复用规则不为补报告重跑 26 分钟任务 | 在允许 `time` 读取该系统信息的同输入环境下，于后续性能批次重新采集；不能从现有报告伪造 | 否，属于非功能证据缺口 |
| 真实蛇版 TW 编译/标题 | 报告仍有 8,571 个 analyzer 诊断，SQL、扩展 HTML/scene 与存档闭环后置 | 本批只证明已承诺 API 不落 trap 及输入/缓存契约；不声称标题、地图或新游戏可用 | 批次 3 完成 SQL，批次 4 汇合 presentation/自身存档 | 否，原计划明确排除 |
| TUI GETKEY latch | 终端没有可靠完整 down/up/toggle 设备面 | 普通/NF/宏/sequence 输入通过；Environment 明确缺能力并返回诊断 | 若将来有可验证终端协议再单独提案 | 否，已确认取舍 |
| 有意参考差异 | SFMT 不复制 UseNewRandom 双状态；MAP 无转义；原版 MIN 负号系统行不注入 Rust 历史；部分诊断文字/拒绝阶段不可比 | 返回、副作用、终态和差异均逐项登记，不将不同改写为相同 | 后续批次继续沿 identity 与 difference ledger 验收 | 否，已写入分类/计划 |

### 交付与续做入口

- 2A–2F 产品范围、三端绑定和功能行为矩阵已交付；没有未登记行为差异。严格完成判定中的
  “无必需未验证项”仍保留上述 peak RSS 基础设施缺口，因此本文不伪称该数值已验收。
- 最终产品产物：release C ABI SHA
  `ff32b62199454db263743862729e8d58b6c3ddd750975f1810d2b8684e8368c8`；WASM SHA
  `ed012d84a6dab23712065e82e756f8810d63b1c2e5818bb89827993bf6c8873a`；Tauri binary SHA
  `62f21992a3bb998a1f39db0732c59ac67560584d812203fd95e4d18e49557f71`，webdriver manifest SHA
  `0f3d7f92f626897f9b9bfb29a4de98ffc09df78dfb366753c1c5b8b5838d431d`。manifest 与 binary、
  WASM、core `68d0f208` 和 Web source 输入一致。
- review、索引、精简 summary、测试脚本及 fixture/config 保留在 `batch-2-work/2A/`–`2F/`；
  最后 Tauri 结论为 `2F/dynamic/tauri-compiled-cache-owner-routing-final/summary.txt`。2026-08-30
  按用户明确要求删除其中可再生构建产物、运行 payload、原始日志和 DOM/runtime 快照；后续若需
  重跑必须重新构建并生成独立证据，不能把精简 summary 当作 cache-only 产物。
- 根 `CHANGELOG_PENDING.md` 已在整批收尾单独追加已验证产品行为；没有调整发布版本、推送或
  合并主线，也没有启动批次 3。

<a id="batch-3"></a>

## 批次 3：安全 SQL（蛇版 TW P0）

计划入口：[改造思路 / 批次 3](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-3)；实施入口：
[批次 3 分批实施方案](SNAKE_EMUERA_BATCH_3_IMPLEMENTATION_PLAN.md)。状态：**已完成约定的安全
SQL 子集**，不代表完整蛇版 TW 已可进入标题、新游戏或存档流程；负责人 / 最近更新：Codex / 2026-08-30。

### 具体实施方案

- 3.0 先冻结蛇版 reference 的 SQL 行为、真实资源摘要和安全差异；3.1 定义
  `rustyera.sql@1` 服务、固定 limits 和脚本 API 目录；3.2 在 core 完成连接、typed value、reader、
  transaction、Resource 派生 revision、MAP XML 与 snapshot/project lifecycle；3.3、3.4 分别接入
  Web/Tauri 的共享 Worker provider 与 TUI APSW provider；3.5 用固定契约 fixture 收敛三端；3.6
  最后使用真实蛇版 TW 数据库和翻译 XML 完成 QOL/GRAPH 流程与整批记录。
- 只支持内存库及清单授权的只读 Resource seed；提交产生项目 `Data/sql` 中的不可变 revision，
  以 CAS current 指针发布。不提供任意路径、URI、外部 `ATTACH`、extension、虚拟表或通用连接串。
  SQL service 保持 v1；SQLite 固定为 Web/Tauri `3.53.0-build1`、TUI APSW `3.53.0.0`；本批未提高
  发布版本，也未修改传统 save 格式。
- core、TUI、Web 均在 `codex/snake-compatibility` 专用 worktree 分仓实现和提交；动态测试使用
  `batch-3-work/` 下忽略的项目副本、独立 Data/OPFS/profile/Wine prefix、端口和证据目录。蛇版
  Emuera、蛇版 TW 及其真实资源全程只读，保留游戏仓库原有 `emuera.config` 修改。
- 3.6 的验收切片覆盖 QOL item/pharmacy/dish/mushi/wood、两个翻译 MAP、GRAPH schema/transaction
  rebuild、BFS、跨地图边、节点属性、reader EOF/close、断连回滚、同会话重启 revision 复用、seed
  摘要变化和 Tauri A-B-A 项目隔离。配额、异常关闭及拒绝面复用 3.3–3.5 已冻结的 provider/contract
  测试；本批不补造真实项目没有的 BBAS MAP，也不运行完整标题、新游戏或传统存档初始化。

### 所作改动

| 功能/修复项编号 | 组件与文件 | 实际改动及理由 | 契约/兼容性影响 | commit 与依赖 |
|---|---|---|---|---|
| 3.0 | core oracle fixture/文档输入 | 冻结 SQL 资源、调用形状、参考返回和预检行为 | 仅测试基线 | core `3276b7d7` |
| 3.1 | core protocol、compatibility、compiler API catalog | 定义安全 SQL v1、能力协商、limits、缺能力预加载拒绝和已支持/延后 API | 新增 `rustyera.sql@1`；snake identity 纳入 service policy | core `3a4deb91`、`be7ec967` |
| 3.2 | core runtime/VM/CBOR/CDDL | 实现连接、执行、scalar/reader、transaction、revision、Resource seed、MAP XML、snapshot 和项目 lifecycle；修复 reader 值模式、静态省略参数及 MAP 导入成功值 | 三端共享同一 typed rows/error/revision 契约 | core `71a53db5`、`e7bf8962`、`fde13ea8`、`73535252`、`44783e2e`、`11ea5ffd` |
| 3.3 | Web Worker、browser/Tauri storage bridge | 在专用共享 Worker 中运行 sqlite-wasm，按项目原子发布 revision；补齐 malformed payload 与浏览器 marker runner | Browser/WASM/Tauri 共用 SQLite/provider | Web `5eef7905`、`9afc4950`、`d6108d2b` |
| 3.4 | TUI RuntimeWorker、APSW provider、打包 | 增加 APSW provider、C ABI service routing、原子存储、取消/epoch 清理及隔离测试 CLI | TUI 与 Web 的 SQLite/协议版本对齐 | TUI `7eb6f6df`、`8d9ae139` |
| 3.5 | TUI/Web 共用 contract fixture 与 core pin | 收敛 typed value、transaction、MAP、restart、snapshot、A-B-A；修复 TUI restore 跨 epoch 回复和精确 revision 候选 | TUI/Web 都绑定 core `11ea5ffd` | TUI `fe49ed75`、`500929dc`、`7cbc7037`、`68d9c824`；Web `4d6bcf60`、`06274945` |
| 3.6-TUI | `sql_provider.py` 与 provider regression | APSW 裸 `VACUUM` 会发出内部空 `ATTACH` authorizer 回调；只在严格、无参数、单语句裸 `VACUUM` 作用域放行，显式 `ATTACH`、`VACUUM INTO`、虚拟表和 extension 仍拒绝 | 修复真实 QOL 派生库压缩；不扩大连接权限 | TUI `13fab3af`；依赖 core `11ea5ffd` |
| 3.6-Web | test-only `runtimeEvidence` 与测试 | 仅将 state chunk、storage write/read/read_chunk 的 bulk byte leaf 投影为长度+BLAKE3，避免 1.3 MiB 数据库在每个完整快照中展开；保留 typed envelope、限额、显式 failure 与 watchdog 语义 | 只影响启用测试控制时的证据体积，不改服务 CBOR 或产品存储数据 | Web `7e946c2b`；依赖 Web `06274945`、core `11ea5ffd` |
| 3.6-fixture | ignored `batch-3-work/3.6/` | 真实资源切片、固定断言、5 秒阶段标记、三端场景与 trace；不提交游戏资源或派生数据库 | 测试材料，不进入产品/协议 | 未跟踪；ERB SHA-256 `4f52aa2d…1ded33` |

### 审查与验收结果

- 最终输入为 core `11ea5ffdf4484b3259900a7e7f060a0e41f63c1f`、TUI
  `13fab3af753ba7487269c9df69afb32a16279d94`、Web
  `7e946c2b7ea616a454eb6efd69b0ad3cb46d290f`；snake TW 为 `667b9cd0…`。固定 seed 的 runner
  实际值为 `123446`，clock 为 `2026-08-30T00:00:00Z`，profile identity 为
  `emuera.skia.snake@10/10`。
- `qol_data.db` 精确 seed 为 1,368,064 bytes、SHA-256
  `e03c5a3279735f68e0cabf108e1a786fb5793ab94c69e5a6f46d078b495593a1`、
  `schema_version=101/user_version=0`；seed 变体只将 `user_version` 改为 1，SHA-256
  `cdabbd3e623c249d611682efc316391b79386568356889c1c742731fe1cbb916`。
  `tw_csv_chs.xml` / `tw_taste_chs.xml` SHA-256 分别为 `6cb8cf45…90635`、
  `56086344…83750`；真实物理条目 4461/2376，经 MAP 主键覆盖后的 SQL 行数为 4431/2344。
- 3.6 发现的 TUI 产品修复和 Web 测试观测修复各触发且只触发一次
  `$refactor-rustyera-code` 审查，均在该修复点首条测试前完成。TUI 采用异常安全的 per-connection
  bare-VACUUM scope；Web 采用精确 bulk leaf 的非递归投影、16 MiB state/64 MiB storage 限额及
  数组分块 hash。全部要求先落实后才启动各自门禁；测试开始后没有再次启动审查。
- 用户明确取消了本次任务全过程的共享 60 分钟测试墙钟上限；未据此放宽每命令 timeout、静态先于
  动态、每套完整 suite 最多一次或 Web/Tauri 每 5 秒完整 DOM/runtime 快照规则。3.6 的 TUI 修复
  只启动一次完整 pytest，Web 修复只启动一次完整 Vitest；失败后均只跑最小受影响集合。
- 蛇版 SQL 语义使用 wrapper repository `0c50ccbe0c2434567ef527d72a54c967bd576f2a`、
  semantic executable baseline `fc4fb21416768c17256d0e82f997e5f99c9bba91`；原版回归基线沿用
  wrapper `ffe560…` / semantic `26a35dc…`。断连未提交 transaction 的 reference 返回 1，而安全
  provider 返回 0；这是已登记的强制回滚差异，未改写成 matched。
- 最终 C ABI dylib SHA-256 为 `76a9782f…9083`，Web WASM SHA-256 为
  `de2dd1ec…1a161`，Tauri binary / webdriver manifest SHA-256 分别为
  `e62049f2…fe436` / `d2cbdddb…a1027e`；Tauri build contract 因 Web testing bundle 变化只重建一次，
  最终定向复验使用 `--require-reuse-build` 严格命中该产物。

| 验收项 / 静态或动态阶段 | 命令与 fixture | 预期 | 首次结果 / 退出码 / 时间 | 修复后定向复验 | 证据与结论 |
|---|---|---|---|---|---|
| 3.0–3.5 既有门禁 | 各子批次 core/TUI/Web focused、唯一 full suite、oracle 与真实客户端 contract | SQL v1 与三端固定 fixture 收敛 | 各提交已绑定当时的门禁；3.3 首次完整 Vitest 暴露 malformed fixture，3.4/3.5 暴露并修复 restore lifecycle | 只执行对应 decoder/runtime/provider 定向集合；3.6 未为整理日志重跑既有全量 | commit 列表为交付绑定；当前忽略目录未保留全部早期原始命令输出，故不补造遗失的逐命令时间/计数 |
| 3.6 TUI 静态 | focused provider、Ruff、一次完整 pytest | 裸 VACUUM 最小放行，拒绝面不泄漏 | focused 初次 9 passed / 1 failed，失败是测试把 `load_extension` 的 SQLite code 误期望为 AUTH；不是产品失败 | 修正该断言后受影响集合 7 passed；Ruff exit 0；唯一完整 pytest exit 0，**530 passed / 5 skipped**，43.16s | `git diff --check` exit 0；无本机路径/凭据 |
| 3.6 Web 静态 | focused Vitest、typecheck/lint/format/build/WASM、一次完整 Vitest | bulk evidence 有界且协议不变 | focused **152 passed**；typecheck/lint/build/build:wasm exit 0；format 首次因两文件 Prettier 失败 | 机械格式化后 format/focused/typecheck/lint 均 exit 0；build/WASM 因语义输入未变复用；唯一完整 Vitest exit 0，**97 files / 1409 tests** | 1,368,064-byte read snapshot 小于 1 KiB；invalid/oversize 显式 failure，digest churn 不掩盖 failure/overflow |
| TUI 真实资源 + oracle | `tui-first-run.json`、同 serve `tui-restart.json`、独立 `tui-seed-variant.json` | QOL/GRAPH/TR、回滚、restart 与 seed identity | 首次实际流程在裸 VACUUM 返回 SQLITE_AUTH；sandbox Wine 另因 `wineserver bind` 被拒，登记为 infra | 最终三条 non-sandbox 均 exit 0；first reference diff equal（除登记 rollback），PERSIST 1→2，variant PERSIST 1/SEED 1 | `batch-3-work/3.6/evidence/tui/*.trace.ndjson`；最终三 trace SHA-256 `1e74296e…c3ab`、`41023b9c…e0bf`、`4193b3b0…1c9` |
| Chromium 实际流程 | `scenarios/web.json`，真实 Chromium headless shell | first/restart、24 个阶段、全部结构化结果，无 fault/evidence failure | 初次在 1.3 MiB storage response 展开后触发相同快照；修复后又发现 fixture 误设 checkpoint、精确 output 未含阶段、restart 首标记超过观察窗 | 逐项只修测试观测/fixture；最终 exit 0，约 57s，PERSIST 1→2、SEED 0、`fault=null`、overflow=false/failure=null | `batch-3-work/3.6/evidence/web/chromium.trace.ndjson`，SHA-256 `a0c7ef37…3b6a` |
| Firefox / Safari 实际首轮 | `test:browser-compat` + `B36_READY` | 真实浏览器、冷 OPFS、同一 SQL 切片 | 无产品差异 | Firefox 154.0.1 exit 0，约 10s；Safari 26.6.2 exit 0，约 1s，窗口未最小化 | `.rustyera/test-runs/browser-compat-firefox-1788078798519/` 与 `browser-compat-safari-1788078821193/`；全部首轮 marker |
| Tauri A-B-A | ignored `snake-sql.spec.mjs`，official `test:tauri` | native bridge、restart、seed 变体和项目隔离 | 首次 cache miss 完成唯一重建并推进到变体，但外层 PTY 被回收，4 个 snapshot 无终态，记为 infra 未验证 | `--require-reuse-build` 命中当前 manifest，exit 0，**1 passing / 17.489s**；PERSIST 1→2、B=1、A return=3，SEED 0→1→0 | `.rustyera/test-runs/tauri-snapshots/2026-08-30T08-39-31.901Z-snake-sql.spec.mjs.jsonl`；4 个完整非相同快照，fault/evidence 均正常，无残留 GUI/WebDriver |
| 固定数据库断言 | 实际 QOL/GRAPH/TR 查询与 reader | 行数、样本和返回值固定 | 最终各客户端一致 | 不需额外复验 | QOL `313/173/83/342/197/13`；GRAPH `1555/36418/132/642`；BFS `1/1/3/11`；cross `10/610/70/50`；reader `1/1/2/1/1/0/1/1`；翻译 `1/4431/.../1/2344/...` |
| BBAS 前提 | 3.0 resource preflight / reference | 只登记实际存在性，不伪造文件 | `bbas_map_schema.xml` 与 `bbas_map.xml` 都缺失，reference DT row length 均为 0 | 不重跑/不生成替代资源 | 外部资源阻塞，按方案不阻碍 Batch 3 安全 SQL 子集完成 |

### 未完成项、阻塞与计划偏差

| 项目 | 未完成原因 / 依赖 | 影响与已验证边界 | 下一步及解除条件 | 是否需更新改造思路 |
|---|---|---|---|---|
| BBAS MAP | 真实蛇版 TW 缺少 `bbas_map_schema.xml`、`bbas_map.xml` | `CREATE_BBAS_DATABASE` 只能完成 3.0 的前提与 reference 行为确认；未构造数据库 | Batch 4 前由上游提供权威资源并锁定摘要后再验收 | 否，批次 3 方案已明确允许此外部阻塞 |
| 延后 SQL/XML API | `SQL_CONNECTION_OPEN`、`SQL_ESCAPE`、Float scalar/reader、DT/custom XML import、MAP/DT/custom XML export 尚未实现 | 当前真实 QOL/GRAPH/TR 安全子集均未调用；缺能力仍稳定诊断 | 后续批次按真实调用优先级单独设计和版本化 | 否，属于已声明不做项 |
| 完整标题/新游戏/存档 | 仍有 presentation/scene/存档及其他语言缺口；冻结规模报告有 8,571 条 analyzer diagnostics | 本批只证明固定 SQL 切片和客户端 provider，不证明完整项目可编译或可玩 | Batch 4 汇合主玩法 presentation、scene 与自身存档；不得以本批结果替代 | 否，改造思路已按批次拆分 |
| 登记的安全差异 | provider 在 disconnect/epoch/project switch 强制回滚未提交 transaction；reference fixture 报告 rollback count 1 | Rust 实际断言为 0，防止未提交数据持久化；其余固定观测匹配 | 保留 difference ledger，后续不得放宽为外部 ATTACH 或隐式提交 | 否，安全边界有意不同 |
| 早期原始门禁细节 | 3.0–3.5 的部分未跟踪原始静态日志在本轮开始前已不在 `batch-3-work` | 各组件提交、当前 pin 和 3.5 真实通过结果仍可核验；不为补报告重跑完整 suite | 后续批次从首条命令持久化精简 summary；不能补写未知时间/计数 | 否，证据留存缺口，不改变产品范围 |

### 交付与续做入口

- Batch 3 的安全 SQL 承诺子集已满足整批验收：TUI、Chromium、Firefox、Safari 与 Tauri 均在
  core `11ea5ffd` 上完成真实 QOL/GRAPH/TR 切片；TUI 对蛇版 reference 的唯一不同为登记的断连
  回滚安全差异。BBAS 缺失、延后 API、完整标题/新游戏/存档和 8,571 条静态诊断不属于完成范围。
- 最终组件提交：core 行为 `3276b7d7`、`3a4deb91`、`be7ec967`、`71a53db5`、`e7bf8962`、
  `fde13ea8`、`73535252`、`44783e2e`、`11ea5ffd`；TUI `7eb6f6df`、`8d9ae139`、
  `fe49ed75`、`500929dc`、`7cbc7037`、`68d9c824`、`13fab3af`；Web `5eef7905`、
  `9afc4950`、`d6108d2b`、`4d6bcf60`、`06274945`、`7e946c2b`。本次 core 文档提交及根
  `CHANGELOG_PENDING.md` 提交见任务最终交付；没有版本 bump、push 或主线合并。
- 3.6 可复现入口为 workspace-local ignored `batch-3-work/3.6/`：`projects/`、`scenarios/`、
  `tauri/snake-sql.spec.mjs` 与 `evidence/`。最终 ERB / fastpath SHA-256 分别为
  `4f52aa2d…1ded33` / `2eea9cd0…be5d7`；Web scenario / Tauri spec SHA-256 分别为
  `96915e11…5d36d` / `afc5b401…1d03`。恢复时先重核 core pin、资源、产物 manifest 和浏览器版本，
  不复用已清理的 Data/OPFS/profile。
- 各 runner 已清理隔离的可写项目/数据库并停止 Wine、browser、Tauri 与 WebDriver 进程；磁盘仍有
  55 GiB 可用。按证据规则保留上述 fixture、trace、浏览器/Tauri snapshot 与唯一当前 Tauri
  binary/manifest；它们是后续复核入口，不随本轮收尾删除。参考实现、游戏资源和用户配置未修改。

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
