# Runtime 调试接口

> 面向前端开发人员和 EraBasic 脚本开发人员。本文描述 Debug 协议 `4.0` 在当前
> `era-runtime`/`erabasic-vm` 中的实际实现。公共信封为 `2.0`，C ABI 为 `3.6`。
> 主要源码：
> [`era-debug-protocol`](../crates/era-debug-protocol/src/lib.rs)、
> [`RuntimeSession` 调试分发](../crates/era-runtime/src/session/debug_session.rs)、
> [`VM debug port`](../crates/erabasic-vm/src/debug_port.rs) 和
> [`VM debug 实现`](../crates/erabasic-vm/src/debug.rs)。

## 1. 范围和稳定性

调试协议是独立、能力受限的公共协议；它通过与 Runtime 协议相同的 C ABI
`session_submit/session_drive/session_poll` 传输，不存在第二套 native debugger API。
协议对象只描述请求、停止视图和类型化结果，本身不能直接检查 VM。

Debug 4.0 是公开且版本化的开发期接口，当前默认不保证向后兼容。数字 message/command/
response 标记是线 ID，不能复用；主版本不兼容，次版本新增必须可忽略或经协商。
Runtime–VM 调试 port 属于内部接口，可随二者同一发布实体同步修改。

调试器没有比 session 创建者更高的权限。创建时 `debug_scope_mask=0`（Rust 默认）会让
握手成功但不授予任何 scope；生产前端应按最小权限创建。调试修改会改变真实游戏状态，
不是只读镜像，也没有独立“沙盒时间线”。

## 2. 模块职责、所有权和线程模型

```text
前端 debugger UI
  │ Debug Hello / Request / Revoke（Channel=Debug）
  ▼
RuntimeSession
  ├─ 校验 session/epoch/sequence、grant、scope、StopToken
  ├─ 冻结 runtime 时间并挂起部分 Runtime 命令
  ├─ 映射公共 Debug 类型 ↔ VM 内部 debug port
  ├─ 增加 runtime revision、更新 grant、输出调试消息
  └─ 持有 DEBUGPRINT UTF-8 环形窗口
  ▼
RuntimeVm / Vm
  ├─ 创建一致的 stop、fiber/frame/变量视图
  ├─ 原子变量写、断点解析、step 计划
  └─ continue 后才恢复执行
```

- session、VM、grant、stop、断点和 script-output buffer 都由 runtime 创建并释放；
  前端只缓存 opaque token，并在 session destroy 时丢弃。
- 调试消息和普通 Runtime 消息都只能在调用方执行 `drive` 时生效；没有 runtime 回调或
  后台 VM 执行。C ABI 当前用进程级 mutex 串行化 session 操作（项目文件投影扩展的
  解码与编码例外地在锁外完成），建议仍由同一 worker 顺序 pump。
- Debug 与 Runtime 各自拥有入站/出站 sequence；两者共享全局递增的 `message_id`。
  Debug 输出不使用 Runtime tag 93 ACK journal，前端只需按序消费。
- `StopToken` 给出一次一致视图。continue、step、reload、epoch/generation/revision
  变化会让旧 stop 失效。任何成功写操作也会刷新 stop，后续请求必须使用响应中的新值。

## 3. 启用、授权和撤销

### 3.1 创建者策略

C/Rust 创建时传入 64 位 mask：

| bit / `DebugScope` | 用途 | 所需命令 |
| --- | --- | --- |
| 0 `VariablesRead` | 列表、读取 EraBasic 变量 | ListVariables、ReadVariable |
| 1 `VariablesWrite` | 原子写 EraBasic 变量 | WriteVariables |
| 2 `GameFieldsRead` | 列表、读取 runtime 游戏字段 | ListGameFields、ReadGameField |
| 3 `GameFieldsWrite` | 写允许的 runtime 字段 | WriteGameFields |
| 4 `ExecutionRead` | fiber、call stack、operand stack | ListFibers、ReadCallStack、ReadOperandStack |
| 5 `ExecutionControl` | pause、continue、step | Pause、Continue、Step |
| 6 `ConsoleEvaluate` | 纯表达式求值 | Console(Evaluate) |
| 7 `ConsoleExecute` | 安全单赋值 | Console(ExecuteSafe) |
| 8 `BreakpointsManage` | 新增、替换、删除断点 | UpdateBreakpoints |
| 9 `ScriptOutput` | 读取/订阅 DEBUGPRINT | Read/SubscribeScriptOutput |

scope 不隐含另一个 scope；例如有 write 没有 read 仍只能调用 write。C 常量
`ERA_DEBUG_SCOPE_ALL` 是 `(1<<10)-1`。

### 3.2 握手

前置条件：Runtime `ClientHello/ServerHello` 已成功，Debug 信封携带当前 session 和
epoch。首条 Debug sequence 从 0 开始。

```rust
DebugMessage::Hello(DebugHello {
    versions: VersionRange,
    requested_scopes: Vec<DebugScope>,
})
```

runtime 求“创建者 mask ∩ requested scopes”，排序、去重后返回：

```rust
DebugMessage::Grant(DebugGrant {
    version: ProtocolVersion,
    token: GrantToken,
    scopes: Vec<DebugScope>,
})
```

`GrantToken` 字段：

| 键 | 字段 | 含义 |
| --- | --- | --- |
| 0 | `grant_id: SessionId` | 不透明授权身份 |
| 1 | `session_epoch:u64` | 发放时游戏时间线 |
| 2 | `program_generation:u64` | 发放时 VM 代码代 |
| 3 | `issued_runtime_revision:u64` | 发放时 runtime revision；不是 stop |

每个 `Request` 都必须原样携带完整 token；只有一个 active grant，新 Hello 会替换旧 grant。
epoch 或 program generation 变化时，runtime 会主动发一个无 `correlation_id` 的新 Grant，
scope 不变，旧 token 立即失效。

版本区间与 4.0 无交集时没有专用 VersionRejected，而是关联 Hello 返回
`DebugError(InvalidState)`；session 仍可运行，前端应关闭不兼容的调试 UI。

前端撤销：

```rust
DebugMessage::Revoke(DebugRevoke {
    grant_id: SessionId,
    reason: String,
})
```

已不存在/不匹配的 grant ID 幂等成功。若 VM 正在 `DebugPaused`，撤销会先 continue，
恢复暂停前 phase 和时间基线，再清除 grant。`reason` 当前只随消息传入，不写入响应或日志。
Revoke 当前不产生 DebugResponse；前端不能等待“撤销 ACK”，应在成功 submit 后清除本地
grant/stop，并继续 pump 可能出现的 Runtime `StateChanged`。

## 4. 信封、消息方向和时序

Debug 复用 [Runtime–前端接口第 4 节](runtime-frontend-interface.zh-CN.md#4-公共-cbor-信封)
的确定性 CBOR 规则。信封必须 `channel=1`、`channel_version={4,0}`，payload tag 与内部
enum tag 相同。

| tag | 方向 | 数据 | 关联规则 |
| --- | --- | --- | --- |
| 0 | 前端 → runtime | `Hello` | Grant/Error 的 correlation 是 Hello message ID |
| 1 | runtime → 前端 | `Grant` | 握手响应有关联；自动续期无关联 |
| 2 | 前端 → runtime | `Revoke` | 当前 runtime 不主动发 Revoke |
| 10 | 前端 → runtime | `AuthorizedDebugRequest` | Response/Error 关联 request ID |
| 11 | runtime → 前端 | `DebugResponse` | 请求响应有 correlation；output subscription 通知无关联 |
| 12 | runtime → 前端 | `DebugStop` | 断点/step/显式 pause stop 通常是通知；显式 Pause 先收到关联 Accepted |
| 13 | runtime → 前端 | `DebugError` | 关联失败请求 |

任何反方向消息产生 `InvalidState` DebugError。`AuthorizedDebugRequest` 是
`{ grant: GrantToken, command: DebugCommand }`。调用顺序示例：

```text
Frontend             Runtime/VM
   │ Hello ─────────────►│
   │◄──────── Grant ─────│
   │ Request(Pause) ────►│
   │◄─ Response(Accepted)│
   │◄──── Stopped ───────│  保存 StopToken
   │ Request(Read...) ──►│
   │◄──── Response ──────│
   │ Request(Write...) ─►│
   │◄─ Response(new stop)│  替换 StopToken
   │ Request(Continue) ─►│
   │◄─ Response(Accepted)│  删除 StopToken
```

## 5. Pause、stop、执行检查和控制

### 5.1 StopToken 与停止事件

`Pause` 只在 Running、WaitingInput 或 WaitingExternal phase 接受。VM 选择 primary fiber，
否则选择第一个存在的 fiber，立即建立一致 stop。已暂停、无 VM 或其他 phase 返回
InvalidState。

`StopToken` 字段为 `session_epoch, pause_epoch, program_generation, runtime_revision`。
四者和 VM 当前 stop 都必须精确匹配，否则是 `StaleStop`。

`DebugStop` 字段：

- `stop`：上述 token；
- `reason`：PauseRequested、Breakpoint{breakpoint_id}、StepCompleted、HostWait、
  FiberCompleted、Fault{message} 或 Reload；
- `selected_fiber?`：建议 UI 默认选中；可空；
- `source?`：当前指令的路径、内容 hash、UTF-8 byte 范围、行号和 UTF-8 byte column。

暂停会把 runtime phase 改为 DebugPaused。以下状态改变型 Runtime 消息被拒绝：
ProjectManifest/ProjectLoad/ProjectAnalysis、macro profile/command、extension registry、
ReturnToTitle、Start、Input/Undo、ServiceResponse、StorageResponse、全部状态传输和
ReloadProject。`AdvanceTime` 与设备时间只保存最新采样，不推进逻辑时间；continue 时
重建时间原点，避免暂停耗时触发游戏 timeout。ClientState、ProjectionObservation、
effect ACK、Runtime ACK/resync 和 shutdown 等仍按源码分发。

### 5.2 Fiber、frame 和 operand

分页 limit 合法范围为 1–1024；`cursor=None` 等价 0，响应的 `next_cursor=None` 表示结束。

| 命令 | 参数 | 响应 |
| --- | --- | --- |
| `ListFibers` | stop、cursor?、limit | `FiberPage {stop,fibers,next_cursor?}` |
| `ReadCallStack` | stop、fiber_id | `CallStack {stop,fiber_id,frames}` |
| `ReadOperandStack` | stop、fiber_id、frame_id、cursor?、limit | `OperandStackPage` |

`FiberSummary` 是 `fiber_id,state,primary,frame_count`。state 枚举定义了 Runnable、
WaitingHost、WaitingResume、Completed、Faulted、Cancelled、DebugPaused；当前 VM 转换
只产生前六类，暂停是 VM 的全局 debugger 状态，不把各 fiber 映射为 `DebugPaused`。
Completed/Cancelled 只在终止事件尚未消费或完成 stop 仍受保护时短暂出现，不构成历史
记录；终止回收后 fiber ID 可在后续 stop 中复用，调试客户端不得跨 stop 缓存 ID。

`FrameSummary` 是 `frame_id,generation,function_key(16 bytes),function_name,instruction,
source?`。call stack 顺序从最新 frame 到最旧 frame。operand value 是
`offset:u64 + DebugValue`，offset 按 frame stack 从底到顶枚举；没有 operand 写接口。

### 5.3 Continue 和 Step

`Continue {stop}` 清除 VM stop，恢复暂停前 phase，并返回 `Accepted`。若从 breakpoint
继续，VM 会跳过当前位置一次，避免立即再次命中同一个断点。

`Step {stop,fiber_id,kind}` 只接受存在且 Runnable、具有 frame 的 fiber。kind：
Instruction、SourceLine、Into、Over、Out。成功先返回 Accepted、phase 变 Running；执行到
步进边界、host wait、fiber 完成、fault 等时再发新的 Stopped。旧 stop 从成功接受起失效。

## 6. 变量接口

### 6.1 类型和列举

`ValueKind`：Integer、String、Boolean、Bytes。`DebugValue` 还可为
`Place(DebugPlace)`；Place 是 VM 栈上的引用描述，不是可写标量值。

`VariableStorage`：Global、FunctionStatic、Character、Local。`VariableDescriptor` 字段：
`symbol_key`（16 bytes）、`name`、`storage`、`value_kind`、`dimensions[]`、`mutable`。

`ListVariables {stop,cursor?,limit}` 返回 descriptor 页。列表为每个 generation 生成：

- 普通/global/static 变量一项；
- character 变量为每个现存角色一项；
- local 变量为实际包含该 local 的每个 fiber/frame 一项；
- 数组 descriptor 报告所有维度，但列表项只代表每维 index 0。读取其他元素时，前端按
  descriptor 构造 `VariableReference.indices`。

但 `VariableDescriptor` 本身没有 generation、character、fiber、frame 或完整 reference，
所以当前 wire 结果会把上述角色/局部/保留 generation 的多项压成无法区分的重复
descriptor。前端只能按当前 generation、角色 0 等约定构造有限目标，或复用其他响应/
operand `DebugPlace` 已给出的目标；不能声称已能从列表可靠遍历所有实例。见第 13 节。

`VariableReference` 字段为 `symbol_key, storage, fiber_id?, frame_id?, generation,
character?, indices[]`。当前目标转换使用 symbol/fiber/frame/generation/character/indices，
`storage` 只携带分类元数据，不参与 VM 目标校验；不要只用变量名，也不要靠修改 storage
改变目标。

`ReadVariable {stop,value:VariableReference}` 返回
`VariableValue {reference,value,revision}`。`DebugPlace` 字段与 reference 类似：
`symbol_key,value_kind,indices,character?,fiber_id?,frame_id?,generation`，用于只读展示
operand 中的引用。

### 6.2 原子写

`WriteVariables {stop,writes[]}` 要求 1–1024 项。每项是
`{reference,value,expected_revision}`；VM 变量当前只接受 Integer 或 String，目标必须
mutable 且类型/维度/角色/frame/generation 有效。

全部 expected revision 必须匹配 VM debug revision。VM 先克隆 memory 和 fibers，任何
一项失败即完整回滚；全成功才统一增加 VM debug revision。Runtime 随后也增加自己的
revision，并返回 `VariablesWritten {stop:new_stop, values}`。调用方必须用返回的
`new_stop` 和每个新 `revision` 替换本地缓存。

常见误用：用 descriptor 的 `dimensions` 当作当前 indices；跨 reload 使用 symbol key
却保留旧 generation；向 Place/Boolean/Bytes 写 VM 变量；一批写中混用不同 revision。

## 7. Runtime 游戏字段

`ListGameFields/ReadGameField/WriteGameFields` 同样要求有效 stop，分页 limit 1–1024。
当前字段全集：

| key | 类型 | 可写性 | 值 |
| --- | --- | --- | --- |
| `input.message_skip` | Boolean | DebugWritable | runtime 的 message-skip latch |
| `runtime.logical_time_ns` | Integer | ReadOnly | 权威逻辑时钟；超过 i64 时饱和为 i64::MAX |
| `runtime.phase` | String | ReadOnly | 当前 `RuntimePhase` 的 Rust Debug 名称 |
| `runtime.revision` | Integer | ReadOnly | runtime mutation revision；超过 i64 时饱和 |

descriptor 是 `key,value_kind,mutability,description`；value 是 `key,value,revision`。
写批次 1–1024，所有 `expected_revision` 都必须等于当前 runtime revision。当前唯一合法
写入是给 `input.message_skip` 传 Boolean；先验证整批，成功后一次提交并把 runtime
revision 增加 1，返回新的 stop 和 values。因此多项重复写同一 key 的最后一项生效，
但这种用法不利于 UI 审计，应避免。

## 8. 断点

`UpdateBreakpoints` 不要求 stop，但要求 active VM、grant 和 BreakpointsManage scope。
它接收：

```rust
BreakpointUpdate {
    requested: Vec<Breakpoint>,
    remove: Vec<u64>,
}
```

每个 breakpoint 为 `breakpoint_id, enabled, location`。location：

- `Source { relative_path, content_hash, byte_offset }`：hash 必须精确 32 bytes，
  offset 是 UTF-8 byte；
- `Function { symbol_key }`：key 必须精确 16 bytes。

返回 `ResolvedBreakpoint {breakpoint_id,generation,binding,source?,message?,hit_count}`。
binding：

- Verified：当前代码代精确解析；
- Moved：源码 hash 已变化，但旧 statement fingerprint 在原函数内唯一重绑；
- Unbound：不存在唯一目标，message 说明原因。

一次最多保有 4096 条。当前上限检查在执行 `remove` 之前，用
`requested.len + existing.len` 计算；同批“先删除再增加”仍可能超限。requested 中相同 ID
替换现有记录；协议请求没有 hit-count 输入，新增/替换会把计数初始化为 0。reload 后 VM
按 fingerprint 重绑，命中会增加返回计数并产生 `Stopped(Breakpoint)`。

EraBasic 开发者应从编译后 `DebugSourceLocation`/项目 content hash 建断点，不能以编辑器
字符列直接充当 byte offset；中文和 emoji 尤其不同。

## 9. 安全控制台

控制台必须处于 DebugPaused，并携带 stop：

```rust
Console {
    stop,
    command: ConsoleCommand::Evaluate { source }
             | ConsoleCommand::ExecuteSafe { source }
}
```

`Evaluate` 支持整数/字符串字面量、可见的未索引标量变量、括号、一元 `+ - ! ~`、常见
二元运算、三元表达式和纯方法白名单。整数算术 wrapping；shift count 按 `&63`；
除零产生诊断；相等要求 `DebugValue` 同型相等。

当前白名单 35 项：

`ABS, SIGN, SQRT, CBRT, LOG, LOG10, EXPONENT, POWER, GETBIT, BITCOUNT, STRLEN,
STRLENU, TOINT, ISNUMERIC, UNICODE, CONVERT, COLOR_FROMRGB, MAX, MIN, LIMIT,
INRANGE, TOSTR, SUBSTRING, SUBSTRINGU, STRFIND, STRFINDU, STRCOUNT, STRLENS,
STRLENSU, REPLACE, ESCAPE, UNICODETOSTR, ENCODETOUNI, UNICODEBYTE, CHARATU`。

不支持索引读取、formatted string、省略参数、前/后置增减和不在白名单的方法。
控制台只查询 `variables(..., limit=1024)` 的第一页，因此超过第一页的变量不可见。

`ExecuteSafe` 只接受按第一个 `=` 切分的单条 `变量名 = 表达式`，目标必须是上述可见标量。
没有流程、wait、I/O、host effect 或多语句；变量写使用 VM 的原子 revision 机制。

响应 `ConsoleOutcome` 字段：

- `stop`：若写成功则包含更新的 runtime revision；
- `value?`：Evaluate 成功值；
- `output[]`：当前实现总为空；
- `changed_variables[]`：成功安全赋值的实际结果；
- `changed_game_fields[]`：当前实现总为空；
- `diagnostics[]`：`code,message,source?`。

控制台语法/安全错误通常在 outcome diagnostics 中返回，而不是顶层 DebugError。稳定
code 包括 `debug.console.parse_error`、`unknown_variable`、`unsupported_expression`、
`unsafe_expression`、`unsafe_method`、`unsafe_statement`、`type_mismatch` 和
`execution_error`。

## 10. EraBasic DEBUGPRINT 输出

这是独立于普通 presentation 的 UTF-8 调试文本流：

- `DEBUGPRINT`/`DEBUGPRINTFORM` 连接参数；
- 带 `L` 的变体追加 `"\r\n"`；
- `DEBUGCLEAR` 清空当前窗口并推进 base cursor；
- runtime 最多保留最新 1 MiB，并只在 UTF-8 边界裁剪。

`ReadScriptOutput {cursor,limit}` 不要求 pause；limit 被截到最多 1 MiB。响应
`ScriptOutputChunk {cursor,next_cursor,text,truncated}`。cursor 单位是 UTF-8 byte。
若请求 cursor 已被 clear/裁剪，返回的 cursor 改为当前 base 且 `truncated=true`；若 cursor
落在字符中间，runtime 前移到下一个边界。

`SubscribeScriptOutput {enabled}` 返回 Accepted。启用后，每次追加会主动发送无
correlation 的 `Response::ScriptOutput`；`DEBUGCLEAR` 本身不主动通知，订阅者应在重连
或 cursor 不连续时用 Read 检查 `truncated`。script output 和 subscription 由 runtime
持有，session destroy 后释放；snapshot 会保存当前文本/base，但订阅开关不属于游戏
持久状态。

对脚本作者：`DEBUGPRINT*` 不进入游戏历史，不应承载玩家必须看到的状态；输出可能被
1 MiB 窗口裁掉。用普通 `PRINT*` 表达游戏可观察信息。

## 11. DebugResponse、错误和恢复

`DebugResponse` 线变体：

| tag | 类型 |
| --- | --- |
| 0 | Accepted |
| 1/2 | VariablePage / VariableValue |
| 3/4 | GameFieldPage / GameFieldValue |
| 5/6/7 | FiberPage / CallStack / OperandStack |
| 8 | ConsoleOutcome |
| 9 | `Vec<ResolvedBreakpoint>` |
| 10/11 | VariablesWritten / GameFieldsWritten |
| 12 | ScriptOutputChunk |

`DebugError {code,message}` code：

| code | 语义 | 建议 |
| --- | --- | --- |
| PermissionDenied | 无 grant、grant 陈旧或缺 scope | 等新 Grant/重新 Hello；不要扩大本地权限假设 |
| InvalidState | phase/VM/命令状态不允许 | 刷新 Runtime phase；必要时先 Pause |
| StaleStop | stop 的 epoch/pause/generation/revision 过期 | 丢弃所有 stopped view，等新 Stopped |
| StaleRevision | 乐观写 revision 过期 | 重新读值再让用户确认写入 |
| UnknownTarget | fiber/frame/symbol/key/hash/索引无效 | 刷新对应列表/断点 |
| TypeMismatch | 值类型不符合目标 | 按 descriptor 重新编码 |
| UnsafeConsoleStatement | 协议保留的安全控制台错误 | 当前大部分安全错误在 outcome diagnostics |
| ResourceLimit | page/batch/断点/cursor 超限 | 缩小 limit/batch，分页或删断点 |

DebugError 只拒绝当前调试请求，不会自动 Fault runtime。若 C ABI submit 本身失败，则是
公共信封/sequence/session 问题，应读 `last_error`；若 Runtime 输出 `Fault`，仍按终止
runtime 故障处理，而不是当作 DebugError。

## 12. Rust 和 Python 调用示例

### 12.1 Rust 类型化请求

以下片段展示握手、pause、读变量和 continue 的签名关系；`submit_debug` 负责
`.envelope(...)`、`encode_envelope` 和 `RuntimeSession::submit_envelope`：

```rust
use era_debug_protocol::{
    AuthorizedDebugRequest, DebugCommand, DebugHello, DebugMessage, DebugScope,
    DEBUG_PROTOCOL_VERSION,
};
use era_protocol::VersionRange;

submit_debug(DebugMessage::Hello(DebugHello {
    versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
    requested_scopes: vec![
        DebugScope::ExecutionControl,
        DebugScope::VariablesRead,
    ],
}))?;

let grant = receive_grant()?;
submit_debug(DebugMessage::Request(AuthorizedDebugRequest {
    grant: grant.token,
    command: DebugCommand::Pause,
}))?;
let stop = receive_stopped()?.stop;

submit_debug(DebugMessage::Request(AuthorizedDebugRequest {
    grant: grant.token,
    command: DebugCommand::ListVariables {
        stop,
        cursor: None,
        limit: 100,
    },
}))?;

submit_debug(DebugMessage::Request(AuthorizedDebugRequest {
    grant: grant.token,
    command: DebugCommand::Continue { stop },
}))?;
```

### 12.2 Python 等价调用

Python wire 构造与 Rust 一一对应：

```python
client.send_debug(0, {0: version_range(*DEBUG_VERSION), 1: [0, 4, 5]})
grant = client.wait_debug(1)
client.request_debug(variant(0), grant)                  # Pause
stop = client.wait_debug(12)[0]
client.request_debug(variant(10, stop, None, 100), grant)  # ListVariables
page = client.wait_debug_response(1)
client.request_debug(variant(1, stop), grant)            # Continue
```

完整可运行版本见下一节。

## 13. 待确认

1. 版本协商失败文本位于
   [`debug_session.rs`](../crates/era-runtime/src/session/debug_session.rs)，当前写
   “debug protocol 3.0 is required”，实际常量是 4.0。客户端应依 `DebugGrant.version`/
   协议常量判断，不能解析该文本。
2. C 头只定义 scope bit 0–8 的命名常量，但 `ERA_DEBUG_SCOPE_ALL` 和实际协议还包含
   bit 9 `ScriptOutput`。需补常量或明确由绑定自行定义。
3. `FiberState::DebugPaused` 已在线协议定义，但当前 `protocol_fiber` 没有产生该值。
   需确认它是未来 per-fiber 暂停状态还是应删除。
4. `DebugMessage::Revoke` 是双向 enum，但 runtime 当前只接受前端 Revoke，不主动发送；
   权限更新使用新的 Grant。需确认是否应定义 runtime 主动撤销的时序。
5. breakpoint 上限在 remove 前检查，且更新请求没有 hit-count 输入，替换会归零。需确认
   这两点是否为期望的公开语义。
6. `ConsoleOutcome.output` 和 `changed_game_fields` 当前总为空；已定义字段不能理解为已经
   支持控制台输出捕获或游戏字段赋值。
7. `DebugErrorCode::UnsafeConsoleStatement` 当前没有由 runtime 顶层分发产生；控制台安全
   拒绝主要作为 `ConsoleOutcome.diagnostics`。需确认错误分层。
8. `VariableReference.storage` 当前不参与 VM 引用转换；而变量读/写响应只根据
   fiber/character 判断 storage，因此 FunctionStatic 结果会投影为 Global，即使 descriptor
   曾报告 FunctionStatic。需确认该字段应参与校验，还是修正响应投影。
9. `ListVariables` 的 VM 内部页包含完整 `VmDebugVariableRef`，公共
   `VariableDescriptor` 却丢弃 generation/character/fiber/frame/indices。角色、局部和
   多 generation 条目因而无法由前端唯一引用；需给 descriptor 增加 reference/实例字段，
   或改变列举模型。

## 14. 最小可运行端到端示例

此脚本创建 session、完成 Runtime 握手和最小项目启动、完成 Debug 授权、pause、纯表达式
求值、读取游戏字段、continue、revoke、shutdown，并通过 context manager 释放 session。

准备：

```sh
cargo build -p era-runtime-capi --release
uv sync --project ../rustyera-tui
export ERA_RUNTIME_LIBRARY="$PWD/../target/release/libera_runtime_capi.dylib"  # Linux 改 .so
```

保存为 `/tmp/minimal_runtime_debugger.py`：

```python
from rustyera_tui.abi import AbiError, RuntimeAbi
from rustyera_tui.wire import (
    CHANNEL_DEBUG, CHANNEL_RUNTIME, DEBUG_VERSION, RUNTIME_VERSION,
    debug_message, decode_envelope, encode_envelope, message_value,
    runtime_message, variant, version_range,
)

class Client:
    def __init__(self, abi):
        self.abi = abi
        self.seq = {CHANNEL_RUNTIME: 0, CHANNEL_DEBUG: 0}
        self.next_id = 1
        self.session = None
        self.epoch = None

    def send(self, channel, tag, value):
        mid = self.next_id
        version = RUNTIME_VERSION if channel == CHANNEL_RUNTIME else DEBUG_VERSION
        payload = runtime_message(tag, value) if channel == CHANNEL_RUNTIME \
            else debug_message(tag, value)
        self.abi.submit(encode_envelope(
            channel=channel, channel_version=version, session=self.session,
            sequence=self.seq[channel], message_id=mid, correlation_id=None,
            payload_tag=tag, payload=payload, epoch=self.epoch,
        ))
        self.seq[channel] += 1
        self.next_id += 1
        return mid

    def runtime(self, tag, value):
        return self.send(CHANNEL_RUNTIME, tag, value)

    def debug(self, tag, value):
        if self.session is None:
            raise RuntimeError("Debug requires ServerHello")
        return self.send(CHANNEL_DEBUG, tag, value)

    def pump(self):
        events = []
        while True:
            report = self.abi.drive()
            while (packet := self.abi.poll()) is not None:
                env = decode_envelope(packet)
                value = message_value(env.payload, env.payload_tag)
                self.session = env.session or self.session
                self.epoch = env.epoch if env.epoch is not None else self.epoch
                events.append((env.channel, env.payload_tag, value,
                               env.correlation_id, env.sequence))
            if report.state not in (1, 2):
                break
        runtime_sequences = [event[4] for event in events
                             if event[0] == CHANNEL_RUNTIME]
        if runtime_sequences:
            self.runtime(93, {0: max(runtime_sequences)})
        return events

    def request(self, grant, command):
        return self.debug(10, {0: grant, 1: command})

def one(events, channel, tag):
    return next(value for ch, actual, value, _, _ in events
                if ch == channel and actual == tag)

try:
    with RuntimeAbi(debug_scope_mask=(1 << 10) - 1) as abi:
        c = Client(abi)
        limits = {
            0: 128 * 1024 * 1024, 1: 127 * 1024 * 1024,
            2: 128, 3: 4096, 4: 100_000, 5: 1024 * 1024,
        }
        caps = {
            0: [0], 1: True, 2: True, 3: False, 4: False, 5: False,
            6: False, 7: True, 8: True, 9: [], 10: [],
            11: {0: False, 1: False, 2: False, 3: False},
        }
        c.runtime(0, {
            0: version_range(*RUNTIME_VERSION), 1: "minimal-debugger",
            2: [3, 10], 3: limits, 4: caps, 5: ["zh-CN", "ja"],
        })
        one(c.pump(), CHANNEL_RUNTIME, 1)  # ServerHello

        source = "@SYSTEM_TITLE\nPRINTL DEBUG READY\nWAIT\nRETURN\n"
        c.runtime(10, {
            0: 1,
            1: [{0: "main.erb", 1: 2, 2: variant(0, source)}],
        })
        load = one(c.pump(), CHANNEL_RUNTIME, 11)
        if not load[1]:
            raise RuntimeError(f"project load failed: {load[2]!r}")
        c.runtime(20, {0: variant(0, 1)})
        c.pump()  # 运行到 WAIT

        c.debug(0, {0: version_range(*DEBUG_VERSION), 1: list(range(10))})
        grant_message = one(c.pump(), CHANNEL_DEBUG, 1)
        grant = grant_message[1]
        print("granted scopes:", grant_message[2])

        c.request(grant, variant(0))  # Pause
        paused = c.pump()
        stop_event = one(paused, CHANNEL_DEBUG, 12)
        stop = stop_event[0]
        print("stopped:", stop_event)

        c.request(grant, variant(40, stop, variant(0, "1 + 2 * 3")))
        console_events = c.pump()
        response = one(console_events, CHANNEL_DEBUG, 11)
        assert response[0] == 8  # DebugResponse::Console
        outcome = response[1][0]
        print("console value:", outcome[1])  # Integer(7)

        c.request(grant, variant(20, stop, None, 16))
        fields = one(c.pump(), CHANNEL_DEBUG, 11)
        assert fields[0] == 3  # DebugResponse::GameFieldPage
        print("game fields:", fields[1][0][1])

        c.request(grant, variant(1, stop))  # Continue
        c.pump()
        c.debug(2, {0: grant[0], 1: "example complete"})
        c.pump()

        c.runtime(90, {0: True})
        assert any(ch == CHANNEL_RUNTIME and tag == 91
                   for ch, tag, _, _, _ in c.pump())
        print("shutdown ready")
except (AbiError, ValueError, RuntimeError, StopIteration) as error:
    raise SystemExit(f"debugger failed: {error}") from error
```

运行：

```sh
uv --project ../rustyera-tui run python /tmp/minimal_runtime_debugger.py
```

## 15. 接口索引

- 权限 mask、scope、grant/revoke：第 3 节
- Debug message tag、方向和时序：第 4 节
- StopToken、pause、fiber、stack、step：第 5 节
- EraBasic 变量读取和原子写：第 6 节
- runtime 游戏字段：第 7 节
- source/function breakpoint：第 8 节
- 安全控制台及支持表达式：第 9 节
- `DEBUGPRINT*`/`DEBUGCLEAR`：第 10 节
- response/error：第 11 节
- Rust/Python 代码：第 12、14 节
- 源码不一致和未落实字段：第 13 节
