# Emuera 参考 CLI

这个仅限 Windows 的命令行程序封装固定在 `reference/emuera.em` 中的 C# 实现，并将其
暴露为持久 NDJSON oracle。它通过 headless console adapter 创建 Emuera 原有 parser、
data、process 和 VM 对象，不创建 `MainWindow`、原生窗口句柄或 modal dialog。

该工具只用于差分测试，不是游戏启动器。它可以暴露 C# 参考实现中的语义分析和 VM
行为，但不能代表 Rust analyzer、compiler、validator、VM 或 runtime 已实现同等能力；
每项 Rust 结论都必须有相同输入的 Rust 测试或差分结果。

## 构建与运行

使用 `Debug-NAudio` 配置，避免 Windows Media Player COM 依赖：

```powershell
dotnet build reference/emuera.em/emuera-reference-cli/Emuera.ReferenceCli.csproj `
  -c Debug-NAudio -p:Platform=x64 -r win-x64

dotnet run --project reference/emuera.em/emuera-reference-cli/Emuera.ReferenceCli.csproj `
  -c Debug-NAudio -p:Platform=x64 -r win-x64
```

进程从 stdin 每行读取一个 UTF-8 JSON object，并向 stdout 每行写出一个紧凑 JSON
object。测试套件应保持进程存活，以便复用已加载项目和 VM 状态。

每个响应都包含 `id`、`ok`、`schemaVersion`、`referenceCommit` 和 `diagnostics`。
成功响应还包含 `result`，失败响应包含 `error.type` 与 `error.message`。`id` 会原样
复制请求中的任意 JSON 值。

```json
{"id":1,"op":"lex","source":"RESULT = 1 + 2"}
{"id":2,"op":"parseExpression","source":"1 + 2 * 3"}
{"id":3,"op":"parseLine","source":"PRINTL hello","reduceArguments":false}
```

## 操作

| 操作 | 主要请求字段 | 结果 |
| --- | --- | --- |
| `capabilities` | 无 | 协议、版本、平台和操作清单。 |
| `reset` | 无 | 释放 headless console 并清除 Emuera global state。 |
| `lex` | `source`；可选 `endWith`、`flags` | 精确参考 token 与已消费 UTF-16/UTF-8 长度。 |
| `parseExpression` | `source` | 确定性 reflection graph 与 operand type。 |
| `parseLine` | `source`；可选 `reduceArguments` | 逻辑行摘要与 graph；默认不执行参数 reduce。 |
| `analyzeLine` | `source`；需要已加载项目 | 解析并进行指令参数语义 reduce。 |
| `analyzeProject` | 需要已加载项目 | 返回函数、reduced line、参数类型和 jump link 的确定性摘要。 |
| `load` | `gameDir`；可选 `debug` | 加载 `csv/`、`erb/` 并返回 VM snapshot。 |
| `eval` | `source` | 已解析表达式及当前 runtime value。 |
| `execute` | `statement`；可选限制和 `watch` | 在当前 VM 执行一条非控制流指令。 |
| `run` | 可选 `entry`、`arguments`、`inputs`、`uiInputs`、限制和 `watch` | 独立运行函数或恢复等待中的输入。 |

`lex.endWith` 和 `lex.flags` 中的名字不区分大小写，并对应 C# enum；默认分别为 `EoL`
和 `None`。调用方应通过 `capabilities` 检测协议变化，而不是只检查 binary version。

## 项目与 VM 示例

```json
{"id":"load","op":"load","gameDir":"C:\\games\\my-era"}
{"id":"set","op":"execute","statement":"FLAG:10 = 123","watch":["FLAG:10"]}
{"id":"run","op":"run","entry":"MY_TEST","arguments":"42, \"text\"","inputs":["0"],"watch":["RESULT","RESULTS"],"instructionLimit":1000000,"timeoutMs":10000}
```

`run.entry` 使用 Emuera 原有 CALL 参数 parser，但把选定函数作为独立 VM run 的 root。
snapshot 的 `termination` 可能为 `completed`、`waitingInput`、`instructionLimit`、
`timeout`、`quit` 或 `error`。如果函数等待输入，可在同一请求提供 `inputs`，或随后发送
只包含 `inputs` 的 `run` 请求。

需要 UI-sensitive input oracle 时，`uiInputs` 接收带 `text` 与 `changedByMouse` 的
object，先使用固定 UI 层的 ONEINPUT normalization，再恢复未修改的参考 VM。普通测试
应继续使用 `inputs`；只有在物理鼠标差异属于待比较行为时才使用 `uiInputs`。

CALL、JUMP、BEGIN 等控制流应使用 `run`。`execute` 只支持 assignment、printing 等独立
指令，因为 synthetic control-flow line 没有有效的源码返回地址。

`output` 是完整当前 display buffer，而不是 delta。数值 runtime value 和 token literal
保持为 JSON number。较深的 parser/AST object 以带 `$id`、`$ref`、`$type` 和
`$truncated` 的确定性 graph 表示，避免另外维护一套 C# AST。

## 冒烟测试

Windows：

```powershell
tools/protocol-smoke.ps1
```

macOS/Wine：

```sh
tools/test-macos-wine.sh
```

脚本会验证 malformed request 不会终止持久进程，并覆盖 lexer、expression parser、
logical-line parser、项目加载、CSV value、独立函数执行、输入、watch 和 reset。
macOS 测试还会执行项目加载与 CSV evaluation，以发现 Wine 下意外的 WinForms 初始化
依赖。

平台冒烟测试通过只表示 oracle 可用。Rust/C# 差分还必须向双方提交相同 fixture 或
source，并比较当前组件相关的字段。

## Oracle 维护

CLI 无法启动、提前退出、不再为每个请求产生响应或发生 hang，都是测试基础设施缺陷，
不能作为跳过差分测试的理由。处理顺序如下：

1. 用最小请求序列复现；
2. 判断问题属于启动、协议投影还是 UI 依赖；
3. 优先修复 wrapper；
4. 必要时才在 `reference/emuera.em` 添加最小 hook，并用 `Program.HeadlessMode` gate，
   或保证只能由 friend assembly `Emuera.ReferenceCli` 调用；
5. 重跑失败序列、完整平台冒烟测试和相同输入的 Rust 差分测试。

参考树修改可以暴露只读状态、跳过不可见 UI 工作或添加确定性测试限制，但不得修改
正常游戏链路使用的 parser、项目加载、验证、执行、状态变化或其他 backend 行为，也
不得为了匹配 Rust 而改变 oracle。

每项参考树修改都必须在
[参考 CLI 修改记录](REFERENCE_CHANGES.md)中逐文件说明目的、headless gate、对普通
游戏链路的影响和验证方式，任务交付也必须再次列出本次修改文件。

## 保真说明

- wrapper 直接调用原始参考类型；为无窗口 console 构造、状态读取、warning、limit 和
  独立函数执行添加的 hook 都有严格 headless gate。普通启动仍创建 `MainWindow` 并沿用
  原 UI 链路。
- 完整 hook 清单和理由见 [REFERENCE_CHANGES.md](REFERENCE_CHANGES.md)。
- 目标为 Windows x64 / .NET 10，因为参考 assembly 依赖 WinForms 与 Windows Desktop
  runtime 类型。
- NDJSON 协议文本使用 UTF-8；游戏文件编码行为保持参考实现原样。RustyEra 自身只接受
  UTF-8。
- 只有 `Program.HeadlessMode` 激活时才抑制 modal dialog；需要回答的问题保守地选择
  “No”。
