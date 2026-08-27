# 蛇版 Emuera 兼容基线迁移：RustyEra 功能分类与实施方案

> 调研日期：2026-08-26\
> 性质：源码与游戏资源的只读静态审计；不是运行通过或行为等价证明\
> 前置事实报告：[蛇版兼容性详查](SNAKE_EMUERA_TW_RUSTYERA_COMPATIBILITY_RESEARCH.md)\
> 建议目标：在保留参考实现兼容性的前提下，让 `games/eratw-sub-modding`（蛇版 TW）逐步进入可玩状态；不把蛇版 Emuera 的 C#/WinForms/Skia 内部实现整体移植到 RustyEra

文中的源码/资源路径以多组件工作区根目录（`rustyera-core` 的上一级）为基准；省略游戏前缀的 `ERB/` 等路径相对于所述游戏目录，不相对于本文目录。审计日期、revision 与“当前缺口”均指本次历史审计，不代表后续实现进度。

## 0. 结论

“兼容基线移动到蛇版 Emuera”不应解释为用蛇版行为全局覆盖当前参考行为。更稳妥的定义是：

1. RustyEra 增加一个有明确版本的 `emuera.skia.snake` 兼容方言；现有行为保留为 `emuera.em`。
2. 项目清单、编译缓存、存档和诊断都记录方言及能力版本，禁止两个方言共用未标识的缓存产物。
3. 纯语言扩展和确定性数据操作直接进入 core；会改变旧游戏结果的项目由方言选择；平台、像素、文件、数据库和扩展代码通过已有 runtime/frontend 协议或新的版本化 service 表达。
4. 不移植蛇版的 lazyloading、WinForms 调试器、Skia/GDI 缓存、CLR 反射和内部 C# 类。RustyEra 已有编译缓存、热重载、调试协议、规范化 presentation/canvas 和 extension service，应在这些机制上补齐可观察契约。

当前蛇版 TW 的首要闭环仍是：`.als/.erd` 摄取、`GETMETH*`、安全且跨客户端的 SQL、`GETPLATFORM/TINPUTNF`、HTML 测量/指针服务，以及主地图所需的 CBG/HTML/定时输入。Float、variadic、OUT、元素 REF 和任意 CLR 插件不是当前游戏启动的先决条件。

## 1. 范围、判定规则与优先级

### 1.1 审计基线

| 对象 | 基线 | 用途 |
|---|---|---|
| 蛇版 Emuera | `emuera_lazyloading_selfmodified_version`，HEAD `fc4fb21416768c17256d0e82f997e5f99c9bba91` | 目标增量，版本签名 `1824+v24+EMv18+EEv56+Skiav12` |
| 参考实现 | `emuera.em`，产品基线 commit `26a35dc9334bb67590b96f7b8efbefbf199e391e` | 当前 RustyEra 声明的兼容参照 |
| 蛇版 TW | `games/eratw-sub-modding`，审计时 HEAD `667b9cd0...` | 目标游戏；4,100 个 ERB/ERH，约 176 MiB ERB |
| RustyEra | core、TUI、Web/Tauri 当前工作树 | 缺口、设计约束及可复用能力 |
| 其他游戏 | `games/` 中除蛇版 TW 外的 7 个项目 | 检查差异项目是否会破坏现有游戏 |

本报告延续前置报告的结论：蛇版相对参考实现活动注册表新增 23 个命令、83 个表达式方法；此外还有语法、类型、存档、运行时、配置和渲染行为修改。这里关注的是“RustyEra 应如何处理”，不是再次罗列实现差异。

### 1.2 四类项目的边界

| 类别 | 判定标准 | 实施含义 |
|---|---|---|
| 1. 简单项目 | 新增且语义局部、确定、跨平台；或只是补通已有抽象。默认不会改变合法旧脚本结果 | 可直接实现，但仍需签名、错误和边界测试 |
| 2. 差异项目 | 会改变既有脚本的解析、诊断、数值、状态、布局或存档；或虽为新增语法，但贯穿语言模型且需要方言隔离 | 放入显式兼容 profile；先对其他游戏做回归基线 |
| 3. 不符合设计准则 | 蛇版的直接实现依赖本机对象、物理像素、任意路径/代码、前端私有状态，破坏跨客户端、跨平台、确定性或 snapshot | 不照搬实现；保留脚本需求，改为规范化状态或版本化 service，并声明降级 |
| 4. 不必要或不应该实现 | 纯内部优化/桌面工具，RustyEra 已有替代机制，或当前 HEAD 已不存在；复制只会制造第二套架构 | 不移植；必要时接受配置并给出 no-op/迁移诊断 |

分类优先级为 **设计冲突（3）高于行为差异（2）高于简单新增（1）**。例如 SQL API 对脚本是新增项，但蛇版的直接 SQLite/任意连接串实现违反 core 无 OS I/O 的原则，因此归入第 3 类；RustyEra 应实现的是同一脚本能力的安全替代契约。

工作量标记：`S` 为局部注册/实现，`M` 为跨 analyzer/compiler/runtime 的中等改动，`L` 为跨类型、存档或多个前端的大改动。它表示工程规模，不改变分类。

### 1.3 RustyEra 设计约束

本报告采用 `rustyera-core/docs/design-principles.zh-CN.md`、`runtime-frontend-interface.zh-CN.md` 和 `runtime-reference-mapping.zh-CN.md` 的现有原则：

- runtime 拥有规范化语义和游戏状态，前端只做投影；core 不保存 WinForms、GDI、Skia、DOM、设备像素缓存等对象。
- 物理字体、viewport、DPI、行坐标、音频播放进度等观察值通过有 revision 的有序 service 返回，不能由 runtime 伪造。
- VM/runtime 不直接做 OS 文件、网络、媒体和平台 I/O；路径必须映射到 `Project/Save/GlobalSave/Data/Log/Resource` 等安全命名空间。
- presentation 的权威宽度是 Unicode `ColumnCell`；特定前端像素测量可作为兼容 service，但不能取代跨客户端规范语义。
- save/cache/snapshot 必须确定、版本化并校验 identity；外部资源若影响游戏结果，应产生可移植性诊断。
- 既有 `PresentationQuery`、`FontMetrics`、`Canvas`、`AudioState`、`Extension`、编译缓存、热重载和状态转移协议应优先扩展，而不是并建第二套宿主接口。

## 2. 其他游戏的现有行为审计

为了判断第 2 类的回归风险，静态扫描了以下 7 个非蛇版项目：`eraAkumaMaid`、`eraMaouEx`、原版 `eraTW`、`erafl`、`erarorona`、`eratohoK`、`era魔界牧場1.050_tc8`。结果只说明当前仓库快照的静态证据，不是所有 Emuera 游戏的证明。

| 差异点 | 非蛇版游戏证据 | 判断与保护策略 |
|---|---|---|
| `PRINTC/PRINTFORMC` 列宽 | 共 1,986 行活动使用：39/346/265/0/3/6/1327；既有中文、日文编码配置 | 极高回归面。不得把像素补齐全局替换 `ColumnCell`；蛇版像素意图单独建布局模式/查询 |
| 普通整数安全算术 | 原版 `eraTW/ERB/COMMON.ERB:2366-2375 @NOISE` 依赖自然回环；`erarorona` 有运行时可能为零的动态除数 | 普通运算保持参考 profile；蛇版 profile 可选择饱和/除零回零并诊断；哈希用显式 `UNCHECKED_*` |
| `INITRAND/DUMPRAND` | 原版 eraTW、erafl、eratohoK 均使用；配置依赖传统 MT/RANDDATA | 状态操作必须对应当前实际 RNG；不可让蛇版规则静默操作一套未使用的 MT 状态 |
| `#DIM/#DIMS REF` | 约 1,400+ 声明，erafl 1,032、原版 eraTW 318 尤其密集 | 现有数组引用必须原样保留；新的标量元素 REF 使用独立语法/类型分支 |
| `EXISTVAR` | erafl 有 16 个单参数调用 | 单参数 bitmask 不变；第二参数只作为加法式重载 |
| `EXISTFUNCTION` | erafl 有 16 个调用，无 lazy 配置 | 必须保持纯符号查询；禁止加入隐式读盘/编译副作用 |
| `SPRITECREATE` | 47 行；只发现 2 参数与 6 参数形式，没有 8/10 参数 | 精确保留 2/6 参数；新重载可在蛇版 profile 开放 |
| `CBGSETSPRITE` | erafl 两行，均为旧 4 参数 | 4 参数行为冻结；新 opacity/matrix/尺寸作为可选尾参 |
| `SETANIMETIMER` | 原版 eraTW 3 行、erafl 1 行，均为命令外形；erafl 参数是表达式 `1000 / フレームレート` | parser 应接受一般表达式，不能只识别整数字面量；表达式式/命令式需兼容迁移测试 |
| `GETDISPLAYLINE` 负数 | 未发现活动负索引 | 当前语料风险低，但返回空串到倒序索引仍是可观察变化，放 profile |
| 多余实参 | 仅在 `era魔界牧場` 找到 5 个高疑似“4 参数后尾随空项”，尚需 parser 复核 | reference 默认继续报错；`emuera.skia.snake` profile 忽略并发 warning，记录函数名和位置 |
| `XML_ADDNODE` 多目标 | 仅 erafl 有 9 行；正常数据目标唯一 | clone 修复在通常数据无差；遇重复目标时按 profile 差分并告警 |
| 非法 `TOINT` | 活动调用量很大（合计 1,517 行），未发现常量非法字面量，但用户/XML/运行时字符串可非法 | `emuera.em` profile 报错；`emuera.skia.snake` profile 返回 0 并 warning，避免静默掩盖数据问题 |
| `GETKEY` latch | eraTW 11、erafl 1；eratohoK 有 3 个 `GETKEYTRIGGERED` | 明确 held/edge/consume 与 tick 顺序；以输入 trace 做回归 |
| 字符串 `>=/<=` | 未确认活动的 string-string 样本 | 风险暂低，仍需操作符矩阵测试 |

这组证据支持“双 profile”而不是“统一改成蛇版”：尤其是 `PRINTC`、整数溢出、RNG 与引用参数，它们会直接改变大量旧游戏的输出或状态。

## 3. 第 1 类：简单项目

这里的“简单”指兼容风险低，不必然表示代码量很小。

| ID | 功能点 | 具体作用 | 当前缺口/实现方向 | 分类理由 | 工作量 |
|---|---|---|---|---|---|
| S01 | `.als/.erd` 项目摄取 | TUI、浏览器、Tauri 扫描并把别名表和用户 ERD 提交 core | 三个扫描器目前都漏掉这两类文件；加入清单、hash 和 project identity | 新文件类型只补全输入，不改变已有文件语义 | M |
| S02 | 用户 ERD 同名 ALS 与大序号别名 | 为 `BUFF.csv/BUFF.als` 等自定义变量解析名称；修复 ALS 序号 10 后的指针/别名 | 在 CSV deferred-index 基础上增加用户表关联、多维 alias 和边界校验 | 对未提供该类文件的项目无影响；现有内置 ALS 行为可冻结 | M |
| S03 | `GETMETH/GETMETHS/EXISTMETH` | 取得/判断动态用户函数并执行，蛇版 TW 构图初始化 P0 依赖 | analyzer 已接受，但 compiler 会 trap；接入现有动态调用、签名和 REF 校验 | 属参考实现能力补完，不是蛇版行为覆盖 | M |
| S04 | 已有 presentation/pointer/canvas service 接线 | 让 `HTML_STRINGLEN/SUBSTRING/STRINGLINES`、`MOUSEX/Y/B`、像素采样等到达实际前端 | core 已有请求模型，前端 capability 未协商或未实现 | 补通已有公共协议；不应改变不调用这些能力的游戏 | M/L |
| S05 | `EXISTVAR(name, storage)` 第二参数 | 非零时解析具体 storage cell，供动态排序/存储判断 | 保留单参数 bitmask，增加严格可选重载；Float bit 待 Float 批次 | 加法式重载，其他游戏只使用单参数 | S/M |
| S06 | 数组/角色 CSV 查询 | `MATCHALL/MATCHALLEX` 批量匹配；四个 `GETCSVNO*` 按 name/nickname/callname/mastername 查角色号 | 在既有数组和 CSV 索引上增加确定性方法 | 纯查询、无 I/O、无旧签名覆盖 | S/M |
| S07 | bit 数组操作 | `BITSET/BITGET/BITTOGGLE/BITINDEXOFFIRST` 在 `long[]` bit storage 上修改或查找 | 定义负索引、越界、空数组和返回值 | 局部确定性运算，名字全新 | S |
| S08 | MAP 确定性扩展 | `MAP_VALUES/MERGE/REMOVEIF/FINDKEY/TOSTRING/FROMSTRING` 提取、合并、过滤、反查和序列化 | 建立稳定迭代顺序、转义及 round-trip 测试 | 名字全新且建立在既有 MAP 状态上；不触碰旧操作 | M |
| S09 | `STRFORMCHECK` | 验证格式字符串能否被解析/展开，用于脚本防御性检查 | 复用 STRFORM parser，以明确错误码返回，不执行 host 副作用 | 纯验证方法、无旧语义 | S/M |
| S10 | `TEXT_BGC_ON/OFF` | 设置/清除后续整行文本背景意图 | 写入规范化 presentation style，不直接指定设备对象 | 状态是抽象颜色/开关，能跨客户端降级 | S/M |
| S11 | 显式非检查整数操作 | `UNCHECKED_ADD/SUB/MUL/NEG` 提供二补码回环，供噪声/哈希使用 | 在 VM 中明确 wrapping 规则 | 新名字使意图显式，不改变普通运算；蛇版 TW 的 `NOISE` 需要 | S |
| S12 | `DT_COLUMN_OPTIONS` 补完 | 设置 DataTable 列选项 | compiler 已下沉为 native，但 VM dispatcher 无实现；补真实 handler | 参考能力的实现缺口，语义局部 | S/M |
| S13 | `GETANIMETIMER` 只读查询 | 读取逻辑动画计时器 | 返回 runtime 的规范化逻辑计时状态，不读渲染帧时间 | 新查询且不改变计时器；设置语法迁移另列 D10 | S |
| S14 | 图像图层存在性查询 | `EXISTSIMAGELAYER` 查询规范化 scene 中是否存在指定层 | 在 D14/C08 的 scene 模型完成后提供 | 纯查询；本身无平台依赖 | S（依赖 scene） |

## 4. 第 2 类：差异项目

这些项目应由项目级兼容 profile 决定。profile 必须进入编译 cache key、运行时状态转移 identity 和存档元数据；不能只做一个不受版本控制的全局设置。

| ID | 功能点 | 具体作用/蛇版行为 | 为什么可能影响现有游戏 | 建议处理 | 工作量 |
|---|---|---|---|---|---|
| D01 | 版本化兼容方言 | 选择解析、运算、错误和布局行为 | “基线移动”若无版本会让旧项目随升级改变结果 | 增加 `emuera.em` 与 `emuera.skia.snake`；项目显式选择，自动探测只给建议不静默切换 | M |
| D02 | Float 完整类型栈 | `#DIMF/#FUNCTIONF/#REFF`、`LOCALF/ARGF/RESULTF`、字面量、提升、运算、变量、数组和 save tag `0x04..0x07` | 改变类型推导、重载选择、动态值、存档格式；数学跨平台确定性也需定义 | 作为方言能力整体实现；二进制以 IEEE-754 bits 编码，固定 NaN/-0/排序/格式化规则 | L |
| D03 | variadic、`ARGLEN`、标量 REF、OUT | `VARIADIC ARG/ARGS/ARGF` 捕获剩余实参；元素 `#REF/#REFS/#REFF`；可省略 OUT 写黑洞 | 触及调用 ABI、别名、生命周期和诊断；大量旧游戏使用数组 `#DIM REF` | 新旧 REF 类型严格分离；call frame 显式描述 alias/null-out/variadic；reference 行为冻结 | L |
| D04 | 字符串动态调用族 | `CALLSTR/JUMPSTR/TRYCALLSTR/TRYJUMPSTR/TRYCCALLSTR/TRYCJUMPSTR` 从字符串确定目标 | 扩大动态可达集合，影响静态裁剪、缓存和 missing-target 错误 | 基于现有 `CALLFORM` lowering，定义 TRY/C/JUMP 返回和 stack unwind；动态目标进入依赖/缓存模型 | M/L |
| D05 | 动态表达式求值 | `EVAL/EVALS/EVALF` 在运行时解析并求值 | 影响可预测性、错误位置、性能和 snapshot；`EVALF` 又依赖 Float | 只在 `emuera.skia.snake` profile 开启；限制为纯表达式语法，复用已版本化 parser/bytecode，缓存以源码+方言为 key | L |
| D06 | 多余实参处理 | 蛇版非 variadic 函数静默丢弃多余参数，参考实现报 `TooManyFuncArgs` | 会让旧脚本错误变成“成功”，也可能掩盖拼写/签名问题 | reference 报错；snake 执行但发带位置 warning；提供严格诊断开关 | M |
| D07 | 普通整数安全算术 | 蛇版溢出饱和并 warning；除/模零返回 0；显式 `UNCHECKED_*` 回环 | 原版 eraTW 的 `NOISE` 明确依赖回环；动态除零错误路径也会变化 | VM 按 profile 选 arithmetic policy；warning 去重且可测试；不修改 Rust release/debug 行为来代替语言规则 | M |
| D08 | `GETDISPLAYLINE` 负索引 | `-1` 取最后一行，依次倒序；参考实现负数为空 | 会改变分支/文本；虽然 7 个非蛇版项目未发现活动负索引，仍是可观察差异 | `emuera.skia.snake` profile 启用；基于稳定 `DisplayLine.line_id`/history 索引，越界规则固定 | S/M |
| D09 | sprite/CBG 新重载 | `SPRITECREATE` 增至 8/10 参数；`CBGSETSPRITE` 增尺寸、opacity、ColorMatrix | 参数解释和默认值可能污染已有 2/6、4 参数形式 | 旧 arity 单独 handler 并做 golden test；新尾参映射到规范化 sprite/canvas replay | M/L |
| D10 | `SETANIMETIMER` / `BITMAP_CACHE_ENABLE` 语法迁移 | 从参考表达式注册迁为蛇版命令；前者设置动画节拍，后者切换位图缓存 | 同名 token 的 parse 形态、返回值、参数表达式可能变化；其他游戏存在命令式 `SETANIMETIMER` | parser 接受已证实语法并标准化为同一 IR；`SETANIMETIMER` 接一般表达式。缓存开关的实际行为归 N01/N03 | M |
| D11 | RNG 状态命令 | 蛇版让 `INITRAND/DUMPRAND/RANDOMIZE` 始终作用于 MT | 非蛇版游戏依赖传统 RANDDATA；错误绑定会改变全局随机序列和存档复现 | RNG algorithm/id 进入 profile 与 save；dump/restore 只能操作当前 generator，导入旧状态需显式 adapter | L |
| D12 | 参考行为修正集合 | `XML_ADDNODE` 多目标 clone、字符串 `>=/<=`、非法 `TOINT→0`、鼠标键 latch | 数据重复、非法输入和输入时序下会改变结果；`TOINT` 使用面很大 | 每项独立 golden/trace；reference 与 snake 结果并存，snake 的宽容行为必须 warning | M |
| D13 | NF 输入、序列输入和宏开关 | `TINPUT*NF` 超时等待但保留上滚位置；`SEQUENCEINPUT` 注入下一次输入；临时开关宏 | 改变 focus、scroll、timeout、队列消费和自动化时序 | 建立统一 input state machine；逻辑事件由 runtime 排序，viewport 保持为前端 policy；录制 input trace 回归 | L |
| D14 | 规范化 HTML / scene 扩展 | `<font size/valign/render/...>`、`<img xpos/display/matrix>`、`<div>`、ImageLayer 的 depth/opacity/锚点 | 旧标签默认、换行、层叠和保存 presentation state 可能变化 | 扩展 canonical AST 和 `SceneLayer`，未知属性按 profile 诊断；物理查询另走 C04/C08 service | L |
| D15 | polygon、canvas 与动画语义 | `G_POLYGON_*` 维护点集并描边/填充；sprite/图像支持尺寸、翻转、动画 | 影响绘制顺序、坐标、ColorMatrix 与 replay；不同前端输出可能不同 | runtime 只产生确定性 CanvasReplay/scene delta；固定点颜色矩阵和稳定同 depth 顺序 | L |
| D16 | 音频“期望状态”控制 | `SOUNDCONTROL/BGMCONTROL` 的 pause/resume/stop/seek/rate/preserve-pitch | 改变 AudioState 与一次性 effect 顺序；客户端支持程度不同 | 扩展规范化 AudioState 与 `Pause/Resume/SetRate/Seek` effect；不把实际播放进度当 core 状态 | M/L |
| D17 | `BEFORE_THROW/BEFORE_ERROR` | 错误抛出前调用脚本事件，可由配置禁用 | 改变错误路径、可重入性、输出和最终异常；可能把原错误覆盖 | 仅 `emuera.skia.snake` profile；事件带原 error id/location，设 recursion guard，hook 自身失败保留原错误 | M |
| D18 | 存档与 ERAZIP 互操作 | 读取蛇版压缩存档、自定义 SAVEDATA/CHARADATA/ERD 数组及 RNG/type 元数据 | 格式、变量排序和类型标签不同；错误兼容会损坏数据 | 先做只读 importer 和 fixture；RustyEra 自身格式版本化，确认后再承诺写回/双向 | L |
| D19 | `GETVARF/GETMETHF/DT_CELL_GETF` | 动态取得 Float 变量、函数结果和 DataTable cell | 是动态值/重载体系的一部分，不能作为孤立 `f64` host stub | 随 D02/D03/D05 一起贯通类型与错误规则 | L（D02 子项） |

## 5. 第 3 类：不符合 RustyEra 设计准则的项目及替代方案

本类拒绝的是蛇版的**直接实现方式**，不是一概拒绝游戏需要。每项都给出跨客户端替代契约，并在第 7 节落入实施批次。

| ID | 蛇版功能/实现 | 冲突原因 | RustyEra 应保留的脚本需求 | 可行替代方案 | 批次 |
|---|---|---|---|---|---|
| C01 | SQL 全族直接使用 `Microsoft.Data.Sqlite`，接受连接串和工作目录/存档路径 | core 直接 I/O；任意路径；浏览器无等价文件系统；连接/reader 不能稳定 snapshot | 蛇版 TW P0 所需连接、事务、参数化语句、scalar、reader、MAP XML 导入 | 新增版本化 `Sql` service：runtime 只持有带 epoch 的逻辑 connection/reader handle；路径映射 `Resource` 种子和 `Data/sql` 可写 overlay；Tauri/TUI 用锁定版本 SQLite，浏览器用 WASM SQLite+OPFS/内存回退；typed canonical wire、参数绑定、配额和稳定错误；活动 reader/transaction 阻止 stable snapshot/reload。首期只做游戏用到的 Integer/String，不做任意连接串 | 3 |
| C02 | `CALLSHARP` 任意 CLR DLL 和反射 | 浏览器/TUI 不可用；绕过 ABI、权限、确定性、资源限制和快照 | 游戏或插件调用受控宿主能力 | 使用已有 typed/versioned `Extension` protocol：manifest 声明 operation、schema、版本、权限和可用客户端；runtime 只发规范化请求。桌面专有插件可以显式 unsupported，不能伪装成跨平台。先修复当前 declaration/builtin 冲突；蛇版 TW 无活动调用，后置 | 7 |
| C03 | `GETPLATFORM` 直接暴露宿主平台字符串/枚举 | 脚本据此分叉会导致跨客户端结果不同；平台名称不稳定 | 蛇版 TW 标题选择桌面 `TINPUTNF` 或普通输入 | 增加版本化 `Environment`/capability 查询，返回稳定能力（如 `viewport-preserving-timed-input`）而非 OS 名；兼容时可映射旧 `GETPLATFORM`，一旦结果进入存档/分支发 portability diagnostic | 2 |
| C04 | `PRINTC/HTML_PRINTC` 依实际字体像素补齐，`GETLINEY` 直接读行的屏幕 Y | 物理字体、DPI、viewport 和前端布局不可跨客户端复现；会破坏 RustyEra 的 `ColumnCell` 权威语义 | 蛇版 UI 需要像素宽度、截断、居中、行锚点 | 保留 canonical `ColumnCell` 为默认；增加 `PresentationQuery`/`FontMetrics` 的 revision-bound 测量和“像素 cell 布局意图”；用稳定 `DisplayLine.line_id` 锚定，必要时查询投影坐标。结果影响游戏分支时发可移植性诊断。TUI 可给 cell 近似或明确 unsupported | 4 |
| C05 | `SPRITECREATEFROMFILE` 直接读取文件/绝对路径 | VM 直接 OS I/O、路径穿越、大小写和编码平台差异 | 从游戏资源生成 sprite | 参数解释为 project resource id 或安全相对路径，经 `Resource`/`Image` service 解码；禁止绝对路径和目录逃逸；content hash 进入资源 identity | 4 |
| C06 | `SET/GET_SKIA_QUALITY`、`SET/GET_TEXT_DRAWING_MODE`、`STRICT_FONT_FALLBACK` 直接操纵 Skia/GDI 文本对象 | 把某一渲染库和设备能力暴露为语言语义；Web/TUI 无对应对象 | 游戏表达质量、fallback、edging/hinting 偏好 | 定义 renderer-neutral hint（quality/fallback strictness/text raster intent）；前端按 capability 接受、降级或拒绝并诊断。getter 返回“被接受的抽象策略”，不返回 Skia 枚举；不能影响 core 文本测量的确定性模式 | 7 |
| C07 | `GETSOUNDORBGMINFO/ISPLAYING*` 直接查询播放器时长、进度和实际播放 | 播放器时钟、解码、后台节流和无音频客户端不一致 | 音乐补丁需要时长、进度和播放状态 | “期望状态”放 D16；实际观测通过 revision-bound `AudioQuery` service 返回 `duration/position/actual_state`，带 capability/时间戳。查询参与玩法时发 portability diagnostic；TUI 可明确 unsupported | 5 |
| C08 | `SETIMAGELAYERL`/ImageLayer manager 以 viewport、滚动和物理坐标管理前端对象，离屏暂停动画 | core 若持有 UI layer/viewport 会造成客户端状态分裂；离屏暂停会改变逻辑时间 | 多图层、depth、随行滚动、opacity/matrix 和稳定锚点 | 建立 `SceneLayer { id/sequence, resource_or_sprite, depth, anchor: Viewport \| DisplayLine(line_id), logical_offset/size, opacity, fixed_matrix, scroll_policy, revision }`；runtime 保证同 depth 稳定顺序和行生命周期，前端裁剪/cull。离屏只能停止绘制，不能改变逻辑动画时间 | 4 |
| C09 | Float/数学直接依赖宿主 `double` 格式化和 libm 结果 | 不同架构/libm 对边界、NaN、舍入和字符串可能不完全一致，破坏 replay/save diff | Float 数学和序列化 | wire/save 用原始 IEEE bits；规定 NaN、-0、比较、round 和格式化；关键 transcendentals 选择固定实现或规定可接受误差且禁止其进入确定性 cache identity。SQL Float 延后到这一契约完成后 | 6 |
| C10 | F11 缩放、窗口宽高、多显示器、tooltip 生命周期作为引擎语义 | 纯客户端 UI/窗口管理，浏览器、TUI、移动端模型不同 | 用户可缩放/全屏以及鼠标逆映射 | 作为 frontend preference 和 viewport transform；core 只接收规范化逻辑坐标与 viewport revision。不得新增 ERB 可依赖的本机窗口对象 | 7/不进入语言 |

### 5.1 SQL 替代契约的最小可用范围

SQL 是蛇版 TW 的 P0，因此不能只写成“未来插件”。首个跨客户端版本建议提供：

- `connect(logical_db_id, access_mode)`：游戏随附 `plugins/qol_data.db` 作为只读 Resource seed，首次使用复制/overlay 到 `Data/sql`；不暴露宿主绝对路径。
- `execute_nonquery`、`execute_scalar_integer/string`、`execute_reader`、`reader_read/get_integer/get_string/is_null/close`。
- 参数采用有序 typed list，不拼接宿主字符串；限制语句长度、参数数、返回行数、内存、执行时间和同时 reader 数。
- 事务和 connection 生命周期由 service 管理；runtime handle 包含 service epoch，重连/项目切换后旧 handle 稳定报错。
- `SQL_IMPORT_MAP_XML` 首期可以在 runtime 解析 XML 后以批量参数调用，或由 service 接受规范化 rows；不要让数据库后端任意读取 XML 路径。
- 先不实现 Float scalar、任意 custom XML import/export 和任意 connection string，因为当前蛇版 TW 初始化不需要；接口保留 typed 扩展位。
- Tauri、Browser、TUI 使用同一 SQLite 语义版本和迁移 fixture；无法持久化的客户端必须在项目启动前明确拒绝或声明临时数据库，不能运行到一半才静默丢数据。

### 5.2 物理布局替代契约

兼容蛇版布局时，应区分三层：

1. runtime 的规范化文本/HTML/scene 意图和稳定 line/layer id；
2. frontend 对指定 font descriptor、逻辑 viewport 的排版结果；
3. revision-bound 查询把测量结果作为有序外部输入返回。

这既允许 Web/Tauri 尽量复现蛇版 TW 的像素 UI，也不会要求 TUI 伪造精确像素。任何脚本读取的物理结果都应标记“presentation-dependent”；存档和重放必须保存输入或拒绝宣称跨客户端确定重放。

## 6. 第 4 类：不必要或不应该实现的项目

| ID | 项目 | 蛇版作用 | 不实现/不照搬理由 | RustyEra 处理 |
|---|---|---|---|---|
| N01 | `UseLazyLoading`、`lazyloading.cfg/bin/files.bin` | 启动跳过大 ERB，首次 CALL/`EXISTFUNCTION` 时读盘解析，按 mtime 更新索引 | runtime 执行中读盘并改变符号表；mtime/二进制索引不确定；RustyEra 已有 content hash、函数/项目编译缓存和热重载 | 不读取蛇版二进制索引，不复现隐式装载。先优化已有缓存/选择；若真实性能仍不足，只允许按 manifest/hash 分页的不可变预编译 bytecode，page-in 不改变 generation/符号存在性 |
| N02 | `EXISTFUNCTION` 触发 lazy 文件加载 | 让尚未装载函数变为可见 | 查询具有 I/O/编译副作用，破坏可预测性和 snapshot；非蛇版 erafl 依赖纯查询 | 永远查询完整静态符号 manifest；source hydration/bytecode page-in 不能改变“存在”答案 |
| N03 | `BITMAP_CACHE_ENABLE` 的蛇版缓存开关、SharedBitmapCache、字体/动画缓存细节 | 控制解码/位图复用和内存 | 属前端内部优化，脚本不应改变资源 identity 或跨客户端结果 | 为兼容可接受命令，记录一次 no-op/deprecation diagnostic；缓存由前端资源预算和 content hash 管理 |
| N04 | SparseArray 的具体 C# storage | 节省一维 int/string/float 数组内存 | 内部数据结构不是兼容契约；RustyEra 可用不同表示满足同一读写/存档语义 | 只验证默认值、边界、枚举与 materialize/save 行为；根据 profile 决定是否优化 |
| N05 | `ExecutionContext`、`NullRefTerm`、`SafeArithmetic` 等具体类 | 修复递归 LOCAL/ARG、表示省略 OUT、实现安全算术 | C# 类层次不属于外部行为，照搬会破坏 Rust VM 架构 | 分别实现 call-frame 隔离、null-out IR 和 arithmetic policy 的可观察契约 |
| N06 | `SELECTCASE` 跳转表的具体优化 | 把可判定 case 编译为 jump table | compiler 优化策略不可成为脚本 ABI；RustyEra 可用其他优化 | 保证 case 顺序、范围、字符串与副作用行为；优化由 benchmark 和 IR 决定 |
| N07 | SkiaSharp/OpenGL/CPU/GDI+、sRGB 修正、GC/内存池的具体实现 | 蛇版桌面渲染和性能 | 单一平台/库内部结构，背离跨前端 presentation 模型 | 前端自由选择 Canvas/WebGL/native renderer；只共享 canonical scene、color 与资源契约 |
| N08 | SoundTouch/NAudio 具体后端 | 桌面变速、保调和播放 | 库和线程模型不是语言契约，浏览器有 WebAudio，TUI 可能无音频 | 实现 D16/C07 的抽象能力；各前端选择后端并声明 capability |
| N09 | WinForms 调试窗口、watch 锁定/改值 UI、tooltip、内存诊断 | 桌面调试与诊断体验 | RustyEra 已有跨客户端 Debug protocol；复制会形成第二套 debugger 且 Web/TUI 不可用 | 修复/扩展统一 debug descriptor、frame、generation 和变量写入协议；各前端自行呈现 |
| N10 | `MemoryDiagnosticEnabled`、.NET GC 统计 | 展示 CLR 内存与对象信息 | Rust 及 WASM 内存模型不同，数字不可比 | 使用 RustyEra 自身 telemetry/debug event；不暴露为兼容 ERB 功能 |
| N11 | `UseNewRandom` 配置本身 | 蛇版取消部分新 RNG 门控 | RustyEra 应固定可识别的 RNG algorithm/version，而不是复制模糊布尔开关 | 通过 D11 的 profile/save metadata 和显式迁移器处理 |
| N12 | 已撤销/不存在 API | `RM_RESOURCECHECK_LOAD`、`RM_RELEASE_ALL`、`RM_RESOURCE_EXIST`、`SPRITEANIMEFRAME`、`HOVER_PAUSE`、`ARGB_TO_HTML_COLOR` | 当前蛇版 HEAD 已无活动注册/实现；不能把历史说明当目标 ABI | 不注册；若真实游戏遇到再按具体版本新增 compatibility shim |
| N13 | 被参考实现已包含的修复 | `GCREATEFROMFILE` 相对路径参数、BREAKBUTTON、ENUMFILES 路径、FORCE_QUIT 等 | 不是蛇版独有差异；若 RustyEra 已正确实现，无需再次“移植” | 只做行为审计和回归测试，缺什么补什么 |
| N14 | “像素完全等同 Skia”作为跨客户端承诺 | 复制某字体、DPI、driver 下的画面 | Web 字体栅格、TUI cell 和桌面 GPU 不可能普遍像素一致 | 验收分为结构/逻辑布局一致与指定 Web/Tauri fixture 的视觉基准，不声称所有客户端像素一致 |

## 7. RustyEra 改造思路

### 7.1 总体架构

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

### 7.2 分批次实施方案

批次编号表示主线交付顺序；允许并行的工作及其汇合门禁见 7.3。每批先完成前置契约与实现，再按仓库规则完成重构审查（如触发）、静态门禁和动态验收。早期批次只验收已具备依赖的最小 fixture/调用断面，不把后续批次的真实标题、地图或存档闭环提前列为通过条件。

#### 批次 0：建立基线、profile 与门禁

- 固定蛇版 Emuera、参考实现、蛇版 TW 和 7 个回归游戏的 revision/资源 hash。
- 引入 `emuera.em`、`emuera.skia.snake` profile，并纳入 project manifest、compiled cache、save/snapshot identity。
- 自动生成“脚本出现 API → analyzer → compiler → VM/service → frontend”覆盖表，区分 unknown、trap、unsupported capability。
- 为第 2 类建立双期望 fixture；先锁定 `PRINTC`、算术、RNG、REF、extra args、`TOINT`、`GETKEY`。

验收：选择 `emuera.skia.snake` profile 不改变 `emuera.em` profile 的既有 fixture；错误中能显示 profile 和缺失 capability。

#### 批次 1：完整摄取与参考能力阻塞项

前置：批次 0 的 profile、identity 与诊断契约。

- 实施 S01/S02：三个客户端都提交 `.als/.erd`，core 建立用户 ERD/ALS。
- 实施 S03/S12：`GETMETH/GETMETHS/EXISTMETH` 与 `DT_COLUMN_OPTIONS` 从注册到 runtime 全链路可用。
- 核验标题必需的 `LOADGLOBAL/SAVEGLOBAL`，以及初始化必需的 `LOADTEXT`、MAP/XML/DT、递归资源清单与安全路径映射；发现参考能力缺口先补齐，供批次 3 的 SQL seed/XML 读取和批次 4 的启动闭环复用。
- 实施 S04 的已有服务接线：先协商并验证现有 HTML 测量、pointer、canvas pixel operation；蛇版新增标签、scene 与投影坐标的一致性留到批次 4。
- 建立蛇版 TW 静态全项目覆盖报告，逐项记录尚未支持的语法/API 及其目标批次；此时不承诺全项目编译通过，但不允许已承诺功能落到 trap。未调用函数中的后置 API 也不能直接忽略，须明确后续实现、显式 unsupported capability 或可证明安全的裁剪策略。

验收：文件数量/hash 无静默遗漏；GLOBAL、数据读取和动态方法最小 fixture 可执行，标题与 `GRAPH_DB_INIT` 的符号/动态目标可解析；所有 unsupported 都有明确层级和 capability 名。不以真实标题或图数据库初始化已运行作为本批结论。

#### 批次 2：确定性 API、输入与兼容差异骨架

前置：批次 1 的完整符号/数据摄取、动态方法和已有 service 接线。

- 实施 S05-S11、S13：EXISTVAR storage 重载、CSV/数组、bit、MAP、STRFORMCHECK、BGC、unchecked、动画查询；S12 已在批次 1 完成，S14 明确留到批次 4 的 scene 模型之后，S05 的 Float bit 留到批次 6。
- 实施 D04、D06-D08、D10-D13、D17 的 profile 分支和输入状态机。先统一 D10 的逻辑计时器再接 S13；先定 D13 的输入顺序再验 D12 的键/鼠标 latch。D04/D06 固定动态调用、实参处理和 call-frame 扩展边界，作为批次 6 新 ABI 的基础。
- 为 C03 提供 capability-based environment；兼容 `GETPLATFORM` 但发 portability diagnostic。
- D11 同步定义 RNG algorithm/state-format identity 与当前已支持状态的保存/恢复契约，不能等到批次 5 才补；外部蛇版存档导入后置。
- 对真实 176 MiB 项目先记录摄取、符号分析和编译缺口/内存基线；用已可编译 fixture 验证 compiled cache 与函数缓存。完整项目缓存收益等批次 4 编译闭环就绪后再验收；优化 N01 的替代路径，不接入 lazy 二进制索引。

验收：独立 fixture 中环境分支、NF 输入、计时器、动态调用和 RNG 状态可重复；7 个非蛇版项目的关键语义 fixture 仍按 `emuera.em` profile；可编译 fixture 有可量化缓存结果。真实标题在 NF 输入前已调用 SQL 并使用扩展 HTML，因此必须等批次 3/4，不能在本批提前宣布标题可交互。

#### 批次 3：安全 SQL（蛇版 TW P0）

前置：批次 1 的动态方法、MAP/XML/DT 与安全资源读取，批次 2 的语义/RNG/调用策略。

- 先定稿 C01 `Sql` service、存储命名空间、epoch/handle、资源限额、错误与 snapshot 规则。
- 同时定稿数据库与存档关系：衍生缓存可重建，用户数据必须进入导出/迁移策略；活动 reader/transaction 阻止 stable snapshot/reload。此契约先于批次 4 的自身存档闭环，不能留到批次 5 再决定。
- 实现蛇版 TW 实际用到的 connect、nonquery、Integer/String scalar、parameter、reader、MAP XML import。
- Tauri、Browser、TUI 使用一致 SQLite 版本/fixture；Resource seed 到 `Data/sql` 采用 copy-on-write 或显式初始化。
- 用具备明确初始变量/资源的调用断面验证 `QOL_DB_INIT`、`GRAPH_DB_INIT`、事务重建、BFS/跨地图边和 reader close；测试项目切换、异常事务、断连和配额。
- 核验同属 `INIT_NG_OR_LOAD` 的 `CREATE_BBAS_DATABASE` 数据前置；对前置报告指出缺失的 `bbas_map_*.xml`，须确认参考容错行为或报告资源阻塞，不能因 SQL 通过而假定整个初始化成功。

验收：上述 SQL 初始化断面可完成，路径不能逃逸命名空间，三个客户端相同 SQL fixture 得到相同 typed rows，数据库保存/重建策略明确。无法支持持久化的客户端在启动前明确拒绝。真实标题与完整新游戏/自身读档初始化在批次 4 汇合；外部蛇版存档读入属于批次 5/6。

#### 批次 4：主玩法 presentation、图像、scene 与自身存档闭环

前置：批次 2 的输入/计时器/RNG 契约，以及批次 3 的 SQL 与数据库保存策略。图形实现可在批次 2 后独立推进，但完整游戏验收必须等批次 3 就绪。

- 实施 D14/D15 和 C04/C05/C08：扩展 canonical HTML AST、SceneLayer、CanvasReplay、line anchor 和资源 service。
- 上述模型就绪后实施 D09 的 sprite/CBG 新重载和 S14 `EXISTSIMAGELAYER`；保留旧 arity，查询必须读取实际 scene，不能提前返回伪造值。
- 补蛇版 TW 活动使用的 `HTML_PRINTC/LC`、font/img/div 属性、CBG、sprite、动画和 pointer 坐标；在本批复验 S04 对新增标签和 viewport/scene revision 的测量、命中与像素采样。
- Web/Tauri 先达到主地图可玩；TUI 明确文本降级和 unsupported 边界，不伪造像素等价。
- 对指定字体/viewport 建结构布局和视觉 fixture；同时保留 `ColumnCell` 的跨客户端参考模式。
- 提前完成 RustyEra 自身 Integer/String、用户 SAVEDATA/CHARADATA/ERD 数组、GLOBAL 与 RNG 的保存/恢复闭环，落实批次 3 的数据库策略；不要求此时导入外部 ERAZIP 或 Float 存档。
- 闭合首个可玩范围的编译门禁：后置数学、音频、渲染等 API 若仍出现在全项目源码中，必须可正确编译并给出明确 capability 处置，或有覆盖动态调用的安全裁剪证据；实际游玩路径不得触发 unsupported/trap。无法满足时按依赖阻塞处理，不能用删游戏代码或虚假成功跳过。编译闭环通过后再验真实项目的重复启动缓存、峰值内存和口上规模。

验收：真实标题、新游戏与自身存档读入的公共初始化、QOL 菜单、地图悬停/点击、状态 UI、depth/scroll 顺序可重复；自身存读档保留变量 shape、GLOBAL、RNG 及数据库策略；重放 scene delta 不依赖前端私有对象。满足这些条件才达到首个可玩里程碑，外部蛇版存档兼容不在此结论内。

#### 批次 5：蛇版存档互操作与音频

前置：批次 4 的自身存档闭环和批次 3 的 SQL 外部状态契约。

- 实施 D18 的非 Float 子集：先读 ERAZIP 和蛇版 Integer/String、自定义数组及已支持的 RNG 状态，复用自身存档闭环；明确单向或双向兼容。未知 RNG/codec 与 Float tags 必须显式拒绝，不得丢弃、转成 Integer 或声称已兼容任意蛇版存档。
- 实施 D16/C07：规范化音频期望状态和 revision-bound 实际查询；缺能力客户端给稳定诊断。
- 按批次 3 的数据库策略验证外部存档导入/迁移，不把外部数据库假装包含在普通 save 内。

验收：已支持类型/codec/RNG 的真实蛇版存档 fixture 保留变量 shape、RNG、GLOBAL 并落实数据库策略，未支持类型有明确拒绝结果；音频查询不支持时不会悄悄返回误导值。Float 存档互操作在批次 6 补验。

#### 批次 6：完整蛇版语言

前置分层：语言主线依赖批次 2 的 profile、动态调用、实参/算术/RNG 策略；SQL Float 接入另需批次 3，Float 自身存档另需批次 4，Float 外部存档导入另需批次 5。接口与文件隔离时可并行推进语言主线，不得把尚未就绪的集成项算作通过。

- 先定 D02/C09 的 Float 类型、bit-exact wire、确定数学/格式化规则，再接 D03 的 variadic、元素 REF、OUT、`ARGLEN`；保持批次 2 的 reference/非 variadic 实参规则。
- 类型与调用 ABI 就绪后完成 D05/D19 的 EVAL/EVALS/EVALF 和 Float 动态 API，同时补 S05 的 Float bit。
- 在批次 4 的自身存档模型上增加 Float save tags；在批次 5 的 importer 上增加外部 Float 存档支持。SQL Float 仅在批次 3 的 service 与本批 Float 契约均就绪后开放。
- 对递归 call frame、alias 生命周期、null OUT、动态表达式错误位置和 cache invalidation 做模型测试。

验收：23/83 API 的语言层 inventory 全部有实现或明确的 service/diagnostic disposition；Float 自身存档 round-trip bit exact，外部 Float 导入与 SQL Float 分别完成集成验证。仅语言主线完成不能宣布本批全部完成。

#### 批次 7：可选 extension 与渲染能力

前置分层：C02 接入依赖批次 0/1 的 profile、capability 与既有 Extension protocol；C06/C10 渲染与 viewport 接入依赖批次 4 的 presentation/scene 契约。只有使用 Float 值/schema 的扩展才额外等待批次 6，不把完整语言作为所有可选能力的统一前置。

- 用 C02 的 Extension protocol 承载明确声明的宿主扩展；不实现任意 CLR 反射。
- 用 C06 的 renderer-neutral hints 表达 strict fallback、quality、text drawing；前端独立选择 renderer。
- C10 的缩放/窗口偏好与 pointer 逆映射复用批次 4 的逻辑坐标和 viewport revision，不新增本机窗口语言对象。
- 改进统一 debug protocol 和各前端 UI，不移植 WinForms debugger、内存窗口或 Skia backend。

验收：同一项目在不具备可选能力的客户端启动前即可得到准确 capability 报告；可选后端不改变 core save/snapshot identity 之外的语义。

### 7.3 批次依赖与首个“可玩”里程碑

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

## 8. 完整 API 归类索引

本节用于证明 23 个新增命令和 83 个新增表达式方法均已得到处置。括号内是主表 ID；“3/替代”表示不照搬蛇版实现，但计划提供兼容脚本契约。

### 8.1 新增 23 个命令

| 命令 | 分类 | 处置 |
|---|---|---|
| `CALLSTR`、`JUMPSTR`、`TRYCALLSTR`、`TRYJUMPSTR`、`TRYCCALLSTR`、`TRYCJUMPSTR` | 2（D04） | 基于版本化动态调用 IR 实现 |
| `TINPUTNF`、`TINPUTSNF`、`TONEINPUTNF`、`TONEINPUTSNF` | 2（D13） | 统一 input state machine；viewport 保持为前端 policy |
| `SETIMAGELAYER`、`CLEARIMAGELAYER`、`CLEARIMAGELAYER_ALL` | 2（D14/D15） | 写规范化 SceneLayer/delta |
| `SETIMAGELAYERL` | 3/替代（C08） | 用稳定 `DisplayLine.line_id` 锚点，不直接持有物理行 Y |
| `HTML_PRINTC`、`HTML_PRINTLC` | 3/替代（C04） | canonical column + revision-bound pixel intent/query |
| `TEXT_BGC_ON`、`TEXT_BGC_OFF` | 1（S10） | 规范化 presentation style |
| `STRICT_FONT_FALLBACK` | 3/替代（C06） | renderer-neutral fallback policy |
| `SET_SKIA_QUALITY`、`SET_TEXT_DRAWING_MODE` | 3/替代（C06） | 抽象渲染 hint，不暴露 Skia 枚举 |
| `SETANIMETIMER` | 2（D10） | 兼容语法迁移，参数接受一般表达式 |
| `BITMAP_CACHE_ENABLE` | 4（N03） | 接受/no-op+诊断；缓存归前端 |

计数：6 + 4 + 3 + 1 + 2 + 2 + 1 + 2 + 1 + 1 = **23**。

### 8.2 新增 83 个表达式方法

| 方法 | 分类 | 处置 |
|---|---|---|
| `TOFLOAT`、`TOSTRF` | 2（D02）/3（C09） | Float 类型与确定性格式化 |
| `SIN`、`COS`、`TAN`、`ASIN`、`ACOS`、`ATAN`、`FLOOR`、`CEIL`、`ROUND` | 2（D02）/3（C09） | Float/Integer 重载与固定数值契约；蛇版 TW 当前仅活动使用 `COS` |
| `UNCHECKED_ADD`、`UNCHECKED_SUB`、`UNCHECKED_MUL`、`UNCHECKED_NEG` | 1（S11） | 明确二补码回环 |
| `GETVARF`、`GETMETHF`、`DT_CELL_GETF` | 2（D19） | 随 Float 动态值全栈实现 |
| `EVAL`、`EVALF`、`EVALS` | 2（D05） | `emuera.skia.snake` profile 的受限动态表达式 |
| `ARGLEN` | 2（D03） | call frame/variadic ABI |
| `STRFORMCHECK` | 1（S09） | 纯格式验证 |
| `MATCHALL`、`MATCHALLEX` | 1（S06） | 确定性数组匹配 |
| `GETCSVNOBYNAME`、`GETCSVNOBYNICKNAME`、`GETCSVNOBYCALLNAME`、`GETCSVNOBYMASTERNAME` | 1（S06） | CSV 索引查询 |
| `BITSET`、`BITGET`、`BITTOGGLE`、`BITINDEXOFFIRST` | 1（S07） | bit storage 操作 |
| `SPRITECREATEFROMFILE` | 3/替代（C05） | Resource/Image service，禁绝对路径 |
| `G_POLYGON_DRAW`、`G_POLYGON_FILL`、`G_POLYGON_POINT_ADD`、`G_POLYGON_POINT_CLEAR` | 2（D15） | canonical CanvasReplay |
| `EXISTSIMAGELAYER` | 1（S14） | scene 只读查询 |
| `GETLINEY` | 3/替代（C04） | line id + projection query |
| `GETANIMETIMER` | 1（S13） | 逻辑动画计时查询 |
| `GETPLATFORM` | 3/替代（C03） | 稳定 capability/environment service |
| `GET_TEXT_DRAWING_MODE`、`GET_SKIA_QUALITY` | 3/替代（C06） | 返回抽象策略，不返回后端对象/枚举 |
| `SEQUENCEINPUT`、`DISABLE_INPUT_MACRO`、`ENABLE_INPUT_MACRO` | 2（D13） | 统一输入状态机 |
| `GETSOUNDORBGMINFO`、`ISPLAYINGSOUND`、`ISPLAYINGBGM` | 3/替代（C07） | revision-bound AudioQuery |
| `SOUNDCONTROL`、`BGMCONTROL` | 2（D16） | 规范化 AudioState/effect |
| `MAP_VALUES`、`MAP_MERGE`、`MAP_REMOVEIF`、`MAP_FINDKEY`、`MAP_TOSTRING`、`MAP_FROMSTRING` | 1（S08） | 稳定顺序和 round-trip |
| `SQL_CONNECTION_OPEN`、`SQL_CONNECT`、`SQL_DISCONNECT`、`SQL_EXECUTE_NONQUERY`、`SQL_EXECUTE_READER`、`SQL_READER_READ`、`SQL_READER_GET_LONG`、`SQL_READER_GET_FLOAT`、`SQL_READER_GET_STRING`、`SQL_READER_ISNULL`、`SQL_READER_CLOSE`、`SQL_EXECUTE_SCALAR_LONG`、`SQL_EXECUTE_SCALAR_FLOAT`、`SQL_EXECUTE_SCALAR_STRING`、`SQL_IMPORT_MAP_XML`、`SQL_IMPORT_DT_XML`、`SQL_EXPORT_MAP_XML`、`SQL_EXPORT_DT_XML`、`SQL_IMPORT_XML_CUSTOM`、`SQL_ESCAPE`、`SQL_P_EXECUTE_NONQUERY`、`SQL_P_EXECUTE_READER`、`SQL_P_EXECUTE_SCALAR_LONG`、`SQL_P_EXECUTE_SCALAR_FLOAT`、`SQL_P_EXECUTE_SCALAR_STRING` | 3/替代（C01） | typed/versioned Sql service；先 Integer/String 和游戏实用子集，Float 后置 |

计数：2 + 9 + 4 + 3 + 3 + 1 + 1 + 2 + 4 + 4 + 1 + 4 + 1 + 1 + 1 + 1 + 2 + 3 + 3 + 2 + 6 + 25 = **83**。

### 8.3 语法、修改行为和内部特性覆盖

| 功能族 | 分类/ID |
|---|---|
| Float 声明、变量、运算、save tags | 2：D02、D19；确定性部分 3：C09 |
| variadic、元素 REF、OUT、多余实参 | 2：D03、D06 |
| `EXISTVAR` 第二参数 | 1：S05 |
| `EXISTFUNCTION` lazy 副作用 | 4：N02 |
| `SPRITECREATE` / `CBGSETSPRITE` 新参数 | 2：D09 |
| `GETDISPLAYLINE` 负数 | 2：D08 |
| `SETANIMETIMER` / `BITMAP_CACHE_ENABLE` 注册迁移 | 2：D10；缓存实现 4：N03 |
| `PRINTC/PRINTFORMC` 像素列宽 | 3：C04 |
| RNG 命令门控变化 | 2：D11；旧布尔配置 4：N11 |
| 饱和/除零算术 | 2：D07；显式 unchecked 1：S11 |
| XML/字符串/TOINT/鼠标 latch 修正 | 2：D12 |
| lazyloading | 4：N01、N02 |
| HTML、ImageLayer、字体与渲染 | 2：D14、D15；3：C04、C06、C08；4：N07、N14 |
| 输入扩展 | 2：D13 |
| 音频扩展 | 2：D16；3：C07；后端 4：N08 |
| SQL | 3：C01 |
| MAP/bit | 1：S07、S08 |
| 递归调用上下文、稀疏数组、编译优化 | 可观察行为按 D03/D07；具体实现 4：N04-N06 |
| 用户 ERD ALS | 1：S01、S02 |
| error hooks | 2：D17 |
| desktop/debug/memory | 3：C10；4：N09、N10 |
| 历史已移除 API | 4：N12 |

## 9. 风险、待动态核验项与验收原则

### 9.1 尚不能由静态审计证明的事项

- 蛇版 TW 动态 `CALLFORM/TRYCALLFORM` 生成的全部目标及其运行覆盖。
- `CREATE_BBAS_DATABASE` 对仓库中未找到的 `bbas_map_*.xml` 的真实容错路径。
- 5 个高疑似“多余空实参”在目标 parser 中的准确归因。
- SQL 查询对 SQLite 版本、collation、类型 affinity、NULL 和迭代顺序的隐含依赖。
- 指定字体缺失、DPI、浏览器字体替换时，蛇版像素布局可接受的误差边界。
- 176 MiB 全量编译在 Browser/Tauri/TUI 的实际启动时间和内存；这决定是否需要预编译 bytecode paging，而不是是否复刻 lazyloading。
- 现有 7 个 `.sav` 的所有 codec/type/RNG 版本，以及 SQL 数据库究竟是纯衍生缓存还是包含不可重建用户状态。

### 9.2 每批次通用验收

1. **参考回归**：`emuera.em` 下旧 fixture 和代表游戏行为不变。
2. **蛇版差分**：同一最小 ERB 在蛇版 Emuera 与 `emuera.skia.snake` 下比较结果、错误、输出和状态。
3. **跨客户端**：规范化 runtime event/state 相同；前端允许声明视觉/音频降级，但不能伪造成功。
4. **缓存/存档身份**：profile、RNG、save codec、service 版本不一致时拒绝复用或显式迁移。
5. **安全边界**：SQL、资源、extension 均不能逃逸项目命名空间、权限和资源配额。
6. **目标游戏断面**：按当前批次已就绪的依赖验证最小断面，未就绪项明确记录而非提前要求通过；批次 4 汇合项目摄取、静态编译、标题、新游戏/自身读档初始化、地图交互、布局、资源、自身存档和大口上性能，批次 5/6 再补外部存档及 Float 集成。

## 10. 最终建议

建议把未来兼容目标表述为：

> RustyEra 以 `emuera.em` 保留现有兼容语义，并新增版本化 `emuera.skia.snake` 方言；对蛇版可观察语言契约提供兼容，对数据库、平台、渲染、音频和扩展代码采用 RustyEra 的跨客户端 service/规范化状态实现，不承诺复制蛇版的 WinForms/Skia/CLR/lazyloading 内部架构。

这样可以先按批次 0-4 达成蛇版 TW 的首个可玩及自身存档闭环，同时把旧游戏风险隔离在显式 profile 中；随后补蛇版存档互操作、Float 和完整语言能力。第 3 类不是永久缺口，而是需要以跨平台替代契约实现；第 4 类则应明确拒绝或迁移，避免为了“名称齐全”牺牲 RustyEra 的架构和长期可维护性。

## 11. 主要证据路径

- 前置事实报告：[蛇版兼容性详查](SNAKE_EMUERA_TW_RUSTYERA_COMPATIBILITY_RESEARCH.md)
- RustyEra 设计原则：`rustyera-core/docs/design-principles.zh-CN.md`
- runtime/frontend 接口：`rustyera-core/docs/runtime-frontend-interface.zh-CN.md`
- 参考语义映射：`rustyera-core/docs/runtime-reference-mapping.zh-CN.md`
- 蛇版 API 注册：`emuera_lazyloading_selfmodified_version/Emuera/Runtime/Script/Statements/Function/Creator.cs`
- 蛇版命令注册：`emuera_lazyloading_selfmodified_version/Emuera/Runtime/Script/Statements/FunctionIdentifier.cs`
- 蛇版 lazyloading：`emuera_lazyloading_selfmodified_version/Emuera/Runtime/Script/Process.LazyLoading.cs`
- 蛇版 SQL：`emuera_lazyloading_selfmodified_version/Emuera/Runtime/Utils/尊尼获加/SqlManager.cs`
- 蛇版 ImageLayer：`emuera_lazyloading_selfmodified_version/Emuera/UI/Game/ImageLayerManager.cs`
- 蛇版 TW 启动：`games/eratw-sub-modding/ERB/TITLE.ERB`、`ERB/SYSTEM.ERB`
- 蛇版 TW SQL/地图：`games/eratw-sub-modding/ERB/魔改内容/qol/`
- RustyEra compiler/VM/runtime：`rustyera-core/crates/erabasic-compiler/`、`erabasic-vm/`、`era-runtime/`
- 客户端扫描：`rustyera-tui/src/rustyera_tui/project_scan.py`、`rustyera-web/src/platform/browserProjectFilesystem.ts`、`rustyera-web/src-tauri/src/project/scan.rs`
