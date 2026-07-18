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
- 调教、`EVENTCOMEND`、SHOP 自动存档存在系统流程差异。
- 很多已进入 Host catalog 的命令最终落入通用 `UnsupportedRuntimeFeature`。
- 前端实际渲染观测尚无 typed service，且现有规范化展示类型仍携带声称由 runtime
  产生的物理布局字段，与更新后的 runtime/前端边界矛盾。

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
| 标题画面 | 部分完成 | 新游戏重置时机、标题内容、输入方式不同 |
| 新游戏初始化、EVENTFIRST | 部分完成 | `SYSTEM_TITLE` 观察到的初始状态不同 |
| TRAIN/连续调教 | 主流程完成 | 物理列布局仍由后续展示兼容任务处理；`STOPCALLTRAIN` 的参考崩溃被有意修正 |
| AFTERTRAIN/ABLUP/TURNEND | 主流程存在 | BEGIN 时的样式、SKIPDISP 仍有差异 |
| SHOP | 部分完成 | 自动存档条件、EVENTSHOP 中 BEGIN 仍有差异；失败确认等待已实现 |
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

- runtime 只解析部分具有运行语义的 Emuera 配置。
- `GETCONFIG/GETCONFIGS` 只暴露一个小型白名单，其他合法参考配置会直接 fault。
- 真实脚本使用的 `GETCONFIGS("描画インターフェース")` 当前不支持。
- 未实现 key macro、Hotkey 文件状态、分析模式启动流程和参考插件系统。
- `CALLSHARP` 是已明确记录的有意不支持项。
- 调试协议已实现主要变量、栈、断点、单步和安全控制台能力，但 EraBasic 的
  `DEBUGPRINT*` 和 `DEBUGCLEAR` 本身仍不可执行。

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

以下内容是当前代码和 protocol 15.0 的现状，不是未来目标设计。它们不能继续被笼统
描述为“规范化展示的有意差异”：

1. **已解决：权威 snapshot 的伪物理文本布局。** Protocol 14.0 删除 `RunLayout` 和
   `DisplayLine::layout_width_millipixels`；文本、按钮、shape 仅保存语义结构。
2. **字体度量能力存在于 Schema，但不可执行。** `ClientCapabilities::font_metrics`
   和 `ServiceKind::FontMetrics` 已定义，runtime 协商时却固定选择 `false`，协议也没有
   为具体测量操作定义 revision-bound typed payload。类型存在不能视为能力已实现。
3. **已解决：`CLIENTWIDTH/CLIENTHEIGHT` 权威来源。** 主展示前端以
   `ProjectionObservation` 提交 revision-bound client size；runtime 验证后供查询使用。
4. **物理历史和 HTML 投影查询仍直接 fault。** `GETDISPLAYLINE`、
   `HTML_GETPRINTEDSTR`、`HTML_POPPRINTINGSTR`、`HTML_STRINGLEN`、
   `HTML_SUBSTRING`、`HTML_STRINGLINES` 当前返回
   `UnsupportedRuntimeFeature`。这曾被记录为稳定有意差异；更新后的原则要求逐命令
   判断：可在规范化 markup/逻辑状态上定义的由 runtime 实现，确实观察物理投影的
   通过权威前端服务返回真实值。该拆分尚未完成，因此当前状态是缺失/待设计，而不是
   最终的稳定不支持。
5. **已解决：`GETLINESTR` 固定 75 列近似。** 该函数使用经 revision-bound
   `ProjectionObservation` 验证的主前端逻辑列数；`DRAWLINE` 继续使用 `Separator`。
6. **canvas/text raster 查询没有统一边界。** `SPRITEGETCOLOR` 查询内容寻址的源图片
   像素，当前 frontend image service 方向合理；而 `GGETTEXTSIZE`、`GGETCOLOR` 等
   依赖实际字体或 raster 算法的操作仍缺少权威前端观测服务，不能由 runtime 自行
   近似，也不能与源图片事实混为一类。
7. **部分解决：投影因果字段。** `ProjectionObservation` 与 `pointer_state` 均绑定
   presentation revision，环境观测使用单调 environment revision。当前单 session
   动态库只有一个主前端；未来 C/S 多客户端仍需在连接层增加明确角色租约。
8. **部分解决：可移植性元数据。** 字节码 operation contract 现持久化 `Portable`、
   `FrontendObservation`、`PlatformIntent`、`ExtensionDefined`，compiler 对直接使用
   发出 source-located Notice。跨表达式追踪到控制流、持久变量、随机种子、动态调用或
   存档等 taint sink 的 Warning 仍待 analyzer 数据流诊断基础设施完成。
9. **坐标单位语义仍含混。** 图片、shape 和 canvas 字段使用 `millipixels` 命名，容易
   被理解为设备像素。脚本提供的坐标可以作为 runtime 持有的逻辑坐标保留，但协议
   必须明确其坐标空间及前端缩放规则；由字体测量产生的设备布局不能混入其中。

这些矛盾的推荐解决顺序是：先建立操作分类和诊断，再定义 revision-bound 前端观测
服务，随后迁移查询命令，最后从下一版协议的规范化 snapshot 中移除 realized-layout
字段。当前开发阶段公共协议不要求向下兼容，但变更时仍需同步升级协议版本、C Schema、
文档和测试。

### 2.3 尚未解决的行为差异

1. **新游戏初始化顺序不同。** 参考实现先调用 `SYSTEM_TITLE`，用户选择新游戏后才
   `ResetData` 并添加 CSV 角色 0 和默认角色。Rust 在调用 `SYSTEM_TITLE` 前已经
   `ResetNewGame`。因此自定义标题函数看到的角色和变量状态不同，其修改还可能在
   选择新游戏后被保留。
2. **内建标题画面不同。** 参考实现输出分隔线、居中标题、版本、作者、年份、详情
   以及 `[0]/[1]` 数字菜单；Rust 只输出两个语义按钮。
3. **BEGIN 状态清理不同。** 参考实现会清除 `skipPrint`，标题还会 `ResetStyle`；
   Rust 只清除 `message_skip`。`SKIPDISP` 和字体、颜色、对齐状态可能跨系统流程
   泄漏。
4. **EVENTSHOP 内 BEGIN 不同。** 参考实现丢弃从 `EVENTSHOP` 发起的 BEGIN；Rust
   会立即取消当前 fiber 并切换流程。
5. **SHOP 自动存档条件不同。** 参考实现只在 SHOP 从 Normal 进入时自动存档；Rust
   只要启用 autosave 就执行。
6. **标题、存档和读档菜单输入不同。** 参考实现接受数字键盘输入，可直接输入其他
   页的 slot 号跳页，`100` 返回、`99` 是自动存档。Rust 使用 opaque interaction
   token 和前后页按钮，不接受等价的 `CommitText("42")`。
7. **普通 PRINT 自动按钮识别不同。** 参考实现把普通文本中的 `[数字]` 转换成可
   点击按钮，`PRINTPLAIN` 才禁止这一行为。Rust 普通 PRINT 永远不会生成按钮，只
   支持显式 `PRINTBUTTON*`。
8. **HTML 行为不同。** Rust 把 `HTML_PRINT` 保存为一个 opaque HTML run 并立即
    提交一条逻辑行；参考实现解析 HTML 的按钮、图片、形状、样式和 div，并可根据
    第二参数追加到当前 print buffer。
9. **图片和形状参数不同。** Rust `PRINT_IMG` 只保留首个资源 ID，忽略背景图、
    mask、宽高和 y 坐标；`PRINT_RECT` 丢失 px/% 的 MixedNum 语义；`PRINT_SPACE`
    被普通 PRINT 分支错误地输出成数字文本。
12. **热替换流程不同。** Rust 使用原子 project delta 和多代 VM；参考 `ReloadERB`
    会保存当前系统状态、重新加载脚本、显示重新加载信息并等待按键。Rust 方案更
    适合当前架构，但不能称为参考行为复刻。

## 3. 参考实现中与客户端实际渲染显示有关的功能

| 功能组 | 参考实现行为或依赖 | Rust 当前状态 |
| --- | --- | --- |
| 文本测量与折行 | `System.Drawing.Graphics`、WinForms `TextRenderer`、实际 Font；决定物理行、宽度和折行 | 普通输出只保存意图；显式测量查询目标为前端观测服务，当前未实现 |
| 左、中、右对齐 | 计算所有 button/run 实际像素宽度，再相对 `DrawableWidth` 平移 | 保存 `LineAlignment` 意图，由前端投影 |
| 字体及样式 | Font family、size、bold、italic、underline、strikeout、前景和背景色 | 部分状态可保存；reset/query/快捷字体命令缺失 |
| 按钮 | `[数字]` 自动识别、generation、hit testing、hover/focus、tooltip、鼠标点击 | 只有显式 token button；自动按钮和 BREAKBUTTON 缺失 |
| 文本框 | 直接操作 WinForms TextBox | `GET/SET/CLEARTEXTBOX` 缺失 |
| 鼠标和窗口 | MainPicBox 坐标、client width/height、鼠标悬停按钮、焦点、hotkey | primitive input 被有意规范化；`CLIENTWIDTH/HEIGHT` 当前错误地使用项目配置，其他查询多缺失 |
| HTML | `<font>`、`b/i/u/s`、`p`、`nobr`、`button`、`img`、`shape`、`div`、对齐和 tooltip | opaque HTML 投影；不解析交互与布局 |
| HTML island | 独立 overlay/island 图层 | 缺失 |
| 静态图片 | 图片文件加载、裁切 sprite、前景/背景/mask、缩放和位置 | 元数据及资源 ID 可用；PRINT_IMG 参数不完整 |
| 背景 | 有深度、透明度的背景 sprite 烘焙 | 保存语义背景列表 |
| 动画 sprite | 帧、持续时间、位置、重绘计时器 | 语义 replay graph 已实现 |
| G canvas | Bitmap 创建/加载/保存、DrawImage、mask、旋转、线、文本、填充、颜色矩阵、像素读写 | 仅部分语义 canvas replay；显式 raster 结果查询目标为前端观测，当前未实现 |
| GDI 对象 | Brush、Pen、Font、DashStyle 查询和修改 | 缺失或有意不支持 |
| CBG | 背景 bitmap、按钮 sprite map、范围移除和合成 | 明确不支持 |
| Bitmap cache | 每行是否缓存为 bitmap | 缺失 |
| Tooltip | WinForms ToolTip 测量、绘制、字体、颜色、延时、图片 | 规范化 tooltip 配置已实现 |
| Rikaichan | 鼠标位置、词典、TextRenderer popup | 未实现，属于具体客户端能力 |
| 日志和回滚显示 | 物理显示行历史、滚动、删除、临时行、最大日志 | Rust 只维护规范化逻辑行历史；真实物理历史查询服务未实现 |

权威边界是：runtime 保存完整语义和布局意图，前端负责字体测量与 raster。普通输出
不会把前端投影反写进游戏状态；只有脚本显式调用投影查询命令时，前端观测值才通过
有序服务响应进入 VM，runtime 负责校验和后续状态转移。当前这些服务尚未实现，而且
不少语义参数本身也尚未保存，不能都归结为“前端差异”。

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
| `PRINTBUTTONC/LC` | 带值按钮和列布局 | 有语义按钮，但无参考补齐和测量 |
| 表格式布局 | `PrintCPerLine` 自动换行；TRAIN、SHOP、PALAM 等内建输出依赖它 | TRAIN 当前输出普通 button，不是 ColumnCell；连续调教也错误输出 |
| `PRINTSINGLE*` | 立即 flush 为单独物理行 | Rust 不提交行，因为名字不以 L/W 结尾 |
| `PRINTN` | 保持 lineEnd=false，同时进入输入等待 | Rust 既不等待，也没有正确的物理行拼接 |
| `PRINT*W` | 输出、换行并等待 | 基本存在 |
| K 后缀 | 按 FORCEKANA 状态进行平假名、片假名、全半角转换 | 使用内置日语 LCID 0x0411 兼容表，不依赖平台 locale |
| D 后缀 | 临时忽略 SETCOLOR，使用默认或用户颜色 | 使用规范化默认前景色，不改变其余样式 |
| L/W 后缀 | 控制换行和等待 | 只按名称末尾粗略处理 |
| 嵌入 `\n` | 递归切成多个显示行 | Rust 将换行保留在同一个 Text run |
| `PRINTPLAIN*` | 不把 `[数字]` 转换成按钮 | Rust 普通 PRINT 本身也不生成按钮 |
| `PRINTDATA*` | 随机数据列表、多行输出、选择索引、K/D/L/W | 已实现，包括带下标选择目标 |
| `STRDATA` | 随机选择并拼接字符串数据块 | 已实现，包括带下标目标 |
| `BAR/BARL/BARSTR` | 按当前值、最大值、长度和配置字符生成进度条 | 仅 `BARSTR` 可用 |
| `DRAWLINE` | 按可绘宽度重复 pattern | Rust 使用确定性逻辑分隔线 |
| `GETLINESTR` | 按实际 console 可绘宽度返回重复 pattern 字符串 | Rust 固定按 75 逻辑列近似；与新前端观测原则冲突 |
| `CUSTOMDRAWLINE/DRAWLINEFORM` | 自定义 pattern 的分隔线 | 输出规范化 Separator，不复刻 GDI 像素重复 |
| `PRINT_RECT/SPACE` | px/% 混合尺寸形状 | RECT 部分实现，SPACE 错误 |
| HTML div | 可形成带宽度、对齐、嵌套内容的表格式布局 | Rust opaque HTML，不生成结构化布局 |
| 临时行/REUSELASTLINE | 替换最近临时行、保留 button generation | 只实现逻辑行层面的近似 |
| 空行 | 强制空行时插入空格，确保形成显示行 | Rust 可形成空 runs，历史行为不同 |

参考 PRINT 分派位于
`reference/emuera.em/Emuera/Runtime/Script/Statements/Instraction.Child.cs`，自动按钮识别
位于 `reference/emuera.em/Emuera/UI/Game/PrintStringBuffer.cs`，Rust 的统一打印分支
位于 `crates/era-runtime/src/session.rs`。

## 状态维护规则

- 实现功能时同步更新本清单、能力协商和相应协议文档。
- 可执行行为必须用最小 Rust 测试覆盖；兼容性声明还需同输入 reference 差分证据。
- reference CLI 尚未覆盖完整系统流程、规范化展示、存储事务和客户端渲染，因此
  oracle smoke test 本身不能证明这些功能一致。
- 扫描 `reference/real-erb` 得到的命中数只用于风险排序，不代表运行时调用次数。
