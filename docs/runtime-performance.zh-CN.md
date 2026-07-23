# Runtime 冷启动性能报告

本文记录完整 eraTW 从内存项目提交到进入日 1 稳定输入菜单的性能基线、瓶颈和优化结果。
性能优化不得改变 EraBasic 可观察行为；本文数字用于发现数量级问题，不作为跨机器的固定
时限测试。

## 测量范围

本地 release 构建使用 `reference/eraTW` 的完整 UTF-8 文件集，并执行以下流程：

1. 前端侧枚举并读取项目文件，构造 `ProjectManifest`；
2. runtime 加载 CSV、分析 ERH/ERB、编译并验证字节码；
3. 以固定 seed 启动新游戏，按 caller-pumped 契约驱动 VM；
4. 为 clock、storage 等请求返回固定测试结果，直到日 1 菜单的稳定 `WaitingInput`。

Rust 二进制自身的构建时间不计入结果。测试没有清空操作系统文件缓存，因此“冷启动”指
没有字节码/增量编译缓存、没有已有 runtime session 的应用级冷启动。计时工具位于本地被
`tools/runtime-playability-audit/`，其生成物由局部 `.gitignore` 隔离，不会成为产品前端或
runtime 的依赖。

## 结果

同一机器上的代表性 release 运行如下（单位为秒）：

| 阶段 | 原始基线 | 第一轮优化 | 当前结果 | 说明 |
|---|---:|---:|---:|---|
| 文件准备 | 0.417 | 0.439 | 0.125 | 前端侧读取和消息构造 |
| 项目加载 | 28.400 | 29.180 | 9.261 | CSV、analyzer、compiler、validator |
| 启动至日 1 菜单 | 290.901 | 3.988 | 2.106 | VM 初始化、系统流程和展示输出 |
| 总计 | **319.720** | **33.609** | **11.367** | 到稳定 `WaitingInput` |

相对原始基线，总耗时下降约 96.4%；相对第一轮优化结果又下降约 66.2%。当前结果已经稳定
越过 60 秒硬目标和 30 秒进阶目标。不同运行仍会受文件系统缓存、CPU 调度和内存压力影响，
因此本文保留分阶段数字，而不把某一次最低值设为跨机器的固定阈值。

真实 Python TUI、C ABI、队列和展示模型均纳入后，使用三个互不复用持久目录的真冷样本，
从 worker 启动到日 1 菜单分别为 **14.39、14.35 和 14.54 秒**；三个最终 release 样本都通过
`ERA_AUDIT_MAX_DAY1_SECONDS=15` 的显式门槛。该门槛是本机端到端回归指标，不宣称为不同
硬件的统一性能承诺。

当前项目加载的单独分段样本为：文件准备 0.428 秒、CSV 0.005 秒、analyzer 1.708 秒、HIR
验证 0.097 秒、compiler 4.888 秒，共编译 58,349 个函数。相同测量方式下，analyzer 此前为
12.356 秒，compiler 此前为 10.703 秒。剩余冷启动成本主要是完整 artifact 的确定性身份计算、
结构验证和实际系统流程；CSV 已不是可见瓶颈。

当前 compiler ABI 32 以带长度、枚举标签和独立版本域的 canonical binary 编码计算大规模函数
与源码映射身份，避免为了摘要把数百万个操作数字节转换成 JSON 十进制文本。分块大小和最终
合并顺序固定，因此并行度不影响 artifact/execution ID；版本升迁会按设计改变产物 ID，旧容器、
编译缓存和旧 VM snapshot 不会被静默当作当前格式恢复。

## 内存与 snapshot 优化

使用同一 release 构建和同一条完整 eraTW 游戏路径测得：

| 指标 | 优化前 | 当前 | 变化 |
|---|---:|---:|---:|
| `heap -s` live malloc bytes | 2,910,947,728 | 1,875,191,264 | -1,035,756,464（-35.6%） |
| 等待输入时 RSS（相邻版本） | 约 5.67 GB | 约 3.99–4.58 GB | 至少约 -1.09 GB |
| 完整运行峰值 RSS | 约 6.03–6.43 GB | 5.56 GB | 至少约 -0.47 GB |
| 源码映射条目 | 5,355,958 | 4,295,566 | -19.8% |
| `SourceMapEntry`（64-bit） | 112 bytes | 64 bytes | -42.9% |
| `VmValue`（64-bit） | 96 bytes | 24 bytes | -75.0% |

RSS 会包含 macOS malloc 保留但当前不再使用的页，且完整运行的峰值仍可能由编译阶段主导，
所以 live heap 是观察常驻对象变化的更稳定指标。当前完整 eraTW 的项目加载约 6.5 秒，测试
前端到达日 1 菜单约 13.35 秒，仍远低于 30 秒进阶目标，说明紧凑存储没有用明显启动回退换取
内存下降。

真实 eraTW 在标题选择的稳定脚本输入点导出的 runtime/VM snapshot 为 133,468 bytes；同一
payload 的未压缩 JSON 长度为 434,246 bytes（不含 60-byte 容器头），压缩后体积约为原来的
30.7%。该快照已通过 C ABI、前端分块传输和恢复测试，恢复后仍停在相同的稳定整数输入等待。
日 1 runtime 自有系统菜单当前返回 `SnapshotStateUnavailable`，因此没有把该不合资格状态
伪装成日 1 snapshot 样本；这属于既有 snapshot 资格/系统流程问题，而非编码器或恢复失败。

本轮采用的内存策略如下：

- 将相邻且源码 origin 完全相同的指令映射合并为半开 code range，并用确定性去重表保存
  statement fingerprint；普通条目不再内联 32-byte digest，也不为空 origin chain 常驻 `Vec`。
- runtime 增量编译状态只保存函数 cache key 和精确 artifact 身份；未变化函数从 runtime 已经
  持有的相同 artifact 物化，不再常驻第二份完整函数、导入表和源码映射。
- VM 的不可变 program generation 通过 `Arc` 在隔离候选之间共享，避免仅为试运行候选深复制
  数百万条指令和索引；热替换提交会移动已验证 artifact，并在所有可恢复检查结束后原地迁移
  memory，不再同时保留完整旧/新 artifact 克隆和第二份完整游戏状态。validator、runtime 和
  活跃 VM generation 也共享同一份已验证 artifact，启动 VM 不再复制全部函数和源码映射。
- dense variable cell 按声明类型分别保存 `Vec<i64>`、`Vec<String>` 或 place descriptor；只有
  操作数栈、Host/调试接口和传统存档边界才构造公共 `VmValue`。角色变量、全局数组、局部变量
  和引用参数仍走原有 shape/type 检查。
- VM snapshot 8 和 runtime snapshot 15 使用流式、确定性的 zlib best-compression 容器，记录
  压缩/展开长度并校验压缩 payload 的 BLAKE3。恢复时同时限制输入和最大展开长度，防止伪造
  长度导致无界解压；格式版本不符时继续拒绝恢复。

## 已实施优化

- 展示输出以完整 snapshot 建立同步基线，随后只发送变化行和变化字段的 delta，消除了每次
  `PRINT` 都复制并投影全部历史所形成的二次增长。该 wire 行为由 runtime protocol 20.0
  引入并由 protocol 22.0 延续，重同步仍发送完整 snapshot；Protocol 22 的 `TrimLines` 让
  runtime 在不改变 `LINECOUNT` 的前提下执行 `MaxLog`，长时间游戏不再无限保留物理历史。
- VM 为函数、全局变量、函数静态/局部变量、指令 byte offset 和源码映射建立每代只读索引，
  热路径不再反复线性扫描完整 artifact。
- 常见指令的 operand 使用栈内小缓冲，避免解释每条短指令时分配新的 `Vec`；超长 operand
  仍自动使用堆存储，编码和执行语义不变。
- 常见角色增删操作先完整验证参数，再原地提交，不再为每条操作复制完整 artifact 和 VM
  memory；可能在重排过程中失败的 `PICKUPCHARA`、`SORTCHARA` 仍保留事务副本。
- runtime 自有的单 cell 同步（包括 `LINECOUNT`、菜单结果和 `SAVEDATA_TEXT`）先完成变量、类型
  与坐标检查，再直接提交唯一写入，不再为每次输出或菜单输入复制完整 VM memory；批量写入、
  fill 和角色变更仍保留原有事务边界。
- compiler 和 runtime 将字节码所有权直接移交 validator，并把增量补丁基线压缩为非函数
  元数据；函数与源码条目复用既有函数缓存，避免大项目的重复深复制。
- analyzer 只在函数确有私有 `#DIM/#DIMS` 时构造全局常量查询表。eraTW 大多数函数没有私有
  声明，避免了对完整常量表约 5.8 万次无效复制和排序；需要常量维度的声明仍走原解析路径。
- ERH 继续严格按源顺序解析并建立宏/变量环境；环境固定后，ERB 文件使用 copy-on-write
  parser context 并行解析。indexed parallel collection 保持源码、函数和诊断的确定顺序。
- compiler 在同一个 indexed parallel pass 中完成函数缓存键序列化、缓存命中判断和未命中
  lowering，避免所有 worker 启动前的串行函数哈希阶段；并行度不同仍产生逐字节相同的容器。
- analyzer 在符号表和函数私有变量预处理完成后并行分析互不依赖的函数体，indexed collect
  保持 HIR 与诊断顺序确定；compiler 和 artifact 身份计算把 JSON 直接流入 BLAKE3，避免为
  完整 HIR、源码映射和函数体创建大型临时序列化缓冲。
- TUI 使用持久化 stat/hash 源码索引。普通启动和重启只重新读取签名变化的 UTF-8 文件，先以
  轻量源码身份尝试载入缓存；只有缓存不匹配时才物化并传输完整 manifest，显式“重新载入所有
  脚本”仍执行完整内容扫描。
- 编译缓存 v4 将 core metadata、globals、增量状态、ProjectData、源码记录、statement
  fingerprints、规范化项目快照、函数和源码映射拆成可独立解压的确定性 section。函数和源码映射
  各最多使用 32 个固定顺序 shard；常见指令 operand 保持内联，源码映射按函数分组并使用 varint，
  fingerprint 直接保存为 32-byte binary。所有 section 仍受展开长度上限和整体 BLAKE3 摘要约束，
  解码后仍执行完整的不可信字节码结构、ABI 与内容身份验证。
- 完整 eraTW 的 v4 产物为 93,390,358 bytes；相对 v3 的 124,534,530 bytes 缩小约 25.0%，相对
  同一链路的 v2 样本 143,674,679 bytes 缩小约 35.0%。前端继续在游戏启动后后台编码；本机冷样本
  10.15 秒到达日 1、20.86 秒完成缓存落盘，编码不位于首次可玩路径。成功写入 v4 后会清理旧
  v1/v2/v3 文件。
- 同一全新 v4 缓存在测试负载结束后连续十次启动都明确报告 `runtime.compiled_cache_hit`，从 worker
  启动到首次 `InputWait` 为 **1.50–1.57 秒**，中位数 1.51 秒、p90 1.55 秒、最大值 1.57 秒，
  10/10 低于 2 秒。这里的首次等待是热启动性能目标；没有通过跳过脚本等待、信任磁盘字节码或
  延后 validator 来缩短数字。
- 保存/读取菜单的目录 List 只返回廉价 stat change token，runtime 只为当前页的已占用槽位按
  64 KiB 稳定区间增量解析存档头；删除时才对单个目标执行完整摘要 Stat，避免打开菜单时读取、
  哈希并解码全部存档。
- 完整 eraTW 的“道具确认[805]”专项测试在 23.91 秒进入稳定等待，展示历史为 230 行，低于
  runtime 的 5000 行上限；页面完成后没有继续增长、watchdog fault 或 TUI 错误。
- compiler 构造的内存 artifact 在生成 ID 前走专用结构验证路径，最终身份只计算一次；runtime
  接收同一进程内的 compiler-owned artifact 时重复结构检查但不再重复序列化完整源码映射。
  解码、磁盘或网络输入仍必须走不可信字节码入口并复算 `execution_id` 与 `artifact_id`。
- `SymbolKey` 的 JSON 十六进制编码改用栈上查表，不再为源码映射中的每一个键调用格式化系统并
  分配临时字符串。输出仍是完全相同的 32 位小写十六进制 JSON，因此无需改变 ABI 或已有 ID。

## 行为安全与后续方向

展示 delta 通过“顺序应用后与同 revision snapshot 的可见状态完全一致”测试；增量补丁仍需
重建出与 clean build 逐字节一致的 artifact；VM 角色失败回滚、UTF-8 源码定位、函数调用、
存档恢复和 runtime 生命周期测试继续覆盖优化路径。字节码指令、编译确定性、游戏状态判定
和参考实现语义均未因计时结果而放宽。

日 1 初始化后半段包含脚本要求的 clock/wait 推进；为缩短基准而跳过这些等待会改变可观察
行为，因此没有采用。持久化字节码缓存显著改善第二次启动，但它属于内容与版本严格命中后的
暖启动方案，不能冒充本文的无缓存冷启动结果；缓存未命中、损坏、版本不符或身份不一致时仍
拒绝加载并回到完整源码路径。
