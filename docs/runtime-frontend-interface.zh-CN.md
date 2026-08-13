# Runtime–前端接口

> 面向前端开发人员。本文描述当前源码，而不是规划中的能力。基线版本为
> C ABI `3.7`、公共信封 `2.0`、Runtime 协议 `30.0`。源码入口：
> [`era_runtime.h`](../crates/era-runtime-ffi/include/era_runtime.h)、
> [`era-runtime-capi`](../crates/era-runtime-capi/src/lib.rs)、
> [`era-protocol`](../crates/era-protocol/src/lib.rs)、
> [`era-runtime-protocol`](../crates/era-runtime-protocol/src/lib.rs) 和
> [`RuntimeSession`](../crates/era-runtime/src/session.rs)。

## 1. 接口定位、稳定性和术语

前端只负责文件与平台 I/O、设备输入、展示投影和外部服务；runtime 创建并持有权威的
游戏、展示、交互、存档传输与协议状态。前端不能直接访问 VM，也不能把终端/浏览器的
物理布局回写为权威游戏状态，除非使用本文定义的 `ProjectionObservation`。

接口分为三层：

| 层 | 当前稳定性 | 用途 |
| --- | --- | --- |
| C ABI 3.7 | 公开、版本化，但开发期默认不保证向后兼容 | 动态库发现、session 和字节缓冲区所有权 |
| 公共信封 2.0 | 公开、版本化 | Runtime 与 Debug 共用的确定性 CBOR 封装 |
| Runtime 协议 30.0 | 公开、版本化，但开发期默认不保证向后兼容 | 生命周期、输入、展示、日志、I/O 和状态传输 |
| `RuntimeSession` Rust API | 内部接口 | Rust 侧测试和嵌入；可随 runtime/VM 同步改变 |

破坏性变更必须提升相应版本，并同步 Schema、C 头、文档与测试。数字消息标记已经是
线标识，退役后也不得复用。主版本不兼容；次版本只能加入旧端可忽略的可选字段或经
能力协商启用的行为。

本文统一使用：

- **session**：一次 C ABI `session_create` 到 `session_destroy` 的对象；
- **epoch**：session 内一条权威游戏时间线；新游戏、恢复和提交代码替换会推进；
- **revision**：某类 runtime 状态的单调修订号，不等同于 epoch；
- **token**：runtime 发放、前端原样返回的 epoch 限定能力；
- **drive**：调用方给定预算，让 caller-pumped runtime 向前执行；
- **可空**：Rust `Option<T>`；CBOR map 编码时 `None` 字段缺席；
- **字节**：`ProtocolBytes`，CBOR byte string；文本始终为 UTF-8；
- **时间**：`*_ns` 是前端单调时钟纳秒，`*_ms` 是毫秒。

除非另有说明，下文结构字段按列出顺序使用 CBOR map key `0,1,2...`；enum 使用源码
`#[n(...)]` 给出的整数 tag。`Option`/`?` 字段可以缺席，其他字段没有隐式默认值，缺失会
解码失败。Rust `Default` 只在明确注明时适用，不会替发送方补齐线上字段。Serde 的
snake_case JSON 表示仅供测试/日志，C ABI 实际只传确定性 CBOR。

## 2. 职责、所有权、线程与生命周期

```text
前端线程/worker
  │ create ─ submit(CBOR) ─ drive(预算) ─ poll(CBOR) ─ release_buffer
  ▼
C ABI 全局 session 注册表（Mutex；session 操作串行化）
  ▼
RuntimeSession（唯一可变所有者；无回调、无内部事件循环）
  ├─ Runtime/Debug 入站队列
  ├─ VM、系统流程和权威 PresentationModel
  ├─ 外部请求、effect、输出 journal、状态传输
  └─ 可取消并在 Drop 时 join 的编译缓存准备线程
```

- `session_create` 创建状态；`session_submit` 在返回前复制并解码输入；调用方随后可释放
  输入字节。
- `drive` 是唯一推进协议和 VM 的入口。一次调用受指令数与 runtime transition 数限制，
  先处理已排队消息，再执行可运行 VM；它不会调用前端函数。
- `poll` 把一条已编码输出移交为 `EraOwnedBuffer`。缓冲区仍由动态库拥有，前端读完后
  必须且只能调用一次 `release_buffer`。
- `session_destroy` 从注册表移除对象并释放全部状态；后台缓存任务会被取消并 join。
- 当前实现用进程级互斥锁保护注册表，session 操作（包括一次 `drive`）会被串行化。
  3.2/3.3 项目文件投影扩展只在检查 handle、记录错误及登记输出 buffer 时持锁，耗时的
  校验、解压和 CBOR 编码在锁外执行；期间销毁 session 会令调用最终返回
  `INVALID_HANDLE`。这不是高并发承诺；不要从回调重入，建议每个 session 固定在一个
  worker。
- session handle、interaction token、request ID、transfer ID、effect ID 都是不透明值；
  不要推导、跨 session/epoch 缓存或自行生成。

推荐生命周期：

```text
get_api → create → ClientHello → ServerHello
                     │
                     ├→ ProjectManifest/ProjectLoad → ProjectLoadReport → Start
                     │
                     └→ 反复 submit → drive → poll → 处理/响应/ACK
                                                   │
                                  ShutdownRequest → ShutdownReady
                                                   │
                                                destroy
```

## 3. C ABI 3.7

### 3.1 数据结构

所有结构均为 C 布局；未写入的 `reserved` 必须置零。`EraCallHeader` 字段是
`struct_size: uint32_t` 和 `abi_version: { major: uint16_t, minor: uint16_t }`。

| 类型 | 字段及含义 | 所有权/约束 |
| --- | --- | --- |
| `EraSessionHandle` | `value: uint64_t` | runtime 发放；零值不是有效 session |
| `EraByteSlice` | `data: const uint8_t*`, `len: size_t` | 借用到调用返回；`len>0` 时指针必须非空 |
| `EraOwnedBuffer` | `data: uint8_t*`, `len: size_t`, `token: uint64_t` | runtime 拥有；成功后必须整体原样传给 `release_buffer` |
| `EraCreateOptions` | `header`; `debug_scope_mask: uint64_t`; `reserved[4]` | mask 是调试权限上限；未知位拒绝 |
| `EraDriveOptions` | `header`; `maximum_vm_instructions: uint64_t`; `maximum_runtime_transitions: uint32_t`; `reserved` | 指令预算可为零；transition 预算 0 在当前实现中提升为 1 |
| `EraDriveResult` | `header`; `state`; `vm_instructions`; `runtime_transitions`; `queued_envelopes` | runtime 填写；计数只对应本次 drive |
| `EraRuntimeApi` | `struct_size`, `abi_version`, `implementation_name`, `implementation_context`, 七个函数指针, `reserved[8]` | 表和名称指针由动态库静态持有；库卸载后失效 |

`EraDriveState`：`IDLE=0` 无立即工作；`MORE_WORK=1` 预算耗尽仍可推进；
`OUTPUT_READY=2` 有待 poll 输出；`STOPPED=3` 正常终止；`FAULTED=4` 终止故障。
即使状态不是 `OUTPUT_READY`，也应依据 `queued_envelopes` 或直接 poll 到 `EMPTY`。

`EraStatus`：

| 值 | 语义 | 调用方处理 |
| --- | --- | --- |
| `OK=0` | 调用完成 | 继续 |
| `EMPTY=1` | `poll` 当前无消息 | 停止本轮 drain |
| `BUSY=2` | 暂时忙 | host 缓存暂存与另一个入站传输冲突时稍后重试 |
| `INVALID_ARGUMENT=3` | 空指针、无效 header/mask、坏 CBOR 或 session 级协议提交错误 | 查 `last_error`，修正调用；不要盲重试 |
| `ABI_MISMATCH=4` | ABI 主版本不同或结构太短 | 加载兼容动态库/绑定 |
| `INVALID_HANDLE=5` | handle 不存在、已销毁或属于别的进程 | 丢弃本地 session 状态 |
| `RESOURCE_LIMIT=6` | 资源限制 | host 缓存超过协商传输上限；协议内限制通常成为拒绝或故障 |
| `INTERNAL_ERROR=7` | drive/runtime 内部错误 | 读 `last_error`，将 session 视为不可继续或重新创建 |

### 3.2 函数索引和契约

```c
EraStatus era_runtime_get_api(EraAbiVersion requested, EraRuntimeApi *out_api);
EraStatus session_create(EraCallHeader, const EraCreateOptions *, EraSessionHandle *);
EraStatus session_submit(EraCallHeader, EraSessionHandle, EraByteSlice);
EraStatus session_drive(EraCallHeader, EraSessionHandle,
                        const EraDriveOptions *, EraDriveResult *);
EraStatus session_poll(EraCallHeader, EraSessionHandle, EraOwnedBuffer *);
EraStatus session_destroy(EraCallHeader, EraSessionHandle);
EraStatus release_buffer(EraCallHeader, EraOwnedBuffer);
EraStatus last_error(EraCallHeader, EraSessionHandle, EraOwnedBuffer *);
```

`era_runtime_get_api` 只接受主版本 3，要求 `out_api != NULL`，成功后填写完整表。
这两个条件任一不满足，当前都返回 `ABI_MISMATCH`。
先保持动态库已加载，再使用函数指针。`implementation_context` 当前为空，调用方不得解释。

其余函数都要求 header 主版本为 3，且 `struct_size` 至少为实现检查的结构大小：
create=`EraCreateOptions`，drive=`EraDriveOptions`，poll/release/last-error=
`EraOwnedBuffer`，submit/destroy=`EraCallHeader`。
`session_create` 的按值 header 与 `options.header` 都会验证；调试 mask 只能包含
`ERA_DEBUG_SCOPE_ALL`。成功后 session handle 非零。

`session_submit` 的字节必须是一条完整信封；成功只表示已接收、排序并排队，不表示语义
命令已经执行。重复的完全相同消息可能被识别为重放；变异重放或序号缺口同步失败。

`session_drive` 写入完整 `EraDriveResult`。即使 `maximum_vm_instructions=0`，仍可处理
入站/输出等 runtime transition；`maximum_runtime_transitions=0` 仍允许一个 transition。
语义错误通常编码成协议
`CommandRejected`，不是 C 错误；无法继续的 runtime 错误成为 `INTERNAL_ERROR` 或
输出 `RuntimeFault`。

`session_poll` 成功返回一个 buffer；`EMPTY` 时输出不可使用。`release_buffer` 不需要
有效 session，但 data、len 和 token 必须与全局 buffer 注册表中的记录完全一致。重复释放是
`INVALID_ARGUMENT`。`last_error` 也返回 owned UTF-8 buffer；没有错误时可为空串。

`session_destroy` 成功后 handle 立即失效；先释放仍持有的输出 buffer。当前 buffer 注册
独立于 session，但依赖这一细节延迟释放没有兼容性保证。

函数表的可选扩展占用以下 `reserved` 槽；调用方必须同时检查 ABI 次版本与非空指针：

- ABI 3.1 的 `reserved[0]` 是 `EraSessionSetProjectProgressFn`，注册只读项目进度回调；
- ABI 3.2 的 `reserved[1]` 是 `EraSessionDecodeProjectFileFn`，校验 `.reraproj` 并返回完整
  `ProjectManifest` 的确定性 CBOR；
- ABI 3.3 的 `reserved[2]` 使用相同函数签名，返回供前端 I/O 使用的紧凑 manifest。
  它保留资源文件、I/O 错误和被缓存诊断引用的源码 payload；其他 payload 被清空，但若
  原记录缺少 `content_hash`，清空前会先计算并保留哈希。因此紧凑投影仍可用于项目身份、
  前端资源和诊断展示；配置与其他权威状态由完整项目文件导入 runtime；
- ABI 3.4 的 `reserved[3]` 是 `EraSessionStageCompiledCacheFn`。它取得一份连续缓存字节的
  所有权副本并返回已提交的 transfer ID，供随后的权威 `ProjectLoad` 使用，从而避免同一
  进程内再构造、编码、解码和拼接分块传输信封。该入口不提交项目、不推进 runtime，也不
  绕过项目加载时的缓存版本、内置摘要、项目身份和字节码校验；
- ABI 3.5 的 `reserved[4]` 是 `EraSessionAllocateCompiledCacheFn`，按协商上限分配一个
  runtime-owned 可写 `EraOwnedBuffer`；`reserved[5]` 是 `EraSessionCommitCompiledCacheFn`，
  把已完整填充的同一 buffer 原样提交并返回 transfer ID。前端必须写满全部 `len` 字节，
  然后恰好选择一次 commit 或 `release_buffer`。结构有效的 commit 会消费 buffer，即使
  runtime 随后返回 `BUSY` 或其他错误，也不得再释放或访问它。该组合让文件 I/O 直接写入
  runtime 最终拥有的连续内存，避免 ABI 3.4 借用输入所需的整块复制；所有项目加载校验保持
  不变；
- ABI 3.6 的 `reserved[6]` 是 `EraPrepareProjectConfigurationUpdateFn`。它校验当前
  `.reraproj`、`reraconfig.toml` 的乐观锁摘要和新 TOML，返回一个 runtime-owned buffer：
  前 8 字节是小端 `u64` 截断位置，余下字节是应追加的紧凑配置事务记录。前端先把中断写入
  留下的不完整尾部截断到该位置，再追加余下字节并持久化；无需重新生成项目主体。完整记录
  带前置配置摘要、结果摘要和校验和，加载时按序验证。该入口不执行文件 I/O。

项目进度阶段的 C ABI 数值保持追加兼容：`SCANNING` 至 `VALIDATING` 为 0–6，ABI 3.3
追加的 `FINALIZING = 7` 表示函数编译完成后的缓存合并、源码映射整理、结构验证与身份计算；
`PREPARING = 8` 表示 Runtime 正在整理资源索引与计算项目身份。解析、finalizing 和 preparing
阶段都会按实际处理批次报告进度，前端不应仅依赖百分比变化判断活性。ABI 3.7 追加的
`PACKAGING = 9` 表示容器正在分段压缩与组装；后台内部缓存也可能报告该阶段，但前端不得
据此锁定游戏交互，交互锁只跟随用户主动导出全量项目文件的生命周期。

两个解码扩展都借用输入 slice 到调用返回，并以 `EraOwnedBuffer` 返回结果；调用方必须用
`release_buffer` 释放。坏文件或超过调用方上限的文件返回 `INVALID_ARGUMENT` 并写入
`last_error`。ABI 3.2 客户端应使用 `reserved[1]`；ABI 3.3 客户端优先使用紧凑槽，并可向
3.2 动态库回退到完整槽。

ABI 3.4 缓存暂存扩展只借用输入到调用返回，成功后 runtime 持有自己的连续副本。输出 transfer
ID 必须原样放入下一条 `ProjectLoad.compiled_cache_transfer_id`，不能跨 session 复用。未知
handle 返回 `INVALID_HANDLE`，已有入站传输返回 `BUSY`，超过协商上限返回
`RESOURCE_LIMIT`；详细文本由 `last_error` 提供。

ABI 3.5 的可写缓存 buffer 与分配它的 session 绑定，不能提交到其他 session；在 commit 前
仍由动态库登记，填充失败时必须把原始三元组交给 `release_buffer`。分配阶段的未知 handle、
超限和内存不足分别返回 `INVALID_HANDLE`、`RESOURCE_LIMIT`、`RESOURCE_LIMIT`。commit 的
handle、buffer 形状或用途不匹配时不会消费 buffer；一旦形状和用途验证成功则取得所有权，
其状态映射与 ABI 3.4 暂存入口一致。

ABI 3.6 的配置更新入口借用三个输入 slice（项目文件、预期配置摘要、UTF-8 TOML）到调用
返回，并用 `EraOwnedBuffer` 返回更新计划。空预期摘要表示项目内尚无 `reraconfig.toml`；
摘要不匹配、TOML 无效、记录链损坏或项目格式不受支持时返回 `INVALID_ARGUMENT`。调用方
必须释放输出 buffer，并在同一已打开文件上依次执行截断、追加和同步。

当前精确同步错误映射：create/drive 的外层 header 错误或空指针是
`INVALID_ARGUMENT`，其 options 内嵌 header 错误是 `ABI_MISMATCH`；submit 的 header
或非法 slice 是 `INVALID_ARGUMENT`；poll/last-error 的 header 或输出空指针是
`INVALID_ARGUMENT`；destroy/release 的 header 错误是 `ABI_MISMATCH`。所有需要 session
的函数对未知 handle 返回 `INVALID_HANDLE`。跨 FFI 捕获到 panic 时只返回
`INTERNAL_ERROR`，不保证能写入 session 的 `last_error`，调用方应记录函数名和输入摘要。

### 3.3 Rust 与 Python 的等价基本调用

Rust 嵌入端可绕过 C ABI，但该 API 是内部接口：

```rust
use era_runtime::{RuntimeDriveBudget, RuntimeOptions, RuntimeSession};

let mut runtime = RuntimeSession::new(RuntimeOptions::default());
runtime.submit_envelope(&encoded_client_hello)?;
loop {
    let report = runtime.drive(RuntimeDriveBudget::default())?;
    while let Some(bytes) = runtime.poll_envelope() {
        handle_one_envelope(bytes)?;
    }
    if matches!(report.state, era_runtime::RuntimeDriveState::Idle) {
        break;
    }
}
```

Python 前端应使用仓库内已检查的 ctypes 投影；它处理 buffer 的恰好一次释放：

```python
from pathlib import Path
from rustyera_tui.abi import AbiError, RuntimeAbi

try:
    with RuntimeAbi(Path("target/release/libera_runtime_capi.dylib"),
                    debug_scope_mask=0) as runtime:
        runtime.submit(encoded_client_hello)
        while True:
            report = runtime.drive()
            while (packet := runtime.poll()) is not None:
                handle_one_envelope(packet)
            if report.state == 0:  # ERA_DRIVE_IDLE
                break
except AbiError as error:
    print(f"C ABI 调用失败：{error}")
```

Linux 使用 `.so`，Windows 使用 `.dll`；也可设置 `ERA_RUNTIME_LIBRARY`。

### 3.4 内部 Rust session API

Rust 内嵌调用的完整主入口为：

```rust
pub fn RuntimeSession::new(options: RuntimeOptions) -> RuntimeSession
pub fn RuntimeSession::submit_envelope(&mut self, bytes: &[u8])
    -> Result<(), RuntimeError>
pub fn RuntimeSession::drive(&mut self, budget: RuntimeDriveBudget)
    -> Result<RuntimeDriveReport, RuntimeError>
pub fn RuntimeSession::poll_envelope(&mut self) -> Option<Vec<u8>>
pub const fn RuntimeSession::phase(&self) -> RuntimePhase
pub const fn RuntimeSession::random_seed(&self) -> Option<u64>
```

`RuntimeOptions` 字段是 `session_id`、`limits`、`wire_limits`、`vm_config`、
`debug_scope_mask`。默认 session 是 `{high:0,low:1}`，limits 见第 5 节，wire limits 是
128/127 MiB，VM config 见 Runtime–VM 文档，debug mask 为 0。所有 options 在构造时
按值复制；session 独占后续状态。

`RuntimeDriveBudget {maximum_vm_instructions,maximum_runtime_transitions}` 默认
100000/1024。`RuntimeDriveReport {state,vm_instructions,runtime_transitions,
queued_envelopes}`；state 与 C `EraDriveState` 一一对应。

同步 `RuntimeError` 是 `Protocol(ProtocolError)`、`InvalidSequence{expected,actual}`、
`SessionMismatch`、`ResourceLimit(&'static str)` 或 `Internal(String)`。它和线上的
`CommandRejected/RuntimeFault` 不同：前者表示本次 Rust API 没能正常接收/驱动，调用方
不能假设存在响应信封。

其余只读 getter：

```rust
pub fn project_revision(&self) -> Option<u64>
pub fn project_sorts_filenames(&self) -> Option<bool>
pub fn project_auto_save(&self) -> Option<bool>
pub fn project_save_slot_count(&self) -> Option<u32>
pub fn project_money_label(&self) -> Option<&str>
pub fn project_money_first(&self) -> Option<bool>
pub fn project_maximum_shop_items(&self) -> Option<u32>
```

项目未加载时均为 `None`；`money_label` 的借用只在下一次 `&mut self` 调用前有效。这些
getter 是内部诊断/系统流程便利接口，不在 C ABI，也不应取代版本化消息成为前端依赖。

## 4. 公共 CBOR 信封

线格式是确定性 CBOR：定长容器、最短整数、map key 的编码字节严格递增且不得重复、
UTF-8 文本、最大嵌套 128；禁止浮点和 indefinite-length。JSON 只用于日志投影，不是
传输格式。

`Envelope` 是数字键 map：

| 键 | 字段 | 类型/可空性 | 语义 |
| --- | --- | --- | --- |
| 0 | `wire_version` | `{0:major,1:minor}` | 当前 2.0；主版本必须匹配 |
| 1 | `channel_version` | 同上 | Runtime 24.0 或 Debug 4.0 |
| 2 | `channel` | `0 Runtime` / `1 Debug` | 决定序号空间和 payload 解码器 |
| 3 | `session` | 可空 `{0:high,1:low}` | 128 位 session 标识 |
| 4 | `sequence` | `u64` | 同方向、同 channel 从 0 严格递增 |
| 5 | `message_id` | 非零 `u64` | 跨 channel 共用的消息身份 |
| 6 | `correlation_id` | 可空 `u64` | 响应关联请求的 `message_id`；通知为空 |
| 7 | `payload_tag` | `u32` | 必须与 payload 内 enum 标记一致 |
| 8 | `payload` | byte string | 再次 CBOR 编码的 `RuntimeMessage`/`DebugMessage` |
| 9 | `session_epoch` | 可空 `u64` | 当前权威时间线 |

首条 Runtime `ClientHello` 必须是 Runtime sequence 0 和第一条语义消息。前端应发送
`session=None`、`epoch=None`；当前实现直到激活后才校验这两个字段，因此错误地带值的
首个 Hello 也可能被接受，见第 11 节。收到 `ServerHello` 后，两字段都必须精确匹配。
激活前禁止 Debug。Runtime 与 Debug 的入站序号分别计数，出站也分别计数；epoch 变化
不重置序号。

runtime 按 channel 保留近期已接受消息的 ID、序号和摘要：完全相同的重发可幂等接受；
同序号不同内容、跳号和陈旧未保留的重放都报 `InvalidSequence`。普通新游戏/重载推进
epoch 时会清理两个 channel 的接受 ID，但仍不重置 sequence；VM snapshot 恢复有一处
不一致，见第 11 节。

输出 Runtime 信封进入有界 journal。前端在完成本地处理后发
`Acknowledge { through_sequence }`（tag 93，累计确认）释放它；不确认最终会触发资源
限制。Debug 输出不进入该 ACK journal。断线/投影丢失时发 `Resynchronize`，并以返回的
完整聚合状态为新基线。

编码/解码错误分为 `EnvelopeTooLarge`、`PayloadTooLarge`、`InvalidCbor`、
`NonCanonicalCbor`、`VersionMismatch`、`ChannelMismatch`、`MessageTagMismatch` 和
`InvalidIdentifier`。C ABI 提交时这些错误表现为 `INVALID_ARGUMENT + last_error`；
前端应记录原始消息元数据，修复编码器或协商版本，而不是更换 message ID 重试坏数据。

## 5. 握手、能力和状态机

`ClientHello` 字段：

| 键 | 字段 | 类型、默认/合法值 |
| --- | --- | --- |
| 0 | `runtime_versions` | `{min,max}`，两端闭区间 |
| 1 | `client_name` | UTF-8，不作为身份授权 |
| 2 | `features` | `RuntimeFeature[]` 请求集合 |
| 3 | `requested_limits` | 六个非负限制字段 |
| 4 | `capabilities` | 下表；session 固定 |
| 5 | `preferred_locales` | 有序 BCP-47；当前选择 `zh-Hans`、`en` 或默认 `ja` |

`RuntimeFeature` 数值为：0 project reload、1 traditional save、2 VM snapshot、3 timed
input、4 rich text、5 HTML、6 graphics、7 audio、8 mouse、9 external services、10 state
resync、11 storage、12 input undo、13 project analysis、14 key macros。当前实际可协商的
实现集合是 0、1、2、3、9、10、11、12、13、14；4–8 虽有枚举，展示能力由
`ClientCapabilities` 单独选择。

`ClientCapabilities`：键 0 `input_modalities[]`（keyboard=0、mouse=1、touch=2、
gamepad=3）；1–8 依次为 `rich_text/html/graphics/audio/video/font_metrics/
column_cells/separators`；9 `available_fonts[]`（ServerHello 按大小写不敏感排序/去重并
保留选中的拼写；runtime 内部再小写化供 CHKFONT 查询）；
10 `services[]`；11 `StorageCapabilities`。当前 `video` 总被选为 false；
`font_metrics` 还要求 `gget_text_size` 服务。布尔值没有缺省，前端必须全部发送。

`RuntimeLimits` 键 0–5：`maximum_envelope_bytes:u64`、`maximum_payload_bytes:u64`、
`maximum_pending_requests:u32`、`maximum_journal_entries:u32`、
`maximum_drive_instructions:u64`、`maximum_transfer_bytes:u64`。服务端逐字段取请求值与
创建者上限的较小者。默认创建者限制分别是 128 MiB、127 MiB、1024、4096、100000、
1 GiB。

成功响应 `ServerHello`：0 选定版本、1 session、2 features、3 limits、4 epoch（初始
1）、5 selected capabilities、6 locale。无交集响应 `VersionRejected {supported,
message}`，不进入活动 session。

`RuntimePhase` 数值：0 Negotiating、1 LoadingProject、2 Ready、3 Starting、4 Running、
5 WaitingInput、6 WaitingExternal、7 DebugPaused、8 Reloading、9 Stopping、10 Stopped、
11 Faulted、12 AnalyzingProject。`StateChanged { phase, revision, epoch }` 是权威相位
通知；前端不要从按钮或输出猜测相位。

## 6. Runtime 消息目录

payload 使用 minicbor enum 形式 `[tag, [value]]`；无值变体为 `[tag, []]`。以下方向
是强约束，反向发送会得到 `CommandRejected(InvalidValue)`。

| tag | 前端 → runtime | 主要结果/效果 |
| --- | --- | --- |
| 0 | `ClientHello` | 1 `ServerHello` 或 2 `VersionRejected` |
| 10 | `ProjectManifest` | 11 `ProjectLoadReport`；兼容便捷入口 |
| 12 | `ReloadProject` | 构建候选、热替换或报告失败 |
| 13 | `ProjectAnalysisRequest` | 14 `ProjectAnalysisReport`；不替换活动项目 |
| 15 | `KeyMacroProfileSubmit` | 17 `KeyMacroStateChanged` |
| 16 | `KeyMacroCommand` | 17；必要时产生 storage 请求 |
| 18 | `ExtensionRegistrySubmit` | 冻结动态调用声明 |
| 19 | `ProjectLoad` | 可先命中 opaque 编译缓存，否则要求 manifest |
| 20 | `Start` | 新游戏或已提交的 save/snapshot |
| 23 | `ReturnToTitle` | 丢弃活动时间线但复用项目 |
| 30 | `Input` | 消费当前 wait/token |
| 31 | `AdvanceTime` | 推进 deadline/countdown |
| 33 | `DeviceStateChanged` | 更新设备采样时间 |
| 34 | `ClientStateChanged` | 更新焦点/音频等前端状态 |
| 35 | `ProjectionObservation` | 36 `ProjectionState` 或拒绝 |
| 37 | `InputUndoRequest` | 38 `InputUndoStateChanged` |
| 43 | `EffectAcknowledgement` | 完成已知 effect |
| 51 | `StorageResponse` | 完成 tag 50 请求 |
| 53 | `ServiceResponse` | 完成 tag 52 请求 |
| 60 | `StateExportRequest` | 61 `StateExportReady` |
| 62/64/65 | import begin/chunk/commit | 63 accepted、66 ready |
| 67 | `StateExportChunkRequest` | 68 chunk |
| 70 | `FullProjectManifest` | — |
| 71 | `StateExportCancel` | — |
| 69 | `StateTransferCancel` | 取消指定传输 |
| 90 | `ShutdownRequest` | 91 `ShutdownReady` |
| 93 | `Acknowledge` | 累计释放 Runtime 输出 journal |
| 94 | `Resynchronize` | 96 完整聚合状态，随后仍存续的 effects |

runtime → 前端还会主动发送：21 `StateChanged`、22 `ExitRequested`、32 `WaitChanged`、
36 `ProjectionState`、38 undo、40 `PresentationSnapshot`、41 `PresentationDelta`、
42 `EffectBatch`、50 `StorageRequest`、52 `ServiceRequest`、54
`CancelExternalRequest`、92 `Fault`、95 `CommandRejected`、97 `Diagnostic`、98 `Log`。

`RuntimeLog { level, message }` 的等级由 runtime 权威决定，依次为 Debug、Info、
Warning、Error。前端可以筛选、着色和添加到达时间，但不得根据正文、消息 tag 或状态
重新定级；前端自己产生的 I/O 或渲染日志不受此限制。

响应的 `correlation_id` 等于请求 `message_id`；状态/等待/故障等通知没有关联 ID。
收到未知 tag、未知主版本或方向错误时不要继续猜测格式。

## 7. 项目、输入与客户端状态

### 7.1 项目数据

| 类型 | 字段（按 CBOR 键顺序） | 约束 |
| --- | --- | --- |
| `FrontendIoError` | `kind`, `message`, `platform_code?` | kind：NotFound、PermissionDenied、InvalidData、Interrupted、ReadOnly、AlreadyExists、Other、Conflict |
| `SubmittedFile` | `relative_path`, `category`, `payload`, `content_hash?` | category：Csv/Erh/Erb/ResourceManifest/Resource/Configuration；hash 是可空 opaque bytes |
| `FilePayload` | `Utf8(String)` / `Bytes` / `IoError` | 源码直接 UTF-8；不要提交本地绝对路径 |
| `ProjectManifest` | `project_revision:u64`, `files[]` | 文件顺序是协议输入的一部分，runtime 内部做确定性处理 |
| `ProjectIdentity` | `project_revision`, `source_digest` | digest 由完整规范项目身份产生 |
| `ProjectLoadRequest` | `identity`, `manifest?`, `compiled_cache_transfer_id?` | cache key 不精确但其嵌入 manifest 身份匹配时直接重编译；缓存无效或源码身份不同且未带 manifest 时才报告 `payload_required=true` |
| `ProjectLoadReport` | `project_revision`, `success`, `diagnostics[]`, `payload_required`, `configuration?`, `game_information?` | `success=false` 时不要 Start；`game_information` 是从已解析 `GameBase.csv` 投影的可选展示信息 |
| `ProjectAnalysisRequest` | `manifest`, `selected_erb_paths[]`, `debug_mode` | 一次性分析，不替换项目 |
| `ProjectAnalysisReport` | `project_revision`, `success`, `diagnostics[]`, `analyzed_erb_paths[]` | 仅报告 |
| `ReloadProject` | `base_revision`, `target_revision`, `changes[]` | change 是 `Upsert{file}` 或 `Remove{category,path}` |

路径会把 `\` 规范为 `/`，忽略空段和 `.`；空路径、绝对路径、盘符和 `..` 被拒绝。
`SourceLocation` 是 `relative_path, byte_start, byte_end, line?, byte_column?`；offset 和
column 都是 UTF-8 byte，不是字符数或 UTF-16 code unit；`line` 和 `byte_column`
均从 0 开始，面向用户展示时应转换为从 1 开始。项目编译诊断会尽可能同时填写行和
byte column，前端可用提交的 UTF-8 源码按 `byte_start..byte_end` 显示源码行和精确
标记范围。`ProtocolDiagnostic` 字段为 `code`、Debug/Info/Warning/Error 等级、
`message`、`source?`。

### 7.2 输入

`InteractionToken { epoch, id }` 由 runtime 创建和撤销。`InputWait` 的 13 个字段是：
`wait_id`、`kind`、`stability`、`one_input`、`stop_message_skip`、`system_input`、
`mouse_input`、`default_value?`、`deadline_ns?`、`display_time`、`timeout_message?`、
`submission_token`、`countdown_remaining_ms?`。`countdown_remaining_ms` 只供显示，超时
判定仍由 runtime 完成。

`WaitKind`：EnterKey、AnyKey、IntegerValue、StringValue、Void、AnyValue、
IntegerButton、StringButton、PrimitiveMouseKey。无 deadline 且用户可恢复的 wait 才是
`StableInput`；带时限和 `Void` 是 `Transient`，不可作为精确 snapshot 点。

`WaitChanged` 是 `Opened(InputWait)`、`Updated(InputWait)` 或 `Closed(wait_id)`。
提交 `FrontendInput { wait_id, token, monotonic_time_ns, intent, message_skip }` 时，wait
和 token 必须仍精确活动；时间必须相对该 session 不倒退。

`InputIntent`：Enter、AnyKey(String)、CommitText(String)、Activate(token)、Continue、
Cancel、Primitive(`input_type,result_1..result_4,selection_token?`)、
ActivateKeyMacro `{group,slot}`。前端从不提供 `RESULT[5]`；按钮必须返回收到的 token，
而非 `ProtocolValue`。`ProtocolValue` 只有 Integer(i64)、String、Boolean、Bytes。

`AdvanceTime { monotonic_time_ns }` 驱动超时。当前边界为：时间采样达到 deadline 会超时；
恰在 deadline 观察到的输入仍可能被接受，而晚于 deadline 的输入让 runtime 先完成
超时。前端应使用同一单调时钟并按观察顺序发送。

`DeviceStateChanged` 字段是 `device, code, pressed, x, y, monotonic_time_ns`。当前 runtime
只吸收单调时间，尚未把其余字段转为通用设备状态。`ClientStateChanged` 字段为
`focused, visible, audio_available, reduce_motion, high_contrast, screen_reader`；当前
只有 `focused` 和 `audio_available` 进入行为判断，其余仅属于已定义协议面。

undo 请求返回 token；`InputUndoState` 字段为 `enabled, available_steps, in_progress,
runtime_revision, token?`。

### 7.3 key macro 和 extension

macro 固定 10 组 × 12 槽。profile 是 `relative_path + FilePayload`；命令为
`SelectGroup(u8)`、`Store{group,slot,text}`、`Clear{group,slot}`。状态字段：
`enabled, selected_group, group_names, entries`（group-major，精确 120 项）和
`serialized`（规范 UTF-8 日文格式 `macro.txt`）。越界命令被拒绝。

extension declaration 字段为 `id, era_name, kind(Instruction/Function), arguments[],
variadic, return_type, argument_style(Normal/Formatted/Raw), operation,
operation_version`。每个 argument 是 `value_type(Integer/String/Void/Any), mutable,
optional`。调用通过 `ServiceKind::Extension`；payload 是
`ExtensionInvocation {extension_id, arguments}`，响应为
`ExtensionResult {value?, writes[{argument_ordinal,value}]}`。只接受已协商、版本精确且
声明相符的调用。

## 8. 展示、投影和 effects

### 8.1 Snapshot/delta

前端以 `PresentationSnapshot` 建立基线，再仅在
`delta.base_revision == local_revision` 时应用 `PresentationDelta`。不相等时停止应用并
请求 resync，不能“尽力合并”。

`PresentationSnapshot` 字段：`revision, title, history, backgrounds, audio, input_wait?,
settings, tooltip, resources, html_island, redraw`。`PresentationDelta` 字段：
`base_revision, new_revision, operations[]`；operation 为 AppendLine、DeleteLines、
Clear、SetTitle、SetBackgrounds、SetAudio、SetInputWait、ReplaceLine、
SetSettings、SetTooltip、SetResources、SetHtmlIsland、SetRedraw、
SetButtonGeneration、TrimLines。

主要公开展示结构：

| 类型 | 字段/变体与单位 |
| --- | --- |
| `Color` | RGBA `u8`；派生默认值全 0（透明黑） |
| `TextStyle` | foreground、background?、bold、italic、underline、strikeout、font_family?、font_millipoints（1/1000 point） |
| `LogicalLength(i64)` | 1 个脚本逻辑单位 = 1000 milliunits；不是像素 |
| `PresentationLength` | `Logical` 或 `FontHeightHundredths` |
| `LogicalRect` | x/y/width/height；均为 LogicalLength |
| `CanvasPoint/Size/Rect` | i32 坐标；size 为 u32；canvas 自身像素空间 |
| `MediaPlacement` | resource_id、x/y/width/height、depth、opacity、revision、hover/mask resource?、requested width/height/y? |
| `RationalOpacity` | numerator:i64、denominator:u32；前端不应制造分母 0 |
| `Shape` | kind、parameters[]、foreground?、background? |
| `DisplayLine` | line_id、temporary、logical_line_start、line_end、alignment、runs[] |
| `PresentationSettings` | drawable_width、line_height、background、button_focus_foreground、maximum_physical_lines、prevent_button_wrap、legacy_nonbutton_wrap |
| `PresentationHistory` | logical_lines 和可重放 operations；snapshot 不是无限审计日志 |
| `RedrawState` | enabled |
| `AudioState` | channel_id、resource_id、repeat_count、volume_millionths、playing、revision |
| `TooltipSettings` | foreground/background、delay_ms、duration_ms、font、font_millipoints、custom、原始 format、images、normalized_format |

`DisplayRun` 变体：

- `Text{text,style,system_text?}`；
- `Button{runs,token,title?,hover_style?,value,generation,enabled}`；disabled 不得提交；
- `HtmlDocument{document}`；
- `Image{placement,alt_text?}`、`Shape{shape}`；
- `ColumnCell{content,alignment,preferred_columns}`；
- `Separator{pattern,role=Rule}`；
- `Space{width}`。

`SystemTextRef {key,arguments}` 让前端本地化 runtime 系统文字。key 为 InvalidValue、
SaveQuestion、LoadQuestion、OverwriteQuestion、NotEnoughMoney、OutOfStock、
AutoSaveFailed、AutoSaveSkipped、PressAnyKey、SaveSlot、Back、NewGame、LoadGame、
ContinuousTrainProgress、ContinuousTrainCommandFailed；argument 仅 Integer/String。

history operation 是 Append、DeletePhysical、ReplaceTemporary、Clear、
SetButtonGeneration、TrimPhysical。`TrimPhysical`/`TrimLines` 只裁掉最旧物理行，不改变
脚本逻辑行计数。

资源重放字段：

- `ResourceReplay {sprites, canvases, animation_timer_ms}`；
- `SpriteReplay {name,size,position,frames,canvas_id?,canvas_rectangle?}`；
- `SpriteFrameReplay {resource_id,source_rectangle[4],offset[2],delay_ms,
  destination_size?,canvas_id?}`；
- `CanvasReplay {canvas_id,size,commands,revision}`；
- canvas 命令为 Clear、DrawSprite、SetPixel、FillRectangle、SetBrush、SetPen、
  SetDashStyle、SetFont、DrawLine、DrawText、DrawCanvas、LoadEncodedImage；字段与
  [`presentation.rs`](../crates/era-runtime-protocol/src/presentation.rs) 同名。颜色矩阵
  是整数数组；DrawCanvas 的 5×5 值为 1/256 定点，rotation 是 millidegrees。

tooltip 的 `normalized_format.flags` 是明确枚举，`unknown_bits` 保留未识别原始位；前端
应使用规范 flags，同时在往返/诊断中保留 unknown bits。

### 8.2 HTML

线上传的是已规范化 AST，不是让前端重新解析的原始 HTML。`HtmlDocument {nodes}`；
node 是 `Text{text,start,end}` 或 `Element{kind,attributes,children,interaction?,
start,end,semantic}`，位置均为原始 UTF-8 byte offset。

element kind：Bold、Italic、Underline、Strike、Font、Paragraph、NoBreak、Button、
NonButton、ClearButton、Image、Shape、Division、Break。attribute 是 name/value 文本。
interaction 是 `epoch,id,integer_value?,string_value?,generation,enabled`。

semantic 为 Style、Font、Paragraph、NoBreak、Button、NonButton、ClearButton、Image、
Shape、Division 或 Break；其公开字段逐一见
[`HtmlElementSemantic`](../crates/erabasic-html/src/markup.rs)。`HtmlLength` 是 Pixels
或 FontHeightHundredths；box model 的 border/radius/margin/padding 和 border_colors
按四边数组传递。前端只投影 semantic，不应通过原始 attribute 猜出另一套含义。

### 8.3 投影反馈

`ProjectionLength(i64)` 是权威前端的设备无关空间，例如 CSS pixel。前端提交：

`ProjectionObservation { environment_revision, presentation_revision, client_size{width,
height}, projection_space_revision, line_columns, text_box, transform }`。
transform 字段为 x/y numerator、非零 denominator 和 origin x/y。

environment revision 必须严格增加，presentation revision 必须是当前基线；宽高和
line_columns 必须大于零，projection-space revision 不可倒退。runtime 回
`ProjectionState {runtime_revision,text_box,hotkey_state,button_generation,
text_box_layout}`；layout 的 width=0 表示配置默认宽度。

### 8.4 Effect

effect 是短暂命令，不替代 snapshot 中可恢复的 audio/scene 状态。`EffectBatch` 含
`EffectEvent {effect_id, kind}`；kind 是 Audio、StartAnimation、Video、
Extension(name,value)、OpenConfiguration、PresentNow{presentation_revision}。
Audio 字段为 channel、Play/Stop/SetVolume、resource?、repeat_count、
volume_millionths；Video 是 resource/skippable。

每个已知 effect ID 只能在一个 `EffectAcknowledgement` 中出现一次，outcome 为
Completed/Failed/Cancelled 和可空 message。未知、重复结果会被拒绝；非 Completed
结果还会产生诊断。未确认 effect 留在 journal，resync 后会重发。

## 9. Storage、Service 和状态传输

### 9.1 Storage

所有实际文件 I/O 都由前端完成。namespace：Project、Save、GlobalSave、Data、Log、
Resource。`StorageRequest` 字段为 `request_id, namespace, relative_path, operation,
idempotency_key, deadline_ns?`。同一幂等键重试必须具有相同效果。

operation：

- `Read`；
- `Write{data, atomic_replace, precondition}`；
- `List{pattern?, recursive}`；
- `Delete{precondition}`；
- `Stat`；
- `ReadRange{offset, maximum_bytes, change_token?}`。

precondition 是 Any、Missing 或 Revision(String)，必须在提交点原子检查；不成立返回
`FrontendIoErrorKind::Conflict`。协商的 `StorageCapabilities` 四字段为 revisions、
atomic_replace、missing_precondition、delete；候选存档要求全部为 true。

response 用同一 request ID，result 是 Read{data,revision?}、Written{revision?}、
Listed{entries}、Deleted、Error{error}、Metadata{byte_length,revision?} 或
ReadChunk{data,offset,complete,change_token}。entry 字段为 path、byte_length、
revision?、change_token?。每个未取消请求必须恰好响应一次；收到
`CancelExternalRequest` 后尽力取消，迟到响应会作为 stale 拒绝。

### 9.2 Service

能力项是 `ServiceCapability {kind, operation, versions}`。`ServiceRequest` 字段：
`request_id, kind, operation, operation_version, payload, deadline_ns?`；payload 是该
操作对应类型的规范 CBOR。response 是 Ready{payload} 或
Error{code,message}。不要返回 JSON、平台对象或错误栈。

当前 1.0 操作：

| kind | operation | 请求 → 响应 |
| --- | --- | --- |
| Clock | `local_date_time` | 空 → year/month/day/hour/minute/second/millisecond/UTC offset minutes |
| Entropy | `random_seed` | 空 → `seed:u64` |
| InputState | `get_key_state` | `key_code:u8` → active/pressed/toggle |
| Image | `image_metadata` | resource ID + digest → width/height/format/animated |
| Image | `image_pixel` | resource ID + digest + x/y → ARGB u32 |
| Network | `update_check` | URL → remote version/download URL |
| OpenUrl | `open_url` | URL → opened bool |
| PresentationQuery | `get_display_line` | context + index → context + string |
| PresentationQuery | `html_get_printed_str` | context + index → context + string |
| PresentationQuery | `html_string_len` | context + markup + argument → context + integer |
| PresentationQuery | `html_substring` | 同上 → context + head/tail |
| PresentationQuery | `html_string_lines` | 同上 → context + integer |
| PresentationQuery | `serialize_physical_history` | context + title + hide_information → context + UTF-8 |
| FontMetrics | `gget_text_size` | context + text/font/size/style bits → context + width/height |
| Canvas | `sample_canvas_pixel` | context + canvas/revision/point → context + revision/ARGB |
| Canvas | `decode_canvas_image` | encoded bytes → width/height |
| Canvas | `encode_canvas_png` | canvas/revision → encoded bytes |
| Extension | 动态声明的 operation | `ExtensionInvocation` → `ExtensionResult` |

presentation query 的 `context` 是 presentation/environment/projection-space 三个 revision；
响应必须原样带回，runtime 用它拒绝已过时的物理观察。普通 service 错误通常成为终止
`ServiceFailure`；少数宿主路径有源码明确的兼容降级，因此前端仍应返回真实 Error，
不能自行伪造成功。

### 9.3 状态传输

kind：TraditionalSave、VmSnapshot、CompiledProjectCache、FullProjectFile、InputReplay。
其中 InputReplay 只允许导出，导入必须明确拒绝。完整描述符是
`transfer_id, kind, total_bytes, digest`（精确 32 字节 BLAKE3）和 `artifact_id?`。

导出：请求 kind → `Ready{descriptor}` 或 `Ineligible{reasons}` → 从 offset 0 连续请求
非零 `maximum_bytes` → 收 `StateExportChunk{offset,data,complete}` → 持久化并可取消。
实际 chunk 还会受 negotiated payload 上限约束。

导入：`StateImportBegin` → Accepted(id) → 从 offset 0 发送非空、无间隙 chunk →
Commit → runtime 校验大小/digest/artifact → Ready → `Start` 引用已提交 ID。session
同时只允许一个同方向传输，大小受 `maximum_transfer_bytes` 限制。

Traditional save 与普通 VM snapshot 只可在没有 deadline 的稳定输入 wait、没有外部
请求和短暂 effect 时精确导出。VM snapshot 的 `snapshot_purpose` 为 Normal、Debug 或
Diagnosis；后两者可在任意执行状态捕获，并在快照中保留不同来源标记，但该标记不放宽
恢复规则。恢复仍完整校验字节码 artifact、项目资源、locale/culture、runtime wait 和
VM fiber 状态；状态确实可恢复时，Debug/Diagnosis 来源会产生稳定代码的 warning
diagnostic，交由前端明确呈现。`CompiledProjectCache` 是仅供仍可直接读取源码目录的宿主
持久化的内部 `RERACACH` v7 缓存：它省略脚本、图片和音频正文，用 manifest 索引、内容
摘要、源码长度及行起点增量重建编译元数据，所有段使用低压缩级别以缩短启动后的后台生成
时间；它不能作为独立项目文件解码，也不能追加配置事务。`FullProjectFile` 才是可分发的
`.reraproj`：文件头 magic 为 `RERAPROJ`，当前单字节格式版本为 `07`，并完整保留 manifest
payload。运行时继续读取自包含的 v6。两种 v7 容器的增量状态都只保存按规范函数顺序排列的
cache key，语句指纹在既有 `Digest` 接口中使用 128 位有效内容。全量项目主体后可顺序追加
`RERACFG1` 配置事务记录；每条保存
完整的规范化 LF `reraconfig.toml`、前一配置摘要、结果摘要和校验和，末尾不完整记录按中断
写入处理并在下次保存前截断，完整但损坏或链不连续的记录拒绝加载。版本 `01`–`05` 和
v7 之前的 `RERACACH` 编译缓存不再兼容；前端仍持有源码时应按普通 cache miss 重新编译，
没有源码的独立旧项目文件会加载失败。全量项目文件同时保留成功
构建产生的项目诊断；精确命中时重放原等级、code 和 source，并在正文前添加
`[cached] `。项目文件准备异步，首次请求可能被可恢复地拒绝为“已开始/仍在准备”，稍后
再请求。

`InputReplay` 导出 UTF-8、无 BOM、每对象一行且以换行结尾的
`input-replay.jsonl`。首行是 schema 1 的 `header`，固定标明
`fidelity="manual_path"`、可用状态、步骤数、当前时间线来源和限制；后续是从 1 编号的
`step`。每个 `origin` 都包含 `project={revision,identity,locale}`，并使用下列稳定的 `kind`
与专有字段：

- `new_game`：`seed,trigger`，trigger 为 `start` 或 `return_to_title`；
- `traditional_save`：`payload_digest,description,save_version`；
- `ordinary_save`：`slot,storage_path,payload_digest`；
- `vm_snapshot`：`payload_digest,snapshot_format,snapshot_origin,original_project_identity`；
- `hot_reload`：前后 revision/identity，以及包含 operation、relative_path、category 的
  `changes`；
- `input_undo`：`checkpoint_slot,save_digest,retained_input_count`；
- `configuration_update`：前后 revision/identity 与 `changed_codes`；
- `external_data_load`：`storage_path,payload_digest,data_type`，data_type 为 `global` 或
  `character`。

起始存档、快照或外部数据载荷本身不在归档中。步骤的 action 固定为 `enter`、`any_key`、
`text`、`button`、`continue`、`primitive` 或 `timeout`，并记录 wait_kind、实际语义 result 与
message_skip。按钮额外记录当时规范展示中的 visible_text、title、alt_text、语义 value 和
可用选择中的 1-based ordinal。步骤只记录 runtime 实际接受并生效的语义输入，按钮描述取自
输入失效前的规范展示；
interaction token、session/epoch、wait/message ID 和绝对路径均不写入。所有 `u64` 和可能超出
JavaScript 安全范围的整数使用十进制字符串；步骤序号、计数、ordinal 和存档 slot 使用
JSON number，摘要使用小写 BLAKE3 十六进制。历史最多
4096 步，预计编码大小最多为 `min(16 MiB, maximum_transfer_bytes)`；超限时当前片段变为
`status="unavailable"`、`reason="history_limit_exceeded"`，下一次成功的时间线边界重新开始记录。
该数据只用于人工路径重现：开发者必须先取得相同起始状态，外部时间、设备和服务结果不保证
完全确定。

开发和诊断工具可调用
`inspect_runtime_snapshot(bytes, maximum_bytes) -> RuntimeSnapshotInspection`，复用正式
恢复路径的容器、版本、大小、BLAKE3、zstd 和 MessagePack 解码检查，并递归解析内嵌
执行快照。结果的 `inspection_schema_version` 当前为 1，包含 `container`、`payload` 和
`validation`；不透明 bytes 只投影为长度及 BLAKE3。该接口没有加载 bytecode artifact，
所以 `artifact_compatibility` 与 `restore_semantics` 明确为 `not_checked`，不能用分析成功
代替实际恢复成功。命令行入口和输出约定见 README 的“Runtime 快照分析器”。

### 9.4 其余公开结构字段速查

本节补齐前述流程中只简述过的结构。空结构 `ReturnToTitleRequest`、
`LocalDateTimeRequest`、`RandomSeedRequest` 没有字段。

生命周期：

- `RuntimeStateChanged {phase, revision, epoch}`；
- `ExitRequested {reason: Quit|Restart, force, runtime_revision}`；
- `SequenceAcknowledgement {through_sequence}`；
- `ResynchronizeRequest {after_sequence?}`；
- `RuntimeResynchronized {epoch, phase, runtime_revision, presentation, exit_requested?,
  selected_locale, input_undo, key_macros}`；
- `StartRequest {mode}`，mode 为 `NewGame{seed?}`、
  `TraditionalSave{transfer_id}`、`VmSnapshot{transfer_id}`；
- `ShutdownRequest {graceful}`；
  `ShutdownReady {final_runtime_revision,pending_operations_cancelled}`；
- `VersionRejected {supported,message}`；
- `CommandRejected {code,message,recoverable,source?}`；
- `RuntimeFault {code,message,origin?}`；
  `ExecutionOrigin {command,function,generation,instruction,source?}`。

状态传输结构按键完整字段：

- `StateExportRequest {kind,snapshot_purpose}`；`snapshot_purpose` 为
  `Normal|Debug|Diagnosis`，非 VM snapshot 必须使用 Normal；
  `StateExportReady {kind,result}`；result 是
  `Ready{transfer}` 或 `Ineligible{reasons[]}`；
- `StateImportBegin {kind,total_bytes,digest,artifact_id?}`；
  `StateImportAccepted {transfer_id}`；
  `StateImportChunk {transfer_id,offset,data}`；
  `StateImportCommit {transfer_id}`；
  `StateImportReady {transfer_id,kind}`；
- `StateExportChunkRequest {transfer_id,offset,maximum_bytes}`；
  `StateExportChunk {transfer_id,offset,data,complete}`；
- `FullProjectManifest {manifest}` 暂存用户主动导出所需的完整运行输入；
  `StateExportCancel {kind}` 可取消尚在准备或传输中的指定导出。
- `CompiledProjectCache` 使用内部 `RERACACH` v7 容器和低压缩率，省略可由宿主直接读取的
  脚本正文及图片、音频正文；`FullProjectFile` 使用自包含 `RERAPROJ` v7 容器。
  `StateTransferCancel {transfer_id}`。

外部请求结构：

- `CancelExternalRequest {request_id,kind}`，kind 是 Storage/Service；
- `ServiceError {code,message}`；
- `ProjectionQueryContext {presentation_revision,environment_revision,
  projection_space_revision}`；
- `ProjectionStringIndexRequest {context,index}`；
  `ProjectionStringResponse {context,value}`；
- `HtmlMeasureRequest {context,markup,argument}`；
  `ProjectionIntegerResponse {context,value}`；
  `HtmlSubstringResponse {context,head,tail}`；
- `TextExtentRequest {context,text,font_family,font_size,style_bits}`；
  `TextExtentResponse {context,width,height}`；
- `CanvasPixelRequest {context,canvas_id,canvas_revision,point}`；
  `CanvasPixelResponse {context,canvas_revision,argb}`；
- `DecodeCanvasImageRequest {encoded}`；
  `DecodeCanvasImageResponse {width,height}`；
  `EncodeCanvasPngRequest {canvas_id,canvas_revision}`；
  `EncodeCanvasPngResponse {encoded}`；
- `SerializePhysicalHistoryRequest {context,title,hide_information}`；
  `SerializePhysicalHistoryResponse {context,utf8}`；
- `LocalDateTimeResponse {year,month,day,hour,minute,second,millisecond,
  utc_offset_minutes}`；
  `RandomSeedResponse {seed}`；
- `ImageMetadataRequest {resource_id,content_digest}`；
  `ImageMetadataResponse {width,height,format,animated}`；
  `ImagePixelRequest {resource_id,content_digest,x,y}`；
  `ImagePixelResponse {argb}`；
- `UpdateCheckRequest {url}`；`UpdateCheckResponse {remote_version,download_url}`；
  `OpenUrlRequest {url}`；`OpenUrlResponse {opened}`；
- `GetKeyStateRequest {key_code}`；
  `GetKeyStateResponse {frontend_active,pressed,toggle_state}`；
- `PointerStateRequest {presentation_revision,environment_revision,
  projection_space_revision}`；
  `PointerStateResponse {x,y,button_value,presentation_revision,environment_revision,
  projection_space_revision}`。该操作当前不能可靠协商，见第 11 节。

Canvas 命令的完整字段：

- `Clear {argb,rectangle?}`；
  `DrawSprite {name,destination,color_matrix?}`；
  `SetPixel {point,argb}`；
  `FillRectangle {rectangle,brush_argb}`；
- `SetBrush {argb}`；
  `SetPen {argb,width}`；
  `SetDashStyle {style,cap}`；
  `SetFont {family,size,style_bits}`；
- `DrawLine {start,end}`；
  `DrawText {text,point}`；
- `DrawCanvas {source_canvas_id,source_revision,source,destination,color_matrix?,
  mask_canvas_id?,rotation_millidegrees,rotation_center?}`；
- `LoadEncodedImage {content_digest,encoded}`。

HTML semantic 的完整字段：

- `Style`、`NoBreak`、`Break` 无字段；
- `Font {face?,color?,button_color?}`；
  `Paragraph {alignment}`；
- `Button {value?,title?,position?}`；
  `NonButton {title?,position?}`；
  `ClearButton {suppress_tooltip}`；
- `Image {source,hover_source?,mask_source?,height?,width?,y?}`；
- `Shape {kind,parameters,color?,button_color?}`；
- `Division {x?,y?,width,height,depth,color?,relative,box_model}`；
- `HtmlBoxModel {border?,radius?,margin?,padding?,border_colors?}`，每项是四边数组；
  `HtmlAttribute {name,value}`；
  `HtmlInteraction {epoch,id,integer_value?,string_value?,generation,enabled}`。

`TooltipFormat {flags,unknown_bits}` 的 flags 依次为 HorizontalCenter、Right、
VerticalCenter、Bottom、WordBreak、SingleLine、ExpandTabs、NoClipping、ExternalLeading、
NoPrefix、Internal、TextBoxControl、PathEllipsis、EndEllipsis、ModifyString、RightToLeft、
WordEllipsis、NoFullWidthCharacterBreak、HidePrefix、PrefixOnly、
PreserveGraphicsClipping、PreserveGraphicsTranslateTransform、NoPadding、
LeftAndRightPadding。原始位值的规范映射见
[`TooltipFormat::from_raw`](../crates/era-runtime-protocol/src/presentation.rs)；wire 上传的是
enum tag 列表和 `unknown_bits`，不是平台 flags 对象。

## 10. 错误分层和恢复建议

1. **C ABI 状态**：调用甚至没有进入/完成协议处理。读取 `last_error`，检查指针、结构
   大小、handle 和编码。
2. **`CommandRejected`**：字段为 code、message、recoverable、source?；code 是
   InvalidState、InvalidValue、StaleRequest、VersionMismatch、PermissionDenied、
   FeatureUnavailable、ResourceLimit。若 recoverable，只修正当前命令，不重建 session。
3. **`ProtocolDiagnostic`**：带后端权威等级的诊断；是否终止由伴随的报告/phase 决定。
4. **`RuntimeFault`**：code 是 InvalidState、InvalidMessage、ProjectLoad、VmFault、
   ServiceFailure、ResourceLimit、Internal、UnsupportedRuntimeFeature；可带
   `ExecutionOrigin {command,function,generation,instruction,source?}`。收到后停止输入和
   外部响应，展示诊断，ACK 已处理消息，随后 destroy/recreate。
5. **Storage/Service Error**：前端平台操作的结构化失败；使用稳定 kind/code，并把平台
   code 放可空字段，不以本地化 message 作程序判断。

常见误用：

- 只 submit 不 drive，或只 drive 不 poll/ACK；
- 把 poll buffer 保存到 `release_buffer` 之后，或重复释放；
- epoch 更新后仍提交旧按钮、wait、debug stop 或 request token；
- 以字符索引解释 `SourceLocation`/HTML offset；
- 在前端自行完成超时、改变 runtime 配置或从按钮值伪造 token；
- 跨 snapshot revision 套用 delta；
- 用 `std::time`/浏览器 wall clock 代替单调时钟；
- 把 service `payload` 当成整个信封或 JSON；
- 在收到取消后仍将迟到结果无限重试。

## 11. 待确认

以下是源码中的实际不一致或未落实字段，不应被前端当作能力：

1. [`era-runtime-protocol/src/lib.rs`](../crates/era-runtime-protocol/src/lib.rs) 的模块注释
   仍写“在前端存在之前不承诺兼容”，但已有独立 `rustyera-tui` 前端。需确认该注释的
   准确措辞；当前按 AGENTS.md 的“开发期公共接口默认不向后兼容”执行。
2. C 头注释表达可按 `struct_size` 接受较短旧结构，但
   [`valid_header`](../crates/era-runtime-capi/src/lib.rs) 要求至少为当前完整 Rust 类型
   大小，且忽略 minor。需确认是实现过严还是注释/兼容设计过早。
3. `ResynchronizeRequest.after_sequence` 当前读取后未用于选择增量；runtime 总是发送
   当前完整聚合状态，再重发仍 journaled 的 effects。需确认字段应删除还是实现增量语义。
4. `ShutdownRequest.graceful` 当前不改变处理路径。需确认 false 是否应强制取消更多状态。
5. `pointer_state` 类型、常量和 host 分发存在，但能力选择没有把它加入可协商 service
   集合；当前前端不能可靠启用。需确认遗漏还是有意不支持。
6. `RuntimeFeature` 的 rich text/HTML/graphics/audio/mouse 变体不会被 feature 协商选中，
   但相应 `ClientCapabilities` 可部分生效。需确认双层协商的长期边界。
7. `ERA_DEBUG_SCOPE_ALL` 覆盖 bit 0–9，而 C 头只为 bit 0–8 提供名称；bit 9 实际是
   ScriptOutput。前端若手写 binding 容易漏掉该权限。
8. negotiating 状态只要求 `ClientHello` 是第一条 Runtime 消息和 sequence 0，没有拒绝
   Hello 信封中非空的 session/epoch；协议范例和现有前端都发送空值。需确认应补严格校验
   还是把字段定义为接收时忽略。
9. VM snapshot 恢复推进 epoch 时只清理 `accepted_message_ids`，没有像通用
   `advance_epoch`/reload 一样清理 `accepted_debug_message_ids`。session/epoch 校验仍会
   拒绝旧信封，但重放缓存生命周期不一致，需确认是否遗漏。
10. `Envelope::validate` 只禁止 `message_id=0`；入站新 sequence 没有拒绝复用既有
    message ID，Runtime/Debug 的接受缓存也彼此独立。为保证 correlation 无歧义，前端仍
    应跨 channel 单调生成唯一 ID；需确认 runtime 是否应强制这一约束。

## 12. 最小可运行端到端示例

以下 Python 3.12 脚本复用仓库实际 C ABI 和 wire 投影，完成创建、握手、提交最小项目、
启动、事件 drain、ACK、错误处理与释放。它不会实现 Storage/Service，因此示例脚本
显式给种子且不调用外部能力。

先执行：

```sh
cargo build -p era-runtime-capi --release
uv sync --project ../rustyera-tui
export ERA_RUNTIME_LIBRARY="$PWD/../target/release/libera_runtime_capi.dylib"  # Linux 改 .so
```

保存为 `/tmp/minimal_runtime_frontend.py`：

```python
from rustyera_tui.abi import AbiError, RuntimeAbi
from rustyera_tui.wire import (
    CHANNEL_RUNTIME, RUNTIME_VERSION, decode_envelope, encode_envelope,
    message_value, runtime_message, variant, version_range,
)

class Client:
    def __init__(self, abi):
        self.abi = abi
        self.sequence = 0
        self.message_id = 1
        self.session = None
        self.epoch = None

    def send(self, tag, value):
        message_id = self.message_id
        packet = encode_envelope(
            channel=CHANNEL_RUNTIME, channel_version=RUNTIME_VERSION,
            session=self.session, sequence=self.sequence,
            message_id=message_id, correlation_id=None,
            payload_tag=tag, payload=runtime_message(tag, value),
            epoch=self.epoch,
        )
        self.sequence += 1
        self.message_id += 1
        self.abi.submit(packet)
        return message_id

    def pump(self):
        events = []
        while True:
            report = self.abi.drive()
            while (packet := self.abi.poll()) is not None:
                env = decode_envelope(packet)
                value = message_value(env.payload, env.payload_tag)
                if env.session is not None:
                    self.session = env.session
                if env.epoch is not None:
                    self.epoch = env.epoch
                events.append((env.payload_tag, value, env.sequence))
            if report.state not in (1, 2):  # MORE_WORK / OUTPUT_READY
                break
        if events:
            self.send(93, {0: max(event[2] for event in events)})
        return events

try:
    with RuntimeAbi(debug_scope_mask=0) as abi:
        client = Client(abi)
        limits = {
            0: 128 * 1024 * 1024, 1: 127 * 1024 * 1024,
            2: 128, 3: 4096, 4: 100_000, 5: 1024 * 1024,
        }
        capabilities = {
            0: [0], 1: True, 2: True, 3: False, 4: False, 5: False,
            6: False, 7: True, 8: True, 9: [], 10: [],
            11: {0: False, 1: False, 2: False, 3: False},
        }
        client.send(0, {
            0: version_range(*RUNTIME_VERSION), 1: "minimal-python",
            2: [0, 1, 2, 3, 10, 12, 13, 14],
            3: limits, 4: capabilities, 5: ["zh-CN", "ja"],
        })
        hello_events = client.pump()
        hello = next(value for tag, value, _ in hello_events if tag == 1)
        print("session:", hello[1], "locale:", hello[6])

        source = "@SYSTEM_TITLE\nPRINTL HELLO FROM RUSTYERA\nWAIT\nRETURN\n"
        manifest = {
            0: 1,
            1: [{0: "main.erb", 1: 2, 2: variant(0, source)}],
        }
        client.send(10, manifest)
        loaded = client.pump()
        report = next(value for tag, value, _ in loaded if tag == 11)
        if not report[1]:
            raise RuntimeError(f"project load failed: {report[2]!r}")

        client.send(20, {0: variant(0, 1)})  # NewGame { seed: Some(1) }
        for tag, value, _ in client.pump():
            if tag == 40:
                print("presentation snapshot revision:", value[0])
            elif tag == 41:
                print("presentation delta:", value)
            elif tag == 32:
                print("wait event:", value)
            elif tag == 92:
                raise RuntimeError(f"runtime fault: {value!r}")

        client.send(90, {0: True})
        shutdown = client.pump()
        assert any(tag == 91 for tag, _, _ in shutdown)
        print("shutdown ready")
except (AbiError, ValueError, RuntimeError) as error:
    raise SystemExit(f"frontend failed: {error}") from error
```

运行：

```sh
uv --project ../rustyera-tui run python /tmp/minimal_runtime_frontend.py
```

Rust 的等价端到端路径是：用 `RuntimeMessage::{ClientHello,ProjectManifest,Start,
ShutdownRequest}` 的 `.envelope(...)` 和 `encode_envelope` 生成同样消息，依次调用
`RuntimeSession::{submit_envelope,drive,poll_envelope}`；第 3.3 节展示了 caller-pumped
循环。生产前端应经 C ABI，以免依赖内部 Rust 布局。

## 13. 接口索引

- C ABI：第 3 节；头文件 `era_runtime.h`
- 公共信封、CBOR、sequence/ACK：第 4 节
- 握手、能力、limits、phase：第 5 节
- 全部 Runtime message tag/方向：第 6 节
- 项目与诊断：第 7.1 节
- 输入、等待、token、时间：第 7.2 节
- key macro、extension：第 7.3 节
- presentation、HTML、projection、effect：第 8 节
- storage、service、状态传输：第 9 节
- 错误和恢复：第 10 节
- 源码不一致：第 11 节
- Python/Rust 端到端用法：第 12 节
