# 蛇版 Emuera 适配：分批次实施与验收记录

> 文档状态：批次 0、批次 1 已完成，保留各批明确差异；其他批次仍待登记。批次 0 完成范围是 profile、隔离、基线与门禁，不是完整蛇版语义或蛇版 TW 可玩性。

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

计划入口：[改造思路 / 批次 1](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-1)。状态：已完成（保留明确差异）；负责人 / 最近更新：Codex / 2026-08-28。

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
| 1D | 已完成；差异与首次失败保留 | 1 | 2026-08-28 05:52:36 +08:00，无总时限 | core/tool/Python/TUI/Web Vitest/Web Rust/双 smoke 各一次；另有用户单次授权TW重跑 | 三浏览器/TUI/Tauri组合、四端生命周期与服务对照、有效TW v3报告已具证据；已披露参考/像素差异，分项提交与最终来源检查完成 |

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

#### 1D 服务层实施入口（core 首次全量通过，其余门禁进行中）

- Web 独立执行者拥有 runtime service 生命周期、pointer 规范按钮值观察及独立 canvas
  replay sampling；不修改正在验证的 core，也不修改 1C 的 browserProject/resource/storage 文件。
- pointer/canvas 保持 v1 payload，使用实际 viewport、三 projection revision、session epoch
  和 request ID；MOUSEY 按固定参考的 clientY-clientHeight 映射。HTML v2 留待 core 契约与
  规范树测量实现，不提前宣告能力；Rust bridge、共享版本、pin/锁文件由主串行整合。
- 以下隔离开发步骤为历史过程；当前 1A–1C 已收尾，D 唯一审查及源码整改已完成。
  D 静态与动态结果单独记录，不继承 C 的通过结论。
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

- 1C 收尾证据独立核对完毕：`1C/acceptance-summary.json` SHA-256 为
  `02542a2f59330e0b7b6519ac81ab6438d39a9dc4b2d5e7a5785e83673a826e43`，Markdown 为
  `5809e8eeec0afaca141119d9b1dbf42f36c17fd1b74d6ef62c37aa27e1959c8c`。
  包含 174 条命令、17 套首次全量（13 项退出 0、4 项非零）、逐 case 有效证据及
  完整日志/产物摘要；历史 dirty 构建身份与后续交付 commit 分列。core 收尾文档
  `3640b9e`、根更新日志 `6449a62` 已提交，不推送、不合并。
- 1D 唯一审查已结束，正式报告 `1D/review.md`、逐文件清单 `review-results.json`。
  六项要求为 R1 单像素替换、R2 逻辑取消释放队列但保留底层解码配额、R3 巨型槽安全
  拒绝差异、R4 坏回包 continuation 终止、R5 真实失焦/挂起取消/切项目驱动、R6 独立
  no-progress 捕获入口。不得再次启动或恢复审查；所有要求落实后才能首测。
- 三仓原 C 输入均已提交，180 项 D 源经活动基线与隔离输出逐项 SHA-256 核对后合入，
  含三份 71 字节 PNG；记录 `1D/active-integration.json`。没有覆盖未列入的文件，
  没有运行旧 assemble 脚本或测试。compiler42/host13/runtimeSnapshot21 仍需最终门禁。
- R1/R2 已补产品及回归源码：单像素局部 ImageData 替换，每请求独有 renderer/surface，
  取消解除逻辑等待；跨请求/代际共享 32 个未结束解码及像素配额，底层结束后才释放。
  四种异步阶段的取消/迟到清理及配额测试已编写，尚未运行。真实 fixture 新增 alpha128
  覆盖、透明清除及邻像素不变观察；不声称所有低 alpha RGB 在平台上逐字节相同。
- R3 保留 Browser/Tauri 32768px 投影上限；menu17 的十亿像素 space 将作为
  `resource_limit` 安全拒绝差异，不能证明单位换算或列作匹配。已补 provider 拒绝回归、
  fixture 说明和验收性质；core 的 int32 单位/helper/累计溢出回归使用 synthetic ready。
  R4 typed 解码失败现转为 `ServiceFailure`，增加三个查询 × 五种坏 CBOR 与重复回包
  的实际 drive 回归源码，保持 RESULT/RESULTS 和后续语句 sentinel；尚未运行。
- R5 已落实真实授权资源后的受限 PNG 流、真实 Image.decode 阶段记录、挂起中重启取消
  与独立项目切换驱动；renderer 的 CORS 仅匹配已配置的精确测试 URL/资源。真实窗口
  失焦不受支持时仍为验收阻塞，不能用合成事件通过。R6 的精确 ready marker 用例已
  合入现有测试；完整快照保留历史，但两类 ledger 的累积记录不计为状态进展。
- 六项整改源码及相应用例均已落实；主智能体完成统一格式化（Rust 60 文件、Web 65 文件），
  未运行测试或构建。`1D/review-resolution.json` 记录逐项入口，`source-freeze.json` 记录
  首测前文件摘要。静态执行仅授权 gpt-5.6-terra / low；先 core 格式/check/Clippy/最小
  回归，随后唯一全量。core 契约提交与前端完整绑定完成后才进入前端静态门禁；所有
  必需静态通过前，禁止任何动态客户端或 Oracle。磁盘约 17 GiB，构建串行且关闭 incremental。
- `1D/command-matrix.json` 已记录静态顺序、单套全量及动态范围，尚未执行。监督器
  逐字节复用已验证的 C 工具，仅输出目录隔离；`.venv-capture` 使用已有 Python3.12.13，
  经批准仅补装 cbor2 6.1.4 与 blake3 1.0.9，不下载 Python/Chromium。四份待验证的
  原版/蛇版普通/危险 fixture 副本位于 `1D/paired-fixtures/`，保留参考配置并显式设置
  Rera font16/line20/window320/profile，项目 font 目录使用固定 4,486,740 字节字体。
  未将准备输入或依赖安装写成测试通过。

- D 首次格式门禁发现三处换行问题，cargo fmt 修复后定向通过；首次 workspace check
  因测试访问私有模块失败，改用现有 re-export 后 era-runtime all-targets 定向通过。
  首次 workspace Clippy 停于新增 HTML 查询模块的转换、导入、函数结构与文档 lint，
  后续已按下述定向修复。首次失败时没有最小回归、全量或动态已启动。原始命令、起止时间及 gzip 日志保留于
  `1D/validation/runs/`；首次失败与修复后结果分别记录，总测试时限仍取消。

- 首测时间为 2026-08-28 05:52:36 +08:00。HTML 的 checked 转换、显式导入、私有拆分与
  文档修复后，HTML 及依赖 crate 的 check/Clippy 均通过；格式门禁也恢复通过。运行时
  最小回归发现两条 fixture 使用了普通字符串赋值而非表达式赋值，以及一条重载测试
  错用 envelope 序号；分别修正后仅重跑三个失败节点，均通过。
- canvas 最小回归发现实际产品问题：REDRAW 0 时观察屏障只发布已有 pending frame，
  未 materialize 新绘图 replay。现显式观察先构建 stale replay，再发布；普通 flush
  行为不变。回归检查 Snapshot/Delta 中真实 canvas id/revision/Clear 命令先于 service
  请求，并验证 ARGB 返回。相关 check/Clippy、canvas、pointer、HTML 及原有 redraw/skip/
  replay 边界定向通过；坏 CBOR 的三种查询 × 五类输入回归也通过。该修复单独提交。
- core workspace 首次且唯一全量 `core-workspace-first` 退出 0，约 96 秒；没有再次运行。
  独立 runtime-tester 的格式/check/Clippy 通过，coverage 最小集首次 17 通过、1 失败：
  CHARADATA fixture 未启用 binary save，且报告用带维度的索引名查变量 schema。
  已修正 fixture 选项与报告中的基变量名/维度映射，格式/check/Clippy 与失败单例
  定向通过。随后工具首次且唯一全量 57 项通过（6.08 秒）；Python 捕获最小 12 项、
  首次且唯一 driver 全量 34 项通过。tester 与 C ABI dev 构建通过。
  前端门禁、真实客户端、双 Oracle 和真实蛇版 TW 新覆盖报告仍未执行。
- 磁盘不足 20 GiB 后保持单构建，关闭 incremental。仅清理本组停用 Web target 内
  30 个旧 runtime/bridge/native `.rlib`，逻辑大小 3,198,141,408 字节；逐项校验路径、
  inode、mtime 与当前构建归属。空闲从约 15.7 增至 18.5 GiB，记录为
  `1D/disk-reclaim-web-{plan,result}.json`。未删除 core 正在使用的 target、任何可执行
  文件、WASM/C ABI、批次 0 或 1C 冻结证据；没有下载 Chromium。


#### 1D 集成进度（2026-08-28，尚未完成）

- 发布契约 core `375e48d3d39f7f146a64edf580bd6648bcf21829`，两前端完整 pin 相同。
  core 分项提交见 `1D/commits/core-commits.json`；TUI 当前
  `93fd19b8c63491a95342e28dcb9f4c76be078af1`。产品版本未调整，无推送/主线合并。
  捕获期间暂保持 core HEAD，不提交后续 fixture/记录更正；这些更正按实际 source hash 与
  dirty 状态披露，不能被误称为已发布代码。
- 唯一重构审查仍为一次，R1–R6 均在首条测试前落实；没有重启审查或重跑已消耗的全量。
  所有测试由 gpt-5.6-terra/low 执行者只读执行。无测试总 deadline，保留单命令限制、
  静态先于相关动态及五秒完整状态看门狗。
- TUI 首次 pytest 480 通过、5 跳过。真实 C ABI 能力测试首次因 basetemp 父目录缺失而
  未执行任何参数；修复任务目录后仅定向 5/5 通过。源码库和打包库均完成实际 RuntimeWorker
  数据组合，GLOBAL:0=7、FLAG:0=55、C1_METHOD_VALUE:0=42；seed=123456。
  `1D/tui-dynamic-summary.json` 保留命令、trace 和库摘要，不能把原跳过描述为首次通过。
- Web 首次完整 Vitest 为 1209 通过、1 失败；mediaImage 测试卸载隔离修复后定向 20 通过，
  不称修复后全量通过。Web Rust 首次 130 通过、1 既有 ignored。类型、lint、格式、build、
  WASM、core pin 检查与适用 Rust 门禁通过。普通 cargo webdriver build 只证明原生 Rust
  宿主构建；真实 Tauri 还须另行嵌入当前 dist 并验收。
- Chromium 真实服务依次暴露并修复：WASM number/bigint 边界（`8e25729`）、相同 viewport
  重复观察误增 revision（`afd6ea4`）、sprite 尺寸/编码 canvas 字节边界（`69f847e`）。
  对应定向测试分别 81、302、155 通过，相关类型/lint/格式/build 恢复通过。
  fallback 字体下一半角单位不足以容纳 A 是正确 NoProgress；两个 client fixture 改用实际
  HTML_STRINGLEN 半角单位生成宽度，保留宽度回调副作用，专用 no-progress fixture 不变。
- DOM 诊断证明按钮已渲染，但逐字符 text-layout 的无障碍名称不等于完整脚本文本。
  `7b06413` 以规范文本提供 aria-label，`dd3584f` 修复测量用 HTML 节点的静态 ref 警告。
  158 项定向与相关静态通过，保留原精确 role/name 断言。
- 两套参考源码均由 PointingString 提供 MOUSEB，与能否提交按钮分开。
  Web `ce58c9ab2e39d91b26e8eb12ebd49bcc6a19498c` 移除错误的输入资格过滤，保留真实 hit test、
  epoch 和原点击门禁。fixture 在 INPUT 后 CLEARLINE 1 移除输入行以恢复实际 hover 几何。
  158 项定向及相关静态通过；两个更改后的 fixture 由正确 core-static 工具编译接受。
- `chromium-snake-services-repair-6` exit0：真实 role/name、hover/click、RESULTS:72="41"，
  HTML、canvas 与像素覆盖标记全部满足；`chromium-snake-batch1-first` exit0，数据和服务组合
  目标满足。此前失败逐条保留于 `1D/chromium-dynamic-summary.json` 和独立 trace。
- 首次 headful lifecycle 在挂起图片 race 处失败：驱动认为 canvas service 已提前回复。
  失败瞬间完整 DOM/runtime 诊断确认：当前请求记录 195 无回复，旧记录 35 因重用 epoch/ID
  被误匹配。`26547e4` 保留失败边界快照，`4fd100c` 增加实际前端会话代次和记录顺序匹配；
  271 项定向与相关静态通过后，repair-1 被五秒看门狗判停：旧解码已取消、新服务已完成，
  但所选 lifecycleGeneration 只随 teardown 变化，重启仍为 0，驱动无法区分。现改为在两处真实
  createSession 时递增独立观察代次，补首次打开/连续重启回归；`59fb408` 的 271 项与相关静态通过。
  repair-2/3 的前五项 pointer 和两个真实 held-image race 均通过内部断言，旧解码 cancelled 后
  新 session 服务完成，随后释放剩余 38/71 字节并观察旧解码 settled；没有旧 session 晚回复。
  整体仍 exit1，唯一阻塞为 trusted blur 未出现。`32cfe7a` 改为独立原生 Chromium 进程、
  default context 与 noDefaults 连接，避免 Playwright 默认焦点模拟；未注入事件或放宽断言。
  `6e623f0` 修复最小服务捕获缺少 remoteFS HTTP 路由的问题，使用真实 FileList 摄取；首次
  original 捕获判停、snake/adapter 尚未运行。两个脚本 syntax、137 项与 lint/format 通过，
  后续定向动态待完成。Firefox 生命周期另需有窗口模式，其余流程保持原来的 headless 设置。
- 固定原版、蛇版 smoke 各首次 exit0；蛇版 8 组共 66 请求通过。
  baseline 仍为原版 `26a35dc9334bb67590b96f7b8efbefbf199e391e`、蛇版
  `fc4fb21416768c17256d0e82f997e5f99c9bba91`，wrapper 与发布 EXE 摘要另见
  `1D/oracle-smoke-summary.json`。本阶段无参考仓修改。smoke 不是同输入差分。
  四个 original/snake 普通/危险 fixture 的 v3 静态报告均编译接受、0 error，
  `1D/oracle-fixture-static-summary.json` 记录实际输入摘要；136 个客户端服务捕获尚未开始。
- TW 覆盖执行者误用了旧 `target/runtime-tester-static`，首次 exit0 的报告实际是 v2 裸
  JSON，不能计入 1D。正确工具应为 `target/core-static/debug/rustyera-runtime-tester`，
  SHA256 `40519bd4475f22b77bcd5c59556b79f23afa11bf314e2be7a625a9f50df9a588`。
  首次 full claim 保留，正确重跑等待用户明确许可，不暗中重置次数。
  2,166,129,477 字节错误报告按原字节流压缩为 120,929,866 字节归档，核对解压 SHA 后
  经批准只删展开重复，见 `1D/tw-coverage/invalid-v2-archive.json`。
- 当前空闲约 16 GiB，保持单构建，不删除 B0/1C 证据、共享游戏、用户数据或其他任务产物。
  没有下载 Chromium，仅复用既有可执行文件，依赖/target/WASM/会话仍属于本组。
  尚需 Firefox/Safari/Tauri、完整生命周期、双 profile 服务差分、有效 TW v3 及最终提交记录；
  根 CHANGELOG_PENDING 尚未追加 1D 功能，批次 1 不标完成，不作真实标题/SQL/GRAPH_DB_INIT
  已运行结论。当前恢复入口为 `1D/delivery-notes-draft.md`、`dynamic-authorization.json`、
  `validation/runs/` 与每条独立 trace。


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

#### 1D 用户要求暂停（2026-08-28，本次验证已结束）

- 用户要求“运行完本次验证后先暂停”；两位执行者已经完成当前命令并停止，未开启新验证。
  只读进程检查没有发现本组 worktree 路径的残留进程。测试总 deadline 仍为 null，
  续做保留唯一审查和已消耗的全量次数，不重置批次。
- Web 当前 `49605f940643de967370ac9b30f31b020f2c693e`，工作树干净。
  新修复分项：`f19849e` canonical full manifest CBOR；`e1ce84d` transfer 前保存 bulk 摘要；
  `91de00c` terminal fault 前写完整快照；`6aeaea9` 全体 HTML 前缀在同一已就绪 DOM 布局中
  独立 shaping 并同步读宽度；`3e63d27` 覆盖真实 bridge 顶层 dataBytes；`49605f9` 捕获阶段日志。
  1D 初次完整 Vitest 的 1209 通过/1 失败不改写，后续全部为定向复验。
- repair16 初次定向 443/444，修正 Node structuredClone 的接收 realm 建模后 testingControl
  11/11；repair17 HTML/service/capture support 162/162，sidecar/runtimeStore 197/197。
  两轮相关 typecheck/lint/format/build 与脚本语法均通过。当前 dist 主包
  `index-DdDviIux.js`，WASM/core pin 未变化。
- Firefox services 定向 repair1 通过；Firefox batch1 初次因 full-prefix/whole-width 不一致
  失败，实际 payload 为 AAAA 整体 48000 millipixels、前缀 0/9000/18000/27000/36000。
  同一 DOM 布局修复后 batch1 repair1 通过所有数据、HTML、canvas、GLOBAL 标记。
  完整原始快照和精确请求/回复见 `1D/firefox-html-font-failure.json`。
- Firefox lifecycle 首次退出 1：resize 后 pointer #3 坐标吻合，但实际命中 INPUT，MOUSEB
  为空而驱动要求 41；两个挂起图片 race 尚未开始。下一步应修正驱动的真实滚动/悬停准备并
  观察命中目标，不修改 runtime 返回值来满足预期。Chromium trusted blur 仍未验收通过。
- Chromium 原版最小服务捕获 repair4 已完成真实项目身份导出，但比对 csv/GAMEBASE.CSV
  时失败：源文件 80 字节，download summary 的 UTF-8 payload 长度为 0。
  下一步核查 projectFileManifest 的轻量身份表示与实际导出 payload 的关系；未证实是产品
  数据丢失，不移除 hash/长度校验。adapter、蛇版最小捕获和后续服务差分均未启动。
- core 继续冻结 `375e48d3d39f7f146a64edf580bd6648bcf21829`，TUI 干净
  `93fd19b8c63491a95342e28dcb9f4c76be078af1`；两前端发布 pin 指向该 core。
  core 的已验证 fixture 更正和实施记录暂保留 dirty，避免在捕获链路中间改变冻结身份；
  暂停清单逐文件记录摘要，恢复后先核对。未增加未验证产品代码，不推送、不合并、不改产品版本。
- 暂停时磁盘可用约 15 GiB，1D 证据约 580 MiB。保留全部 trace、fixture、专属项目副本和
  失败报告；没有下载 Chromium，也没有删除用户数据或批次 0/1C 证据。
  `CHANGELOG_PENDING.md` 本段未追加 1D 功能，最终验收完成后再汇总。
- 恢复入口：本组 `batch-1-work/1D/pause-20260828.json`、`delivery-notes-draft.md`、
  `repair16-acceptance.json`、`repair17-acceptance.json`；继续未完成的三浏览器/Tauri、双 Oracle
  服务差分及有效 TW v3 覆盖。TW 首次误用旧工具的记录保留，重跑例外仍待用户明确同意。
  批次 1 未完成；SQL 和缺失 bbas_map_schema.xml/bbas_map.xml 仍是后续明确阻塞。

#### 1D 恢复与 repair18（2026-08-28）

- 用户恢复执行并再次要求管理磁盘；核对暂停清单中的全部 core dirty 文件摘要一致，
  Web/TUI 基线未变化。空闲约 16 GiB，保持单构建，10 GiB 以下停止新增高写入任务。
- 已确认 capture 的零长度来自 `decode_project_file_frontend_manifest` 的既有 compact
  行为，并非已证实的项目导出数据丢失。Web shared bridge 新增只读、有界的实际导出
  identity 观察：复用完整项目 decoder，逐 payload 校验 BLAKE3 并返回长度/摘要，无源码
  或资源字节回传。Browser 经 WASM Worker、Tauri 经 native blocking command 调用同一实现，
  仅测试诊断记录使用；不改变 core 协议、C ABI、产品版本或前端 core pin。
- 生命周期驱动在目标 hover 前显式执行真实 scrollIntoView，避免 resize 后 WebDriver 将
  指针移到被 INPUT 覆盖的元素中心；不修改 pointer 返回值或降低坐标/脚本值断言。
- 161 个 JS 定向用例及 typecheck/lint/format/脚本语法通过；Rust workspace check、Clippy、
  最小真实项目导出回归通过。WASM 与后续动态验证进行中，不能据此标记批次完成。
  唯一审查、首次全量及失败记录全部继承，未重置；没有下载 Chromium。


#### 1D repair18–20 定向验证进展（2026-08-28）

- repair18 的 WASM/build/core-rev 均已通过，Web `488aa44` 保存实际导出 payload identity，
  `e87b30d` 修正生命周期 hover 前滚动。Firefox lifecycle repair1 exit0，含真实失焦和两个
  挂起解码换代场景；大 stdout JSON 被 driver warning 插入，保留原始记录，不宣称它是完整
  可解析报告，后续原生报告需单独持久化。
- Safari services 首次因真实输入框已填写 `1` 但 pointer submit 未推进而被原有看门狗终止。
  `cc1ef9b` 复用已有 Safari 原生 Enter 提交，repair1 exit0；Safari batch1 首次 exit0，
  摄取/动态方法/资源/MAP/XML/DT/GLOBAL/HTML/canvas 组合标记均达成。
- `7ecca52` 补充 typed inspection 失败现场记录。capture repair6 证实同 epoch2、wait7 下，
  BigInt debug stop 被 JSON.stringify 拒绝，停留 debug_paused；并非游戏流程越过终点。
  主体修复以精确整数比较全部 stop 字段，并对测试观察返回值复用 JSON 安全序列化。
  定向行为回归已运行；首轮类型检查的测试 mock 错误已记录，修复后门禁仍在进行。
- repair19 的 49 项定向测试及全部相关静态门禁通过，证据位于 `1D/repair19-static.json`；
  动态证据索引 `1D/repair19-dynamic-summary.json`。未重跑全量、未重开审查、未下载 Chromium。
  1D 尚未完成：Tauri、其余生命周期/真实采集差分及 TW v3 覆盖例外授权仍待处理。
- 当前磁盘约 23 GiB 可用，继续串行构建；未清理任何用户数据、批次0证据或续做材料。

#### 1D repair21–24：证据整数与覆盖看门狗（2026-08-28）

- Web `79ed2eb` 保留精确 BigInt debug stop 身份并安全返回测试观察，`af1c545` 复用 Safari
  原生输入提交。repair20 最终 188 个 runtimeStore 定向用例及 typecheck/lint/format/build
  均通过；此前两次类型失败保留，未重跑 workspace 全量。
- Python adapter 分别修复合法十进制版本号、JS UTF-16 完整路径排序与 Path 分段排序差异、
  typed reference 的整数表示及省略/null 可选字段等价。原始协议证据不改写，错误 index、
  generation、fiber 仍拒绝。repair21/22/23 定向测试分别 14/14/15 通过；最初错误 Python
  环境缺 blake3 的失败单列，不当成产品问题，也不删除失败证据。
- Chromium `s04-empty-lazy` 两个 profile 均完成真实 capture、adapter 校验及固定 seed123456
  的同输入 Oracle 差分：原版、蛇版均 `matched_observables`，ok/termination/output/watches/
  diagnostics 无差异。原版 adapter SHA256 `95903d3e06221ce4f208dd404b1e4945d22e478cdb428dca99efb239df259840`，
  蛇版 `af429ea0eb83250bbba47e6b3b6c0da999f532a0e76c654dbfb49a5ae6b57584`；证据在
  `1D/oracle-original-empty-first/`、`1D/oracle-snake-empty-first/`。仅一个用例，不代表服务矩阵通过。
- 上次获准的 TW v3 全量重跑使用正确 tester `40519bd…`，115.46s exit2：最后状态为
  `appearance_parse`、`PLAY_GOMOKU.ERB`、5,068,424 appearances；连续两次 5s 状态相同，
  看门狗正确执行终止，未生成完整报告。首次旧 v2 工具报告和此次失败分别保留；没有用重命名
  批次恢复次数。用户随后仅有条件允许再跑一次：先证明能解决该看门狗终止，避免重复浪费。
- 排查发现排序阶段沿用最后一个文件的解析状态。repair21/24 在相同稳定排序比较器中每
  16,384 次真实比较发布状态；补齐标题/GRAPH_DB_INIT 断面的索引、引用与边遍历进度。
  不改变排序键、稳定同键顺序、报告字段或 watchdog 的 5s 相同即失败规则。
  流式报告已有实际成功写入字节进度，未改用计时心跳。新增显式 ignored 压力测试使用生产
  watchdog 父子进程、5,068,424 条乱序记录、100,000 条断面引用，不落地大报告。
- repair24 fmt/check/Clippy/build、19 个 coverage 定向测试、5 个 watchdog 测试均通过。
  首次压力主体通过：100,823,044 次比较、排序 14.39s、引用图 1.15s、完整键稳定顺序正确；
  三个 5s 排序样本实际比较数持续增加。外层 `/usr/bin/time -l` 因系统资源读取权限返回1，
  该命令仍记失败，正以审批后的同一定向命令补齐峰值内存记录；TW 全量尚未释放执行。
- 下游最小断面保留真实 TW 的 206CSV/20ALS/2ERD/169ERH 和最后一个 ERB；只有约1MiB。
  原游戏没有 reraconfig.toml，隔离副本复用既有最小 fixture 的显式 snake 配置并注明来源。
  此断面用于验证数据加载、声明分析及报告生成，不宣称完整游戏可编译或初始化成功。
  恢复材料为 `1D/coverage-post-sort-minimum-provenance.json` 和对应目录。
- core HEAD 仍为 `375e48d`，前端 pin 不变；上述工具及 fixture 改动尚未提交，保留真实 dirty
  身份。没有修改参考仓库、游戏、产品版本；没有下载 Chromium。当前约21–23GiB可用，
  继续单构建，不删除批次0或暂停续做证据。1D 仍未完成。

#### 1D repair24 全量例外的结果与 repair25 定位边界（2026-08-28）

- 经审批的压力资源记录命令 exit0：同为5,068,424行、100,823,044次比较，本次排序28.98s、
  引用图2.79s；六个连续5s排序样本均推进。`time` maximum resident set size 为2,510,848,000B；
  peak memory footprint 字段24,625,776B的父/子进程统计范围不同，不当作总内存，也不把该命令
  swaps=0解释成系统未使用swap。上一条外层exit1的压力结果仍单列。
- 399文件断面11.51s exit0，CSV accepted、9,503 rows、0输入错误；有109项分析错误，编译
  被加载诊断阻断。gzip 757,261B，原始JSON13,959,621B，completion manifest齐全。
  新的忽略工具 `1D/summarize_coverage_stream.py` 语法和此断面定向验证均通过，使用ijson3.5.1，
  单遍流式核对压缩/原始SHA256、BLAKE3和字节数，不展开大JSON；没有读取0B的TW失败产物。
- 主智能体核验八项前置结果后释放用户附条件的第二次例外，使用tester
  `1f40d3c756116d41a119b1a13f5e2389595cea7546a811bfcd2b78319a8f4e05`。
  `tw-coverage-user-authorized-repeat2` 200.50s exit2：完整排序完成（5,069,815条，比上一轮
  最后解析开始时的计数多出最后一个ERB的1,391条），随后分析计数到达112,727/112,727，
  连续两个5s观察相同而终止。stdout SHA256
  `b91b3dc96d4d15c3ccc3300dd63c14bc32d8acd5895aee110c61b8164d5868f0`。
  仅有0B gzip，无manifest/Markdown；绝不标记覆盖报告通过。证据
  `1D/tw-coverage-authorized-repeat2/summary.json` 与原始validation目录保留。
- 排序修复得到验证，但前置断面未覆盖11万函数的分析收尾，不能证明后续全部阶段都有进度。
  本次授权已经消耗，不再自动全量重跑，也不以改名批次恢复次数。磁盘约21.85GiB可用，
  未触及容量停止阈值，因此此次失败不能归因为磁盘不足。
- 源码显示 `Analyzing` 完成之后还有可移植性固定点、诊断排序、AST释放；返回后还有工具符号
  投影。现有失败快照无法确定具体阻塞在哪一步。repair25仅补上工具的`analysis_returned`
  边界及真实已投影符号计数，增加112,727函数规模的显式ignored定向压力；产品crate和公共
  协议未改。该压力只验证符号投影，不冒充分析收尾修复。repair25 fmt/check/Clippy/build、
  pipeline 4项、report 4项均通过；112,727函数投影1.733s、监督命令2.51s exit0，名称完整且不含
  HIR body。该小于5s的投影运行没有跨越采样周期，不宣称它证明整个分析收尾可持续推进。
  新工具SHA256 `b39f7b3fd33164524a03c182980f4547776fb5fbff1678c91fb28f2923810189`，
  证据 `1D/repair25-targeted-summary.json`；不得将它与上一轮全量使用的`1f40d3c…`混淆。
- 1D仍待分析收尾诊断、有效TW v3报告、Tauri、剩余生命周期与服务差分矩阵。根CHANGELOG
  未追加这些工具/测试/流程改动，前端pin与core HEAD仍保持375e48d；无推送或合并。

#### 1D 用户再次授权 TW 全量（repeat3，2026-08-28）

- 用户在上一条失败说明后明确要求“在尝试重跑一次全量”，本次仅增加一次TW v3覆盖例外，
  不扩展其他全量次数，不重开审查。使用repair25已通过门禁的tester `b39f7b3f…`，
  未将分析收尾根因描述为已修复。开跑前核对门禁结果文件摘要、二进制SHA与专用worktree，
  磁盘约21.82GiB；单任务执行，直接gzip输出，不展开大JSON。
- 命令及授权分别保存在 `1D/tw-coverage-repeat3-command.json`、
  `1D/tw-coverage-repeat3-authorization.json`；独立证据目录为
  `1D/tw-coverage-authorized-repeat3/`。5s完整状态相同即失败的规则不变，失败不自动重跑。
  若再次停留在函数分析终点，尝试旁路采集实际进程栈，不延迟看门狗终止。
- 本次252.35s exit2。已确认分析API返回，越过变量投影（总数480,574）并完成112,727个
  函数条目的投影；最后状态为`coverage_symbol_projection/functions 112727/112727`，
  连续两个5s观察相同而终止。具体停滞操作仍未由快照证明，但范围已缩小到函数条目投影后、
  graph/rows之前，不再把它表述为分析API内部未返回。
- stdout SHA256 `89bd0e5629ae9e4f1eb07883a6e7d4cdd962afcf99cef6db57987a0935cddf8a`。
  仅留下0B gzip，无completion manifest或Markdown，故未运行流式完整报告校验。旁路栈采样
  未触发：约定条件是分析终点后未返回，而本次越过该边界；没有伪造栈定位结论。
  `1D/tw-coverage-authorized-repeat3/summary.json` 保存实际结果、快照路径与压缩日志信息。
- 此次授权已消耗，未自动重试。结束时约21.81GiB可用，容量未触发停止；代码输入未变，
  本轮仅追加实施/证据记录，未新增提交或更新根CHANGELOG，批次1仍未完成。

#### 1D 加载阶段看门狗规则调整及 repeat4 授权（2026-08-28）

- 用户要求“项目加载阶段放宽为4个5s周期不变才退出，之后再次重跑tw全量”。本次将 core
  审计工具的明确加载阶段改为连续4次5s完整观察相同才终止：文件摄取/hash、解析/排序、
  CSV、分析/编译、符号准备，以及真实runtime LoadingProject/Reloading/ProjectProgress。
  首次相同观察计为1；有真实进展或阶段变化则重置。非加载与未知阶段仍为2次。
- 观察快照与采样频率不变，策略/计数日志独立于实际观察，不能用它掩盖停滞；进入报告图
  准备时显式切到非加载阶段，避免继承加载宽限。此调整未改变Browser/Tauri周期规则，
  未修改产品crate、格式协议、版本、core pin或已有测试总时限。
- 在现有watchdog测试中覆盖第4次终止、真实进展重置、request id不能重置计数、离开加载
  恢复2次及未知阶段保守处理；增加显式ignored父子进程测试，实际观察20秒无进展后终止。
  仍沿用1D唯一审查及原全量结果；先通过相关静态和定向看门狗验证，再启动此次获准的唯一
  TW全量repeat4。所有旧失败材料保留，构建串行，gzip直写并监控10/20GiB阈值。
- repair26 fmt/check/Clippy/build、watchdog 7项、report 4项通过；真实父子负向回归监督命令
  20.35s exit0，证据确认为同一状态的`1/4 → 2/4 → 3/4 → 4/4`后终止，而非伪造进度。
  工具SHA256 `f22a4759ff4513e520b9cb3dce32698af7be4a5e90f90d955ed3e0c268c64fad`；
  `1D/repair26-watchdog-summary.json` 保存门禁摘要与实际策略样本。
  前置结果核验后释放 `1D/tw-coverage-repeat4-authorization.json` 的唯一授权，命令在
  `1D/tw-coverage-repeat4-command.json`。
- repeat4 `tw-coverage-user-authorized-repeat4` exit0，耗时2012.55s（约33分33秒）。完成
  manifest确认JSON闭合、gzip结束及目标flush：原始4,668,806,077B，gzip150,730,066B；
  最后进度中的150,730,047B尚未包含完整gzip收尾，不能替代最终文件长度。
  原始SHA256 `b4dd4441e7f0e731fd1434fecfb54a24f3f706640b4a54ab6960f362a57700db`，
  gzip SHA256 `d365704edff41c8e47b3179b52db8a7b0239d469d001765efa337f9d9d45c88b`。
  独立流式解析/摘要核验另行记录，不能只凭退出码认定内容验收完成。
- 主智能体复核全部403条完整观察：函数符号112727/112727处确有一次`2/4`，随后进入
  报告准备并持续推进；非加载阶段相同计数最大为1。旧规则会在该`2/4`处终止，新规则
  本次允许继续；专门父子进程负向回归则证明`4/4`仍会终止。实时汇报遗漏了该`2/4`，
  因此未取得现场调用栈，不能将“未采到”写成“没有触发条件”。完整日志及更正证据在
  `1D/tw-coverage-authorized-repeat4/watchdog-observations-summary.json`；日志原始SHA256
  `751f972ad703c8e34528da82d3f134bbbffd83b7ef67d1cc376ec881910172fd`。
- 随后的 `tw-coverage-stream-repeat4` exit0，89.49s：以ijson完整解析到EOF，验证gzip尾部
  及原始/压缩SHA256、BLAKE3和长度，未落盘展开副本。v3报告保留5,069,815条记录、
  112,727个函数、44,431条目标解析及title/GRAPH_DB_INIT静态断面。15,761项输入中
  ALS20、ERD2、CSV206、ERB3931、ERH169、Resource11337、ResourceManifest96，读取
  错误0，另明确排除901项。原始/压缩BLAKE3分别为
  `3fd9c56657263cdea7ec12ce1f3b3ceab3dbd6de0b87460335ba4be6036d6d40` /
  `afd28f320f201b9336fb09888123c00ff3b1e0239fd1a03dba79c8820c2a3978`。
  可读摘要和清单在 `1D/tw-coverage-authorized-repeat4/summary-verified.json`，流式核验
  命令/结果在 `1D/validation/runs/tw-coverage-stream-repeat4/`。
- 结论仅为完整静态覆盖报告生成与校验成功：CSV accepted；analyzer仍有8601项诊断，
  其中8198项error，compiler为`blocked_by_load_diagnostics`，未产生字节码。该报告没有
  执行游戏或GRAPH_DB_INIT，也没有绑定前端执行capture。索引摘要只见COLUMNDIV@2的
  已解析条目；BUFF/SEMEN_MATRIX未出现在选定resolved摘要中，不能用完整摄取代替其
  真实项目索引解析通过结论，留待按原始报告和已有最小fixture定位。
- 本次授权已消耗，不再自动重跑。末次流式核验后约22.67GiB可用，原始JSON只在流中
  处理；保留gzip、manifest、摘要、失败历史及续做材料，未清理B0或用户数据，未下载
  Chromium。core仍为`375e48d3d39f7f146a64edf580bd6648bcf21829`，Web/TUI仍为
  `af1c545f0842918fb8a586800dbb987b78f5a1df` /
  `93fd19b8c63491a95342e28dcb9f4c76be078af1`。为保留1D尚未完成的capture绑定，暂不
  推进core HEAD；本次工具/记录改动随1D分项提交收尾，产品crate/pin/根CHANGELOG未改。
  1D其余客户端、Oracle差分和批次1集成门禁仍未完成，本次验证结束后暂停，不开展新全量。

#### 本批验收后：TW验证流程与输出优化待办（2026-08-28）

- 用户要求等本批次验收完毕后再考虑优化；当前仅登记，不修改1D验收输入、报告格式、
  看门狗或门禁，不恢复测试，也不因此增加本批全量次数。
- 基线为repeat4：覆盖报告2012.55s，随后独立完整性核验89.49s，原始JSON4.67GB、
  gzip150.73MB。403条5s观察中，328条处于`coverage_write_report`、5条处于
  `coverage_rows`，约八成观察在报告生成阶段，粗估27–28分钟。采样不能精确区分
  序列化、诊断关联、压缩、hash或磁盘I/O耗时，不能据此声称已找到具体热点。
- 验收后先做分阶段计时和资源测量，再评估重复数据/序列化、flush频率、压缩参数与
  缓冲策略；报告生成时同步产出小摘要，评估避免重复扫描大报告的方案，同时保留独立
  完整性检查。日常定向回归与完整审计的输出需求分开设计，完整审计仍保留全部证据，
  不以删记录、抽样、忽略诊断或放宽看门狗换取提速。
- 比较指标包括总耗时、各阶段耗时、峰值内存、原始/压缩字节、磁盘峰值及报告语义等价；
  输入与工具身份必须可核验，缓存失效和冷/热路径分别验证。沿用本任务磁盘隔离及
  10/20GiB阈值，不保留多份展开报告。尚未实施，不预先承诺提速比例。

#### 1D 验收恢复（2026-08-28，repeat4之后）

- 用户要求继续完成批次1，恢复原1D验收，唯一审查及既有全量次数不重置。确认三仓仍为
  同组分支：core375e48d、Webaf1c545、TUI93fd19b；产品crate/pin没有新增变化，磁盘
  约22.6GiB。复用本组产物和已安装Chromium，原生窗口测试串行，不下载浏览器。
- 按既有repair20/23/26及原门禁恢复剩余生命周期、Tauri、真实服务采集/双Oracle差分，
  并核清TW索引断面；不再运行TW全量，不提前实施已登记的验收后性能/输出优化。
- repair27仅修复原生生命周期报告的证据完整性：Firefox此前通过但大stdout被driver
  warning穿插，改为复用现有CaptureWriter写独立gzip，stdout只给摘要/摘要hash。没有
  删除观察或改变断言/看门狗，不影响产品bundle；先执行现有writer/runner定向回归及
  Node语法、lint/format，恢复相关门禁后再做Firefox定向复验和Safari首次生命周期。
- repair27的79项及Node/lint/format通过；Web分项提交`acc4b27`。Firefox生命周期定向
  12.86s exit0，独立gzip355644B/解压8058927B，SHA256分别
  `72087d4e8d1d8dd44a482c7a8b8e8d7b7ac74ea8708b92f1c2cb998fbd604501` /
  `0ed5b9ac5053d6f0693f9fb0152b41ffdc8f45111317602835f7ed19b7f3b349`；6组pointer、
  trusted blur清零及两个真实挂起解码换代均通过，blocked为空、显示canvas数量0。
  Safari生命周期首次7.10s exit1，在“blur后新pointer事件前清零”断言失败，未进入两组
  race；不改写为通过。repair28只补充原生失败即时完整快照及真实DOM事件顺序，保持
  清零断言和产品不变，先恢复相关静态门禁再采集定向失败现场。
- TW索引问题闭环：已验证报告的实际`system_save_in_binary=false`，BUFF和SEMEN_MATRIX
  两条真实CHARADATA声明恰被`user-defined CHARADATA variables require binary saves`
  拒绝；工具明确不推断legacy配置，而真实`emuera.config`启用binary。不是漏摄取或
  resolved投影过滤。原始选项、精确span及诊断在
  `1D/tw-coverage-authorized-repeat4/selected-index-audit-options.json`。
- `1D/tw-index-configuration/`保留真实CHARADATA声明、常量200、BUFF主表/ALS及两份ERD，
  仅对照binary=false/true。首次小断面遗漏已启用的_Rename.csv导致验证失败，保留原结果；
  补齐真实_Rename/_Replace后两份小coverage和严格校验均exit0。false仍拒绝两声明；
  true得到BUFF Character[50]、SEMEN_MATRIX Character[200,4]、COLUMNDIV[10,20]及三个
  索引，alias/主名和ERD第二维断言通过。该断面可编译但未执行游戏，不覆盖其他legacy
  选项，也不把TW原8198项审计错误当成真实游戏配置下的错误总数。provenance、两次
  报告及`acceptance-summary.json`保留，没有重跑TW全量或修改产品/原始游戏。


- repair28 的79项定向及语法/lint/格式通过，Web提交`6d37865`。Safari诊断复验保留
  失败完整gzip，确认前五组pointer和两组解码race已执行，但独立焦点窗口未创建，整条
  命令仍exit1。随后repair29改用WebDriver原生createWindow返回的精确handle，不再
  依赖高层window.open与无序句柄列表；未改产品、清零断言或快照规则。新增3项使相关
  定向共82项通过；首次lint仅因遗留globals注释失败，清理后定向通过，其他相关门禁通过。
- repair29首次Safari复验在pointer采样前失败，未走到新窗口验证；同时启动过无头Chrome，
  是否存在macOS焦点干扰尚无证据。恢复时确认无残留测试进程，改为串行原生会话。
  Chromium原版`s04-first-row-half-units`已采集完成；首次adapter误把配置JSON当作客户端
  可执行文件而拒绝hash。保留失败记录，纠正clientArtifact参数后离线定向exit0，未重新
  采集或放宽校验，adapter SHA256为
  `c41e25d8ac2a96b6e7a32a3de27bd3726b70647056b810154e98224484cd0361`。
  此结果仅验证观察证据，尚非Oracle差分通过；snake对应case尚未启动。磁盘恢复时约24GiB。


#### 1D 用户暂停（2026-08-28，原生验收续做中）

- 用户要求“先暂停”，停止全部后续测试和实现；检查无本任务残留构建、浏览器或Wine进程。
  磁盘约23.8GiB，未清理任何续做材料或批次0证据。当前core375e48d、Web6d37865、
  TUI93fd19b，三端pin仍375e48d；本次续做没有新提交、推送、合并或产品版本变更。
- 原版`s04-first-row-half-units`单case Oracle首次差分exit0（26.20s），输出、termination、
  watches与空diagnostics为matched_observables。evidence SHA256
  `346bb1c4997be22a6c29ce516dca1b8ce9fdd317a5fc5cfff3e08a3e18e10935`。
  snake同case及其余服务矩阵仍待执行；没有重跑TW全量或提前做验收后性能优化。
- repair30修正WebDriver createWindow的字符串参数；相关82项与语法/lint/格式通过。
  随后Chrome失败现场显示：两次trusted blur后确实发生新的可信pointermove，非零值
  不能据此判为旧状态泄漏。repair32按Enter前的事件序列和几何决定期待值：无新pointer
  仍严格要求0/0/，有新pointer则必须trusted/focused且返回其新坐标；取消/离开恢复清零。
  同时先移动到独立位置再hover，避免已有OS位置不产生事件；失败保留完整事件现场。
- repair32的84项及语法/lint/格式通过；Chromium lifecycle定向repair7 exit0（16.80s），
  六组pointer完成，本次第六组实际为cleared-after-blur、blur=2、0/0/，两种真实挂起
  解码取消/换代均完成，blocked为空。result SHA256
  `2cd5eb2b02f9bf8f3fcf646d2a65c0f851b67b9aaf221435a86da9e5f938c8b2`。
  Safari随后exit1（12.59s），五组pointer及两race完成，但新窗口未实际得到focus，
  不计完整lifecycle通过。历史失败及完整gzip均保留，不能用此前Chrome结论代替Safari。
- repair33在独立焦点窗口中导航到固定DOM探针并经WebDriver可见button点击取得焦点，
  不合成DOM事件、不放宽trusted blur或5秒看门狗。84项、两项语法、lint/格式均通过；
  用户暂停前尚未运行这版动态。当前三个Web dirty文件仅为上述驱动和测试修复。
- Tauri本地provider1.2.0源码确认newWindow不支持、switch仅换上下文，键鼠使用JS事件，
  不能据此证明真实pointer/blur。隔离草稿`1D/tauri-native-provider-source/`仅修改依赖侧
  四个文件，用当前WKWebView所属NSWindow投递AppKit事件，提供受限原生窗口/焦点；
  不更改产品unsafe规则、全局registry或运行时语义。主格式化及rustfmt检查通过；
  Cargo check/Clippy/unit/build均未启动。集成草稿尚未合入Web，可信事件必须由最小真实
  WKWebView探针证明，不能把源码或格式检查写成验收成功。
- 暂停清单、源hash、已完成命令和恢复次序在
  `1D/pause-2026-08-28-native-acceptance.json`；repair33静态记录单列。恢复后先核对源与
  静态门禁，再串行处理Safari焦点和Tauri provider；随后完成服务矩阵、最终提交及整批
  验收。批次1仍未完成，根CHANGELOG_PENDING本次未改。

#### 1D 再次续做（2026-08-28，原生输入验收）

- 恢复后核对暂停文件摘要、三个 worktree 与 core pin，未发现输入漂移。继续复用已有
  Chromium，不再次运行已完成的 TW repeat4；批次 1 仍未完成。
- 当前 repair33 的 Safari 定向生命周期通过：六个 pointer 样本、真实 window blur 后
  `0/0/` 清理、两种异步解码取消/项目切换竞态。证据
  `browser-compat-safari-1787902768683/lifecycle/capture.ndjson.gz`，压缩摘要
  `ec05ca878609668760839f610626e248922937dc521939274d3333e5d8ed6b61`。
- 同输入 Chromium 定向通过，六个 pointer 样本及两种竞态均满足断言；
  `1D/validation/artifacts/chromium-snake-lifecycle-focus-control-repair8/result.json`
  摘要 `fe4da000d9f6c86ed176526edfd980050c4986e71939d405ecae8376a42adb3f`。
- Firefox repair3 在第二个 pointer 样本失败：请求后出现额外 trusted pointermove，
  预查询坐标与执行查询时坐标不同。保留失败采集，不放宽坐标断言，不据此认定产品错误；
  已请求短暂安静输入时段后再定向复验。原有通过记录只代表当时驱动输入。
- Tauri 测试专用 provider 的宿主 check 通过；独立 manifest 的 Clippy 首次发现上游两处
  integer cast API 不符合其 Rust 1.77 声明。用等宽 `as` 转换修复，保持数据与接口不变；
  修复后 Clippy 与两项 native key 回归通过。命令选择和格式失败分别保留，不混作成功。
- 将带摘要清单的 provider overlay、显式 runner 选项和真实 WKWebView 输入探针接入 Web
  测试工具。普通构建、发布锁文件与 core pin 不变；这些代码不是原生行为通过的证据。
  既有 `tauriTestSupport.test.js` 定向 68 项通过；探针 lint 发现并修复重复 globals 与
  finally 抛错问题，后续静态复验及真实宿主构建/输入探针仍在推进。
- 继续使用本组 target，关闭 incremental，串行 Cargo；本次观测可用空间约 22–23 GiB。
  未删除批次 0、失败采集或续做材料。精确命令、退出码、gzip 摘要见
  `1D/validation/runs/` 与 `1D/resume-native-acceptance-progress.json`。

#### 1D pointer 边界事件修复（repair35，2026-08-28）

- 用户授权安静输入后，Firefox repair4 在 index4（滚动）失败：实际 `99/-85/`，预期
  `580/-294/`。采集明确显示新 `pointermove/down/up(580,332)` 后出现滚动引发的
  `pointerout(99,541)`，且 relatedTarget 存在。前端 `outside` 将旧边界坐标重新写入
  pointer position；这次确认为产品缺陷，而非沿用上次“额外移动”的推测。
- 修复仅让 move/down/up 更新坐标；元素间 pointerout 不改写坐标，离开窗口、cancel、
  blur 仍清理。扩展既有 runtimeServices 回归，覆盖滚动后的旧坐标与失焦后的边界事件；
  生命周期测试也不再将单独的 pointerout 当作失焦后新移动。断言仍使用实际预查询几何，
  不放宽误差或删除滚动场景。定向两文件 150 项、typecheck/lint/format 均通过。
- repair34 原生测试宿主构建成功，发布锁摘要仍为
  `6fe3a08d45b45542dcab8e29a0f6b97530ad6c7a562aed70d10c317edb239a70`；其二进制摘要
  `156dbdfa30bc4932a32690854d1c75c92bc69d4a22f07db1d81419d65f983145` 只对应修复前
  输入。待该构建退出后才应用 repair35；修复后单独重建，不混用旧产物作为新输入证据。
- 用户随后要求使用电脑期间暂不运行需要静止的测试。暂停 Firefox/Safari/Tauri 的
  焦点相关动态验收，不暂停开发、静态检查及不占用桌面的 headless 测试；真实客户端
  生命周期仍待授权后定向复验。未再次启动 TW 全量，也未新增重构审查。

#### 1D 错误现场观察阻塞（repair36，2026-08-28）

- repair35 重建通过，二进制摘要
  `0f163981c00a5a05d52976869531af190797fd64b58f6e6b811f09a18bc99e98`，core 仍为
  `375e48d3d39f7f146a64edf580bd6648bcf21829`，发布锁未变。静态结果不能代替尚未执行
  的真实 WKWebView 输入探针。
- 继续不占用桌面的 Chromium headless 采集。蛇版 first-row capture 与 adapter 通过，
  adapter 摘要 `5cac183ea02290c8280926b00f0554e350df8ee711e93667caa26583ce5203fb`，
  状态仅为 `validated_observations_not_comparison_verdict`，尚无该项 reference 差分。
- 下一项 original canvas-invalid-dimensions 正确进入 fault，但 typed observation 无法
  取得 `RESULT:10`。capture exit2，保留 `captured_with_observation_blocks`，未执行其
  adapter。队列首错停止，后续 59 条命令未启动；不把成功保存 capture 文件当作通过。
- 根因同时位于 frontend 的 pause 阶段过滤和 core 的 `DebugCommand::Pause` 阶段限制：
  faulted 不可建立 stop。延长 timeout 无效；删除 watch、填入预期 777 或只观察异常前
  值均不能证明错误后的副作用。正在沿用现有 grant/stop 协议补齐故障现场只读观察；
  结束观察必须恢复 Faulted，不能重启脚本或修改现场。该修复属于本次错误差分阻塞，
  不新建批次、不重置全量次数、不新增重构审查。
- Web 请求路径与 typed stop 测试已更新：定向 describe 29 项通过，另 160 项因明确
  name filter 未选中；typecheck/lint/format 通过。core 实现与协议回归尚在进行，
  之后先完成验证和契约提交，再同步三端 core pin/锁/本组产物。当前停止新 capture。
- 固定参考源码只读核查表明剩余服务脚本本身不读取桌面鼠标/焦点，但 Wine 启动层没有
  证明不抢焦点的证据。用户使用电脑期间仍不启动 Wine；不以 headless 名称替代证明。

#### 1D repair36 契约提交与恢复桌面验收（2026-08-28）

- core 故障现场只读观察已经提交为 `b8b5bee45d1a7d3fc31f4df42dcbe0048422794a`。
  workspace fmt/check/Clippy 及 7 项定向回归通过：4 项 postmortem、1 项 revoke、2 项
  safe console。测试确认异常后读取 `RESULT:10=777`，拒绝写入/步进/执行，Continue
  或 revoke 后仍为 Faulted，不执行故障后的语句；未重跑全量。
- Web/TUI 的完整 core pin 及 Web Cargo Git source/锁已机械同步；不改变依赖版本图或
  C ABI/协议布局。构建与两 host 契约验证仍在推进，不能使用旧产物作为新 SHA 的证据。
  当前发布锁 SHA256 为 `f164ec73c5e0846673d42fdf55cd536e1861ce935e9ac2b269019a1021b16002`。
- 用户随后允许恢复需要鼠标的测试。待当前绑定静态门禁通过后，串行复验三个浏览器
  生命周期并执行 Tauri 原生输入探针；不会并发占用桌面，不自动重跑 TW 全量。
  仍复用已安装 Chromium。命令与新证据位于 `1D/*postmortem*` 和 `validation/runs/`。

#### 1D 离线比较器握手前缀修复（repair37，2026-08-28）

- 为复用已有 reference 观察核查离线比较器，发现其只接受 load 开头，但当前真实记录为
  capabilities→load→run。修复仅接受可选的精确 capabilities 前缀，校验能力版本和成功
  状态；保留全部消息的 schema/baseline 校验，并拒绝重复握手、缺失或额外步骤。
- 既有测试文件内 2 项定向回归通过；随后使用现有 original empty/first-row 和 snake
  empty 的不可变证据进行 3 次离线重比较，均为 `matched_observables`。没有重启参考
  引擎、没有更改旧 capture/adapter 身份，也不新增三端动态覆盖。
- 首次 reference 仍有 31 项未运行；完整清单与前置条件见
  `1D/first-reference-queue-repair36.json`，无进展 hazard 单列，不伪装成成功结果。
  以上证据及摘要见 `1D/recompare-prefix-repair37-*-evidence.json`。

#### 1D 构建结果与终端回报规则更正（2026-08-28）

- 新 core 绑定的 Web revision/fmt/check/Clippy、bridge 定向 3+1、WASM 和原生宿主构建
  均有正常落盘结果。WASM SHA256 `bbe455923aca722a49c8f4dde3cc35498393455eae7f3a301b2f9d9d439205bf`；
  原生直接 Cargo 目标 SHA256 `1d42f2d831152d157759e6a438939d5f4a3d3db40b2e7753ad110186c286ba8e`。
- 主智能体因外层 PTY 退出回报被回收，重复了本来已有 exit0/Finished/产物证据的构建，
  造成不必要耗时。用户明确要求不再关注此回报回收，只要实际结果正常即可。今后按保存的
  退出码、完整日志和实际产物判断；不得仅因 PTY 回收重跑。真实失败、超时和五秒看门狗
  仍须处理。原始记录不改写，具体授权见 `1D/terminal-result-acceptance-user-override.json`。
- TUI CABI 构建及增量确认正常，库 SHA256
  `a190f4957816d7ed973179efeb03e7f19ddfa9a986f17310d9237183d462014c`，core 锁未变。
  PyInstaller 首次因用户级缓存权限失败，改用任务专属缓存及新输出目录后打包/--help
  均正常；不改原用户缓存。打包库摘要单独记录，仍须实际 RuntimeWorker 场景验证。
- 此次未重跑完整测试套件或 TW；开始新 pin 的定向生命周期、CABI 和组合场景。

#### 1D 新绑定动态结果（2026-08-28）

- Firefox 与 Safari 的 repair35 生命周期定向通过：各 6 个样本、真实失焦后 `0/0/`、
  restart 取消和切换项目的异步解码竞态。Firefox 原滚动失败点现在准确返回 `580/-294/`。
  gzip 摘要分别为 `cf191a90cd5cecb785cc34f8e80c48680df46a9a0af9a87a0da5fa6d2f9b3558`、
  `bf93ebefd3d26b365aa76ec31d836834eee1b0e168c0388d8dd0f093e11c5aa1`。
- Chromium 同输入在失焦后的 pre-query focus 断言失败，实际输出 `1133/-98/`；前置
  事件记录为 focused=false，而查询后的观察已有同位置 focused=true 事件。保留失败
  `24eeca190afb39713b6689cfa77a8ce3b32142692711883a6b3192dcade4590e`，正在只读分析，
  不盲目重跑或把两个浏览器通过扩展为 Chromium 通过。Tauri 原生输入探针独立开展。
- original canvas-invalid-dimensions 的新 Chromium capture 和 adapter 均成功，真实
  `RESULT:10=777`，终止保持 Faulted，watch 具有实际 stop/request/response provenance。
  adapter SHA256 `566892dea13dd48501772564422ebf1ae3ade7059c7e23dfd3afb1b448ba3fce`；
  当前仅为有效观察，尚未进行本项 reference 差分。
- TUI 新绑定提交 `ad5c018b7c73bac441a9064d3339a174eff7dcfa`，依赖 core `b8b5bee…`。
  真实 debug/step/fault 3 项通过；缺能力 5 项最初因漏传显式开关跳过，补上 opt-in 后
  仅这 5 项定向通过（HTML v2、pointer/canvas v1），不把跳过算通过。source 和打包库
  的 snake-data RuntimeWorker 组合场景均通过。未重跑完整 pytest，TUI worktree 已干净。

#### 1D 验收效率修复与继续（repair38，2026-08-28）

- 用户 17:20 暂停时 Tauri 原生探针仍在编译，未进入 GUI；相关进程已停止，部分日志保留。
  用户随后要求先修复拖慢验收的问题再继续。恢复核查无残留构建/测试进程，可用约 21 GiB。
- 实施范围为同一 1D 的定向修复，不重置审查或全量次数。已有唯一重构审查及 R1–R6
  落实继续有效，不启动第二次审查；TW repeat4 结果保留，不再运行 TW 全量。
- Web 正式 Tauri runner 增加受校验的 `--reuse-build` 和 `--build-only`：仅蛇版/native-input
  无 state 场景可复用，项目路径走已有测试 picker 配置，源码/绑定/锁/配置/工具链/资源/
  provider 与实际二进制摘要匹配才复用。仅使用正式 CLI 参数，不混入直接 Cargo 确认构建。
  编译、构建完成/复用、GUI 启动分别报告；PTY 回报回收不再触发重复运行。
- 同时修复 Chromium 失败的两条已定位原因：后台/隐藏 pointer 事件不得复活失焦前坐标；
  测试须关闭焦点探针并恢复主窗口后再确认焦点，在实际 pointer 查询点记录独立 DOM
  观察并按三个实际请求分别验证 X/Y/B，不能拿 Enter 前快照代表查询时状态。
- 新改动完成后仅运行受影响 Vitest、类型/lint/格式/build 门禁；随后优先原生输入探针，
  再恢复服务/组合/生命周期验收。磁盘串行复用本组 target，无 Chromium 下载，不改参考库。
  本节为实施方案，尚不代表上述新改动已通过验证；实际结果随后追加。

#### 1D repair38 静态与首次真实 Tauri 探针结果

- 定向 Vitest 首次 361/361 通过（测试执行者选择了完整 store 入口，范围比所需 describe
  更宽）；其后误补跑 describe 29/29，记录保留但不重复计覆盖。typecheck 通过。
  ESLint 首次缺少 Node runner 中浏览器回调的 `window` global 声明，修复后最小复验通过；
  Prettier、Web build 均通过。后续不得对已被通过集合覆盖的用例做形式性确认重跑。
- 正式 Tauri `--reuse-build --build-only` exit0，132.18s，未启动 GUI；实际产物
  SHA256 `314dd31fb47a414d2793f13a43a7e445acaf2393e02bc693fa2ceb1645964492`，
  53,881,120 bytes，主智能体核对与 manifest 一致。结束可用约 20.8 GiB。
- `repair38-tauri-native-input` 实际复用了上述产物并启动真实 WebView，确认
  `bridgeKind=tauri`；未进行第二次编译。首次探针在“禁止关闭主窗口”的预期错误处
  被 WDIO 自动重试拖住，5 秒完整快照看门狗正确判定静止。exit7，约59.4s，失败和
  cleanup 日志保留，不记为原生输入通过。
- repair39 定位到本地 `@wdio/tauri-service` standalone 实现硬编码十次连接重试；
  已在 session 建立后关闭动作重试、限制单次请求 5s，不改产品或看门狗。仅两份非编译
  Node 入口变化时，在其他源码、build argv/env、工具链、资源、provider、二进制摘要
  完全一致的情况下复用原构建；旧 manifest 不改写。补充最小缓存拒绝/复用回归后，
  仅复验受影响的 Node 静态门禁，再定向重跑此探针，不重新编译产品。

#### 1D 后续证据与驱动修复（repair39–41）

- repair39 缓存定向 2 项、ESLint/格式/Node 语法均通过；原生探针 2.25s 内失败，
  证明十次自动重试已关闭。实际原生错误被 WDIO ContextManager 的 closeWindow
  回调错误地改写为“所有窗口消失”。repair40 仅把这条负向基础设施检查改为同一
  loopback WebDriver session 的真实 DELETE 请求，仍严格检查 400/invalid argument，
  并确认主窗口存在；正向输入仍使用 WDIO selectors/actions。
- repair40 的文件级静态门禁通过；探针真实窗口创建、主窗口关闭保护、缺失窗口拒绝均
  通过，后续 data 页面上 `__NATIVE_INPUT_PROBE__` 一直不存在，exit7/5.29s。
  `No window could be found` 属于刻意的 missing-window 负向测试，不是创建窗口失败；
  已更正测试执行者首次错误归因。尚无 trusted pointer/Unicode/Enter/PageUp/blur 通过结论。
- repair41 将独立探针页放到只返回固定 HTML 的随机 loopback HTTP 端口，避免依赖
  provider 的 location.href 顶层 data 导航；Tauri 生命周期使用已有原生 focus window
  验证实际 blur/focus，Browser 的可见按钮路径不变。仅改测试 Node 文件，不重建产品；
  原有二进制摘要保持，编译输入完全一致才复用。
- Chromium 剩余 29 对首次 capture→adapter 共58条命令全部 exit0，主智能体逐条核验。
  输出为 `1D/captures/chromium/{original,snake}/*-repair38/`；连同已有5份，34个普通
  host/profile 观察齐全。它们是有效实际观察，不自动等于参考差分匹配。
- 首次 reference 队列完成 snake first-row-half-units：`matched_observables`；下一项
  original canvas-invalid-dimensions 工具 exit0 但 `incomparable_schema`，队列按规则停止，
  29项未执行。参考错误进入 output/termination=error，而 Rust 为 diagnostics/faulted；
  正在核查保守规范化或真实差异。不可把工具成功冒充行为匹配，不重新执行已采集引擎。
  汇总为 `1D/repair38-chromium-remaining-result.json`、`repair38-first-reference-result.json`。

#### 1D 原生坐标与错误观察定向修复（repair41–42）

- repair41 静态缓存/生命周期 18 项及文件级 lint/格式/语法通过。原生探针复用实际
  构建，5.81s 内失败：loopback 页面已加载，pointer move/down/up/click 均为 trusted，
  但事件落在约 `(98, 5.5)`，未命中按钮。Unicode/Enter/PageUp/blur 尚未执行。
  正在修复 WKWebView 标题栏遮挡与 viewport→NSView→NSWindow 坐标转换，不使用固定偏移。
- repair42 的比较器最小 3 项通过；仅离线重比已有 original canvas-invalid-dimensions，
  未重跑任一引擎。报错终止及整数 `RESULT:10=777` 相同，错误时也比较变量类型和缺项；
  timeout/limit/quit 不可冒充脚本拒绝。机器结论仍为 `incomparable_schema`。
- 对该份实际证据逐项登记有意错误呈现差异：原版在 output 写入 GCREATE 宽度0错误，
  Rust 发带 api/source 的 vm_fault；双方定位 services.erb:109、拒绝宽度0且未覆盖777。
  不声称错误文本等价，也不将其他未知错误统一豁免。该项允许继续下一首次 reference，
  后续未审定差异仍须停止。重比 SHA256
  `1a0c8a83c01cf7deabc7b8c97067c66c0d8eccdc2194fdc3ceb27f84ffeb91a4`，
  具体限定见 `1D/repair42-original-canvas-disposition.json`。批次1仍未完成。
- 蛇版同项首次 reference 也观察到 line109 拒绝宽度0且777不变；独立登记相同呈现差异，
  未声称机器比较通过。证据 SHA256
  `630385c7ccfdb03e6f94c5173ffb4c26880624cfa6012f7509e0f56cb898fa81`，
  限定见 `1D/repair42-snake-canvas-disposition.json`；余下首次 reference 为28项。

#### 1D 定向门禁与验收推进（repair43）

- 原生坐标修复以实际 contentLayoutRect 与 WebKit 顶部遮挡条件转换，不硬编码标题栏。
  provider fmt/check/Clippy 及5项native_input_tests全部通过；新provider inventory为
  `5d6f3ee812ea13e6236997403f539caa37c2062fce7f36488979bc2ce58b9643`。
- Chromium repair42 在真实输出 `118/-28/41` 后被测试证据解码拒绝：bridge BigInt 字节
  经JSON保存为规范十进制字符串。仅修复辅助解码，仍严格限制0..255且拒绝非法表示，
  不改请求/session/revision绑定。8项定向测试、lint/format/语法通过；动态复验待执行。
- 因provider源码确实改变，正式Tauri build-only执行一次并通过，约74s、未启动GUI；
  二进制53,887,472 bytes，SHA256
  `bea1513467e9854ea159e415940afe946f649518bfe12befeda325b8a4adc6e4`。
  可用空间20.01 GiB，继续串行复用本组target。真实native-input接续，尚未宣称通过。
- 首次reference队列已完成15项：11项matched_observables，4项机器incomparable，另有
  repair37保留的3项匹配。新通过10项包括两profile的canvas外界、两像素revision、实体、
  文件像素及后续行布局。两profile的late-parse-error均在services.erb:64未匹配闭合标签
  处拒绝，FLAG:90=0、RESULT:10=777一致；分别登记错误呈现差异，原始诊断不改写。
  见 `repair43-first-reference-progress.json` 与 `repair43-*-late-error-disposition.json`。
  剩余16项首次reference未运行；已retire Wine以交出GUI焦点，不重跑已完成项。

#### 1D 原生输入通过、服务定位与监督器效率修复（repair44–45）

- repair43 Chromium lifecycle通过：6样本/2竞态/无blocked；Firefox同样通过，gzip
  `721447b5447360321b15d8afdaa18aaa3b30daab90c60e9de1b1d4c2f9203e1d`。
  Safari在重启后的clear失败；快照证明仍negotiating、input disabled。旧restart判断
  误接受保留的旧ready，现要求确认前后的sessionGeneration/epoch变化及新integer wait。
- 主智能体发现之前8项过滤漏掉20项invalid-byte负向测试，已补齐20/20；不得称这些
  负向检查在此前Browser运行前已通过。后续改用受影响单文件避免过滤漏选；repair44
  生命周期helper单文件100/100及lint/format/语法通过。Safari修复后动态仍待复验。
- 原生坐标已实际命中后，repair43点击仍失败；移除elementClick/Down中额外异步激活，
  同一window发送前检查active/key，坐标公式不变。新增可捕获stderr几何诊断及焦点测试。
  repair44 provider fmt/check/Clippy/6测试通过；正式build-only约75s通过，二进制
  SHA256 `4b88a856b28720f646d4f6854b41087fe02e21f7012005324d486d502d01a570`。
- repair44 native-input真实通过：trusted move/down/up/click命中(98,35)，clicks=1；
  Unicode输入/清空、Enter submit=1、PageUp及blur/focus恢复均验证。仅测试用例0.721s，
  总命令53.99s；不再重跑探针，直接进入集成。当前原生provider inventory
  `711f8c2cc45f7ec315125bfe49809b97bf5ca520eab322a53b063bbf6b702d32`。
- Tauri services首次集成失败，未启动其后的lifecycle/batch1。实际fault为
  `presentation_query/html_substring@2.0` backend_failure，services.erb:15等待HTML
  字体/媒体超时；完整snapshot明确捕获。不能凭统一错误消息判定是fonts.ready还是rAF，
  正在补等待阶段诊断，不延长10s截止或跳过服务。失败记录为repair44-tauri-snake-services。
- 发现D/run_check.py每消费一行输出都启动ps扫描；6102行日志造成重复进程扫描和反压。
  改为250ms限频，启动/退出/finally仍扫描，deadline/cancel/disk检查和完整gzip不变。
  新增扫描数、观察退出及cleanup时间字段；继承既有监督器回归并加6102行完整性检查，
  权限正确的直接验证11/11通过。首次沙箱ps被拒单独保留；不因PTY回收重试业务测试。
  仅本任务监督器变更，源码与测试位于 `1D/run_check.py`、`1D/test_run_check.py`。
- 两profile的int32十亿像素case实际different：reference完成，Browser返回resource_limit
  且RESULT保持0。这是唯一审查R3已要求保留的32768px安全边界，分别记录实际证据
  `repair45-*-int32-resource-disposition.json`，不作单位换算通过证明。
- 低于20GiB后仅清理已被替代的本任务host rlib/rmeta：427.1MiB及284.1MiB；当前binary、
  旧执行证据、fixture、批次0均保留。清理清单 `repair43-cache-cleanup.json` 与
  `repair44-cache-cleanup.json`，最近恢复约20.12GiB；继续串行构建。

#### 1D 隐藏窗口测量修复与普通服务差分收束（repair45–47，进行中）

- repair45 定向真实 Tauri 证据将阻塞定位为 `phase=animation-frame`，同时
  `visibilityState=hidden`、`hasFocus=false`、`fonts.status=loaded`；不是字体下载失败。
  HTML projection 不再以绘制帧作为同步 DOM 测量的前置，保留 Vue flush、字体、媒体、
  revision 校验、10s 截止和同步几何读取。错误中保留并发等待阶段及窗口/字体状态。
- repair46 受影响 HTML/canvas 单元 77/77、typecheck、对应 lint/format、Web build
  通过；正式 Tauri build-only 通过（69.53s）。当前二进制 53,888,272 bytes，SHA256
  `4ae8f024d90d83219ffd96141700569e47c88974faa7aafe631dae0112db0fa6`，core pin
  仍为 `b8b5bee45d1a7d3fc31f4df42dcbe0048422794a`，provider 为 materialized-repair44。
- repair46 Tauri services 在 5.67s 后失败于原生鼠标的前台窗口 guard；完整日志已出现
  `SNAKE_HTML=0/0/0/1/1/1/1` 和 `SNAKE_CANVAS=4294901760/4278190335/2/2`。
  首个5s快照尚在协商，不能据此否定后续 HTML/canvas 的真实执行，也不能将整项标为通过。
  repair47 仅调整 Node 测试启动流程：通过当前 WebDriver handle 激活同一原生窗口，
  确认可见且有焦点后才运行用例；不逐事件抢焦点，不使用合成 DOM 输入，不放宽 native guard。
  本次仅 Node helper 改动通过精确缓存排除复用上述二进制；仍须受影响静态检查通过才续验。
- 本地监督器修复已通过11项定向测试（2.419s）：将逐日志行全进程扫描改为250ms采样，
  子进程退出和最终清理仍强制扫描；完整 gzip/摘要/截止/取消/磁盘检查保留。原6102行输出
  导致数十秒监督开销的原因已消除，不以回收不到 PTY 退出回报为理由重跑。
- 普通服务的34项双参考执行已齐全；不同平台字形像素、预先确认的32768px资源限制、
  错误诊断表示及 Unicode 边界差异分别登记，原始 `different/incomparable` 结论不改写。
  蛇版 Unicode 用例在 services.erb:40 因 `GetSubStr` 按 UTF-16 单单元测量，触发
  `BuildFallbacks/Char.ConvertToUtf32` 非法代理项异常，四个 watches 均未赋值；这不同于
  原版返回替换字符。Rust 保留有效 Unicode 字符的完整性，符合本批明确要求，证据为
  `repair46-{original,snake}-unicode-disposition.json`；参考正常执行源码未修改。
- 删除的仅为本组已替换的 rlib/rmeta，清单为 `repair46-cache-cleanup.json`（184.6MiB）；
  原生二进制、WASM/C ABI、fixture、失败记录与批次0证据保留。当前约20GiB，继续串行构建。
  Tauri services/lifecycle/组合fixture、Safari race修复复验、各原生客户端服务矩阵与
  no-progress 用例仍待完成；1D及批次1继续标为未完成，无第二次审查或全量次数重置。

#### 1D repair47–48 定向结果与剩余前置修复

- repair47 原生窗口前置对应单文件104/104、lint/format/Node检查通过。Tauri services
  复用上述 `4ae8f024…` 二进制，4.15s 后在最终鼠标断言失败：HTML/canvas标记齐全，
  `SNAKE_POINTER=84/0/` 缺少期望41。native实际move/down/up处于相同窗口、active/key=true，
  CSS点均为(84,454)；目前缺查询时独立DOM观察，不能先把预期改为空或认定按钮模型错误。
  下一次只补充独立事件、几何、命中元素与实际请求/回复证据，不替换输入或服务返回值。
- Safari repair47（6.30s）在首次图片gate启用前读取restart session时，过早要求decode
  evidence enabled。已将真实transport identity与已启用的decode记录要求分开；图片待决、
  取消和迟到结果断言仍严格要求完整decode证据。repair48受影响104/104及lint/format/Node
  通过。Safari repair48（3.77s）随后因前台缺失而未捕获可信pointer事件停止，尚未到race；
  Firefox未因此启动。准备以一次原生应用激活和WebDriver当前窗口选择替换无效的
  `window.focus()`+固定200ms等待，不对每个事件抢焦点，不伪造focus/输入。
- 已核对普通双参考34项：22项 `matched_observables`、6项 `incomparable`、6项
  `different`。12项非匹配均有逐项处置及原始证据hash，汇总为
  `1D/repair46-ordinary-reference-audit.json`；这些处置不将机器结论改为通过。
  8项跨客户端 no-progress capture及2项参考命令已准备在
  `1D/repair48-hazard-queue.json`，尚未执行；参考看门狗终止仍记录为未完成的失败观察。

#### 1D repair49 原生服务与组合断面通过，生命周期与跨端证据继续修复

- Browser foreground helper 的受影响单文件69/69、lint/format/Node通过；services独立
  DOM观察器相关106/106通过，随后lint因测试缺`console`声明失败；仅注释修复后定向
  lint及余下format/Node恢复通过，未重复全量或将首次lint失败隐去。
- Tauri services（`repair49-tauri-snake-services`）exit0，3.83s；真实
  `SNAKE_POINTER=84/-28/41`，HTML/canvas及替换像素标记齐全。随后lifecycle
  `repair49-tauri-snake-lifecycle` 在33.72s因 `pointer stage 2 did not complete`
  失败；已完成pointer0/1与基本service查询，未到图片gate。原生输入均有active/key窗口，
  最新完整DOM的prompt为空、当前wait未推进，继续定位真实输入/提交时序，不延长等待。
- 与上述生命周期测试步骤独立的 Tauri 组合断面 `repair49-tauri-snake-batch1` exit0，
  3.67s。实际到达 DATA_INDEX=2/main/42、RESOURCE=1/1/0、OVERLAY=1/1/1/2、
  STRUCTURED=1/station/29/29/42/from-schema、GLOBAL_MISSING=0/66/55、
  GLOBAL=1/7/55/1/12/saved-map/saved-xml，及HTML、canvas、BATCH1_READY。
  本组三条命令均复用SHA256 `4ae8f024…db0fa6` 二进制，无重建；不是整批完成。
- Firefox普通服务矩阵使用无头原生Firefox，不使用OS鼠标键盘，已确认可与Tauri独立
  并行。首项 original/empty capture exit0，但adapter关联校验失败后停止，其余33项
  未执行。实际6个watch均有同epoch/messageId关联；wire的可选reference字段省略、
  inspect显式null导致原始dict比较误拒绝。正对该既有Option契约做限定规范化及负例测试，
  保留原capture、不重跑浏览器、不放宽真实引用、值或关联ID校验。首次修复的16项定向
  测试有1项失败，尚未放行adapter重验；系统Python缺blake3的尝试单独记录为环境错误。
- 最新磁盘约20.09GiB。剩余为生命周期、Safari/Firefox当前定向复验、各端普通服务
  capture/离线比较与no-progress断面；批次1保持未完成。

#### 1D repair50–52 输入证据与 Firefox 对照结果

- core adapter仅将 `variable_value.value.reference` 的 `fiber_id/frame_id/character`
  省略与null规范化，关联ID/epoch、其它字段及typed值仍严格检查。首轮16项中的新增测试
  因fixture共享reference别名而误改请求；修正测试复制边界后，该项含11个submode定向
  通过，其余15项原结果保留。Firefox首项adapter离线重验通过，不重复已完成capture。
- Firefox其余33项 capture→adapter 共66条命令全部exit0（首项已有）；随后34条离线
  对照全部完成，逐项与Chromium结论相同：22 matched_observables、6 incomparable、
  6 different。12项非匹配的具体watches和终止字段逐项核对通过，未改写机器结论。
  汇总为 `repair51-firefox-remaining-result.json` 与 `repair52-firefox-comparison-result.json`。
- 生命周期输入前置改为确认真实DOM文本、enabled及焦点后再移动指针；107/107及对应
  lint/format/Node通过。Tauri repair50实际文本0已就绪，但原生Enter前active/key丢失；
  repair51加入失败时OS前台观察，证实该次前台仍是era-web-tauri，实际失败变为指针回到
  输入框(316,507)、未悬停目标；不把两次不同错误归为同一根因。Safari repair51则在
  文本2已就绪后记录documentFocused=false，未到图片race。
- 新的本地只读前台追踪工具每100ms查询NSWorkspace、仅变化时记录，不激活应用；烟测
  保留被包裹命令exit7且退出后observer已回收。后续原生复验附该记录，避免仅凭DOM猜测。
- provider源码核实原路径只构造NSEvent而不移动系统cursor；修复先将验证后的窗口点
  转到主屏原点的逻辑屏幕坐标，检查所有显示器半开边界，调用CGWarpMouseCursorPosition，
  检查错误后仍由原NSWindow投送事件。不用全局CGEvent、不绕过active/key、不乘Retina比例，
  不裁剪屏幕外/显示器间隙。新增映射测试，零依赖变更；保留edition2021/MSRV1.77语法。
  这处静态缺口已修，不声称此前所有失焦/回跳都已因此解决，仍需真实断面确认。
- repair52 provider格式、check、Clippy、8项inline测试及来源校验已通过；正式测试host
  构建与独立probe/生命周期仍在推进。最终overlay SHA256为
  `c8694a73f0c3ea41bc230fb2ba94a94c7563869f0dad59ed69b4e7c60de2bc4d`，使用
  `tauri-native-provider-source/materialized-repair52`；旧materialized44和此前失败证据保留。


#### 1D repair52–53 原生光标通过，滚动断面继续定位

- repair52 正式测试host构建exit0，72.14s；本组原生二进制53,886,912 bytes，SHA256
  `fe66494b0295f23ad2ef4e1cf7cb704068adde7c0f29777f2411a5b409f7f882`。
  独立native-input探针exit0，3.95s，真实系统光标、可信事件、Unicode输入、清除、Enter、
  PageUp与blur断言通过；没有重复探针或因PTY回报回收重新构建。
- repair52 lifecycle前3项指针通过，随后resize因测试错误要求旧pointer非空而失败：窗口
  缩小时旧光标离开新窗口、pointerout清空是合法状态。repair53仅将尺寸/scroll观察与
  旧pointer是否存在解耦，仍要求文档前台、实际后续查询的独立指针断言不变。
- repair53受影响单文件108/108、ESLint、Prettier、Node及观察脚本语法均通过。
  lifecycle exit7，7.86s，已通过pointer0–3，实际失败为
  `PageUp did not scroll the real viewport`；未继续services/batch1。
  完整证据为 `validation/runs/repair53-tauri-lifecycle/`，失败时OS前台为era-web-tauri。
- 只读前台追踪改用NSRunLoop刷新NSWorkspace缓存；旧NSThread sleep版本只记录初始
  应用，不能据此断言整段前台未变化。新的trace实际记录Firefox→Tauri，不抢焦点。
- Safari34普通服务capture→adapter与离线对照已接续，期间冻结Web源码/产物；Tauri
  PageUp问题只读诊断，禁止为了抢测而修改正在使用的输入或同时启动第二个GUI会话。
- `repair53-cache-cleanup.json`记录仅删除已替换的provider44 rlib/rmeta，共297,859,604
  bytes；provider52二进制及其当前rlib/rmeta、WASM、所有fixture/失败证据与批次0均保留。
  当前可用约19.6GiB，继续串行，暂不新增大型构建。批次1仍未完成。


#### 1D repair53 Safari普通服务证据完成

- Safari34组capture→adapter共68命令全部exit0，随后34条离线对照全部完成；复用固定
  参考输出，不启动Wine。22 matched_observables、6 incomparable、6 different，逐项与
  Chromium/Firefox结论相同；12项非匹配的watches/终止状态核验符合已登记处置，原机器
  结论完整保留。结果为 `repair53-safari-ordinary-result.json`、
  `repair53-safari-comparison-result.json`；不包含尚未通过的生命周期或no-progress。
- Tauri普通34项使用 `repair54-tauri-followup-queue.json` 接续，固定provider52和
  当前fe664…测试host；独立PageUp诊断尚未影响产品或队列输入。
- 危险断面队列准备时发现首个Chromium `--case` 错指普通canvas条目，在启动前已改成
  fixture唯一的 `s04-lines-no-progress`，并核对全部8条实际fixture/config/case匹配。
  `repair54-hazard-queue.json`同样同步provider52；尚未执行，不能算作验收证据。


#### 1D repair54–55 矩阵缓存未命中与禁止隐式重建

- Tauri矩阵首条repair54在GUI启动前出现缓存未命中并开始正式构建，按本次“不得重建”
  要求中断；未执行任何case、adapter或后续33项。外层返回130且cleanup遇PermissionError，
  保留原request/gzip，不补造成功result；随后确认本任务构建进程已无残留。
  原fe664…二进制未替换，publish lock仍为f164ec73…；不重跑已通过native-input探针。
- 只读比对确认core/public/config及编译产品源码均未改变；仅允许的Node生命周期helper
  与构建时不同。执行者shell的PATH构造不同，不能把shell中尚未由queue/runner注入的
  Cargo/Vite变量误报为launcher缺失。新增明确 `--require-reuse-build`：同一严格契约，
  不匹配时报告字段名并在编译/GUI前退出；原`--reuse-build`回退语义保留。
- 本地 `run_cached_tauri.py`以已核验构建manifest还原构建敏感环境、使用同一Node执行
  官方launcher；不放宽compiler/SDK/来源/二进制hash校验。整个外层监督器使用原生GUI
  所需权限，避免沙箱父进程无法回收自身外部子进程。
- repair55同次加入PageUp真实DOM焦点前置和有界key/scroll事件，捕获dispatch后的取消状态，
  失败完整记录是否焦点未到、事件取消或实际滚动回弹；不合成焦点/滚动，不修改产品或
  provider，不延长3秒等待。静态与定向原生结果待下条登记。


#### 1D repair55–56 PageUp遮挡根因已确认

- repair55受影响单文件116/116、ESLint、Prettier、Node均通过。严格cached wrapper实际
  命中fe664…，没有build-start；原生lifecycle exit7，4.82s，GUI已结束。
- 完整PageUp独立观察显示点击(350,233)命中兼容警告SPAN，pointer事件97–99的
  targetScope=other，实际activeElement=BODY；viewport仍在scrollTop2091底部。
  这解释了此前PageUp未滚动：resize后的警告浮层遮住视口中心。不是provider键映射问题，
  也不是已到顶部；probe的实际538→529→418滚动证据保留。
- repair56只在该步骤前通过真实UI关闭warning通知（不关闭error、不删日志），有16次
  限额和每项3s快速状态等待；然后仍要求真实viewport焦点与PageUp实际scrollTop下降。
  新增同一测试中的warning遮挡分支；未修改产品滚动逻辑，后续定向结果待登记。


#### 1D repair56–58 生命周期收敛与Tauri导出前置

- repair56受影响117/117及lint/format/Node通过；Tauri严格复用fe664…，lifecycle
  exit0/11.13s，6个独立pointer样本、2次真实image race、blocked=[]、未挂载canvas=0；
  真实blur、取消、新session进展、旧decode晚settle及无旧回复均通过。
  services exit0/3.73s，pointer=84/-28/41；batch1 exit0/3.42s，数据、HTML、canvas与
  BATCH1_READY齐全。三个命令无编译，不重复native-input探针。
- repair57 Safari lifecycle exit0/10.58s、Firefox exit0/22.01s。Chromium exit1/6.22s，
  Playwright适配器的警告关闭selector同时匹配4项，严格模式拒绝，而WDIO `$`默认首项；
  repair58仅将适配器click对齐首项语义，原警告/焦点/PageUp断言不变，正在定向验证。
- Tauri普通矩阵repair55首项严格缓存命中，但在actual project identity export阶段
  保存诊断归档等待原生save对话框。两次完整快照相同触发watchdog；最终result exit7，
  17.27s。ECONNREFUSED/重复done均为关闭后的次生错误，不是实际首败；未运行case或adapter。
  不能把摘要中的“exit1/连接拒绝”当作本次根因。普通34项仍未完成。
- 现有可复用测试host只在会话配置project picker，未配置saveDiagnosis的原生输出路径；
  计划在同一test-build配置提供一次性隔离诊断路径，仍真实inspect project identity和
  stream archive/write_export_chunk。此项涉及嵌入TS，需静态门禁后重建一次；后续所有case
  使用同一验证二进制，不编译per-case路径。no-progress危险断面尚未运行。

#### 1D repair59–63 原生捕获前置修复与真实首败

- repair58 Chromium lifecycle 定向 exit0/17.62s，四客户端生命周期均已有成功执行证据。
- repair59在仅测试构建的会话配置中增加一次性诊断输出路径；真实原生项目身份检查、Worker
  流式归档和写盘链路不变。55项定向、类型/lint/格式/core pin通过；官方build-only 69.05s、
  GUI未启动，二进制9869e095…（53,886,912 bytes）。后续严格cache-only，不因PTY回报重复构建。
- repair60项目归档实际写出，但转移给Worker的ArrayBuffer已detached，归档完成后的测试证据
  再读取该buffer抛错。保留首败，不将已写归档等同于整个capture成功。修复仅在转移前复制
  小型测试证据，并在回归中使用真实structuredClone transfer复现所有权转移；已知导出失败
  立即结束，不再等待看门狗。严格复用入口补充capture.json完整性检查，子进程exit0但缺证据
  不再被接受。
- repair62相关147/147、类型/lint/格式/Node及本地完成清单回归通过；一次官方build-only约68s。
  二进制178f59b027c8703cb90b45de1dee543e9f354c7b345e0f0c01ad6f53c9619955，53,886,912 bytes，
  core pin b8b5bee45d1a7d3fc31f4df42dcbe0048422794a，provider52未变。当前源码Chromium组合
  断面通过，三项HTML操作、canvas与BATCH1_READY实际执行；使用已安装Chromium，无下载。
  之前repair61默认headless-shell缺失属于启动基础设施失败，未启动浏览器。
- repair63严格命中上述二进制，首项s04-empty-lazy归档与脚本执行成功，但typed inspection
  超时。保存的inspection-frontier表明同epoch2/wait7，输出与值未变，检查停在debug_paused；
  调试variable_value响应correlation47先于submitDebug返回后的sent47记录，等待者登记过晚。
  未产生capture.json，持久化命令结果exit7；adapter、后续33项及对照均未执行。当前修复通用调试响应
  登记竞争，保留终态、stop/generation和错误断言，不靠延长超时或重试掩盖问题。
- 磁盘约19GiB，构建/GUI串行；core发布绑定、WASM和既有参考输入不变。
  无进展危险边界仍未执行，批次1D及整批尚未完成。

#### 1D repair64 调试响应登记竞争修复

- Web通用debugCommand/debugRequest在原生提交ID返回前登记submission，按精确correlation
  暂存早到完成通知（最多64条）；UI响应保持实际receive顺序，不重排证据账本。
  reset立即取消未返回的提交并清缓存，迟到旧ID不能登记到替换会话；pump不等待提交回报。
- request-state定向4/4通过。store相关describe首次31通过/2失败：新增fixture的fiber_page
  写死旧stop、生产暂停计数误含初始化hello。仅修fixture为实际command.stop及操作前计数，
  两个失败用例定向复验2/2通过；其余31项不重跑。覆盖早到值/错误、合法i64::MIN、旧stop
  不恢复、故障后只读观察和生产register-only暂停命令。
- 四文件ESLint/Prettier、typecheck通过。官方build-only exit0/67.95s，GUI未启动；
  二进制53,903,424 bytes，SHA256
  `7c109b37f7d2a97c926e1e21984547691ab4353f7e1ae524282f46b4613908ef`，
  contract SHA256 `d2b08da50260ceac05d01154479a7ae81a52aa6c983c1dbe035351d1d7dfeef7`。
  core pin及provider52未变，磁盘约19.32GiB。
- repair65准备34个新输出目录，保留repair63失败现场；使用严格复用入口继续Tauri捕获，
  不重跑TW、普通参考引擎或已通过浏览器矩阵。最终结果尚待登记。

#### 1D repair65–68 原生导出后输入焦点边界

- repair65首项严格复用7c109b…，诊断导出后在setValue的原生clear步骤失败：session window
  不是active/key。OS现场前台为Firefox（用户默认profile，PID37148，自8月27日运行），
  不是本组残留driver；未终止、隐藏或修改该用户浏览器。命令exit1，无capture，未运行adapter。
- 仅native oracle spec在导出后、首次真实输入前调用既有focusCurrentTauriWindow，显式
  激活当前WebDriver窗口并确认DOM焦点；随后仍原生setValue/Enter。provider焦点保护不变，
  不增加输入重试或忽略错误。属于Node测试脚本，不嵌入产品binary，未重建。
- repair67 spec Node语法、ESLint、Prettier与既有foreground helper 4/4通过；repair68使用
  新目录继续普通矩阵。前述失败、无进展危险断面和整批未完成状态均保留。

#### 1D repair68 Tauri普通矩阵捕获完成

- 34个case均已实际完成capture→adapter，共68条命令exit0；持久结果
  `batch-1-work/1D/repair68-tauri-matrix-result.json` 为completed_success，34个capture.json齐全。
  首项typed变量观察与输入恢复均成功，修复后的早到响应不再丢失。
- 所有case严格复用7c109b…二进制，没有逐项构建、普通oracle重跑或TW重跑。
  固定34参考证据离线对照与12项已知差异实际值核对仍在执行，不预先称全部差分相等。
- 余下no-progress独立危险断面按repair66队列启动；Rust必须明确runtime错误并保留777，
  参考的看门狗/超时仅作为未完成/失败证据，不能替代成功返回。整批仍待收尾。

#### 1D repair68 离线对照首败与视口候选回退

- 34项capture/adapter完整不代表语义通过。离线对照首项empty匹配，第二项原版first-row
  发现新差异后停止：实际stale_projection，参考正常完成。随后只读逐项终态审计确认18项
  因同类stale_projection失败，见repair68-terminal-audit.json；未将其豁免为平台差异。
- 精确账本：env5/space5、325×430已提交；输入2后env6/space6、327×430使用旧presentation45，
  core明确拒绝；HTML请求用presentation50/env5/space5，实际尺寸已回325×430，前端却因
  清空全部观测而拒绝。修复保留有界未拒绝候选，仅在最新候选被明确拒绝后恢复前一候选；
  新候选pending或未被拒绝时仍不能使用旧identity，实际尺寸/样式/三revision继续严格检查。
- repair66仅Chromium/original危险断面已执行：capture与adapter exit0，实际vm_fault、
  context.api=html__lines_step、诊断`html.query.NoProgress`，RESULT:10=777。
  预案中的runtime.html_query_no_progress不是实现的实际诊断名，不能按manifest字串搜索
  漏掉trace中的真实错误；不修改fixture或重测来追逐预案名称。其余7个host/profile和两个
  参考尚未运行，因视口修复暂停；GUI已释放。
- 修复后只定向重验18个失败Tauri case，保留其余16项原capture；旧7c109b二进制与构建清单
  冻结在本组1D/frozen/native-repair64/（53.9MB），避免新构建覆盖既有证据对应产物。

#### 1D repair69 视口拒绝回退门禁通过

- RuntimeViewportState保留最多256个已提交未拒绝候选，只有最高候选可参与matches；较新
  提交未返回时仍不可回退，明确reject仅删除对应候选，恢复前一有效候选。matches未放宽。
  IPC异常使当时所有在途revision失效，迟到旧ack不能重新填候选；reset隔离新会话。
- runtimeServices 85/85、store投影定向9/9、两文件ESLint/Prettier、typecheck通过。
  官方build-only exit0/68.74s，GUI未启动；新binary53,903,424 bytes，SHA256
  `ae9d33dae648ea17f3523d538863fcbe4d017f155c16dc58e962ea41b01355fc`。
- repair70只重新捕获18个stale_projection失败用例，每个adapter后立即做对应离线对照，
  首个新行为差异立即停止；原16个capture保留。未重跑任何全量或普通参考引擎。
  剩余7个危险断面排队等待GUI释放；Chromium/original已有明确NoProgress/777证据。

#### 1D repair70–74 原生窄窗口布局根因与定向修复

- repair70首例capture后即时对照仍发现stale_projection并停止；这次327px观测已在输入前被
  core接纳，输入后实际宽度变为325px，拒绝正确，不能继续放宽revision校验。
- repair73只读原生几何记录确认：window.innerWidth始终320；app-shell的隐式Grid列及
  game-area/game-viewport实际宽324.6875→326.6875→324.6875，input自身宽度随填入/清空
  变化183.15625→185.15625→183.15625。没有status-bar元素参与，不归因于滚动条。
  此次诊断执行exit0但脚本终态faulted，明确保留为失败证据。
- 修复仅为app-shell和game-area增加`grid-template-columns: minmax(0, 1fr)`，让原生窄窗口
  不再被输入框intrinsic width撑宽。现有原生oracle用例保留只读几何日志，并断言两个容器
  不超出window.innerWidth及输入前/填入后/提交后viewport clientWidth一致。
- repair74 spec语法、ESLint、spec/CSS格式检查全部通过；官方build-only exit0/67.26s，
  guiStarted=false，实际执行vue-tsc与Vite。新二进制53,903,424 bytes，SHA256
  `e6355b5425d1e25926871b1f4f9cecea4a3d80147dd81a966c3b3f58e77bc6be`，provider52不变。
- repair75仅重验18个受影响Tauri用例，每项capture→adapter→实际终态检查→固定参考离线
  对照后才继续；其余16项旧证据继续绑定冻结7c109b产物。相关输入和HEAD保持不变，
  无全量、TW或普通参考重跑。磁盘19GiB，构建/原生会话串行，未清理续做或批次0证据。

#### 1D repair75 定向捕获与实际比较分开记账

- 18个受影响用例capture及adapter共36条命令exit0，原生输入宽度断言全部通过，实际终态
  不再出现stale_projection。错误断面保留vm_fault/resource_limit，不改写为成功。
- 首项first-row的9个watch均与固定原版参考一致，ok=true、completed、output一致。
  原机器结论incomparable的唯一原因是额外info级runtime.compiled_cache_ready；已单列
  repair75-cache-info-disposition.json，保留原结论，不清洗原日志或扩大到其他诊断。
- 执行者随后17项未按要求逐项插入comparison，是调度偏差。未将其capture exit0称为
  差分通过；改为repair76明确命令队列离线核对现有18新+16旧adapter，不重跑客户端。
- 来源检查repair75-final-source-audit.json确认root/core/Web/TUI diff --check通过，
  core crates相对b8b5bee无差异，Web发布锁f164ec…、WASM bbe455…、native e6355b…
  均保持；磁盘18.78GiB。产品/fixture/HEAD在捕获期间冻结。

#### 1D repair76 / repair71 最终服务对照与无进展边界

- Tauri普通34项实际对照已完成：33次离线recompare exit0，首项直接复用；旧repair68
  adapter16份、新repair75 adapter18份。每份路径/hash和实际watches、output、termination
  位于repair76-tauri-offline-result.json，不重新启动客户端或普通参考。
- Tauri原始分类为7 matched_observables、21 incomparable、6 different。相比三浏览器
  各22/6/6，多出的不可比来自info级compiled_cache_ready；未过滤日志或把原分类改为匹配。
  既有12项差异均核对；混合字体样式的Tauri RESULT:10为85，三浏览器84，固定原版104、
  蛇版88；其余RESULT:11–15为16/32/0/0/0，错误仍在services.erb:76 InvalidMarkup。
  平台像素差异按已确认边界单列repair76-tauri-style-pixel-disposition.json，不承诺字节级字体来源。
- 无进展断面8/8 Rust host/profile均已验证：Chromium/original复用repair66，其余7项
  repair71实际capture和adapter exit0；全部ok=false、faulted、RESULT:10=777，诊断
  vm_fault、context.api=html__lines_step及html.query.NoProgress。明确错误来自runtime，
  不是监督器杀死客户端。首次临时检查器把完整message与诊断前缀作相等比较的错误保留，
  修正检查后复用既有捕获，不重跑真实用例。
- 两个固定reference各只执行一次相同输入：capabilities/load成功到WaitInput；进入
  S04_CASE_NO_PROGRESS后连续两次5s完整观察相同，看门狗终止，exit1、warm0/retire0。
  都没有可比返回值或最终watch，均保留status=failed，不称oracle通过或预期拒绝匹配。
  本批要求无法推进必须明确报错，故Rust的有界NoProgress与该参考停滞作为明确安全差异登记，
  不延长等待、不修改参考实现，也不以此重跑普通矩阵。
- 原版失败evidence SHA256：4653686e8084dc93abeac5e03af818a068f928a790920fb68a9ce583358db107；
  蛇版：19645b3e493a35dc43062d7d6920ee2cc61af50b5d53ff6f773e6567f12ffadf。
  完整请求/5s快照、失败原因和清理结果在repair71-hazard-result.json及oracle-*-no-progress目录。
- 所有本轮原生/Wine会话已结束，未下载浏览器、未新增全量、未再运行TW，转入分项提交和
  文档/来源最终检查。上述差异不改写为兼容成功，不扩大到SQL、缺失资源或真实标题可玩性。

#### 1D 交付完成（2026-08-28）

- 批次1的1A–1D必要实施、唯一审查、静态与实际客户端断面、TW覆盖及分项交付已完成。
  [验收汇总](BATCH_1_ACCEPTANCE_SUMMARY.md)保留首次失败、定向结果、原始差分类别与未可比范围；
  [交付提交](BATCH_1_DELIVERY_COMMITS.md)列本次31个分项提交的标题、正文和验证依据。
- Web最终`e3633311233df4a502faa41b32d8807c8c38de33`，TUI`ad5c018b7c73bac441a9064d3339a174eff7dcfa`，
  发布core pin均为`b8b5bee45d1a7d3fc31f4df42dcbe0048422794a`。core工具收尾为
  `e919d3719a2b0f5394c545783caa27289dcd7f7d`，此后的文档提交不改产品crate或前端绑定。
- 根CHANGELOG_PENDING已追加五项已验收产品功能/修复，独立提交`40fdea805c2fac3065a69fe541b8dd6265046efb`；
  未把构建/测试/流程改进登记成功能。无推送、合并主线或产品版本调整。
- repair77所有产品来源、48文件内容、pin/lock/产物和文档链接检查通过；仅新增native-input.patch
  中14个合法单空格context行被git diff --check报错，保留首次失败。只在.gitattributes对该
  patch设置精确whitespace规则，repair78属性/历史及工作树diff检查通过，补丁SHA不变。
- 测试原生/Wine进程已释放，磁盘约19GiB；仅清除本次可再生Oracle .pyc缓存116594 bytes。
  原失败、fixture、流式压缩报告及批次0证据保留，未删除用户数据或其他任务产物。
- 后续仍受SQL、bbas_map_schema.xml/bbas_map.xml缺失与批次2/4/6语义约束；真实游戏完整编译、
  标题和GRAPH_DB_INIT执行未宣称通过。TW测试耗时/输出优化备忘继续保留，不借本批收尾启动新全量。

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
