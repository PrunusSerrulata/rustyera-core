# 输入与等待兼容性

本文记录 runtime 对输入操作的预期行为，参考基准为 Emuera 提交
`26a35dc9334bb67590b96f7b8efbefbf199e391e`。当前 analyzer、compiler、协议、VM
能力检查以及基础 runtime 等待/服务链路均已存在。

## 临时等待的含义

`Transient` 是 VM snapshot 对等待中 Host 操作的分类，表示该操作不能从精确 VM
snapshot 重新绑定。它包括活动 deadline、新发出的前端服务查询和不可恢复的 Void
等待，但不保证操作一定会结束。

稳定等待没有 deadline，只能由携带相同等待身份的版本化用户输入消息恢复。

## 参考行为

| 操作 | 参考行为 | 等待分类 |
| --- | --- | --- |
| `TINPUT` | 参数为 `time, default[, display, message, mouse, canskip]`；产生整数输入，并以 `mouse == 1` 表示鼠标输入。 | 提供第六个参数且 message skip 已启用时不等待；否则 `time > 0` 为临时等待，`time <= 0` 为稳定等待。 |
| `TONEINPUTS` | 使用相同六个参数槽位的字符串版本并设置 `OneInput`；长默认值会保留。 | 与 `TINPUT` 相同。 |
| `TWAIT` | 参数必须是 `time, flag`；`flag == 0` 请求 Enter，其他值请求 Void。 | `time > 0` 为临时等待；`time <= 0 && flag == 0` 为稳定等待；`time <= 0 && flag != 0` 为不能生成 snapshot 的临时等待。 |
| `FORCEWAIT` | 不接收参数，请求 Enter，并设置 `StopMesskip` 以结束当前 message-skip。 | 稳定输入等待。 |
| `GETKEY` | 接收一个整数。前端未激活或键码不在 `0..=255` 时返回 0，否则按 pressed bit 返回 0 或 1。 | 非法键码立即返回；合法键码发起新的前端查询，在响应前属于临时等待。 |
| `TINPUTNF` | 固定参考实现未定义此操作。 | analyzer 报错，不创建等待。 |

参考指令不会把 `canskip` 的值本身求值为 Boolean；只要提供了第六个参数槽，并且
message skip 已经生效，就允许走快捷路径。构造请求所需的参数仍会在选择快捷路径前
完成求值。`display` 默认启用，timeout message 默认使用配置中的超时标签。

## Runtime 现状

runtime 在发布 `InputWait` 前决定 `WaitStability`。正的毫秒限制转换为 monotonic
deadline；零和负值不创建 deadline。timeout 会提交类型正确的默认值，按配置处理展示
与消息，然后恢复 fiber。

对 `GETKEY` 而言，范围内键码会发出版本化的 `InputState/get_key_state` 服务请求；
runtime 只有在收到相关响应后才完成 VM Host 请求。runtime 持有与
`GETKEYTRIGGERED` 共用的逐键 toggle 观察状态。前端未激活时必须返回 0，VM 和 runtime
库本身不得读取操作系统输入 API。

只有稳定输入等待允许精确 snapshot。传统 Era 存档不会序列化 VM stack 或等待中的
输入/服务请求。

当前 runtime 已实现类型化等待、正值 monotonic deadline、默认值、timeout message、
`FORCEWAIT` 标志、`TWAIT` Void 分类、新鲜 `GETKEY` 查询、共享的
`GETKEYTRIGGERED` toggle 观察，以及有/无时间限制的 message-skip 快捷路径。

基础鼠标与键盘输入以经过前端规范化的 EraBasic 结果字段到达 runtime；runtime 仍会
验证 wait、token、epoch 与顺序，并独自合成 timeout。这样既把平台事件解释留在前端，
又保持游戏结果的权威性。

one-input 等待会把非空手工文本规范化为第一个 Unicode scalar。无时间限制的空输入、
timeout 和 message skip 使用完整默认值。只有 `AllowLongInputByMouse` 启用时，语义
按钮 `Activate` 才能保留多字符值；手工 `CommitText` 不享有这一例外。这一设计把参考
UI 的物理鼠标差异映射为可移植交互意图，同时避免为非 BMP 输入生成无效 UTF-8
surrogate。
