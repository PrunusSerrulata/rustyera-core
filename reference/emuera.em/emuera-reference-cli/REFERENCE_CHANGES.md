# 固定 Emuera 参考树的 CLI 修改记录

`reference/emuera.em` 是兼容性 oracle，默认视为只读。本文件是为
`reference/emuera.em/emuera-reference-cli` 所需少量例外设置的强制审计日志，并把
参考树修改与 Rust 实现、wrapper 修改分别记录。

固定兼容基准仍为 `26a35dc9334bb67590b96f7b8efbefbf199e391e`。以下 hook 不构成
参考版本更新。

## 不变量

每个参考树 hook 都必须满足：

- 为暴露或稳定 reference oracle 所必需；
- 受 `Program.HeadlessMode` gate、只能由 friend assembly 调用，或完全只读；
- 普通 Emuera entry point 继续使用原 backend 逻辑；
- 不通过改变语言或 runtime 语义迫使 C# 结果匹配 Rust；
- 已记录平台冒烟测试和相同输入的 Rust 差分测试。

CLI 失败或 hang 时应先修复 wrapper。只有 wrapper 无法避开 UI 依赖，或无法观察所需
backend 状态时，才能修改参考树。

## 当前参考树 hook 清单

下表是当前参考树中全部有意 oracle 修改。

| 参考文件 | Oracle 目的 | 隔离方式与普通游戏影响 |
| --- | --- | --- |
| `Emuera/Emuera.csproj` | 通过 `InternalsVisibleTo` 授予 `Emuera.ReferenceCli` 访问 internal 参考类型的权限。 | 只改变 assembly 可见性，不改变 runtime 行为。 |
| `Emuera/Program.Headless.cs` | 定义 `HeadlessMode`，并在不进入 `Program.Main` 的情况下配置游戏目录和 debug flag。 | 只由 friend CLI 调用；普通 entry point 和目录设置不变。 |
| `Emuera/UI/Dialog.cs` | 防止不可见 modal dialog 阻塞 NDJSON 请求；prompt 保守地回答 “No”。 | 仅在 `HeadlessMode` 为 true 时分支；普通模式仍调用相同 WinForms message box。 |
| `Emuera/Runtime/Script/Data/ParserMediator.Headless.cs` | 按响应原子投影并清除 parser warning。 | 只读 helper，仅由 CLI 调用；warning 产生逻辑不变。 |
| `Emuera/Runtime/Script/Process.Headless.cs` | 添加有界执行、独立指令 dispatch、独立函数 entry、counter 与 termination state。 | entry point 为 internal 且只由 CLI 使用；非 headless 模式下 limit check 立即返回。 |
| `Emuera/Runtime/Script/Process.ScriptProc.cs` | 在真实 VM dispatch 边界调用 headless 指令/timeout check。 | `HeadlessCheckLimit` 在非 headless 模式下为空操作，其余 dispatch 不变。 |
| `Emuera/Runtime/Script/Process.cs` | 防止独立 headless root function 完成后继续进入标题/系统状态机。 | `HeadlessFinishFunctionRun` 在非 headless 模式下返回 false，保留原 system-process loop。 |
| `Emuera/UI/Game/EmueraConsole.Headless.cs` | 在没有 `MainWindow` 时构造 console，暴露 state/process/input，恢复执行并适配 textbox change。 | 非 `HeadlessMode` 禁止构造；普通模式 textbox adapter 调用原 `MainWindow.ApplyTextBoxChanges`。 |
| `Emuera/UI/Game/EmueraConsole.cs` | 允许 backend console/process 无原生窗口运行，并在 headless 模式跳过 debug window、repaint、scrollbar、timer 和 title widget 操作。 | 每项变化都以 `HeadlessMode` 为条件；保留公开 `EmueraConsole(MainWindow)` 游戏链路。脚本执行、input state 与 output buffer 仍使用原 backend。 |
| `Emuera/UI/Game/EmueraConsole.Print.cs` | 保留 snapshot 所需 display buffer，同时抑制原生 repaint/textbox 工作。 | 只在 headless 模式跳过 UI side effect；普通 rendering call 不变。 |
| `Emuera/Runtime/Script/Statements/Instraction.Child.cs` | 通过 console adapter 路由 INPUT-family textbox layout update，避免等待输入时解引用不存在的 window。 | 六条 INPUT-family 指令仍构造相同 request 并调用相同 backend wait；普通模式 adapter 执行原 textbox call。 |
| `Emuera/UI/Framework/Forms/MainWindow.Headless.cs` | 保留早期 hidden-window host 使用的 internal console accessor。 | 纯只读 property。当前 CLI 已不再创建 `MainWindow`，普通游戏不受影响。 |

## 修改记录

### 2026-07-23：CLI 与固定参考树同目录

wrapper 从 `tools/emuera-reference-cli` 移至
`reference/emuera.em/emuera-reference-cli`，使参考实现与 headless adapter 位于同一
树中。Windows 与 macOS 启动脚本仍在参考树外：
`tools/protocol-smoke.ps1` 和 `tools/test-macos-wine.sh`。

本次迁移进入参考树的文件：

- `emuera-reference-cli/Emuera.ReferenceCli.csproj`：移动后只把相对
  `ProjectReference` 改为 `../Emuera/Emuera.csproj`。
- `emuera-reference-cli/JsonProjection.cs`、`OracleService.cs`、`Program.cs`、
  `ReferenceHost.cs`：只移动位置，不改变内容或执行语义。
- `emuera-reference-cli/README.md`：移动并更新命令路径。
- `emuera-reference-cli/REFERENCE_CHANGES.md`：移动并记录本次迁移。
- `emuera-reference-cli/tests/fixture-oneinput-long/csv/_fixed.config`、
  `tests/fixture-oneinput/erb/oneinput.erb`、`tests/fixture-system/csv/TRAIN.CSV`、
  `tests/fixture-system/erb/oracle.erb`：只移动位置。
- `emuera-reference-cli/tests/fixture/csv/.gitkeep`、`ABL.CSV`、`ABL.als`、
  `CHARA0.CSV`、`CSTR.CSV`、`GAMEBASE.CSV`、`ITEM.CSV`、`STR.CSV`、
  `VarExt-oracle.csv`、`VariableSize.CSV`、`_Replace.csv`：只移动位置。
- `emuera-reference-cli/tests/fixture/erb/linecount.erb`、`oracle.erb`、
  `print-family.erb`、`restart.erb`：只移动位置。
- `emuera-reference-cli/tests/wine-smoke.ndjson`：只移动位置。

本次迁移没有修改 `reference/emuera.em/Emuera` 下的文件，因此 headless 隔离与普通游戏
语义不变；上表中的 gate 仍是对固定实现的唯一修改。

### Headless oracle 初始集成

初始 CLI 集成加入 friend-assembly 声明、headless mode 配置、诊断投影、有界/process
执行 helper、console/process state 访问、modal-dialog 抑制，以及早期
`MainWindow.Headless.cs` accessor。

验证由 Windows protocol 冒烟测试，以及消费等价输入的 Rust lexer/parser 测试提供。

### 2026-07-14：Wine 无窗口项目加载

问题：仅 lexer/parser 的请求可在 Wine 下工作，但加载完整 fixture 会在 CLI 初始化
WinForms 并强制创建隐藏原生窗口句柄时阻塞。

参考树修改：

- `Emuera/UI/Game/EmueraConsole.Headless.cs`：添加仅 headless 的无窗口 constructor 和
  textbox adapter。
- `Emuera/UI/Game/EmueraConsole.cs`：添加 headless UI guard，同时保留普通 constructor
  与 backend 初始化。
- `Emuera/UI/Game/EmueraConsole.Print.cs`：只在 headless 模式抑制原生 paint/textbox
  side effect。
- `Emuera/Runtime/Script/Statements/Instraction.Child.cs`：将六条 INPUT-family UI call
  路由到 gated adapter。

参考树外的 wrapper/test 修改从 `ReferenceHost` 移除
`ApplicationConfiguration.Initialize`、`MainWindow` 和原生 handle 创建；macOS Wine
脚本扩展到项目加载、CSV value、语义行 reduce、执行、函数调用、输入与 reset，并加入
watchdog。

验证命令：

```sh
dotnet build reference/emuera.em/emuera-reference-cli/Emuera.ReferenceCli.csproj \
  -c Debug-NAudio -p:Platform=x64 -r win-x64 --no-restore
tools/test-macos-wine.sh
cargo test -p erabasic-csv reference_cli_fixture_has_the_same_rust_projection -- --exact
cargo test --workspace
```

Wine 测试完成全部请求且 stderr 为空。Rust/C# CSV projection 在 ABL size/name lookup、
item price、初始 STR data、character ABL data 和 GAMEBASE code 上一致。

### 2026-07-15：无窗口 oracle 的 timed one-input wait

问题：加载在 `SYSTEM_TITLE` 执行正时间 `TONEINPUTS` 的最小项目时，系统构造了正确的
`InputRequest`，随后在更新 `MainWindow` last-input marker 时抛出
`NullReferenceException`。

参考树修改：

- `Emuera/UI/Game/EmueraConsole.cs`：仅在 `Program.HeadlessMode` 激活时跳过
  `window.update_lastinput()`。input request、timer 设置和 backend state transition
  不变。普通游戏中 `HeadlessMode` 为 false，仍执行原 UI call。

若为 wrapper 提供该 UI object，会重新引入 headless mode 原本要避免的 hidden-window
依赖，因此不能只在 wrapper 中修复。验证使用最小 timed `TONEINPUTS` load request、
完整 macOS Wine 冒烟测试，以及同输入 Rust analyzer fixture
`reference-input-signatures.json`。

### 2026-07-23：项目文档简体中文化

文档修改：

- `emuera-reference-cli/README.md`：把项目自有 CLI 说明翻译为简体中文，并按当前路径和
  冒烟测试约定更新。
- `emuera-reference-cli/REFERENCE_CHANGES.md`：把审计记录翻译为简体中文，并保留全部
  hook、隔离条件、验证方式与固定基准信息。

本次只修改 Markdown 文档，没有修改 `Emuera` 源码、headless hook、parser、项目加载、
执行或状态变化。普通游戏链路和 oracle 语义不受影响。

## 后续记录模板

新增记录必须包含：

- 原始 failure/hang 与最小 reproducer；
- `reference/emuera.em` 下每个修改路径；
- wrapper-only 修复为何不足；
- headless/friend-only 隔离机制；
- 普通游戏 backend 语义为何不变；
- 平台 smoke command 与相同输入 Rust comparison；
- 剩余平台或行为限制。
