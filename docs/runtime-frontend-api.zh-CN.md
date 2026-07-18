# Runtime 前端公共 API 指南

本文面向后续 GUI、TUI、启动器或 C/S 前端开发者，说明 RustyEra runtime
边界的接口用途、参数、返回值、消息顺序和资源所有权。

发生冲突时遵循项目统一优先级：跨客户端/跨平台支持，其次架构纯净，最后才是对
参考客户端实现细节的严格复刻。Runtime 提供唯一的规范化语义表示，各前端负责自身
平台上的渲染和设备适配；完整规则见 [设计准则](design-principles.md)，能力缺口见
[runtime 兼容性状态](runtime-compatibility-status.zh-CN.md)。

> 当前开发阶段不承诺协议向下兼容。本文描述已经定义的 ABI 与消息合同，不表示
> 所有预留的 runtime、存档、热重载、媒体或调试能力均已实现。前端必须以握手返回
> 的功能集合为准，不能仅根据类型或消息标签存在就假定功能可用。

接口分为两层：

1. C ABI：加载动态库、创建 session、提交消息、驱动 runtime、取出消息和释放内存。
2. Runtime wire protocol：通过确定性 CBOR 信封传输项目、输入、展示和服务消息。

相关定义：

- C 头文件：`crates/era-runtime-ffi/include/era_runtime.h`
- C ABI Rust 定义：`crates/era-runtime-ffi`
- 公共信封：`crates/era-protocol`
- Runtime 消息：`crates/era-runtime-protocol`
- Debug 消息：`crates/era-debug-protocol`，与本文 runtime channel 独立

## 1. 基本工作模型

Runtime 不回调前端，也不执行文件 I/O、窗口绘制、操作系统输入采样或系统时钟
采样。前端使用 caller-pumped 循环：

```text
加载动态库并取得 EraRuntimeApi
  -> session_create
  -> submit(ClientHello)
  -> 重复 session_drive + session_poll，直到收到 ServerHello
  -> submit(ProjectManifest)
  -> drive/poll，处理 ProjectLoadReport
  -> submit(Start)
  -> 持续执行：
       session_drive
       session_poll（循环取空）
       渲染 PresentationSnapshot/Delta
       响应 ServiceRequest/StorageRequest
       提交 Input/AdvanceTime
  -> submit(ShutdownRequest)
  -> 收到 ShutdownReady
  -> session_destroy
```

调用 `session_submit` 只会校验并排队消息，不执行脚本。实际状态推进发生在
`session_drive` 中。前端必须主动调用 `session_drive`，并在每次驱动后调用
`session_poll` 直到返回 `ERA_STATUS_EMPTY`。

同一 session 的消息提交和驱动应由一个串行执行器负责。即使前端使用多个线程，
也必须保证入站 `sequence` 的顺序唯一且连续。

## 2. 动态库与 ABI 协商

ABI 当前版本为 `3.0`：

```c
#define ERA_RUNTIME_ABI_MAJOR 2u
#define ERA_RUNTIME_ABI_MINOR 1u
```

动态库只要求导出一个固定符号：

```c
EraStatus era_runtime_get_api(EraAbiVersion requested, EraRuntimeApi *out_api);
```

### `era_runtime_get_api`

用途：取得所有后续操作的函数表。

参数：

| 参数 | 类型 | 含义 |
| --- | --- | --- |
| `requested` | `EraAbiVersion` | 前端请求的 ABI 版本。主版本必须匹配。 |
| `out_api` | `EraRuntimeApi *` | 前端分配的可写函数表空间，不能为空。 |

返回值：

- `ERA_STATUS_OK`：`out_api` 已完整初始化。
- `ERA_STATUS_ABI_MISMATCH`：主版本不匹配，或 `out_api` 为空。
- `ERA_STATUS_INTERNAL_ERROR`：跨 ABI 边界捕获到内部异常。

成功后应检查：

- `out_api->struct_size` 是否足以覆盖前端需要访问的字段；
- `out_api->abi_version`；
- 对应函数指针是否非空；
- `implementation_name` 仅用于日志和诊断，不应用作功能判断。

`implementation_context` 和全部 `reserved` 字段目前必须忽略。

## 3. 通用 ABI 类型和约定

### `EraCallHeader`

所有版本化参数结构都以该头部开始：

| 字段 | 含义 |
| --- | --- |
| `struct_size` | 调用方实际提供的结构体字节数。 |
| `abi_version` | 调用方使用的 ABI 版本。 |

前端必须将未使用字段和 `reserved` 字段清零。不要假定 Rust 结构体内存布局；仅使用
C 头文件中的 `repr(C)` 投影。

当前各函数首个 `header` 参数应按下表填写：

| 函数 | `struct_size` |
| --- | --- |
| `session_create` | `sizeof(EraCreateOptions)`，通常直接传 `options.header` |
| `session_submit` | `sizeof(EraCallHeader)` |
| `session_drive` | `sizeof(EraDriveOptions)`，通常直接传 `options.header` |
| `session_poll` | `sizeof(EraOwnedBuffer)` |
| `session_destroy` | `sizeof(EraCallHeader)` |
| `release_buffer` | `sizeof(EraOwnedBuffer)` |
| `last_error` | `sizeof(EraOwnedBuffer)` |

### `EraSessionHandle`

`value == 0` 表示无效句柄。句柄只能传回创建它的动态库实例；`session_destroy`
成功后立即失效，不得复用。

### `EraByteSlice`

这是前端拥有的只读输入：

- `data` 指向编码后的完整信封；
- `len` 是字节数；
- `len > 0` 时 `data` 不能为空；
- runtime 在函数返回前完成读取，前端可在返回后立即释放原缓冲区。

### `EraOwnedBuffer`

这是 runtime 拥有的输出：

| 字段 | 含义 |
| --- | --- |
| `data` | 输出字节首地址。 |
| `len` | 输出字节数。 |
| `token` | 分配标识；前端不得解释或修改。 |

前端可复制或立即解码 `data[0..len]`，随后必须将原结构体逐字段不变地传给
`release_buffer`。每个 buffer 只能释放一次；不能使用语言自身的 `free`、
`delete` 或其他 allocator 释放它。

### `EraStatus`

| 状态 | 含义 |
| --- | --- |
| `ERA_STATUS_OK` | 调用成功。业务操作仍可能在输出消息中被拒绝。 |
| `ERA_STATUS_EMPTY` | `session_poll` 暂无输出。 |
| `ERA_STATUS_BUSY` | 接口预留的忙状态；当前前端不应依赖一定会收到它。 |
| `ERA_STATUS_INVALID_ARGUMENT` | 空指针、结构大小、输入信封或参数值无效。 |
| `ERA_STATUS_ABI_MISMATCH` | ABI 主版本或版本化结构 header 不兼容。 |
| `ERA_STATUS_INVALID_HANDLE` | session 或 buffer token 不存在、已销毁或已释放。 |
| `ERA_STATUS_RESOURCE_LIMIT` | 接口预留的资源上限状态；具体业务也可能通过错误消息报告。 |
| `ERA_STATUS_INTERNAL_ERROR` | Runtime 内部错误或跨 ABI 捕获到异常。 |

状态枚举使用固定的 `uint32_t` 表示。前端必须保留未知值的错误处理分支，不能假定
未来版本永远只有上表成员。

## 4. `EraRuntimeApi` 函数

### `session_create`

```c
EraStatus session_create(
    EraCallHeader header,
    const EraCreateOptions *options,
    EraSessionHandle *out_handle);
```

用途：创建一个隔离的 runtime session。

参数：

| 参数 | 含义 |
| --- | --- |
| `header` | 使用 `options->header`。 |
| `options` | 创建选项，不能为空。 |
| `options->debug_scope_mask` | 创建者允许的调试权限上限。使用头文件中的 `ERA_DEBUG_SCOPE_*` 位；未知位会被拒绝。调试客户端只能申请其子集。 |
| `options->reserved` | 必须全部为 `0`。 |
| `out_handle` | 成功时写入非零句柄，不能为空。 |

返回值：

- `OK`：创建成功。
- `INVALID_ARGUMENT`：指针、结构大小、保留字段或调试权限掩码包含未知位。
- `ABI_MISMATCH`：`options->header` 的 ABI 不兼容。
- `INTERNAL_ERROR`：内部异常。

创建成功后 session 处于 `Negotiating`，第一条入站消息必须是 `ClientHello`。

### `session_submit`

```c
EraStatus session_submit(
    EraCallHeader header,
    EraSessionHandle handle,
    EraByteSlice input);
```

用途：提交一个完整的 runtime CBOR 信封。

参数：

| 参数 | 含义 |
| --- | --- |
| `header` | `sizeof(EraCallHeader)` 和当前 ABI 版本。 |
| `handle` | 有效 session 句柄。 |
| `input` | `encode_envelope` 产生的完整字节序列，不是裸消息 payload。 |

返回值：

- `OK`：信封校验成功并进入入站队列。
- `INVALID_ARGUMENT`：CBOR、版本、channel、session、tag、大小或 sequence 无效。
- `INVALID_HANDLE`：句柄不存在或已销毁。
- `INTERNAL_ERROR`：内部异常。

失败详情通过 `last_error` 获取。`OK` 不表示消息对应的业务操作已经成功；业务拒绝
通过后续的 `CommandRejected` 或 `Fault` 输出。

### `session_drive`

```c
EraStatus session_drive(
    EraCallHeader header,
    EraSessionHandle handle,
    const EraDriveOptions *options,
    EraDriveResult *out_result);
```

用途：在确定性预算内处理入站消息和 VM 指令。

`EraDriveOptions`：

| 字段 | 默认值 | 含义 |
| --- | ---: | --- |
| `maximum_vm_instructions` | `100000` | 本次调用最多执行的 VM 指令数。 |
| `maximum_runtime_transitions` | `1024` | 本次调用最多处理的 runtime 状态转换数。实现至少允许一次转换。 |
| `reserved` | `0` | 必须为零。 |

`EraDriveResult`：

| 字段 | 含义 |
| --- | --- |
| `state` | 本次驱动后的调度建议。 |
| `vm_instructions` | 实际执行的 VM 指令数。 |
| `runtime_transitions` | 实际处理的 runtime 转换数。 |
| `queued_envelopes` | 当前等待 `session_poll` 的输出数量。 |

`state` 取值：

- `ERA_DRIVE_IDLE`：当前没有可立即执行的工作，可能正在等待前端输入或服务响应。
- `ERA_DRIVE_MORE_WORK`：预算耗尽但仍有工作，前端应尽快再次驱动。
- `ERA_DRIVE_OUTPUT_READY`：有输出待取；先循环调用 `session_poll`。
- `ERA_DRIVE_STOPPED`：runtime 已停止，可在取完输出后销毁 session。
- `ERA_DRIVE_FAULTED`：runtime 已进入故障状态；先读取 `Fault` 等输出。

函数返回 `OK` 表示驱动调用本身完成；`state == FAULTED` 是正常报告的业务状态，不等同
于 C ABI 返回 `INTERNAL_ERROR`。

### `session_poll`

```c
EraStatus session_poll(
    EraCallHeader header,
    EraSessionHandle handle,
    EraOwnedBuffer *out_buffer);
```

用途：按生成顺序取出一个 runtime 输出信封。

返回值：

- `OK`：`out_buffer` 已填充，使用后必须 `release_buffer`。
- `EMPTY`：当前没有输出，`out_buffer` 不应读取。
- `INVALID_ARGUMENT`：header 或输出指针无效。
- `INVALID_HANDLE`：句柄无效。

### `session_destroy`

```c
EraStatus session_destroy(
    EraCallHeader header,
    EraSessionHandle handle);
```

用途：立即销毁 session。该操作不会隐式执行优雅关闭，也不会返回剩余输出。

参数：`header` 使用 `sizeof(EraCallHeader)`；`handle` 必须是尚未销毁的 session。

建议先提交 `ShutdownRequest`，驱动到收到 `ShutdownReady`，取空输出后再销毁。紧急
退出时可以直接销毁，但前端应视所有未完成服务和未取消息为已丢弃。

返回 `OK` 或 `INVALID_HANDLE`。

### `release_buffer`

```c
EraStatus release_buffer(
    EraCallHeader header,
    EraOwnedBuffer buffer);
```

用途：释放 `session_poll` 或 `last_error` 返回的 runtime buffer。

参数必须是原样的 `EraOwnedBuffer`。成功返回 `OK`；token 未知、已释放或被修改返回
`INVALID_ARGUMENT`。

### `last_error`

```c
EraStatus last_error(
    EraCallHeader header,
    EraSessionHandle handle,
    EraOwnedBuffer *out_buffer);
```

用途：取得指定 session 最近一次 C ABI 层错误的 UTF-8 文本。

参数：`header` 使用 `sizeof(EraOwnedBuffer)`，`handle` 必须有效，`out_buffer` 不能为空。
成功返回 `OK` 和一个 `EraOwnedBuffer`；句柄无效返回 `INVALID_HANDLE`，参数无效返回
`INVALID_ARGUMENT`。内容可能为空。该文本用于日志，不是稳定的机器可读错误码。
前端逻辑应优先使用 `EraStatus`、`CommandRejected.code` 和 `Fault.code`。

## 5. Wire envelope

C ABI 传输的是 `era_protocol::Envelope` 的确定性 CBOR 编码，而非 JSON、Rust 内存
布局或单独的 `RuntimeMessage`。

| 字段 | 含义 |
| --- | --- |
| `wire_version` | 公共信封版本，当前为 `2.0`。 |
| `channel_version` | `Runtime` channel 当前为 `19.0`；`Debug` channel 当前为 `4.0`。 |
| `channel` | 正常运行必须为 `Runtime`；调试使用独立 `Debug` channel。 |
| `session` | 首次 `ClientHello` 可为空；握手成功后必须等于 `ServerHello.session`。 |
| `session_epoch` | 首次握手可为空；之后必须等于当前时间线 epoch。新游戏、恢复或热替换提交后旧 epoch 消息失效。 |
| `sequence` | 每个 channel、每个方向独立，从 `0` 开始并严格连续递增。 |
| `message_id` | 非零消息标识，在该方向保持唯一。 |
| `correlation_id` | 响应关联的请求 `message_id`，无关联事件为空。 |
| `payload_tag` | `RuntimeMessage` 的稳定数字标签。 |
| `payload` | 对完整 `RuntimeMessage` 再次进行 CBOR 编码得到的字节串。 |

前端应使用 `era_protocol::encode_envelope`/`decode_envelope` 和
`RuntimeMessage::encode_payload`/`from_envelope`，或在其他语言中严格实现相同的
canonical CBOR。不要把 Serde JSON 投影作为 wire 数据发送。

默认限制为：完整信封 16 MiB、payload 15 MiB、1024 个待处理请求、4096 条 journal、
单次最多 100000 条 VM 指令。握手结果取前端请求与 runtime 上限的较小值。

## 6. 生命周期消息

### 握手

前端第一条消息：`ClientHello`（tag `0`）。

| 字段 | 含义 |
| --- | --- |
| `runtime_versions` | 前端接受的 runtime protocol 版本区间。当前应包含 `19.0`。 |
| `client_name` | 用于诊断的前端名称。 |
| `features` | 前端能够处理的功能集合。 |
| `requested_limits` | 希望采用的资源限制。 |
| `capabilities` | 输入模态、富文本、HTML、图形、音视频、字体度量能力，以及握手后冻结的 `available_fonts` 字体族名称列表。 |

Runtime 返回：

- `ServerHello`（tag `1`）：选择的版本、session ID、可用功能和最终限制；
- `VersionRejected`（tag `2`）：版本区间不重叠。

当前前端只能依赖 `ServerHello.features` 中实际出现的功能。类型系统中定义但未协商的
`ProjectReload`、`TraditionalSave`、`VmSnapshot`、`Storage`、`Html`、`Graphics`、`Audio`、
`MouseInput` 等功能必须视为不可用。

### 项目加载

前端提交 `ProjectManifest`（tag `10`）：

| 字段 | 含义 |
| --- | --- |
| `project_revision` | 前端生成的项目版本，项目内容变化时递增。 |
| `files` | 文件路径、类别及前端读取结果。Runtime 不自行读文件。 |

每个 `SubmittedFile`：

- `relative_path`：以项目根为基准；不能是绝对路径、盘符路径或包含 `..`；
- `category`：`Csv`、`Erh`、`Erb`、`ResourceManifest`、`Resource` 或 `Configuration`；
- `payload`：UTF-8 字符串、原始 bytes，或前端观察到的 `IoError`；
- `content_hash`：可选的原始 32 字节 BLAKE3 digest，不是十六进制文本。

CSV、ERH 和 ERB 必须提交 UTF-8 文本。源码位置统一为 UTF-8 byte offset。

Runtime 返回 `ProjectLoadReport`（tag `11`）：原 revision、`success` 和按确定性顺序排列
的诊断。诊断包含稳定 `code`、严重度、文本以及可选的相对路径和 byte span。

`ReloadProject`（tag `12`）提交基于当前 revision 的规范化增量。Runtime 在隔离候选
状态中完成加载、分析、增量编译和验证，再通过 VM 多代热替换原子提交；失败保持当前
artifact 和运行状态。没有协商 `ProjectReload` 时不得发送。

Protocol 16.0 的 `ProjectAnalysisRequest`（tag `13`）是一次性分析操作。前端必须协商
`ProjectAnalysis`，并在首次加载前的 `Negotiating` 或已加载的 `Ready` 发送完整 manifest、可选 ERB 路径
列表和 debug mode。空列表表示分析全部 ERB；ERH 始终参与。Runtime 返回
`ProjectAnalysisReport`（tag `14`），不编译、不创建 VM，也不替换已加载项目。

协商 `KeyMacros` 后，前端以 `KeyMacroProfileSubmit`（tag `15`）提交 `macro.txt` 内容或
I/O 错误，通过 `KeyMacroCommand`（tag `16`）选择组、保存或清除槽位，并投影
`KeyMacroStateChanged`（tag `17`）。物理快捷键映射由前端负责，但宏内容、组和展开由
Runtime 持有；`ActivateKeyMacro` 只把槽位内容召回规范 textbox，`CommitText` 才提交。
若同时协商 Storage，编辑会产生 Project namespace 的 `macro.txt` 写请求。

可移植扩展须在首次项目加载前通过 `ExtensionRegistrySubmit`（tag `18`）声明，并协商
`ExternalServices`。声明包括稳定 ID、EraBasic 名称、参数/可变性、返回类型和 Host
operation 版本。调用以 `ServiceKind::Extension` 发送；前端返回 typed value 和按参数
序号的写回列表。Runtime 在任何写入前统一验证类型、序号和可变性。CLR `CALLSHARP`
不会加载。

### 启动

加载成功进入 `Ready` 后，前端提交 `Start`（tag `20`）：

- `NewGame { seed: Some(u64) }`：使用前端提供的确定性 seed；
- `NewGame { seed: None }`：runtime 会发出 `Entropy/random_seed` 服务请求；
- `TraditionalSave` 和 `VmSnapshot`：仅在相应功能已协商时发送。

传统存档与 VM Snapshot 不再内联到 `Start`。前端先发送 `StateImportBegin`（tag `62`），
按 Runtime 返回的 transfer ID 连续发送 `StateImportChunk`（tag `64`），再发送
`StateImportCommit`（tag `65`）。收到 `StateImportReady`（tag `66`）后，`Start` 只携带
该 transfer ID，且成功启动会一次性消费它。声明包含总字节数和原始 32-byte BLAKE3
摘要；乱序、缺块、超限或摘要不符都会被拒绝。

导出由 `StateExportRequest`（tag `60`）开始。`StateExportReady`（tag `61`）返回长度、
摘要和 transfer ID，前端随后使用 `StateExportChunkRequest`（tag `67`）顺序读取
`StateExportChunk`（tag `68`）。任一方向都可用 `StateTransferCancel`（tag `69`）释放；
同一会话每个方向最多一个活动 transfer。

Runtime 使用 `StateChanged`（tag `21`）报告 `Negotiating`、`LoadingProject`、`Ready`、
`Starting`、`Running`、`WaitingInput`、`WaitingExternal`、`DebugPaused`、`Reloading`、`Stopping`、`Stopped` 或
`Faulted`；`revision` 在每次 phase 变化时递增，消息同时报告当前 epoch。

### 关闭

前端发送 `ShutdownRequest`（tag `90`），其中 `graceful` 表示调用方意图。Runtime
返回 `ShutdownReady`（tag `91`），包含最终 runtime revision 和被取消的待处理操作数。

## 7. 展示模型

Runtime 是展示语义状态的权威持有者；前端只负责渲染投影。

- `PresentationSnapshot`（tag `40`）是完整状态，可直接替换前端缓存；
- `PresentationDelta`（tag `41`）要求前端当前 revision 等于 `base_revision`，然后按顺序
  应用 operations 并更新为 `new_revision`；
- revision 不匹配时不要猜测或部分应用，应请求 `Resynchronize`（tag `94`）。

Snapshot 包含标题、行、背景、tooltip 策略、逻辑音频状态、当前输入等待、全局展示设置
以及 `ResourceReplay`。后者提供 Runtime 已解析的 sprite 定义、动态 sprite 与 canvas
command graph，以及 `SETANIMETIMER` 选定的 `animation_timer_ms` 重绘间隔；前端按此重放
和调度绘制，但仍自行解码像素并持有 renderer 对象，且不得用该计时器推进 Runtime 的
逻辑时间。

`DisplayRun` 支持文本、嵌套按钮、HTML、图片、形状、`ColumnCell` 和 `Separator`。
`ColumnCell` 保存 PRINTC/PRINTLC 的对齐和建议列数，`Separator` 保存 DRAWLINE 的
分隔意图；前端可使用 grid、flex、TUI pattern 或无障碍结构投影，但不得把字体测量
结果写回权威游戏状态。按钮携带 opaque `InteractionToken` 和仅供规范化 HTML
序列化的 EraBasic 原始值；前端只能回传 token，不能用原始值绕过交互鉴权。尺寸使用
互不混用的强类型整数单位：

- `LogicalLength`：Era 逻辑空间中的 1/1000 logical unit，不是设备像素；
- `PresentationLength`：绝对逻辑长度或字体高度百分比；
- `ProjectionLength`：权威前端的设备无关布局单位，例如 CSS pixel；
- canvas point/rectangle：runtime canvas 的整数 texel；
- `font_millipoints`：1/1000 point；
- `volume_millionths`：百万分比；背景透明度使用 `RationalOpacity` 精确保留脚本值/255。

前端不得根据本地字体重新推导脚本可观察的语义值；需要字体或资源信息时，应通过
对应 `ServiceRequest` 返回版本化结果。

这里的“不得写回”只针对普通展示投影。若 EraBasic 命令本身明确查询实际 viewport、
字体测量、物理折行、显示历史或 raster 结果，该值属于前端观测，而不是 Runtime
规范化展示状态。Protocol 19.0 会向当前 session 唯一的权威投影前端发送携带 presentation
revision 和前端环境 revision 的 typed service request；前端必须先应用到指定 revision，
再返回其实际投影引擎的结果。Runtime 只负责关联、版本、revision、类型和范围验证，
不能用逻辑列宽或默认字体伪造回退值。当前协议不支持多客户端或 authority transfer。

`ProjectionObservation` 包含 environment、presentation、projection-space revision、
设备无关 viewport、逻辑列数、textbox 和逻辑到投影空间的有理数 affine transform。
物理历史、HTML 排版、字体尺寸和 canvas pixel 使用逐命令 typed service，并回显全部
适用 revision；canvas 查询还绑定 canvas replay revision。前端可延迟响应直到指定
replay 已应用，但不得返回其他 revision 的缓存值。

Protocol 19.0 不再发送 opaque HTML 字符串：`HtmlDocument` 是固定方言的语义树，节点
保留 UTF-8 byte span，并包含归一化 MixedNum、box model、颜色和布局；`button` 节点携带
runtime 分配的 token、generation 和 enabled。前端不得再次解析 attribute 字符串决定语义。`PRINT_IMG`、
`PRINT_RECT`、`PRINT_SPACE` 的可选资源和 MixedNum 尺寸同样保存在规范化 run 中。
前端负责把这些语义投影为本地布局，不得重新解释按钮 value 或自行生成 token。
Protocol 19.0 与开发期 Protocol 18.0 不兼容，前端 Schema 与 C 绑定必须同步更新。

`PresentationHistory.operations` 是物理历史的唯一有序输入：前端从空历史按顺序重放
append、delete-physical、replace-temporary、clear 和 button-generation，再按
`maximum_physical_lines` 裁剪已排版的物理行。`logical_lines` 是同一时刻供语义化展示、
无障碍和调试使用的规范化视图，不能在重放 operations 前再次作为物理初始行插入。
两者均已按握手能力投影。Runtime 不接收普通投影回写。`ProjectionState.text_box_layout` 使用
Era 逻辑坐标，前端负责变换及平台裁切；被接受的输入后必须应用 runtime 发出的默认布局。
`PresentationSnapshot.redraw.enabled` 只控制自动呈现，snapshot 仍须同步；`PresentNow` effect
要求立即呈现指定 revision 并按普通 effect 规则确认。

`TooltipSettings.format` 保留脚本传入的原始整数；`normalized_format.flags` 按固定顺序列出
参考 `TextFormatFlags` 的全部已知非零位，`unknown_bits` 保留尚未定义的位。前端不得把它
简化成普通/富文本三态；无法投影的 flag 可以忽略视觉效果，但不能改写规范化状态。

一次性音频、视频和动画设备动作通过 `EffectBatch`（tag `42`）发送，与可恢复展示
状态分离。前端必须为每个 `effect_id` 单独返回 `EffectAcknowledgement`（tag `43`）中的
`Completed`、`Failed` 或 `Cancelled` 结果；不能用累计前缀掩盖中间失败。失败只产生
`Diagnostic`（tag `97`），不会由前端反向修改 Runtime 已判定的游戏结果。

## 8. 输入与逻辑时间

Runtime 通过 `WaitChanged`（tag `32`）报告 wait 打开、更新或关闭，并在展示 snapshot
中携带当前 `input_wait`。收到 `Opened` 后，前端根据 `InputWait.kind` 选择控件。

关键字段：

| 字段 | 前端处理 |
| --- | --- |
| `wait_id` | 每次提交输入时原样带回。 |
| `submission_token` | 文本提交、继续或 primitive 输入时必须原样带回。 |
| `kind` | 决定 runtime 如何解释规范化输入意图。 |
| `stability` | `Transient` 表示当前状态不适合精确 VM snapshot。 |
| `one_input` | 一次输入模式提示；runtime 仍会权威执行单字符规范化。 |
| `stop_message_skip` | 需要终止消息跳过状态。 |
| `system_input` | Runtime 自身菜单输入。 |
| `mouse_input` | 允许鼠标相关输入。 |
| `default_value` | 超时或空输入时的默认语义值。 |
| `deadline_ns` | 前端单调时钟域中的绝对截止时间；为空表示无截止时间。 |
| `display_time` | 是否显示剩余时间。 |
| `timeout_message` | 超时提示文本。 |

前端提交 `Input`（tag `30`），参数包括 `wait_id`、交互 token、当前
`monotonic_time_ns` 和 `CommitText`、`Activate`、`Continue`、`Cancel` 或
`Primitive` 意图。前端负责设备/IME 编辑，runtime 负责整数、默认值和选项语义。
ID、epoch、token 或意图不匹配会收到
可恢复的 `CommandRejected(StaleRequest/InvalidValue)`。

在 `one_input` wait 中，前端不应自行决定最终字符串。runtime 将非空
`CommitText` 截取为第一个 Unicode scalar；空的非计时输入、timeout 和 message-skip
使用完整默认值。`Activate(token)` 表示用户激活 runtime 发布的语义化按钮，只有项目
启用 `AllowLongInputByMouse` 时才可保留多字符按钮值。这是参考 WinForms
`changedByMouse` 的跨设备投影，并不要求前端上报“鼠标点击”这一物理事实。
`Primitive` 仍由前端整理为 EraBasic 结果字段，供不透明平台输入函数使用。

即使没有用户输入，前端也必须按需要提交 `AdvanceTime`（tag `31`），让 runtime 推进
QTE/超时。Runtime 从不主动读取系统时钟。如果输入和超时发生在同一时刻，消息
`sequence` 决定处理顺序。

握手协商 `RuntimeFeature::InputUndo` 后，Runtime 会发送 `InputUndoStateChanged`
（tag `38`），其中包含是否启用、可撤销步数、是否正在回放以及单次有效 token。
前端可以把 Ctrl-Z、触摸手势或无障碍操作映射为 `InputUndoRequest`（tag `37`），但
不得发送原始平台键事件、读取存档槽或自行回放输入。Runtime 验证 token/epoch，恢复
其保留的传统存档和 RNG 状态，并重新执行输入轨迹。回放期间不能请求 VM snapshot；
热替换成功后旧撤销状态会失效。`RuntimeResynchronized` 同样携带完整撤销状态。

## 9. 外部服务

Runtime 需要操作系统能力时发送 `ServiceRequest`（tag `52`）：

| 字段 | 含义 |
| --- | --- |
| `request_id` | 服务关联 ID，响应必须原样返回。 |
| `kind` | 服务类别。 |
| `operation` | 稳定操作名。 |
| `operation_version` | 操作自身的版本。 |
| `payload` | 对该操作专用结构进行 canonical CBOR 编码后的 bytes。 |

前端返回 `ServiceResponse`（tag `53`），其 `result` 为：

- `Ready { payload }`：操作成功，payload 是对应响应结构的 CBOR；
- `Error { code, message }`：操作失败。`code` 应稳定、可记录，`message` 面向日志。

当前定义的核心操作：

| kind / operation | 请求 | 成功响应 |
| --- | --- | --- |
| `Entropy / random_seed` v1.0 | 空 `RandomSeedRequest` | `RandomSeedResponse { seed: u64 }` |
| `Clock / local_date_time` v1.0 | 空 `LocalDateTimeRequest` | 年、月、日、时、分、秒、毫秒、UTC offset 分钟 |
| `InputState / get_key_state` v1.0 | `key_code: u8` | `frontend_active`、`pressed`、`toggle_state` |
| `Image / image_metadata` v1.0 | 资源 ID 与内容摘要 | 宽、高、格式与动画标志 |
| `Image / image_pixel` v1.0 | 资源 ID、内容摘要与坐标 | ARGB 像素值 |
| `Network / update_check` v1.0 | 更新地址 | 远端版本和下载地址 |
| `OpenUrl / open_url` v1.0 | URL | 是否已交给平台打开 |
| `Extension / 声明的 operation/version` | `ExtensionInvocation` | `ExtensionResult`（返回值及可变参数写回） |

未来的字体测量、实际布局、物理显示历史和 canvas raster 查询也必须各自定义稳定的
operation、版本和专用 payload，并绑定 presentation/environment revision。未协商精确
operation 时必须返回不支持，不能使用 `Ready` 返回近似值。此类请求属于瞬态等待，
未完成时阻止稳定 VM snapshot 和热替换提交；成功响应应进入可重放的外部输入轨迹，
但物理布局本身不进入 Runtime 的规范化展示 snapshot。

服务响应也必须作为正常入站消息取得连续 `sequence`。未知或已完成的 `request_id`
会被视为 stale request。前端不能在处理 `ServiceRequest` 的同一 C 调用栈中回调
runtime；应先取出消息，再异步或同步完成平台工作，最后通过 `session_submit` 排队响应。

## 10. Storage 接口

Runtime 使用 `StorageRequest`（tag `50`）和 `StorageResponse`（tag `51`）完成
`Project`、`Save`、`GlobalSave`、`Data`、`Log`、`Resource` 命名空间中的读取、写入、
列举、元数据查询和删除。路径仍是相对路径。

写入参数包括 `atomic_replace`、`StoragePrecondition` 和 `idempotency_key`。前置条件为
`Any`（不检查）、`Missing`（仅当目标不存在）或 `Revision(value)`（仅当 revision
一致）。删除同样携带前置条件。前端应在重试时保持相同 idempotency key，避免重复
写入。前置条件不成立时返回 `FrontendIoErrorKind::Conflict`。`List` 还携带
`recursive`；`Stat` 只返回长度和 revision，不传输文件内容。
读取/写入/列表/元数据结果可以携带前端生成的 revision。

协议 9.0 通过 `RuntimeFeature::Storage` 协商该能力。只有握手协商且实际收到
`StorageRequest` 时前端才应执行 I/O；前端不得主动发送无对应 request ID 的
`StorageResponse`。

## 11. 错误处理

错误分为三层：

1. `EraStatus`：C ABI 参数、句柄和调用级错误。
2. `CommandRejected`（tag `95`）：单条命令语义无效，通常可恢复，不必终止 session。
3. `Fault`（tag `92`）：项目、VM、服务、资源限制或内部故障，runtime 进入 `Faulted`。

`CommandRejected` 包含稳定 `code`、说明、`recoverable` 和可选源码位置。`Fault` 包含
`FaultCode`、说明和可选源码位置。前端应显示或记录源码位置，但不能依赖本地化错误
文本执行逻辑。

当输出 sequence、展示 revision 或本地投影发生缺口时，前端可发送
`Resynchronize`。当前 runtime 返回 `RuntimeResynchronized`，其中包含 epoch、phase、
runtime revision 和完整 `PresentationSnapshot`；前端收到后替换本地展示缓存。

## 12. C 前端循环示意

以下代码省略 CBOR 编解码和动态库加载细节：

```c
EraRuntimeApi api = {0};
EraStatus status = era_runtime_get_api(
    (EraAbiVersion){ERA_RUNTIME_ABI_MAJOR, ERA_RUNTIME_ABI_MINOR},
    &api);
if (status != ERA_STATUS_OK) abort();

EraCreateOptions create = {0};
create.header.struct_size = sizeof(create);
create.header.abi_version = api.abi_version;

EraSessionHandle session = {0};
status = api.session_create(create.header, &create, &session);
if (status != ERA_STATUS_OK) abort();

/* submit_envelope_bytes 包含完整 CBOR Envelope。 */
EraCallHeader submit_header = {
    .struct_size = sizeof(EraCallHeader),
    .abi_version = api.abi_version,
};
api.session_submit(
    submit_header,
    session,
    (EraByteSlice){submit_envelope_bytes, submit_envelope_len});

for (;;) {
    EraDriveOptions drive = {0};
    drive.header.struct_size = sizeof(drive);
    drive.header.abi_version = api.abi_version;
    drive.maximum_vm_instructions = 100000;
    drive.maximum_runtime_transitions = 1024;

    EraDriveResult result = {0};
    status = api.session_drive(drive.header, session, &drive, &result);
    if (status != ERA_STATUS_OK) break;

    for (;;) {
        EraOwnedBuffer buffer = {0};
        EraCallHeader poll_header = {
            .struct_size = sizeof(EraOwnedBuffer),
            .abi_version = api.abi_version,
        };
        status = api.session_poll(poll_header, session, &buffer);
        if (status == ERA_STATUS_EMPTY) break;
        if (status != ERA_STATUS_OK) goto cleanup;

        decode_and_dispatch_envelope(buffer.data, buffer.len);
        api.release_buffer(poll_header, buffer);
    }

    if (result.state == ERA_DRIVE_STOPPED || result.state == ERA_DRIVE_FAULTED)
        break;
    if (result.state == ERA_DRIVE_IDLE)
        wait_for_frontend_event();
}

cleanup:
api.session_destroy(submit_header, session);
```

## 13. Rust 直接调用接口

Rust 前端或测试工具可以直接依赖 `era-runtime`，绕过 C 函数表；消息格式和调用顺序
仍与 C ABI 完全相同。应用前端若需要未来切换到 C/S transport，建议仍把 CBOR 信封
作为内部边界，而不要直接依赖 runtime 私有状态。

### `RuntimeOptions`

| 字段 | 类型 | 默认值/用途 |
| --- | --- | --- |
| `session_id` | `SessionId` | 默认 `{ high: 0, low: 1 }`；嵌入方应为并存 session 分配唯一 ID。 |
| `limits` | `RuntimeLimits` | 协议资源上限；握手时与前端请求取较小值。 |
| `wire_limits` | `WireLimits` | CBOR 信封与 payload 的解码前字节限制。 |
| `vm_config` | `VmConfig` | fiber、调用深度、操作数栈、热重载代数、watchdog 和 snapshot 大小限制。 |

### `RuntimeSession::new(options)`

用途：创建处于 `RuntimePhase::Negotiating` 的 session。

返回：新的 `RuntimeSession`。该类型是单一所有者 actor；调用方应通过可变引用串行
访问，不要把内部状态复制到前端作为第二权威来源。

### `submit_envelope(bytes)`

参数：包含完整 canonical CBOR `Envelope` 的字节切片。

返回：`Result<(), RuntimeError>`。

可能错误：

- `Protocol`：大小、CBOR、wire/channel 版本或消息 tag 不合法；
- `InvalidSequence { expected, actual }`：入站 sequence 不连续；
- `SessionMismatch`：握手后的 session ID 不一致；
- `ResourceLimit`：入站 journal 已满。

该方法只排队，不运行 VM。

### `drive(budget)`

`RuntimeDriveBudget` 参数：

- `maximum_vm_instructions`：本次最多执行的 VM 指令；
- `maximum_runtime_transitions`：本次最多处理的 actor 转换。

返回 `Result<RuntimeDriveReport, RuntimeError>`。Report 的 `state`、指令数、转换数和
待取信封数与 C ABI 的 `EraDriveResult` 含义相同。`RuntimeError::Internal` 表示 VM 或
runtime 不变量被破坏。

### `poll_envelope()`

返回 `Option<Vec<u8>>`：`Some` 是一个完整输出信封，`None` 表示队列为空。Rust 调用方
直接拥有返回的 `Vec`，无需调用 C ABI 的 `release_buffer`。

### 状态只读方法

- `phase() -> RuntimePhase`：当前生命周期阶段；
- `random_seed() -> Option<u64>`：新游戏 seed 尚未确定时为 `None`，确定后为 `Some`。

前端仍应以输出的 `StateChanged` 为跨 transport 的公共状态来源；这些方法主要用于
同进程 Rust 集成和测试。

## 14. 前端实现检查清单

- 使用动态符号查找取得 `era_runtime_get_api`，不直接链接内部 Rust 符号。
- 所有结构体清零并正确填写 `struct_size`、ABI version 和 reserved 字段。
- 每个方向的 sequence 从 0 开始且严格递增；message ID 非零。
- 握手后保存并在所有入站信封中携带 `ServerHello.session`。
- 只使用 `ServerHello.features` 中协商成功的功能。
- `session_poll` 返回的每个 buffer 都恰好释放一次。
- 文件 I/O、UTF-8 解码结果或 I/O error 都由前端放入 manifest。
- 使用 UTF-8 byte offset，不把 UTF-16 code-unit 当作源码偏移。
- 展示 delta 只应用到匹配的 revision，否则请求完整同步。
- 输入必须携带当前 wait ID、opaque interaction token 和单调时钟时间。
- 每个 Service/Storage response 都关联当前待处理 request ID。
- `EraStatus`、`CommandRejected.code`、`Fault.code` 用于机器逻辑，文本仅用于日志。
- 正常退出先等待 `ShutdownReady`，紧急退出才直接 destroy。
