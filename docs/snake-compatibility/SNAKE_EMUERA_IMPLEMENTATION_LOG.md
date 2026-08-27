# 蛇版 Emuera 适配：分批次实施与验收记录

> 文档状态：批次 0 已完成；其他批次仍待登记。完成范围是 profile、隔离、基线与门禁，不是完整蛇版语义或蛇版 TW 可玩性。

## 文档职责与填写规则

- [改造思路](SNAKE_EMUERA_MIGRATION_PLAN.md)维护总体架构、批次范围、依赖顺序和验收目标；本文维护每批的具体实施方案、实际改动、证据、未完成项与恢复入口。
- [功能分类](SNAKE_EMUERA_BASELINE_MIGRATION_CLASSIFICATION.md)提供 S/D/C/N 项目编号、兼容语义和替代契约；[兼容性详查](SNAKE_EMUERA_TW_RUSTYERA_COMPATIBILITY_RESEARCH.md)提供历史源码与游戏证据，不能直接作为当前实现状态。
- 开工或续做前，先读改造思路和本批及其上游批次记录，核对代码 revision、环境、接口与已有验证；先填具体方案和依赖证据，再实施。批次编号沿用计划，不得为重置预算或审查次数临时拆分。
- 原版 profile 为 `emuera.em`，蛇版 profile 为 `emuera.skia.snake`；引擎、游戏、profile/codec/service 版本、seed 与资源 hash 分别记录，不混用原版 eraTW 和蛇版 TW。
- 状态可填“未开始 / 进行中 / 阻塞 / 暂停 / 已完成”；初始“待登记”不作事实判断。结论必须区分通过、失败、未执行和不适用，后两者须写原因；计划、命令已提交或 oracle smoke 不能冒充验收通过。
- 若范围、依赖、接口决策或验收目标变化，先在本文记录理由和影响，并同步改造思路；具体结果、提交、未完成项和下一步必须写回本批记录，并更新总览。不要把未验证设想改成历史调研事实。
- 审查、测试顺序、子智能体职责、单次全量限制和 60 分钟预算遵守[工作区规则](../../../AGENTS.md)、[core 规则](../../AGENTS.md)及相关组件规则；本模板不增加测试范围。首次全量与修复后定向复验分开记录，不把未重跑全量描述为修复后通过。
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
| [1](#batch-1) | 完整摄取与参考能力阻塞项 | 待登记 | 待填写 | 待填写 |
| [2](#batch-2) | 确定性 API、输入与兼容差异骨架 | 待登记 | 待填写 | 待填写 |
| [3](#batch-3) | 安全 SQL（蛇版 TW P0） | 待登记 | 待填写 | 待填写 |
| [4](#batch-4) | 主玩法 presentation、图像、scene 与自身存档闭环 | 待登记 | 待填写 | 待填写 |
| [5](#batch-5) | 蛇版存档互操作与音频 | 待登记 | 待填写 | 待填写 |
| [6](#batch-6) | 完整蛇版语言 | 待登记 | 待填写 | 待填写 |
| [7](#batch-7) | 可选 extension 与渲染能力 | 待登记 | 待填写 | 待填写 |

<a id="batch-0"></a>

## 批次 0：建立基线、profile 与门禁

计划入口：[改造思路 / 批次 0](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-0)。状态：已完成；负责人 / 最近更新：Codex / 2026-08-27。

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

计划入口：[改造思路 / 批次 1](SNAKE_EMUERA_MIGRATION_PLAN.md#batch-1)。状态：待登记；负责人 / 最近更新：待填写。

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
