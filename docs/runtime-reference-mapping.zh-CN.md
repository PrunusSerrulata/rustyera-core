# Emuera runtime 参考映射

本清单的兼容基准固定为原版 `emuera.em` 提交 `26a35dc9334bb67590b96f7b8efbefbf199e391e`，
不代表蛇版 emuera 的行为清单。涉及蛇版的改动须按
[core 测试 skill](../.agents/skills/test-rustyera-core/SKILL.md) 选择蛇版或双 oracle 验证，
不能直接把本文结论套用到蛇版。本清单记录
参考实现如何影响公共接口和当前 runtime 设计。表中一行描述的是目标契约，不能据此
推断每个参考系统状态或 Host 操作都已经实现。

参考行为按[设计原则](design-principles.zh-CN.md)解释：可移植且对脚本可见的规则仍是
兼容目标；WinForms/GDI 实现细节则转换为语义意图，或明确标为不支持的可移植操作。

| 参考区域 | 观察到的行为 | RustyEra 契约表示 |
| --- | --- | --- |
| `Runtime/Script/Process.cs`、`Process.SystemProc.cs` | 中央脚本/系统状态机覆盖标题、opening、train、ablup、shop、存档读写、正常执行和 ERB reload。 | `RuntimePhase`、生命周期消息、项目 revision 与 reload transaction。 |
| `Runtime/Script/Process.State.cs` | 系统状态决定 BEGIN、存档与读档是否合法；call frame 由 process state 持有。 | runtime 保持聚合权威；VM frame 只通过 `VmDebugInspect` 检查。 |
| `Runtime/InputRequest.cs` | 九种有效输入类型、默认值、one-input、message-skip、system/mouse 标志与可选 timeout 状态。 | `WaitKind`、`InputWait`、规范化 `InputIntent`、不透明交互 token 与 `WaitChange` 保留可观察契约，不向前端暴露游戏值。 |
| `UI/Game/EmueraConsole.cs` | console 在初始化、运行、输入等待、sleep、quit 和 error 间转换；UI timer 与输入竞争，过期按钮按 generation 检查。 | 有序 monotonic time/input 消息、wait ID、epoch-scoped 不透明 token、临时 QTE 等待和明确 shutdown/fault 状态。 |
| `UI/Game/EmueraConsole.Print.cs` | 文本、样式、按钮、HTML、图像、shape、background、窗口标题和日志缓冲区可观察。 | 先发送带 revision 的 `PresentationSnapshot` 同步基线，再发送 `PresentationDelta`；前端渲染对象只是投影。 |
| `Statements/Instraction.Child.cs`、`Function/Creator.Method.cs` | 音频、动态图形、字体度量、文件、网络更新和外部 URL 直接调用平台 API。 | 异步 storage/platform 服务消息；VM 指令和 runtime 层不执行 OS I/O。 |
| `Statements/Variable/VariableEvaluator.cs` | 直接列出、读写 save slot、global save 和 DAT 文件，并检查游戏/版本兼容性。 | runtime 持有内存 codec、验证与 transaction，通过相关前端 storage namespace 完成 I/O。 |
| `UI/Game/PrintStringBuffer.cs` 与 console line 类型 | `PRINTC`、对齐、分隔线、自动按钮、HTML 与物理历史混合了语义输出和 GDI/WinForms 度量。 | runtime 持有逻辑行、`ColumnCell`、`Separator`、history operation 与 token。客户端像素布局不是规范化状态；显式查询使用绑定 revision 的类型化前端服务，缺失操作报告不支持。 |
| GDI/canvas/resource 类 | 图像解码、光栅修改、字体度量和动画调度使用 Windows 设备对象。 | 规范化 resource/canvas replay 加类型化前端服务。按内容寻址的源图像事实与依赖前端的文本/canvas 光栅观察分离；无法形成可移植契约的物理 GDI/CBG API 保持不支持。 |
| `Process.CalledFunction.cs` | frame 保留函数身份、返回地址、event 状态、参数和 local scope。 | 带 generation/frame 的 VM debug descriptor 和源码映射 call stack。 |
| `EmueraConsole.DebugCommand`、`UI/DebugDialog.cs` | watch 对当前表达式求值；debug command 会 clone/restore 控制状态，并拒绝 flow、wait、partial 和不安全指令。 | 独立 debug channel、一致 stop token 与参考安全 console 子集。 |

## 有意的架构差异

参考 console 持有 timer 和大量展示状态，RustyEra 将其转换为 runtime 权威消息；参考
存档和资源代码直接执行 I/O，Rust 契约从不这样做；权限模型、breakpoint 与 stepping
属于 debugger 扩展，不宣称是 Emuera 原生行为。

runtime 的权威范围包括语义展示，以及接受观察结果后发生的游戏状态变化；它本身并非
renderer。当脚本显式请求依赖字体、viewport 或 raster 的结果时，目标契约使用当前
权威前端绑定 revision 的响应。若该值影响持久玩法，系统会把它诊断为可移植性风险；
只有在存在可移植替代方案并审查真实脚本用法后，才可能弃用。

当前 Python/Textual TUI 用于检查上述公共 C ABI 和消息契约能否驱动真实项目。其终端
排版、启动耗时或渲染吞吐不能代表 runtime 的规范化语义质量，也不能作为其他前端的
性能基线。

reference CLI 的 `load` 响应会把 `Now Loading...`、CSV 协调警告和依赖机器时钟的
`Elapsed time` 行混入 console output。RustyEra 有意不把这些宿主加载进度写入可恢复的
规范化展示历史；诊断通过日志/诊断事件投影，计时留在客户端遥测。固定 fixture 的差分
因此分别核对终止原因与进入 `SYSTEM_TITLE` 后的脚本输出，并保留原始加载输出差异记录。

FORM/PRINTFORM 的字符串字段宽度按 Unicode 终端显示列计算。固定参考实现会按
`useLanguage` 选择的 ANSI code page 字节数填充；这会让不可编码字符与同为全角的
可编码字符落在不同列。RustyEra 在这里采用跨客户端一致的显示列语义；`STRLENS` 等
明确依赖传统编码的操作不受此差异影响。
