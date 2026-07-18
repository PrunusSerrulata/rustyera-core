# Runtime 兼容性与功能状态

本文是 runtime 全生命周期能力的持续维护清单，而非某一批次的完成报告。状态以仓库
当前实现和固定的 `reference/emuera.em` 为依据，覆盖项目加载、标题、新游戏、系统流程、
脚本执行、输入与计时、展示、存档、热替换、调试和退出。

冲突裁决遵循
[项目最高设计准则](design-principles.md)：**跨客户端/跨平台支持 > 架构纯净 > 与参考
实现严格行为一致**。因此，下文把遗漏功能、稳定不支持和因更高优先级原则产生的有意
差异分开记录。协议或 catalog 中存在名称并不表示该能力已经实现。

## 结论

当前 runtime 不能视为“已完成”或“与参考实现完整兼容”。基础协议、VM 驱动、存档、
snapshot、热替换和主要系统流程框架已经存在，但仍有数个会阻止真实游戏正常运行的
高优先级缺口：

- `PRINTDATA*`、`STRDATA`、带下标目标、动态 label、事件调用和候选调用列表已经
  可用；这些原 1.1 阻断项不再属于已知缺口。
- `PRINT*` 的 K/D 后缀和常用专用输出已经实现，但 N/SINGLE/C 等后缀仍缺少完整语义。
- 调教和 `EVENTCOMEND` 主流程已实现；SHOP 自动存档已按进入来源限定。
- 很多已进入 Host catalog 的命令最终落入通用 `UnsupportedRuntimeFeature`。
- Protocol 20.0 保留前端实际渲染观测 typed service、强类型坐标和结构化 HTML，并新增完整展示 delta；具体应用前端仍需
  实现这些可选能力，runtime 不提供布局或 raster 近似回退。

主要证据集中在：

- `crates/era-runtime/src/session.rs`
- `crates/era-runtime/src/host.rs`
- `crates/erabasic-compiler/src/lowering.rs`
- `crates/erabasic-compiler/src/registry.rs`
- `reference/emuera.em/Emuera/Runtime/Script/Process.SystemProc.cs`
- `reference/emuera.em/Emuera/Runtime/Script/Statements/Instraction.Child.cs`

## 生命周期覆盖概况

| 阶段 | 当前状态 | 主要问题 |
| --- | --- | --- |
| 握手、能力协商 | 基本完成 | 一些能力被固定关闭；部分 catalog 能力实际不可执行 |
| 项目提交、CSV/ERH/ERB 编译 | 部分完成 | 1.1 的调用兼容配置已投影；其他非调用配置项覆盖仍不完整 |
| 资源加载 | 架构已实现 | 图片解码前端化是有意设计；物理绘图能力大量不支持 |
| 标题画面 | 主流程完成 | 内建内容、数字输入和 slot 跳页已对齐；实际绘制由前端投影 |
| 新游戏初始化、EVENTFIRST | 主流程完成 | `SYSTEM_TITLE` 先于 ResetData 和初始角色插入 |
| TRAIN/连续调教 | 主流程完成 | 物理列布局仍由后续展示兼容任务处理；`STOPCALLTRAIN` 的参考崩溃被有意修正 |
| AFTERTRAIN/ABLUP/TURNEND | 主流程存在 | BEGIN 清除 SKIPDISP 并重置展示样式 |
| SHOP | 主流程完成 | 自动存档来源和 EVENTSHOP 延迟 BEGIN 已对齐；实际菜单布局由前端投影 |
| 普通脚本执行 | 部分完成 | 1.1 所列动态调用、打印数据块和阻断指令已实现；其余差异见后续各节 |
| 输入/QTE/计时 | 主框架完成 | ONEINPUT 权威规范化已完成；部分 UI 输入函数缺失 |
| 文本、HTML、图片、音频 | 语义模型部分完成 | PRINT 后缀、HTML、图片参数和样式操作缺失 |
| 传统存档、VM snapshot | 基础完成 | 菜单和部分失败行为不同；Protocol 15.0 已实现 runtime 自有 Ctrl-Z 轨迹 |
| 热替换 | Rust 扩展已实现 | 与参考 ReloadERB 流程并不相同 |
| 调试 | 协议与主要接口存在 | DEBUGPRINT 系列、分析模式及部分高级能力缺失 |
| 退出、重启、错误 | 协议化实现 | 参考的系统声音、窗口错误状态和日志 UI 未复刻 |

## 1. 缺失功能

以下项目尚未完整实现；部分曾只被“其他未实现 Host 操作”这一兜底说明笼统覆盖。

### 1.1 已补齐的原编译或执行阻断项

#### PRINTDATA、STRDATA 和数据列表

- 已实现 `PRINTDATA*` 和 `STRDATA` 的惰性随机选择：skip 时不消耗 RAND，未选中的
  DATA 表达式不求值，`DATALIST` 作为一个多行候选，支持带下标选择索引及 K/D/L/W。
- parser、analyzer、compiler 识别全部 K/D/L/W 组合，并将 `ENDLIST` 正确匹配
  `DATALIST`；三类 `TRY*LIST` 的候选执行也已落地。
- 旧文档中的 `TRYLIST/ENDLIST` 是审计笔误；参考语法实际为
  `TRYCALLLIST/TRYJUMPLIST/TRYGOTOLIST`、`FUNC`、`ENDFUNC`。结构解析已修正，候选
  调用使用 VM 原生的惰性逐项解析。
- `real-erb` 中检出约 2,432 次 `PRINTDATA*` 词法使用。

#### 动态调用

本轮已实现 VM 原生的两阶段动态函数解析与调用：目标先解析，成功后才求值实参；
`CALLFORM*`、`JUMPFORM`、`TRYCALLFORM*`、`TRYCCALL*` 和 `TRYCJUMP*` 使用固定于调用帧
generation 的函数表，JUMP 使用帧替换，缺失 try 目标不求值实参。参数默认值已进入
字节码并在 VM 绑定。

`TRYCGOTO*` 使用函数内版本化 label 表，并保留参考实现中“常量缺失”和“运行时动态
缺失”进入 CATCH 路径不同的行为。`CALLEVENT` 在当前 fiber 内按 ONLY/PRI/normal/LATER
顺序运行事件组，保留 SINGLE 返回值规则，并拒绝事件上下文中的递归 CALLEVENT。
`CompatiFuncArgOptional`、`CompatiFuncArgAutoConvert`、`CompatiCallEvent` 已从文本/JSON
配置投影至 HIR 和字节码，VM 对 normal/method/event 种类进行运行时限制。静态
`TRYCALL/TRYJUMP` 的缺失目标也不会求值参数。
`real-erb` 中约有 1,251 个 `CALLFORM`、748 个 `TRYCALLFORM` 和 73 个
`TRYCCALLFORM`。

#### 其他指令

下列参考指令已进入稳定执行 catalog：

- `ASSERT`、`THROW`、`FORCEKANA`
- `CUPCHECK`、`UPCHECK`
- `CUSTOMDRAWLINE`、`DRAWLINEFORM`
- `PRINT_ABL`、`PRINT_TALENT`、`PRINT_MARK`、`PRINT_EXP`
- `PRINT_PALAM`、`PRINT_ITEM`、`PRINT_SHOPITEM`

真实脚本中检出 138 个 `THROW`、20 个 `DRAWLINEFORM` 和 8 个
`CUSTOMDRAWLINE`。

指令形式的 `PRINTCPERLINE` 和 `SAVENOS` 现在通过 VM 验证的 `HostWrite` 写入传入
place；同名函数形式仍返回整数。

### 1.2 已实现的扩展 Host 功能

输入及客户端状态：

- `GETTEXTBOX`、`SETTEXTBOX`、`CLEARTEXTBOX`
- `HOTKEY_STATE`、`HOTKEY_STATE_INIT`
- `MOUSEX`、`MOUSEY`、`MOUSEB`
- `FLOWINPUT`、`FLOWINPUTS`
- `BREAKBUTTON`

文本及样式：

- `BAR`、`BARL`
- `DEBUGPRINT`、`DEBUGPRINTL`、`DEBUGPRINTFORM`、`DEBUGPRINTFORML`、`DEBUGCLEAR`
- `HTML_PRINT_ISLAND`、`HTML_PRINT_ISLAND_CLEAR`、`HTML_TAGSPLIT`
- `SETCOLORBYNAME`、`RESETCOLOR`
- `SETBGCOLORBYNAME`、`RESETBGCOLOR`
- `FONTBOLD`、`FONTITALIC`、`FONTREGULAR`
- `REDRAW`
- `CURRENTALIGN`、`CURRENTREDRAW`
- `GETBGCOLOR`、`GETCOLOR`、`GETDEFBGCOLOR`、`GETDEFCOLOR`
- `GETFOCUSCOLOR`、`GETFONT`、`GETSTYLE`
- `HTML_ESCAPE`、`HTML_TOPLAINTEXT`

运行时元数据：

- `ENUMFUNCBEGINSWITH`、`ENUMFUNCENDSWITH`、`ENUMFUNCWITH`
- `ENUMVARBEGINSWITH`、`ENUMVARENDSWITH`、`ENUMVARWITH`

`GETMEMORYUSAGE` 已被明确记录为有意不支持，不属于遗漏。

本节列出的全部 Host 名称现已接入稳定执行路径。跨客户端边界如下：`HOTKEY.ERB`
的平台按键映射由主展示前端执行，runtime 只持有并投影 `HOTKEY_STATE*` 数组；鼠标
查询通过绑定 presentation revision 的 `pointer_state` 服务返回逻辑/CSS pixel 与
EraBasic button value。`DEBUGPRINT*` 始终进入有界 runtime 缓冲区，仅具有
`ScriptOutput` 权限的调试客户端可读取或订阅。普通 `HTML_PRINT` 与 island 共用
`erabasic-html` 的确定性 tokenizer，不依赖 DOM、WinForms 或 GDI。枚举函数此前已有
实现，本轮审计确认其保留声明顺序和大小写不敏感匹配行为。

### 1.3 系统流程遗漏

本节已完成。runtime 现在跟踪 `EVENTCOMEND` 内显式 WAIT，并在需要时自动创建稳定
任意键等待；失败命令不会错误进入该事件。连续调教实现输出抑制、进度/失败提示、
`STOPCALLTRAIN` 丢弃调用方后执行系统清理、自然结束及 BEGIN 离开时的
`CALLTRAINEND` 顺序。
`DOTRAIN` 同时校验详细系统阶段和 `TRAINNAME` 物理长度。自动存档失败会输出两条
参考消息并等待确认后才进入 `SHOW_SHOP`。

`STOPCALLTRAIN` 有一项经 oracle 确认的有意修正：固定参考实现会先丢弃后续语句并
执行 `CALLTRAINEND`，随后因空返回地址触发 `NullReferenceException`。runtime 保留
前两项脚本可观察副作用，但不会复制客户端后端崩溃；它会恢复 TRAIN 系统状态机并
进入稳定输入等待。该取舍优先保证跨客户端运行和架构纯净。

Protocol 15.0 还实现了协商式 `InputUndo`：runtime 在成功的手动存档/读档后保存传统
存档基线、精确 SFMT 状态和标量输入轨迹，并通过相同输入判定路径进行 Ctrl-Z 回放。
回放期间禁止 VM snapshot；稳定 snapshot 会保存撤销基线和轨迹。
参考实现回放前会经过 `BeginTitle`；当前实现直接恢复 runtime 保留的基线并执行现有
读档 hooks。标题初始化的精确状态与调用顺序依赖 2.3 的完整标题/新游戏流程，因此该
项作为后续依赖保留，不阻塞 1.3 的输入轨迹、RNG 和存档恢复能力。

### 1.4 已补齐的输入验证遗漏

本节已完成。runtime 对 `ONEINPUT/ONEINPUTS/TONEINPUT/TONEINPUTS` 以及按钮输入
变体执行权威规范化：手动 `CommitText` 的非空内容截取第一个 Unicode scalar，整数
输入随后按截取结果解析；空的非计时提交和 timeout/message-skip 路径使用完整默认值，
不会把长默认值截断。`INPUT*` 的省略参数、默认值、mouse 和 canskip 槽位也已按参考
签名统一处理。

前端的 `Activate(token)` 是物理鼠标按钮输入的可移植语义对应物。只有项目配置
`AllowLongInputByMouse`（或参考日文配置项）启用时，`Activate` 才保留多字符按钮值；
否则使用同一单字符规范化。runtime 仍验证 wait、submission token、activation token、
选项成员关系、epoch 和事件顺序。`PrimitiveInput` 继续由前端规范化为 EraBasic 字段，
不进入这条文本/按钮路径。

### 1.5 初始化、配置和调试遗漏

本节可移植部分已补齐。`erabasic-config` 保存固定参考实现的完整配置目录、日文/英文/
`ConfigCode` 别名、类型、默认值、颜色及列表解析；default、emuera、setting、fixed 和
debug 的优先级不再依赖前端提交顺序。runtime 只把可移植配置用于游戏语义，窗口位置、
绘图后端等客户端配置仅保留脚本查询值，不把它们冒充设备事实。

`GETCONFIG/GETCONFIGS` 现在按目录和值类型工作，包含真实脚本所需的
`GETCONFIGS("描画インターフェース") == "TEXTRENDERER"`，类型不匹配仍按参考行为
fault。Replace.csv 的值由 CSV 加载结果同步进入同一查询视图，不能从 emuera.config
越权覆盖。

Protocol 16.0 增加以下能力：

- one-shot 项目分析使用 analyzer analysis mode，支持选择 ERB、debug mode 和不可达
  函数检查；只返回结构化诊断，不编译字节码、不创建 VM、也不替换活动项目。
- key macro 由 runtime 持有 10×12 槽位，读取日文/英文 legacy `macro.txt`，输出规范化
  UTF-8 内容并通过前端 Storage contract 持久化。键盘 `CommitText` 统一支持嵌套重复、
  `\\n`、`\\r`、`\\e` 和跨 wait 顺序消费；活动展开禁止 VM snapshot。
- CLR 插件不加载；新增声明式、版本化的可移植 Host extension ABI。扩展参与 analyzer
  和 compiler，调用通过 `ServiceKind::Extension`，runtime 验证返回类型和按参数序号的
  可变写回后原子提交。`CALLSHARP` 继续是稳定的有意不支持项。

`HOTKEY_STATE/HOTKEY_STATE_INIT` 与 `DEBUGPRINT*`、`DEBUGCLEAR` 早已可执行；旧版
1.5 将其列为缺失属于状态文档遗漏，现已更正。物理 `HOTKEY.ERB` 键位映射继续归前端，
runtime 只持有并投影 `HOTKEY_STATE`。物理 ButtonWrap、文本历史、剪贴板和 Rikai
仍依赖后续客户端/展示能力，不属于本节 runtime 配置查询的阻塞项。

## 2. 与参考实现行为不同的功能

### 2.1 因最高设计准则产生的有意差异

- 全部源码和文本使用 UTF-8，不支持 GBK/Shift-JIS 文件编码。
- 文件 I/O 由前端完成，runtime 只处理提交内容和 I/O 结果。
- PrimitiveInput 由前端整理为 EraBasic 结果字段。
- ONEINPUT 的“鼠标长输入”以语义化 `Activate(token)` 表示，不依赖前端报告物理设备；
  `CommitText` 始终走文本单字符规则。参考实现按 WinForms 的 `changedByMouse` 区分。
- ONEINPUT 的单字符单位是 Unicode scalar；固定参考实现按 UTF-16 code unit 截断，
  因而非 BMP 字符在参考实现中可能形成孤立 surrogate。本项目选择有效 UTF-8 文本。
- runtime 保存规范化逻辑展示模型，不追求 Windows GDI 像素一致。
- 图片解码由前端完成；runtime 使用固有尺寸和像素查询服务。
- 物理 GDI/CBG 对象 API 和 `GETMEMORYUSAGE` 明确不支持。显式查询实际投影结果的
  命令不属于对象 API；目标设计是由权威前端返回真实观测值。
- `GGETTEXTSIZE/GGETCOLOR` 以协商的实际前端能力为准，不因 legacy
  `TextDrawingMode=WINAPI` 字符串拒绝一个可执行的 typed service。
- `GGETCOLOR` 对负 X、负 Y 和上界执行对称验证并返回 `-1`；不复刻固定参考实现负 Y
  分支因重复检查 X 而可能进入底层异常的缺陷。
- CLR 插件和部分动态元数据查询明确不支持。
- Unicode 大小写转换取代进程文化相关的 .NET casing。
- 整数格式只支持已验证的格式子集。
- XPath 是确定性子集；DataTable ID 使用确定性递增值。
- 存储采用前端事务、revision、atomic replace；删除菜单是 Rust 扩展。
- 候选存档失败回滚展示和效果，而参考实现可能已经泄漏输出。
- VM snapshot、热代际切换是 Rust 扩展。
- `STOPCALLTRAIN` 保留参考实现的调用方丢弃和 `CALLTRAINEND` 副作用，但修复其随后
  的空返回地址异常，并恢复 TRAIN 系统状态机。
- Ctrl-Z 使用 runtime 保留的精确传统存档字节，不会因前端槽位随后被覆盖或删除而
  改变；SFMT 可精确恢复，因此 `UseNewRandom=true` 也不禁用撤销。成功热替换会明确
  清除撤销基线。前端只发送语义化 `InputUndoRequest`，不提交平台 Ctrl-Z 键事件。

相关稳定差异记录于 `docs/runtime-operation-contracts.md`。

### 2.2 更新后设计原则与当前实现的矛盾

本节记录更新后原则发现的矛盾及当前 Protocol 20.0 的处理结果：

1. **已解决：权威 snapshot 的伪物理文本布局。** Protocol 14.0 删除 `RunLayout` 和
   `DisplayLine::layout_width_millipixels`；文本、按钮、shape 仅保存语义结构。
2. **已解决：字体度量能力。** `font_metrics` 仅在前端协商 `gget_text_size` typed
   operation 时选中；请求和响应绑定 presentation、environment 与 projection-space
   revision。`GGETTEXTSIZE` 原子返回宽度并写入高度，缺少能力时显式 unsupported。
3. **已解决：`CLIENTWIDTH/CLIENTHEIGHT` 权威来源。** 主展示前端以
   `ProjectionObservation` 提交 revision-bound client size；runtime 验证后供查询使用。
4. **已解决：物理历史和 HTML 查询拆分。** `GETDISPLAYLINE`、
   `HTML_GETPRINTEDSTR`、`HTML_STRINGLEN`、`HTML_SUBSTRING`、`HTML_STRINGLINES`
   使用逐命令 revision-bound 权威前端服务；`HTML_SUBSTRING` 的返回值和 `RESULTS`
   写回原子提交。`HTML_POPPRINTINGSTR` 由 runtime 确定性序列化并消费 pending semantic
   runs，不把打印缓冲所有权交给前端。
5. **已解决：`GETLINESTR` 固定 75 列近似。** 该函数使用经 revision-bound
   `ProjectionObservation` 验证的主前端逻辑列数；`DRAWLINE` 继续使用 `Separator`。
6. **已解决：source image 与 raster 边界。** `SPRITEGETCOLOR` 保持内容寻址的源图片
   事实服务；`GGETTEXTSIZE` 使用 FontMetrics service，`GGETCOLOR` 使用绑定 canvas
   replay revision 的 Canvas service。Runtime 在发出 canvas 查询前完成对称 X/Y 越界
   验证；负 Y 返回 `-1`，不复刻参考实现重复检查 X 导致的异常缺陷。
7. **单客户端范围内已解决：投影因果。** 一个 session 只有一个握手绑定的权威前端，
   以 envelope session/epoch/sequence 和各类 revision 校验；当前不支持多客户端或
   authority transfer，因此不加入无实际消费者的租约状态机。
8. **已解决：可移植性数据流诊断。** analyzer catalog 是 callable portability 的
   统一来源；直接观测发 source-located Notice，传播到控制流、持久变量、随机种子、
   动态调用或存档 sink 时发 Warning。字节码仍持久化 operation portability contract。
9. **已解决：坐标空间。** Protocol 19.0 区分 Era logical milliunits、字体相对长度、
   canvas texel 和设备无关 projection units；`ProjectionObservation` 提供有理数 affine
   transform。字体和物理布局结果只存在于外部观测，不进入规范化 snapshot。

Protocol 19.0 不向下兼容 Protocol 18.0；前端必须同步更新 Schema、坐标投影与 service
capabilities。仓库不包含具体 GUI/TUI，因此“已解决”指 runtime 端契约、验证、VM 提交
和缺能力行为完整，不能据此声称任一外部前端已经实现对应 renderer operation。

### 2.3 已解决或已裁决的行为差异

1. **已解决：新游戏初始化顺序。** title VM 只建立标题期变量默认值；先执行
   `SYSTEM_TITLE`，内建菜单选择 0 后才原子执行 ResetData、加入 CSV 角色 0 和
   `DEFAULTCHARA`，再调用 `EVENTFIRST`。自定义标题可观察到 `CHARANUM=0`。
2. **已解决：内建标题内容。** Runtime 输出分隔线、居中的标题/版本/作者/年份/详情，
   再输出 `[0]/[1]`。这些仍是规范化语义行，不引入参考 WinForms 像素布局。
3. **已解决：BEGIN 清理。** BEGIN/FORCE_BEGIN 在指令执行时 ResetStyle，切换系统时
   清除 `skipPrint`；`message_skip` 继续按其独立的 SKIPLOG/FORCEWAIT 语义管理。
4. **已解决：EVENTSHOP 内 BEGIN。** 与参考调用栈一致，BEGIN 先返回当前 EraBasic
   frame；EVENTSHOP 期间不立即切换，待当前系统根结束后再消费延迟 BEGIN。
5. **已解决：SHOP 自动存档条件。** 仅从 Normal 进入 SHOP 且配置启用时执行；延迟
   BEGIN 会跨越 EVENTSHOP/自动存档边界后再切换。
6. **已解决：标题、存档和读档菜单输入。** wait 接受数字 `CommitText` 和 token；
   slot 号可直接跨页，`100` 返回，load 的 `99` 指向自动存档。Token 只是跨前端鉴权，
   不替代参考可观察的数值选择。
7. **已解决：普通 PRINT 自动按钮。** 完整逻辑行提交时按参考分组算法识别 `[数字]`，
   支持十进制、十六进制、二进制和指数形式，并分配 opaque token；`PRINTPLAIN*`
   保持不可选择。
8. **已解决到可移植语义边界：HTML。** Protocol 19.0 使用固定方言的结构化
   `HtmlDocument`，解析样式、段落、按钮、图片、shape、div、换行等节点，按钮由
   runtime 绑定 token；`HTML_PRINT` 第二参数可追加当前 buffer，island 独立保存。
   字体测量、div 实际布局和 raster 仍属于第 3 节的前端投影责任。
9. **已解决：图片和形状参数。** `PRINT_IMG` 保留普通/hover/mask 资源及宽、高、Y；
   `PRINT_RECT` 和 `PRINT_SPACE` 使用带单位的 `PresentationLength` 保存 MixedNum，
   不再把 space 输出为数字文本。
12. **热替换流程不同。** Rust 使用原子 project delta 和多代 VM；参考 `ReloadERB`
    会保存当前系统状态、重新加载脚本、显示重新加载信息并等待按键。Rust 方案更
    适合当前架构，但不能称为参考行为复刻。
13. **有意修正：`GSETCOLOR` 负 Y。** 参考实现重复检查 X，负 Y 可能落入 GDI 异常；
    Rust 与 `GGETCOLOR` 一样对称检查 X/Y，越界稳定返回 `0`，不把客户端缺陷变成脚本
    可依赖的平台行为。
14. **有意确定化：资源重名。** 参考并行加载可能让同名 sprite 的胜者受调度影响；
    Rust 按规范化 manifest 路径及行号处理，第一个有效定义胜出并产生稳定 warning。
15. **有意无操作：bitmap cache。** `BITMAP_CACHE_ENABLE` 返回参考值 `0`，但不改变
    runtime 语义状态；是否缓存物理行由前端自行决定。
16. **有意收紧：canvas 文件命名空间。** 参考实现允许 `GCREATEFROMFILE` 受当前进程
    目录及绝对路径影响，并由 runtime 自行读写；Rust 将 `relative=0` 固定映射到前端
    提交的 `Resource`，非零映射到 `Data`，`GLOAD/GSAVE` 固定使用 `Save/imgNNNN.png`。
    绝对路径和 `..` 被拒绝，所有字节仍经版本化 Storage/Image/Canvas 消息交换。
17. **有意固定：canvas 字体意图。** 新画布从项目配置取得有效前景、背景和字体；
    `GSETFONT` 的成功不取决于握手客户端的本地字体列表。前端可以选择视觉 fallback，
    但 `GGETFONT` 和 replay 继续返回脚本指定的字体名，避免更换客户端改变游戏状态。

## 3. 参考实现中与客户端实际渲染显示有关的功能

| 功能组 | 参考实现行为或依赖 | Rust 当前状态 |
| --- | --- | --- |
| 文本测量与折行 | `System.Drawing.Graphics`、WinForms `TextRenderer`、实际 Font；决定物理行、宽度和折行 | 普通输出保存意图；`MaxLog`、`ButtonWrap`、`CompatiLinefeedAs1739` 作为投影策略发布；显式查询使用 revision-bound service |
| 左、中、右对齐 | 计算所有 button/run 实际像素宽度，再相对 `DrawableWidth` 平移 | 保存 `LineAlignment` 意图，由前端投影 |
| 字体及样式 | Font family、size、bold、italic、underline、strikeout、前景和背景色 | 规范化状态、reset、query 和快捷样式命令已实现；实际字体由前端选择 |
| 按钮 | `[数字]` 自动识别、generation、hit testing、hover/focus、tooltip、鼠标点击 | 自动、显式与 HTML 按钮均携带 token、generation、enabled；runtime 验证 token，hit testing、hover/focus 由前端投影 |
| 文本框 | 直接操作 WinForms TextBox | Runtime 保存文本及 Era 逻辑坐标；`MOVETEXTBOX/RESUMETEXTBOX` 由前端变换/裁切，成功输入后 runtime 权威复位 |
| 鼠标和窗口 | MainPicBox 坐标、client width/height、鼠标悬停按钮、焦点、hotkey | primitive input 被有意规范化；client size 和 pointer 查询来自 revision-bound 前端观测 |
| HTML | `<font>`、`b/i/u/s`、`p`、`nobr`、`button`、`nonbutton`、`clearbutton`、`img`、`shape`、`div`、对齐和 tooltip | Runtime 验证固定方言，保存 typed MixedNum、box model、颜色、布局和交互；前端只排版/绘制 |
| HTML island | 独立 overlay/island 图层 | 保存结构化 island 列表并支持清除；实际 overlay 由前端投影 |
| 静态图片 | 图片文件加载、裁切 sprite、前景/背景/mask、缩放和位置 | 保存资源 ID、hover/mask 和 MixedNum 尺寸；解码与绘制由前端完成 |
| 背景 | 有深度、透明度的背景 sprite 烘焙 | 保存允许重复、稳定按 depth 降序的列表；删除首个精确匹配，透明度为精确有理数 |
| 动画 sprite | 帧、持续时间、位置、重绘计时器 | replay graph 已实现；timer 的禁用、10ms 下限及 `Int16.MaxValue` 上限兼容参考实现 |
| G canvas | Bitmap 创建/加载/保存、DrawImage、mask、旋转、线、文本、填充、颜色矩阵、像素读写 | portable replay 已覆盖；文件经 Resource/Data/Save 受控命名空间及 typed image/canvas service；具体 rasterizer 不在仓库内 |
| GDI 对象 | Brush、Pen、Font、DashStyle 查询和修改 | 保存由项目配置初始化的 portable Brush/Pen/Font/Dash 状态和命令，不创建或暴露 GDI handle；字体名作为脚本意图固定，前端可视觉替代但不能改变返回值 |
| CBG | 背景 bitmap、按钮 sprite map、范围移除和合成 | 明确不支持 |
| Bitmap cache | 每行是否缓存为 bitmap | `BITMAP_CACHE_ENABLE` 为兼容 no-op 并返回 0；缓存只影响客户端性能，是有意差异 |
| Tooltip | WinForms ToolTip 测量、绘制、字体、颜色、延时、图片 | 配置已规范化；`TOOLTIP_FORMAT` 同时保留 raw 值、完整已知 `TextFormatFlags` 列表及 unknown bits |
| Rikaichan | 鼠标位置、词典、TextRenderer popup | 有意不进入 runtime；可由只读前端插件基于投影文本实现 |
| 日志和回滚显示 | 物理显示行历史、滚动、删除、临时行、最大日志 | Snapshot 携带有序语义 journal；物理查询与 `OUTPUTLOG` 绑定 revision，由权威前端投影 |

权威边界是：runtime 保存完整语义和布局意图，前端负责字体测量与 raster。普通输出
不会把前端投影反写进游戏状态；只有脚本显式调用投影查询命令时，前端观测值才通过
有序服务响应进入 VM，runtime 负责校验和后续状态转移。这些查询服务已在 runtime
端实现版本、revision、类型和范围校验；仓库不包含具体应用前端，因此仍不能声称已有
renderer 实现了对应观测能力。

投影依赖命令本身不必立即弃用。用于字体 fallback、响应式排版等展示适配时可以合理
存在，但必须报告结果随前端、字体、DPI、viewport 或 renderer 变化。若结果影响剧情、
角色属性、持久变量、随机性、动态调用或存档内容，应给出更强的
`portability.frontend_projection_affects_gameplay` 类诊断，说明该用法仅为兼容保留且
命令可能在提供可移植替代后弃用。正式弃用必须先有替代方案、真实脚本审计和明确的
语言/协议版本迁移。

对 `reference/real-erb` 的只读词法检查在典型投影查询中只发现 4 处 `CHKFONT`；它们
均用于 Saitamaar 字体和字符画的展示 fallback，没有发现以这些查询决定持久游戏内容
的证据。该结果只说明当前语料风险较低，不是完整跨函数数据流证明。

## 4. 非平凡文本输出功能

| 功能 | 参考语义 | 当前问题 |
| --- | --- | --- |
| `ALIGNMENT RIGHT/CENTER` | 按实际物理宽度右对齐或居中 | 只保存意图；这是允许的跨平台投影 |
| `PRINTC` | Shift-JIS 字节宽度补齐，并用 GDI 测量修正；右对齐 | 只对 4 个精确命令生成 ColumnCell |
| `PRINTLC` | 左对齐并补齐到列宽 | 同上 |
| `PRINTBUTTONC/LC` | 带值按钮和列布局 | 使用带 token 的 ColumnCell；物理补齐和测量由前端投影 |
| 表格式布局 | `PrintCPerLine` 自动换行；TRAIN、SHOP、PALAM 等内建输出依赖它 | TRAIN 当前输出普通 button，不是 ColumnCell；连续调教也错误输出 |
| `PRINTSINGLE*` | 立即 flush 为单独物理行 | Rust 不提交行，因为名字不以 L/W 结尾 |
| `PRINTN` | 保持 lineEnd=false，同时进入输入等待 | Rust 既不等待，也没有正确的物理行拼接 |
| `PRINT*W` | 输出、换行并等待 | 基本存在 |
| K 后缀 | 按 FORCEKANA 状态进行平假名、片假名、全半角转换 | 使用内置日语 LCID 0x0411 兼容表，不依赖平台 locale |
| D 后缀 | 临时忽略 SETCOLOR，使用默认或用户颜色 | 使用规范化默认前景色，不改变其余样式 |
| L/W 后缀 | 控制换行和等待 | 只按名称末尾粗略处理 |
| 嵌入 `\n` | 递归切成多个显示行 | Rust 将换行保留在同一个 Text run |
| `PRINTPLAIN*` | 不把 `[数字]` 转换成按钮 | 普通 PRINT 自动识别，PLAIN 保持不可选择 |
| `PRINTDATA*` | 随机数据列表、多行输出、选择索引、K/D/L/W | 已实现，包括带下标选择目标 |
| `STRDATA` | 随机选择并拼接字符串数据块 | 已实现，包括带下标目标 |
| `BAR/BARL/BARSTR` | 按当前值、最大值、长度和配置字符生成进度条 | 三者均已进入稳定执行路径；BARL 提交逻辑行 |
| `DRAWLINE` | 按可绘宽度重复 pattern | Rust 使用确定性逻辑分隔线 |
| `GETLINESTR` | 按实际 console 可绘宽度返回重复 pattern 字符串 | 使用前端最近提交的 `ProjectionObservation.line_columns`，不是 WinForms/GDI 像素宽度 |
| `CUSTOMDRAWLINE/DRAWLINEFORM` | 自定义 pattern 的分隔线 | 输出规范化 Separator，不复刻 GDI 像素重复 |
| `PRINT_RECT/SPACE` | px/% 混合尺寸形状 | 保存 font-relative/logical MixedNum 语义，由前端布局 |
| HTML div | 可形成带宽度、对齐、嵌套内容的表格式布局 | 保存结构化 div 与属性；实际布局由前端完成 |
| 临时行/REUSELASTLINE | 替换最近临时行、保留 button generation | 只实现逻辑行层面的近似 |
| 空行 | 强制空行时插入空格，确保形成显示行 | Rust 可形成空 runs，历史行为不同 |

参考 PRINT 分派位于
`reference/emuera.em/Emuera/Runtime/Script/Statements/Instraction.Child.cs`，自动按钮识别
位于 `reference/emuera.em/Emuera/UI/Game/PrintStringBuffer.cs`，Rust 的统一打印分支
位于 `crates/era-runtime/src/session/host_dispatch.rs` 与 `crates/era-runtime/src/presentation.rs`。

## 状态维护规则

- 实现功能时同步更新本清单、能力协商和相应协议文档。
- 可执行行为必须用最小 Rust 测试覆盖；兼容性声明还需同输入 reference 差分证据。
- reference CLI 尚未覆盖完整系统流程、规范化展示、存储事务和客户端渲染，因此
  oracle smoke test 本身不能证明这些功能一致。
- 扫描 `reference/real-erb` 得到的命中数只用于风险排序，不代表运行时调用次数。
