# `reraconfig.toml` 设计

`reraconfig.toml` 是 RustyEra 项目唯一的外部配置文件。它采用 UTF-8 编码，所有字段均可
省略；读取时，缺失字段使用 `era-config` 目录中的统一默认值。客户端必须接受 LF、CRLF
和单独 CR 行尾，写入时使用所在操作系统的原生行尾。

## 格式原则

- 文件使用 TOML，设置按用途放入浅层表中，例如 `text.font_size` 和
  `save.auto_save`。设置名称使用小写、完整且直观的英语单词。
- `[meta]` 只保存格式元数据。`schema_version` 当前为整数 `5`；
  `locked_settings` 是不可由客户端设置面板修改的规范路径数组，用来承接旧
  `_fixed.config` 的语义。
- 版本 1、2、3、4 文件及未写版本号的旧文件会在读取时升级为版本 5。版本 1 升级会移除退役字段；
  `compatibility.drawline_starts_new_line = true` 会转移为仍受支持的
  `compatibility.legacy_nonbutton_wrapping = true`，对应锁定状态也一并转移。
- 版本 2 的 `interface.menu_visible` 布尔项会升级为 `interface.menu_mode`：`true` 转为
  `"auto"`，`false` 转为 `"hide"`，对应锁定路径也一并转移。
- 未知表、未知字段、错误类型、越界整数和无效枚举值均被拒绝，
  防止拼写错误被静默忽略。
- 修改单项设置时保留文件中其他字段、排序、空行和注释。配置文档层不执行文件 I/O，
  由客户端负责读取、原子写入和选择原生行尾。
- 配置目录为每项分配稳定的正整数 ID，并记录旧 Emuera code、适用客户端和热应用策略。
  新项目配置和未来客户端共用同一目录，不允许客户端另设隐藏的项目配置文件。

完整中文注释示例位于
`crates/era-config/schema/reraconfig.example.toml`，机器可读 schema 位于
`crates/era-config/schema/reraconfig.schema.json`。两者由
`cargo run -p era-config --example generate_reraconfig_artifacts` 从代码目录确定性生成。

## 新增的通用设置

### 版本 4 兼容 profile 与版本 5 故障钩子配置

`[compatibility] profile` 缺省为 `"emuera.em"`，也可显式声明
`"emuera.skia.snake"`。未知 profile 或不支持的格式版本会阻止加载，不静默回退。
该字段是项目执行身份，不属于客户端偏好，也不允许通过热重载或设置事务切换；修改后必须完整重开项目。

snake profile 仍为实验状态。当前身份选择逐操作蛇版整数策略、可保存重放的 SFMT19937
和 Unicode 逻辑列宽布局；不保证蛇版 `UseNewRandom` 双状态路径的同 seed 结果，也不表示
按键、像素布局或完整游戏已兼容。
缓存、字节码、snapshot 和 snake 自身存档携带完整身份，不能跨 profile 复用；
原版 profile 保留现有裸 1808 存档读写。三个客户端先调用 core 解析配置，再绑定对应存储，
不各自解释该字段。

```toml
[meta]
schema_version = 5

[compatibility]
profile = "emuera.skia.snake"
```

### 用户函数多余实参诊断

`diagnostics.strict_user_call_arguments`（设置 ID `128`）默认 `false`，仅影响 snake
非 variadic 用户函数：默认告警并且不求值多余实参；设为 `true` 时提升为错误。内置函数
始终按原有 arity 校验，原版 profile 始终拒绝用户函数多余实参。修改此项需要重新加载项目，
会改变编译身份并使不兼容缓存失效。Schema、锁定路径和客户端设置目录同步包含该可选字段。

### 蛇版最终故障钩子

`runtime.disable_before_error_throw`（设置 ID `129`）映射蛇版
`DisableBeforeErrorThrow`，默认 `false`。仅 snake profile 会在最终脚本故障前运行
`BEFORE_ERROR`，显式 `THROW` 则只运行 `BEFORE_THROW`。设置为 `true` 后两个 hook 均禁用；
原版 profile 始终不触发。该设置参与编译身份，修改后需要重新加载项目。

### 展示设置

- `audio.volume`：整数 `0..=100`，默认 `100`，表示游戏主音量百分比；支持热应用。
- `text.replace_full_width_spaces`：布尔值，默认 `false`。启用后仅由前端把游戏输出中的
  一个全角空格显示为两个半角空格，不改变 core 中的原始文本；支持热应用。
- `text.character_width_mode`：字符串枚举，默认 `"automatic"`，可热应用。运行时格式化与
  客户端显示共用同一逻辑列宽。`"ambiguous_narrow"` 和 `"ambiguous_wide"` 分别把 East
  Asian Ambiguous 字符按窄字符和宽字符处理；`"automatic"` 使用兼容 Era 的 CJK/CP932
  规则，并把没有显式文本变体标记的 Unicode 图形符号（例如 `☀`、`❤`）按宽字符处理。
  `STRLENS`、`STRLENSU` 等具有历史编码语义的函数不受此项影响；显式 HTML 内容仍按其
  字体和 CSS 自然布局。

TUI 不暴露音量设置，因为终端音频不由 TUI 控制；浏览器和 Tauri 暴露全部三项。

## 客户端偏好与项目设置

`reraconfig.toml` 仍是项目设置的唯一来源。TUI、浏览器和 Tauri 可以把其中标记为
`QueryOnlyClientPreference` 且适用于本客户端的字段保存为客户端偏好；这些稀疏覆盖只改变
展示、窗口、声音或交互方式，不改变游戏逻辑。生效优先级为项目偏好、明确项目设置、全局
偏好、客户端默认值，项目偏好允许覆盖 `meta.locked_settings`。

全局偏好保存在应用自己的配置目录；源码项目偏好保存在项目内
`.rustyera/preferences-v1.json`。独立项目文件不能写回内部目录：TUI/Tauri 使用规范化项目
路径的 BLAKE3 作为应用数据目录命名空间，浏览器使用项目文件内容的 BLAKE3 在 OPFS 中
隔离。偏好文件 schema 1 的顶层 `profiles` 分别保存 `tui`、`browser`、`tauri` 分区，
客户端写入自己的分区时必须保留其他分区；未知版本或损坏文件按只读处理，不得覆盖。

## 统一默认值

过去各客户端对部分缺失设置使用不同默认值。`reraconfig.toml` 统一采用：历史日志
`1000` 行、每行 `5` 个 PRINTC 项、每项宽度 `24` 列。其他设置使用配置目录列出的默认值。

## 版本 3 菜单显示模式

`interface.menu_mode` 是仅适用于浏览器和 Tauri 的字符串枚举，可热应用。`"show"` 始终
显示菜单，`"auto"` 在页面高度不足时隐藏菜单，`"hide"` 始终隐藏菜单；默认值为
`"auto"`。

这是与 Emuera 参考实现的有意不兼容项。参考实现的 `UseMenu` 是布尔值，真值表示始终显示；
RustyEra 为适应小屏幕和触控客户端，将旧 `YES`、`TRUE`、`1` 以及旧 TOML `true` 迁移为
`"auto"`，将旧假值迁移为 `"hide"`。需要始终显示时必须在新配置或客户端偏好中显式选择
`"show"`。该差异只改变客户端菜单投影，不影响 EraBasic 游戏逻辑或 runtime 状态。

## 旧项目迁移

仅当项目不存在 `reraconfig.toml` 时，客户端才收集旧设置文件并调用迁移器。迁移器按下列
顺序合并内容，后者覆盖前者：

1. `CSV/_default.config` 或 `CSV/default.config`；
2. `emuera.config`；
3. `setting.json`（仅识别并报告其中已经退役的旧设置）；
4. `CSV/_fixed.config` 或 `CSV/fixed.config`，并把这些设置写入
   `meta.locked_settings`；
5. `debug.config`；
6. 启用 `replacement.enabled` 时读取 `_Replace.csv` 中的固定替换设置。

迁移结果只写出不同于 RustyEra 统一默认值或被锁定的字段。旧文件保留在项目中但不再作为
后续配置来源；生成成功后立即按普通 `reraconfig.toml` 路径读取。迁移器对无法解析的旧行
产生诊断，并按来源汇总报告被忽略的退役设置，避免静默丢失信息。
客户端写入迁移或版本升级生成的内容后，会用一次无设置变更的现有配置事务确认新摘要，
使 runtime 与宿主文件采用同一基线；无需重载项目即可继续修改其他设置。

## 版本 2 删除的设置

筛选以“它是否仍属于 Era 项目的可移植行为”为边界，同时保留可能由未来客户端实现、且项目
作者确实可能希望随游戏分发的设置。窗口、图标、性能提示、脚本兼容性和键盘宏等项目因此
仍在目录中；纯粹绑定旧 WinForms 辅助工具、旧配置文件写法或 RustyEra 已固定架构选择的
项目则退役。版本 1 文件会自动清理下列字段，旧配置迁移会以信息诊断报告它们。
这些设置原有的稳定 ID 保留为空洞，后续设置不得复用，以免旧客户端把新语义误认成旧字段。

| 旧 code | 删除理由 |
|---|---|
| `EnglishConfigOutput` | 只决定旧 `CONFIG` 文件用日文键还是英文键；TOML 键已固定为直观英语，RustyEra 也不再回写旧文件。 |
| `TextEditor` | 将项目绑定到本机外部编辑器命令，不是可移植的游戏设置；文本编辑集成应由客户端或开发工具管理。 |
| `EditorType` | 只选择几个旧 Windows 编辑器的命令行约定，不适用于跨平台客户端。 |
| `EditorArgument` | 属于本机编辑器启动参数，随 Era 项目分发既不安全也不可移植。 |
| `RikaiEnabled` | 控制已不属于 RustyEra 游戏运行职责的 Rikaichan 弹窗工具。未来词典能力应作为独立客户端扩展。 |
| `RikaiFilename` | 是上述旧词典工具的本机文件路径，不应成为项目配置。 |
| `RikaiColorBack` | 仅修饰已退役的词典弹窗。 |
| `RikaiColorText` | 仅修饰已退役的词典弹窗。 |
| `RikaiUseSeparateBoxes` | 仅控制已退役词典弹窗的短语高亮方式。 |
| `SkipFrame` | 是旧 WinForms 刷新循环的丢帧旋钮；RustyEra 前端各自调度绘制，跨客户端共享这个数值会产生错误语义。 |
| `CompatiFunctionNoignoreCase` | 与项目级 `script.ignore_case` 重叠且制造函数名特例；未来兼容性应由统一大小写策略描述。 |
| `CompatiDRAWLINE` | 旧实现中与 `CompatiLinefeedAs1739` 表达同一非按钮换行兼容行为；升级时合并到后者，避免两个开关冲突。 |
| `ReduceArgumentOnLoad` | 描述旧加载器的参数预解析优化阶段；RustyEra 编译管线没有可互换的对应模式，将来也不应暴露内部优化策略。 |
| `LastKey` | 是旧更新检查器的内部识别码，不影响游戏语义；更新状态应属于客户端全局服务。 |
| `UseNewRandom` | RustyEra 的随机算法由 runtime 固定以保证一致性和可重放，项目不能选择旧/新实现。 |
| `UseSaveFolder` | 所有 RustyEra 客户端已经用统一保存命名空间隔离存档；保留旧根目录/`sav` 二选一只会形成不可移植路径。 |
| `DisplayReport` | 只控制旧 Emuera 加载报告窗口；现代客户端通过统一诊断与进度界面呈现，不应由项目隐藏诊断。 |
| `EmueraLang` | 是旧 Emuera 自身界面语言。未来 RustyEra 本地化属于客户端/用户全局偏好，不属于 Era 项目。 |
| `TextDrawingMode` | 选择 `GRAPHICS`/`TEXTRENDERER`/`WINAPI` 的旧 WinForms 绘制后端；RustyEra 客户端已有固定渲染架构。脚本查询仍固定返回 `TEXTRENDERER` 以兼容分支判断。 |
| `DebugShowWindow` | 依赖旧独立调试窗口；RustyEra 调试 UI 的开启方式由客户端会话管理。 |
| `DebugWindowTopMost` | 只适用于旧独立调试窗口的桌面层级。 |
| `DebugWindowWidth` | 只保存旧独立调试窗口几何尺寸。 |
| `DebugWindowHeight` | 只保存旧独立调试窗口几何尺寸。 |
| `DebugSetWindowPos` | 只决定是否恢复旧独立调试窗口位置。 |
| `DebugWindowPosX` | 只保存旧独立调试窗口的本机屏幕坐标。 |
| `DebugWindowPosY` | 只保存旧独立调试窗口的本机屏幕坐标。 |
| `CBUseClipboard` | 属于旧版自动复制辅助器；剪贴板访问需要客户端权限和用户级隐私选择，不能由游戏项目启用。 |
| `CBIgnoreTags` | 仅服务上述自动复制辅助器。 |
| `CBReplaceTags` | 仅服务上述自动复制辅助器。 |
| `CBNewLinesOnly` | 仅服务上述自动复制辅助器。 |
| `CBClearBuffer` | 仅服务上述自动复制辅助器。 |
| `CBTriggerLeftClick` | 把游戏输入事件绑定到自动复制，既是旧 UI 细节也会跨客户端失真。 |
| `CBTriggerMiddleClick` | 同上，仅针对旧鼠标中键事件。 |
| `CBTriggerDoubleLeftClick` | 同上，仅针对旧双击事件。 |
| `CBTriggerAnyKeyWait` | 同上，仅针对旧 `WAIT` 触发模型。 |
| `CBTriggerInputWait` | 同上，仅针对旧输入等待触发模型。 |
| `CBMaxCB` | 只控制上述辅助器每次写入剪贴板的行数。 |
| `CBBufferSize` | 只控制上述辅助器的内部缓冲区。 |
| `CBScrollCount` | 只控制上述辅助器的滚动行为。 |
| `CBMinTimer` | 只控制上述辅助器写剪贴板的节流计时。 |

## 设置范围

配置目录覆盖参考实现中仍属于 Era 项目语义的 `emuera.config`、默认/固定配置和
`_Replace.csv` 固定设置，以及 RustyEra 通用客户端设置。上表列出的宿主工具偏好不再是
项目配置。`_Rename.csv` 仍属于游戏数据映射，不是固定配置，因此不并入本文件。
