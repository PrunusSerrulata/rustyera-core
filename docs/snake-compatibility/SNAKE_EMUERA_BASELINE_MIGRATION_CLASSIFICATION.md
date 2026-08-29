# 蛇版 Emuera 兼容基线迁移：RustyEra 功能分类与实施方案

> 调研日期：2026-08-26\
> 性质：源码与游戏资源的只读静态审计；不是运行通过或行为等价证明\
> 前置事实报告：[蛇版兼容性详查](SNAKE_EMUERA_TW_RUSTYERA_COMPATIBILITY_RESEARCH.md)\
> 建议目标：在保留参考实现兼容性的前提下，让 `games/eratw-sub-modding`（蛇版 TW）逐步进入可玩状态；不把蛇版 Emuera 的 C#/WinForms/Skia 内部实现整体移植到 RustyEra

文中的源码/资源路径以多组件工作区根目录（`rustyera-core` 的上一级）为基准；省略游戏前缀的 `ERB/` 等路径相对于所述游戏目录，不相对于本文目录。审计日期、revision 与“当前缺口”均指本次历史审计，不代表后续实现进度。

2026-08-29 按用户批准的[批次 2 详细实施方案](BATCH_2_IMPLEMENTATION_PLAN.md)修正
CALLSTR 完整调用文本、EXISTVAR 解析模式、STRFORMCHECK 实际展开、MAP 无转义格式、
全局历史文本背景及 RNG 选择。下述实施方向是已选契约，不是完成或验收结论；历史 oracle
原始差异保持不变，实际结果仅在整批最终结束后写入实施记录。

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
| 普通整数安全算术 | 原版 `eraTW/ERB/COMMON.ERB:2366-2375 @NOISE` 依赖自然回环；`erarorona` 有运行时可能为零的动态除数 | 普通运算保持参考 profile；snake 按固定参考逐操作实现安全算术及诊断，保留 `MIN/-1`、postfix 等特殊边界；哈希用显式 `UNCHECKED_*` |
| `INITRAND/DUMPRAND` | 原版 eraTW、erafl、eratohoK 均使用；配置依赖传统 MT/RANDDATA | 状态操作必须对应当前实际 RNG；不可让蛇版规则静默操作一套未使用的 MT 状态 |
| `#DIM/#DIMS REF` | 约 1,400+ 声明，erafl 1,032、原版 eraTW 318 尤其密集 | 现有数组引用必须原样保留；新的标量元素 REF 使用独立语法/类型分支 |
| `EXISTVAR` | erafl 有 16 个单参数调用 | 单参数 bitmask 不变；第二参数只作为加法式重载 |
| `EXISTFUNCTION` | erafl 有 16 个调用，无 lazy 配置 | 必须保持纯符号查询；禁止加入隐式读盘/编译副作用 |
| `SPRITECREATE` | 47 行；只发现 2 参数与 6 参数形式，没有 8/10 参数 | 精确保留 2/6 参数；新重载可在蛇版 profile 开放 |
| `CBGSETSPRITE` | erafl 两行，均为旧 4 参数 | 4 参数行为冻结；新 opacity/matrix/尺寸作为可选尾参 |
| `SETANIMETIMER` | 原版 eraTW 3 行、erafl 1 行，均为命令外形；erafl 参数是表达式 `1000 / フレームレート` | parser 应接受一般表达式，不能只识别整数字面量；表达式式/命令式需兼容迁移测试 |
| `GETDISPLAYLINE` 负数 | 未发现活动负索引 | 当前语料风险低，但返回空串到倒序索引仍是可观察变化，放 profile |
| 多余实参 | 仅在 `era魔界牧場` 找到 5 个高疑似“4 参数后尾随空项”，尚需 parser 复核 | reference 默认继续报错；`emuera.skia.snake` profile 忽略并发 warning，记录函数名和位置 |
| `XML_ADDNODE` 多目标 | 仅 erafl 有 9 行；正常数据目标唯一 | 现有 clone 实现补双 oracle 回归并记录差异，不为建立 profile 分支回退原版行为或额外告警 |
| `TOINT` 整数读取异常 | 活动调用量很大（合计 1,517 行），未发现常量非法字面量，但用户/XML/运行时字符串可超范围 | 两版对空串、普通非数字串均返回 0；原版传播整数读取异常，蛇版捕获后返回 0。批次 2 不添加参考不存在的 warning，不改变 ISNUMERIC 或吞掉无关故障 |
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
| S05 | `EXISTVAR(name, mode)` 第二参数 | 非零模式执行表达式解析，保留调用方 scope、ERD/ALS、REF 解析和参数求值顺序；不读取 storage cell，不额外验证访问越界 | 单参数及 mode=0 保留 bitmask；复用动态表达式前端，Float bit 留批次 6 | 加法式重载，其他游戏只使用单参数；解析不能误作实际存储访问 | M |
| S06 | 数组/角色 CSV 查询 | `MATCHALL/MATCHALLEX` 批量匹配；四个 `GETCSVNO*` 按原始 name/nickname/callname/mastername 查角色号 | 索引在 CALLNAME 回填前按 NO 逆序写入，正常域重复名取最小 NO；保留缺字段/空字段区别，极端 NO 使用 i64 安全总序并登记参考截断排序差异 | 无 I/O、无旧签名覆盖；数组输出仍有写回副作用 | S/M |
| S07 | bit 数组操作 | `BITSET/BITGET/BITTOGGLE/BITINDEXOFFIRST` 在 `long[]` bit storage 上修改或查找 | 定义负索引、越界、空数组和返回值 | 局部确定性运算，名字全新 | S |
| S08 | MAP 确定性扩展 | `MAP_VALUES/MERGE/REMOVEIF/FINDKEY/TOSTRING/FROMSTRING` 提取、合并、过滤、反查和序列化 | 保留插入顺序、无转义格式及逐条写入；key/contains/equality 为 ordinal，前后缀及键值分隔符查找固定为参考环境的 ICU invariant-culture 规则；UTF-16 切分不可表示时明确报错 | 名字全新且建立在既有 MAP 状态上；不触碰旧操作，round-trip 仅限无分隔符冲突数据 | M |
| S09 | `STRFORMCHECK` | 解析并实际展开格式字符串，成功返回 1、受捕获的脚本解析/展开错误返回 0，保留已发生副作用 | 复用正常 STRFORM、服务权限与等待；参数求值失败仍传播；取消、资源耗尽、坏字节码和协议故障不能伪装为检查失败；捕获 continuation 恢复 caller/REF/资源 | 新增 API，不是纯检查；跨动态执行与错误生命周期，按 2B 大批次实现 | M/L |
| S10 | `TEXT_BGC_ON/OFF` | 设置/清除全局整行文本背景，影响符合条件的已有历史行 | 独立规范化背景状态保存 RGB/alpha/行资格，进入 snapshot/delta；不复用 run 背景，TUI 显式颜色合成降级 | 状态是抽象颜色/开关，能跨客户端降级；历史投影需同步更新 | M |
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
| D04 | 字符串动态调用族 | 六变体接收一个字符串表达式，运行时解析完整 `目标(实参...)` 或 `目标, 实参...`；空白文本无操作 | 扩大动态可达集合，影响动态参数求值、静态裁剪、缓存和 missing-target 错误 | 复用动态表达式前端、调用方符号环境、REF/generation/continuation，不做 CALLFORM 名称别名；保留 TRY/CATCH/JUMP、转换错误与栈展开边界，动态目标进入保守缓存依赖 | L |
| D05 | 动态表达式求值 | `EVAL/EVALS/EVALF` 在运行时解析并求值 | 影响可预测性、错误位置、性能和 snapshot；`EVALF` 又依赖 Float | 只在 `emuera.skia.snake` profile 开启；限制为纯表达式语法，复用已版本化 parser/bytecode，缓存以源码+方言为 key | L |
| D06 | 多余实参处理 | 蛇版非 variadic 函数丢弃多余参数且不求值，参考实现报 `TooManyFuncArgs` | 会让旧脚本错误变成“成功”，也可能掩盖拼写/签名问题；先求值再截断会产生错误副作用 | reference 报错；snake 执行并发稳定 code/位置/generation 去重 warning，提供严格诊断开关；覆盖静态/动态调用、方法和 STRFORM，内置 arity 不放宽 | L |
| D07 | 普通整数安全算术 | 蛇版按操作采用安全算术和 warning，除/模零返回 0；`MIN/-1`、postfix 等边界须逐项保留参考期望；显式 `UNCHECKED_*` 回环 | 原版 eraTW 的 `NOISE` 明确依赖回环；动态除零错误路径也会变化 | `erabasic-compat` 集中策略贯穿 analyzer/HIR/compiler/VM/runtime、折叠及优化；warning 按 code/位置/generation 去重，不笼统将所有溢出改成饱和；独立诊断不进入脚本历史，保留相对蛇版的输出及历史查询差异 | L |
| D08 | `GETDISPLAYLINE` 负索引 | `-1` 取最后一行，依次倒序；参考实现负数为空 | 会改变分支/文本；虽然 7 个非蛇版项目未发现活动负索引，仍是可观察差异 | `emuera.skia.snake` profile 启用；基于稳定 `DisplayLine.line_id`/history 索引，越界规则固定 | S/M |
| D09 | sprite/CBG 新重载 | `SPRITECREATE` 增至 8/10 参数；`CBGSETSPRITE` 增尺寸、opacity、ColorMatrix | 参数解释和默认值可能污染已有 2/6、4 参数形式 | 旧 arity 单独 handler 并做 golden test；新尾参映射到规范化 sprite/canvas replay | M/L |
| D10 | `SETANIMETIMER` / `BITMAP_CACHE_ENABLE` 语法迁移 | 从参考表达式注册迁为蛇版命令；前者设置动画节拍，后者切换位图缓存 | 同名 token 的 parse 形态、返回值、参数表达式可能变化；其他游戏存在命令式 `SETANIMETIMER` | 保留原版表达式/命令外形与一般表达式参数；和 S13 共用逻辑计时器，越过 `i32::MIN..=32767` 原子报错、非正停用、1–9 取 10；snake 命令不改 RESULT；缓存行为归 N03 | M |
| D11 | RNG 状态命令 | 蛇版状态命令作用于 MT；固定基准 dump 写入临时副本，restore 丢状态；开启 UseNewRandom 还存在另一随机路径 | 非蛇版游戏依赖 RANDDATA；错误绑定或直接修复基准缺陷会改变可观察序列，同算法名不代表状态兼容 | 批次 2 已选择统一 SFMT 与 625 项 RANDDATA 权威状态，正确 dump/restore，不复刻缺陷或 `.NET Random` 双状态；明确不保证开启 UseNewRandom 的同 seed 序列，升级实际 identity，旧 snake cache 重建、不兼容状态拒绝，外部适配后置 | L |
| D12 | 参考行为修正集合 | `XML_ADDNODE` 多目标 clone、字符串 `>=/<=`、`TOINT` 整数读取异常返回 0、鼠标键 latch | 数据重复、超范围输入和输入时序下会改变结果；`TOINT` 使用面很大 | 每项双 oracle fixture/trace；已有 XML/字符串实现不回退，TOINT 不加 warning；键鼠按统一输入状态机与真实泵验证，TUI 撤销无法提供的完整按键能力 | M |
| D13 | NF 输入、序列输入和宏开关 | `TINPUT*NF` 超时等待但保留上滚位置；`SEQUENCEINPUT` 注入下一次输入；临时开关宏 | 改变 focus、scroll、timeout、队列消费和自动化时序 | 建立统一 input state machine；逻辑事件由 runtime 排序，viewport 保持为前端 policy；录制 input trace 回归 | L |
| D14 | 规范化 HTML / scene 扩展 | `<font size/valign/render/...>`、`<img xpos/display/matrix>`、`<div>`、ImageLayer 的 depth/opacity/锚点 | 旧标签默认、换行、层叠和保存 presentation state 可能变化 | 扩展 canonical AST 和 `SceneLayer`，未知属性按 profile 诊断；物理查询另走 C04/C08 service | L |
| D15 | polygon、canvas 与动画语义 | `G_POLYGON_*` 维护点集并描边/填充；sprite/图像支持尺寸、翻转、动画 | 影响绘制顺序、坐标、ColorMatrix 与 replay；不同前端输出可能不同 | runtime 只产生确定性 CanvasReplay/scene delta；固定点颜色矩阵和稳定同 depth 顺序 | L |
| D16 | 音频“期望状态”控制 | `SOUNDCONTROL/BGMCONTROL` 的 pause/resume/stop/seek/rate/preserve-pitch | 改变 AudioState 与一次性 effect 顺序；客户端支持程度不同 | 扩展规范化 AudioState 与 `Pause/Resume/SetRate/Seek` effect；不把实际播放进度当 core 状态 | M/L |
| D17 | `BEFORE_THROW/BEFORE_ERROR` | 错误抛出前调用脚本事件，可由配置禁用 | 改变错误路径、可重入性、输出和最终异常；可能把原错误覆盖 | 仅 `emuera.skia.snake` profile；事件带原 error id/location，设 recursion guard，hook 自身失败保留原错误 | M |
| D18 | 存档与 ERAZIP 互操作 | 读取蛇版压缩存档、自定义 SAVEDATA/CHARADATA/ERD 数组及 RNG/type 元数据 | 格式、变量排序和类型标签不同；错误兼容会损坏数据 | 先做只读 importer 和 fixture；RustyEra 自身格式版本化，确认后再承诺写回/双向 | L |
| D19 | `GETVARF/GETMETHF/DT_CELL_GETF` | 动态取得 Float 变量、函数结果和 DataTable cell | 是动态值/重载体系的一部分，不能作为孤立 `f64` host stub | 随 D02/D03/D05 一起贯通类型与错误规则 | L（D02 子项） |

## 5. 第 3 类：不符合 RustyEra 设计准则的项目及替代方案

本类拒绝的是蛇版的**直接实现方式**，不是一概拒绝游戏需要。每项都给出跨客户端替代契约，并在[改造思路的分批次方案](SNAKE_EMUERA_MIGRATION_PLAN.md#batches)中安排实施批次。

| ID | 蛇版功能/实现 | 冲突原因 | RustyEra 应保留的脚本需求 | 可行替代方案 | 批次 |
|---|---|---|---|---|---|
| C01 | SQL 全族直接使用 `Microsoft.Data.Sqlite`，接受连接串和工作目录/存档路径 | core 直接 I/O；任意路径；浏览器无等价文件系统；连接/reader 不能稳定 snapshot | 蛇版 TW P0 所需连接、事务、参数化语句、scalar、reader、MAP XML 导入 | 新增版本化 `Sql` service：runtime 只持有带 epoch 的逻辑 connection/reader handle；路径映射 `Resource` 种子和 `Data/sql` 可写 overlay；Tauri/TUI 用锁定版本 SQLite，浏览器用 WASM SQLite+OPFS/内存回退；typed canonical wire、参数绑定、配额和稳定错误；活动 reader/transaction 阻止 stable snapshot/reload。首期只做游戏用到的 Integer/String，不做任意连接串 | 3 |
| C02 | `CALLSHARP` 任意 CLR DLL 和反射 | 浏览器/TUI 不可用；绕过 ABI、权限、确定性、资源限制和快照 | 游戏或插件调用受控宿主能力 | 使用已有 typed/versioned `Extension` protocol：manifest 声明 operation、schema、版本、权限和可用客户端；runtime 只发规范化请求。桌面专有插件可以显式 unsupported，不能伪装成跨平台。先修复当前 declaration/builtin 冲突；蛇版 TW 无活动调用，后置 | 7 |
| C03 | `GETPLATFORM` 直接暴露宿主平台字符串/枚举 | 脚本据此分叉会导致跨客户端结果不同；平台名称不稳定 | 蛇版 TW 标题选择桌面 `TINPUTNF` 或普通输入 | 增加版本化 Environment 与 `ENV_HAS_CAPABILITY(name[, major])`，查询已协商能力返回 0/1；GETPLATFORM 对保持视口定时输入能力映射 0，否则 5，按调用位置发 portability diagnostic，不表示实际 OS | 2 |
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
| N11 | `UseNewRandom` 配置本身 | 蛇版取消部分新 RNG 门控，状态命令和随机取值可走不同状态 | RustyEra 应固定可识别的 RNG algorithm/version，而不是复制模糊布尔开关 | D11 统一 SFMT，不复刻 `.NET Random` 双状态或临时副本写入缺陷；差异进入实际 policy/save identity，外部状态适配后置 |
| N12 | 已撤销/不存在 API | `RM_RESOURCECHECK_LOAD`、`RM_RELEASE_ALL`、`RM_RESOURCE_EXIST`、`SPRITEANIMEFRAME`、`HOVER_PAUSE`、`ARGB_TO_HTML_COLOR` | 当前蛇版 HEAD 已无活动注册/实现；不能把历史说明当目标 ABI | 不注册；若真实游戏遇到再按具体版本新增 compatibility shim |
| N13 | 被参考实现已包含的修复 | `GCREATEFROMFILE` 相对路径参数、BREAKBUTTON、ENUMFILES 路径、FORCE_QUIT 等 | 不是蛇版独有差异；若 RustyEra 已正确实现，无需再次“移植” | 只做行为审计和回归测试，缺什么补什么 |
| N14 | “像素完全等同 Skia”作为跨客户端承诺 | 复制某字体、DPI、driver 下的画面 | Web 字体栅格、TUI cell 和桌面 GPU 不可能普遍像素一致 | 验收分为结构/逻辑布局一致与指定 Web/Tauri fixture 的视觉基准，不声称所有客户端像素一致 |

## 7. RustyEra 改造思路

本章已独立为[《RustyEra 改造思路》](SNAKE_EMUERA_MIGRATION_PLAN.md)，统一维护总体架构、分批次实施方案、依赖关系与首个可玩里程碑。本处保留原章号作为入口，不再重复维护计划正文；后续开发以独立文档为准。

## 8. 完整 API 归类索引

本节用于证明 23 个新增命令和 83 个新增表达式方法均已得到处置。括号内是主表 ID；“3/替代”表示不照搬蛇版实现，但计划提供兼容脚本契约。

### 8.1 新增 23 个命令

| 命令 | 分类 | 处置 |
|---|---|---|
| `CALLSTR`、`JUMPSTR`、`TRYCALLSTR`、`TRYJUMPSTR`、`TRYCCALLSTR`、`TRYCJUMPSTR` | 2（D04） | 运行时解析完整调用文本及实参，复用版本化动态调用 IR |
| `TINPUTNF`、`TINPUTSNF`、`TONEINPUTNF`、`TONEINPUTSNF` | 2（D13） | 统一 input state machine；viewport 保持为前端 policy |
| `SETIMAGELAYER`、`CLEARIMAGELAYER`、`CLEARIMAGELAYER_ALL` | 2（D14/D15） | 写规范化 SceneLayer/delta |
| `SETIMAGELAYERL` | 3/替代（C08） | 用稳定 `DisplayLine.line_id` 锚点，不直接持有物理行 Y |
| `HTML_PRINTC`、`HTML_PRINTLC` | 3/替代（C04） | canonical column + revision-bound pixel intent/query |
| `TEXT_BGC_ON`、`TEXT_BGC_OFF` | 1（S10） | 独立全局整行背景状态，作用于符合条件的历史行 |
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
| `STRFORMCHECK` | 1（S09） | 实际展开并保留副作用；受捕获的解析/展开错误返回 0 |
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
| `MAP_VALUES`、`MAP_MERGE`、`MAP_REMOVEIF`、`MAP_FINDKEY`、`MAP_TOSTRING`、`MAP_FROMSTRING` | 1（S08） | 稳定顺序、无转义格式；仅无分隔符冲突数据 round-trip |
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
| 逐操作安全/除零算术 | 2：D07；显式 unchecked 1：S11 |
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
