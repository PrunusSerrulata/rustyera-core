# Runtime 可玩性与纯 TUI 前端审计

本文记录 RustyEra 从项目加载到退出的全生命周期可玩性审计、纯 TUI 前端可行性审计，
以及后续以完整 `reference/eraTW` 为输入的 C# / Rust 差分执行进度。本文描述经过实际代码
检查和测试确认的事实；协议中存在类型、catalog 中存在名称或小型测试通过，都不单独视为
真实游戏已经可玩。

初始审计日期：2026-07-18；完整工程复验更新：2026-07-19。

## 当前结论

完整 `reference/eraTW` 的基线可玩路径现已通过：Rust 接收与 C# oracle 相同的 2,434 个
CSV/ERH/ERB/配置文件，项目加载为 0 error、0 fatal；随后完成标题选择、角色和地图初始化、
自动存档、跳过 UPDATE，并到达第 1 日含 SAVE/LOAD/UPDATE 的稳定系统主菜单。审计前端
实际持久化 6,070,273 字节传统存档，销毁 session 后在新 session 中重新加载工程、导入存档、
执行 `EVENTLOAD` 并恢复到同一稳定主菜单；有序 shutdown 另见执行记录。

这证明当前 runtime 与纯文本、无图片、无音频的 caller-pumped 前端能够运行该真实游戏的
基线生命周期，不等于所有长期分支均已覆盖。TRAIN/QTE、热替换、VM snapshot、调试和
兼容性状态表中的稳定缺失项仍主要由专门测试覆盖；展示层也仍有 PRINT 物理行语义和
snapshot 增长问题。完整工程未执行到的脚本分支不得据此自动标记为已实现。

`reference/eraTW-minimal` 是不完整脚本子集。C# reference CLI 也会因为被裁掉的函数而
终止，因此它不能单独证明完整游戏可玩；但它能够暴露 Rust 额外产生的大量基础语法和
声明错误。真正的放行标准必须使用完整 `reference/eraTW`。

## 全生命周期覆盖审计

| 生命周期阶段 | 实现状态 | 审计结论 |
| --- | --- | --- |
| 握手与能力协商 | 已实现 | Rust API 可驱动；公共 C ABI 版本曾不一致 |
| 工程文件提交 | 已实现内存边界 | 2,434 个项目根相对路径文件已实测；runtime 在内部剥离 category root |
| CSV/ERH/ERB 分析编译 | 基线路径通过 | 完整 eraTW 为 0 error/fatal；仍有 559 个兼容性/可移植性 warning |
| 标题与新游戏 | 基线路径通过 | 与 C# 使用对应选择，均到达第 1 日稳定系统主菜单 |
| TRAIN/AFTERTRAIN/ABLUP/TURNEND/SHOP | 主框架存在 | 分阶段单元测试通过，未在完整游戏连续执行 |
| 普通脚本与动态调用 | 基线路径通过 | 完整初始化实际覆盖静态/动态调用、REF、对象函数、控制流和随机数；未执行分支仍以状态表为准 |
| 输入、QTE 与逻辑时间 | 主框架存在 | token、epoch、超时由 runtime 判定；前端驱动时钟 |
| 展示与交互 | 部分实现 | 规范化模型存在；若干 PRINT 物理行语义和增量传输缺失 |
| 传统存档与 VM snapshot | 传统存档完整工程通过 | 6,070,273 字节存档已跨 session 导入并恢复；VM snapshot 仍由专门测试覆盖 |
| 热替换 | 已有实现 | 缺少完整游戏修改后继续执行的端到端证明 |
| 调试 | 协议和主要接口存在 | 不作为可玩性的必要前置条件 |
| 退出与重启 | 完整工程通过 | 恢复后的稳定等待执行 graceful shutdown，依次进入 Stopping/Stopped 并返回 `ShutdownReady` |

仓库中的 `focused_eratw_system_slices_exercise_runtime_owned_save_flows` 只读四个真实脚本并
检查字符串是否存在；测试源码也明确说明功能行为由小型 controller fixture 覆盖。它不是
真实项目加载或生命周期测试。

## 已修复的初始阻断项

### 合法 `#DIM` 声明被拒绝

隔离 fixture 在现有可运行 C# oracle 工程中加入：

```erb
#DIM AUDIT_DECLARATION; comment without whitespace before semicolon
```

C# reference CLI 成功进入 `waitingInput` 并输出 `ORACLE_READY`；Rust 项目加载返回：

```text
analyzer.invaliddeclaration: a comma is required before a #DIM size
```

原因是 `erabasic-analyzer` 的声明扫描没有在 lexer 已判定的注释边界停止。本轮已在声明
解析前按参考转义规则剥离注释，并加入无空白分号注释的回归测试。

### 项目路径语义不一致

公共 API 规定 `SubmittedFile.relative_path` 相对于项目根。自然扫描会产生
`CSV/_Rename.csv` 和 `ERB/DIM.ERH`；CSV loader 当前却以 `_Rename.csv`、
`GAMEBASE.CSV` 等 category-root 文件名进行精确查询。按文档提交
`reference/eraTW-minimal` 时，即使文件存在仍会报告：

```text
csv.missingrenamefile: _Rename.csv is enabled but was not submitted
```

runtime 现已根据 `FileCategory` 接受项目根相对路径，并只在送入 CSV loader/analyzer 的
内部边界去除匹配的 `CSV/` 或 `ERB/` category 前缀；完整工程已用 root 路径实测通过。

### 展示与长会话增长

该问题已在 Protocol 20 中解决：`emit_presentation()` 首次同步和重同步发送
`PresentationSnapshot`，之后发送只包含变化行及恢复字段的 `PresentationDelta`。Runtime
仍保留权威 history 供存档、重同步和物理历史服务使用，但不再在每次 PRINT 时复制和传输
全部历史。完整 eraTW 的 UTF-8 项目清单约为 77 MiB，creator-owned 默认上限为 127 MiB；
客户端仍可在握手时协商更低的传输上限。

已确认仍未对齐的输出行为包括：

- `PRINTSINGLE*` 没有立即提交独立物理行；
- `PRINTN` 没有进入参考等待；
- PRINT 文本中的嵌入换行没有拆分为多个逻辑行；
- L/W 语义仍主要按名称末尾判定。

`BAR`、`BARL` 和 `BARSTR` 均已有执行路径；旧状态文档中“仅 BARSTR 可用”的说明过期。

## 纯 TUI 前端可行性与当前实现

仓库现包含 `frontends/era-tui`：它使用 Python Textual 构造真实 TUI，通过 `ctypes`
动态加载 `era-runtime-capi`，不直接依赖 `era-runtime` 或共享 Rust 内部对象。独立 worker
串行执行 `session_submit`、`session_drive` 和 `session_poll`；Textual 事件循环只交换前端
命令和展示事件。它支持文本、颜色、对齐、列单元格、分隔线、按钮 token、键盘输入、
QTE 计时、Storage、VM snapshot、热重载、调试和退出，不协商图片、HTML、音频或视频。

当前可行性分级：

| 前端形态 | 当前结论 |
| --- | --- |
| Python Textual TUI + C ABI + 小型兼容脚本 | 已实测项目加载、标题等待、热重载、snapshot 跨 session 恢复和调试协议 |
| Python Textual TUI + C ABI + 完整 eraTW | 已按基线 20 个输入实测到达第 1 日 SAVE/LOAD/UPDATE 菜单，并通过前端 Storage 写出自动存档 |
| Rust TUI + 完整 eraTW 基线路径 | 已实测可玩到第 1 日主菜单，并跨 session 保存/恢复 |
| C/C++ TUI + 动态库 | C ABI/协议契约已同步；Python `ctypes` 调用已放行，尚无 C/C++ 应用前端测试 |
| 不渲染图片/音频，但读取图片 metadata | 架构可行 |
| 当前 Textual TUI 完全不处理图片文件 | 不提交 resource manifest/图片，文本流程可继续；依赖 sprite/image 可观察值的脚本会看到能力缺失 |

当前 Textual TUI 已做到：

- 在完整 poll 批次后以最终 `SessionEpoch` 累计确认 runtime outbound sequence，避免热重载
  同批次的旧/新 epoch 交界产生 stale ACK；
- 对无法播放的一次性 effect 明确返回失败结果，不伪造成功；
- 提交 projection observation 和单调逻辑时间；
- 实现原子、带 revision/precondition 的跨平台用户数据 Storage，以及 clock、entropy、
  GETKEY、`get_display_line`、`serialize_physical_history` 和终端 cell 字体测量；
- 明确协商 `html=false`、`graphics=false`、`audio=false` 和 `video=false`。普通 HTML 文本可
  降级，但 HTML button token、图片和音视频不在该前端支持范围内。

## 完整 eraTW 差分执行门槛

按本轮任务要求，后续工作严格遵循以下顺序：

1. 先用 C# reference CLI 加载完整 `reference/eraTW` 并实际推进到稳定输入等待或其他可验证
   的正常游戏状态。
2. 如果失败，先判断是 reference CLI/headless 接入问题、游戏输入缺失、普通参考运行错误，
   还是游戏使用了固定参考实现根本未定义的指令。
3. 如果确认存在“eraTW 使用参考实现未定义指令”，在本节记录指令、源码位置和参考实现
   查找证据，然后停止，不运行 Rust 完整游戏修复。
4. 只有 C# oracle 正常运行后，才对同一文件集合运行 Rust，并以同一生命周期里程碑逐项
   修复，直至达到下述放行标准。

### 正常运行放行标准

完整游戏至少必须：

- 项目加载成功，没有 error/fatal 诊断；
- 进入标题输入等待，并能接受一个合法标题选项；
- 新游戏初始化完成并进入第一个稳定游戏输入点；
- 至少完成一次实际系统流程转换和普通脚本输入；
- 能创建传统存档，销毁 session 后重新加载并恢复到稳定状态；
- 能执行正常退出；
- 过程中没有 unsupported Host、VM fault、协议资源耗尽或未处理 service 请求。

## 执行记录

| 阶段 | 状态 | 结果 |
| --- | --- | --- |
| 初始代码、协议、文档和测试审计 | 完成 | 结论见本文前述章节 |
| 保存审计并清理过期公共契约 | 完成 | C ABI 统一为 3.0；CDDL 补齐 lifecycle control；前端指南补齐确认、退出、snapshot-only 和现有 service；状态表移除 BAR/GETLINESTR 过期结论 |
| C# reference CLI 加载/运行完整 eraTW | 完成 | 完整工程无 error/fatal 诊断；从标题选择初次游玩，经过角色、地图、私室和新游戏初始化，跳过 UPDATE 后到达第 1 日主菜单的稳定系统输入（input ID 32） |
| Rust 加载/运行同一 eraTW | 完成 | 2,434 文件、0 error/fatal；完成新游戏、6,070,273 字节自动存档、跳过 UPDATE，并到达第 1 日 SAVE/LOAD/UPDATE 主菜单 |
| Rust 跨 session 传统存档恢复 | 完成 | 新 session 重新加载完整工程，`StateImportReady` 后执行 `EVENTLOAD`，恢复到同一稳定系统主菜单，无 fault |
| Rust 有序退出 | 完成 | 恢复后的稳定等待提交 graceful shutdown，最终 phase 为 `Stopped`，`ShutdownReady` 报告 1 个等待操作已取消 |
| Python Textual TUI + C ABI 到第 1 日菜单 | 完成 | 修复可选等待清除解码后，从全新 Storage 开始完成 20 个基线输入，在 wait 5524 到达 SAVE/LOAD/UPDATE 菜单并写出 6,070,269 字节 `save99.sav` |

初始审计曾执行并通过 `cargo fmt --all -- --check`、`cargo test --workspace`、
`cargo clippy --workspace --all-targets -- -D warnings` 和 macOS/Wine reference smoke test；
2026-07-19 修复后的最终命令结果在本次提交交付说明中记录。审计全程没有修改
`reference/emuera.em`。

### C# 完整工程运行证据

macOS/Wine 下的持久 reference CLI 会话加载了
仓库内的 `reference/eraTW`（通过 Wine 的 `Z:` 映射提交）。首次 `load` 在
`TITLE.ERB:43` 以 `waitingInput` 正常停止，并输出标题以及“初次游玩/存档再开”选项；随后
选择初次游玩，累计执行超过十万条指令，依次完成初始化问答、博丽神社地图、自宅私室和
新游戏自定义阶段。新游戏处理单次执行 315,319 条指令并报告“游戏開始處理完成”。跳过
游戏 UPDATE 后继续执行 606 条指令，最终显示第 1 日状态、体力/气力、主菜单和 SAVE、
LOAD、UPDATE 项，并在系统整数输入 ID 32 稳定等待。

整个序列均返回 `ok: true`、`termination: waitingInput`，没有 parser/runtime error，也没有
发现 eraTW 使用固定参考实现未定义的指令。因此任务规定的提前退出条件没有触发，可以
进入 Rust 同工程运行阶段。本次没有修改任何 `reference/emuera.em` 文件。

### Rust 完整工程运行与修复证据

Rust 初次完整加载报告 619 条诊断，其中 559 条 warning、0 error/fatal。为到达与 C# 相同
里程碑，本轮修复了 category-root 路径、声明/表达式语法、CSV 名称索引、控制流、动态调用、
REF 参数、函数局部/静态存储规模、角色数组查询、逻辑短路、`RAND(min,max)` 和旧式
`RAND:max` 等实际阻断项。每个代码路径均有最小 Rust 回归测试；完整运行不是这些测试的
替代品。

最终 Rust 运行从标题开始，依次完成口上选择、START、自定义角色、经典地图、博丽神社、
默认私室和新游戏选项，输出“游戏開始処理完成”，写出 6,070,273 字节 `save99.sav`，跳过
UPDATE 后在 SAVE/LOAD/UPDATE 主菜单稳定等待。恢复审计把该字节串作为
`StateExportKind::TraditionalSave` 导入全新 session，收到 `StateImportReady`，执行
`EVENTLOAD` 后再次在相同主菜单稳定等待。两个执行阶段均没有 unsupported Host、VM fault、
协议资源耗尽或未处理 service；恢复所需 `Clock/local_date_time` 由审计 TUI 明确协商并响应。
随后 graceful shutdown 产生 `Stopping`、`Stopped` 和 `ShutdownReady`，最终 runtime phase 为
`Stopped`。

### Python Textual TUI 完整工程复验

2026-07-19 使用 `frontends/era-tui` 的真实 `RuntimeWorker`、`ctypes` C ABI、canonical CBOR
协议和全新前端 Storage 目录，按上述 Rust 审计完全相同的 20 个输入重新运行完整 eraTW。
首次提交标题选项后暴露出 `SetInputWait(None)` 的 wire 解码遗漏：minicbor 会省略为 `None`
的枚举 tuple 字段，前端却无条件读取第一个字段。修复为正确接受零字段清除操作后，运行依次
完成口上选择、START、自定义角色、经典地图、博丽神社、默认私室、新游戏初始化和自动
存档，跳过 UPDATE 后在系统输入 wait 5524 显示含 SAVE、LOAD、UPDATE 的第 1 日菜单。

该次运行写出 6,070,269 字节 `save99.sav` 和 1,317 字节 `global.sav`，没有 Runtime fault、
unsupported Host、协议资源耗尽或 Storage 错误。完整 C ABI/Textual 投影路径耗时约 260 秒，
明显高于本文既有的 Runtime 内部 release 基线；这不阻断可玩性结论，但属于独立的前端/C ABI
性能课题，不能用内部 11 秒基线替代。前端现把每次 VM 驱动预算限制为 10,000 条指令以保持
caller-pumped 响应性，并在 poll 批次内合并可恢复展示状态的重复行更新；两项都不改变脚本
指令顺序、输入判定或最终规范化展示状态。

用于复现这些长运行的本地源码和生成日志位于 `.audit/runtime-playability/`。根 `.gitignore`
忽略整个 `.audit/`，因此工具、完整游戏路径、Wine 输出和存档不会进入版本控制。

## 维护要求

- 每次完整游戏尝试都在“执行记录”追加输入范围、终止状态、首个阻断错误和修复结果。
- 不以警告数量代替错误归因；大量 analyzer/compiler 诊断可能是少量早期语法错误的级联。
- C# 与 Rust 必须接收同一游戏文件集合；允许忽略绝对路径、请求 ID 和耗时等环境元数据。
- 对参考源码的任何 headless 修复逐文件追加到
  `tools/emuera-reference-cli/REFERENCE_CHANGES.md`，并证明正常 UI 游戏链路不受影响。
- 只有实际执行到的基线路径可以描述为已放行；未执行分支继续按兼容性状态表和专项测试记录。
