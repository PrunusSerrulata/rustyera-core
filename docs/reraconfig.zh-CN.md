# `reraconfig.toml` 设计

`reraconfig.toml` 是 RustyEra 项目唯一的外部配置文件。它采用 UTF-8 编码，所有字段均可
省略；读取时，缺失字段使用 `era-config` 目录中的统一默认值。客户端必须接受 LF、CRLF
和单独 CR 行尾，写入时使用所在操作系统的原生行尾。

## 格式原则

- 文件使用 TOML，设置按用途放入浅层表中，例如 `text.font_size` 和
  `save.auto_save`。设置名称使用小写、完整且直观的英语单词。
- `[meta]` 只保存格式元数据。`schema_version` 当前为整数 `2`；
  `locked_settings` 是不可由客户端设置面板修改的规范路径数组，用来承接旧
  `_fixed.config` 的语义。
- 版本 1 文件及未写版本号的旧文件会在读取时升级为版本 2。升级会移除退役字段；
  `compatibility.drawline_starts_new_line = true` 会转移为仍受支持的
  `compatibility.legacy_nonbutton_wrapping = true`，对应锁定状态也一并转移。
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

- `audio.volume`：整数 `0..=100`，默认 `100`，表示游戏主音量百分比；支持热应用。
- `text.replace_full_width_spaces`：布尔值，默认 `false`。启用后仅由前端把游戏输出中的
  一个全角空格显示为两个半角空格，不改变 core 中的原始文本；支持热应用。
- `text.character_width_mode`：字符串枚举，默认 `"automatic"`。当前提供
  `"automatic"`、`"ambiguous_narrow"` 和 `"ambiguous_wide"` 三个占位策略；支持热应用，
  但本阶段不实现具体列宽算法。

TUI 不暴露音量设置，因为终端音频不由 TUI 控制；浏览器和 Tauri 暴露全部三项。

## 统一默认值

过去各客户端对部分缺失设置使用不同默认值。`reraconfig.toml` 统一采用：历史日志
`1000` 行、每行 `5` 个 PRINTC 项、每项宽度 `24` 列。其他设置使用配置目录列出的默认值。

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
