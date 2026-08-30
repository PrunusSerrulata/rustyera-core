# 蛇版兼容批次 4：完整图形场景与自有存档闭环

## 现状与目标

- 批次 1–3 的输入、定时器、RNG、基础 HTML AST、CanvasReplay、Sprite/Canvas
  资源、投影测量和安全 SQL 子集已经落地；`TEXT_BGC`、`SETANIMETIMER`、
  `GETANIMETIMER` 等只做回归，不重复实现。
- 当前真实缺口是统一场景与行锚点、蛇版 HTML 扩展属性、`HTML_PRINTC/LC`
  像素列、CBG/ImageLayer、Sprite/Canvas 扩展重载、`GETLINEY` 等投影查询，
  以及自有存档对 RNG 和 SQL 修订的原子恢复。
- 冻结审计中的 8,571 条诊断含大量级联错误，不能直接当作批次 4 待办；必须
  先按首个根因和真实可达性重新收敛。
- 蛇版 TW 当前缺少权威 `bbas_map_schema.xml`、`bbas_map.xml`。不得生成替代
  数据；若参考实现的零行分支仍可继续，则按零行结果验收，否则将最终新游戏/
  地图验收明确标为外部资源阻塞。
- 批次完成标准：Web/Tauri 可从真实标题进入新游戏、QOL 与地图交互并完成
  保存—重启—加载；TUI 提供诚实的文本降级或启动前能力拒绝；完整项目可编译，
  真实可达路径不存在未实现陷阱。

## 依赖顺序

`4.0 → 4.1 → 4.2 → 4.3 → {4.4a、4.4b、4.5} → 4.6 → 4.7`

其中 4.4a、4.4b、4.5 在 4.3 的 core 契约提交后并行：分别只修改 TUI、Web、
core，最终在 4.6 统一绑定 SHA。

## 子批次 4.0：基线与契约冻结

### 实施内容

- 固定 core/TUI/Web、蛇版引擎和蛇版 TW SHA，以及字体、视口、工具链和资源摘要。
- 为 `HTML_PRINTC/LC`、扩展 HTML、CBG、ImageLayer、Sprite 8/10 参数、文件
  Sprite、Polygon、RNG/SQL 存档建立最小参考夹具。
- 把旧全量诊断按首个根因、静态可达、动态目标和未来测试代码重新分类，不把
  级联诊断直接转化为实现任务。
- 预检 BBAS 文件；不得修改参考实现、蛇版 TW 或补造缺失资源。
- 过程证据写入已忽略的 `batch-4-work/`，不写入实施日志。

### 出门条件

- 参数、返回值、默认值、深度顺序、坐标系、错误和资源前提均有冻结证据。
- 真实启动、新游戏、QOL、地图和存档路径的可达 API 清单已经确定。

## 子批次 4.1：规范化 HTML 与场景协议

### 实施内容

- 扩展 HTML AST：`font` 的 size/valign/render intent，`img` 的 xpos/ypos、
  width/height、display 和矩阵，`div` 的位置、尺寸、深度、边框、内边距、圆角和
  盒模型。未知属性产生 profile 诊断，不保留任意原始 HTML。
- 将 `ColumnCell.preferred_columns` 改为
  `CellWidthIntent::{ProjectColumns, LogicalPixels}`，使 `HTML_PRINTC/LC` 复用
  现有列单元并保留像素宽度、左右对齐及超宽换行语义。
- 新增统一的 `SceneStateV1` 和增删改 delta；现有背景降为场景层，不再作为第二份
  权威状态。

### 公共场景类型

`SceneLayerV1` 固定包含：

- `layer_id` 和单调 `sequence`；
- `source`：Resource、Sprite 或 Canvas，并携带资源修订；
- `depth`；
- `anchor`：Viewport 或 `DisplayLine(line_id)`；
- logical offset/size；
- `opacity`，范围为 0–255；
- 可选 25 元素、1/256 定点色彩矩阵；
- scroll policy、可选 interaction 和 scene revision。

场景操作固定为 `UpsertLayer`、`RemoveLayer`、`ClearDepth`、
`ClearAnchoredLine` 和 `ReplaceScene`。Snapshot 是唯一权威状态，delta 必须可从
任意合法 revision 重放。

### 出门条件

- CDDL、Rust、JSON/CBOR golden 一致。
- Snapshot 和 delta 重放得到同一场景。
- 原 profile 的既有 HTML/PRINTC 行为不变。

## 子批次 4.2：图形与资源能力补全

### 实施内容

- 实现 `SPRITECREATE` 2/6/8/10 参数：8 参数增加目标偏移，10 参数增加目标尺寸；
  源矩形负方向规范化为翻转，目标尺寸按参考行为取正。
- 实现 `SPRITECREATEFROMFILE`：`isRelative=0` 从项目内容根解析，非零从声明
  脚本目录解析；绝对路径、路径遍历和符号链接逃逸一律拒绝；资源以内容摘要标识。
- 补齐 Polygon 点集的 add/clear/draw/fill replay。
- 复用已有 1/256 定点 5×5 色彩矩阵和动画计时器，不新建另一套表示。

### 出门条件

- 旧重载 golden 不变。
- 新重载、路径安全、摘要去重、翻转、Polygon 和动画与参考夹具一致。
- CanvasReplay 仍执行预算、修订和资源生命周期校验。

## 子批次 4.3：场景运行时与脚本 API

### 实施内容

- 实现 CBG 全组中批次 4 所需的 set/remove/clear/button-map API。`z=0` 按参考
  规则拒绝；CBG 按参考视觉顺序排列，同深度按插入序稳定。
- 实现 `SETIMAGELAYER`、`SETIMAGELAYERL`、clear/all 和
  `EXISTSIMAGELAYER`。同深度允许多层，clear-depth 删除该深度的全部层。
- `SETIMAGELAYERL` 使用当前稳定 `line_id`，不保存易漂移的显示索引；行删除、
  trim 和 clear 时同步释放行锚定层。
- 离屏只停止绘制，不暂停逻辑动画时间。
- 新增修订绑定的 `GetLineGeometryV1`。`GETLINEY` 以显示索引解析稳定行，并验证
  presentation、environment、projection 三重修订。

### 堆叠与查询语义

- 非定位文本是锚点内 depth 0 内容；定位 HTML 和 ImageLayer 进入同一局部堆叠
  上下文。
- CBG、全局背景和 viewport ImageLayer 使用 viewport anchor。
- Runtime 按参考视觉顺序输出协议数组；客户端不得自行重新排序。
- `SETIMAGELAYERL` 不依赖同步测量，直接保存 `line_id`；只有脚本显式调用
  `GETLINEY` 才发起投影查询。

### 出门条件

- CBG、ImageLayer 和定位 HTML 能按参考深度及稳定序列合成。
- `EXISTSIMAGELAYER` 查询实际规范化场景。
- 过期投影回复不能改变 VM 或被后续请求复用。

## 子批次 4.4a：TUI 投影边界

### 实施内容

- 同步新协议和 HTML 像素列，将 HTML/场景投影为确定性的终端文本和单元格近似。
- 保留场景生命周期，但不绘制像素层。
- 不声明物理行坐标、像素级 Scene 或命中测试能力；真实可达的 `GETLINEY` 或图形
  交互要求在启动前给出稳定的 unsupported-capability 诊断。

### 出门条件

- 标题、菜单、文本、按钮和存档流程可用。
- 未支持图形不会伪装成功，重连和重放后文本状态一致。

## 子批次 4.4b：Web/Tauri 场景投影

### 实施内容

- 将背景层、虚拟历史和定位 HTML 合并为单一 compositor。
- 按 scene anchor/depth/sequence 绘制 Canvas、Sprite、动画、opacity、矩阵和
  line-relative layer。
- 扩展现有 pointer/measurement 管线，支持场景命中测试、hover、CBG button map
  和 `GetLineGeometryV1`。
- Web 与 Tauri 共享渲染代码，只保留存储和传输差异。

### 出门条件

- 相同 scene snapshot/delta 在 Chromium、Firefox、Safari 和 Tauri 生成等价的
  逻辑布局与交互结果。
- 重连、resize、scroll、trim、hover 和 click 不丢层、不重复提交输入。

## 子批次 4.5：自有存档闭环

### 实施内容

- 将蛇版自有 `RERASAV` 升级为 envelope v2。内层继续支持 Text1808、
  Binary1808 和 GZip；外层新增 canonical `OwnedSaveStateV1`，校验覆盖 identity、
  状态和内层负载。
- `OwnedSaveStateV1` 包含完整 SFMT 快照，以及按逻辑数据库身份排序的
  `{identity, exact durable revision}` 清单。
- 保存前要求不存在 pending SQL、reader 和 transaction。
- 加载时先在候选状态中恢复变量、角色、自定义数组、GLOBAL、RNG 和精确 SQL
  修订；全部成功后一次提交，失败时保持原会话不变。
- 数据库字节不嵌入存档。原 profile 继续输出裸 1808。
- 旧 snake envelope v1 和外部蛇版存档明确拒绝；外部导入继续留给批次 5。

### 出门条件

- 保存后继续产生一段 RNG 序列，重启加载后产生完全相同的序列。
- SQL 恢复到保存时修订，而不是加载时的 current 修订。
- 缺失修订、损坏 envelope、错误 profile 和活动事务均原子失败。
- A→B→A 项目隔离成立。

## 子批次 4.6：编译与版本收敛

### 实施内容

- 汇合所有目录、签名、类型检查和 lowering；“已登记但运行时未实现”视为失败。
- 静态常量动态调用纳入调用图；无法证明目标集合的动态调用保持阻塞。
- 批次 5–7 API 可以生成显式 unsupported 节点，但若在 Web/Tauri 目标路径可达，
  则项目预检失败，不能运行到 trap。
- 完整源审计仅在本子批次冻结输入后执行一次。
- 将蛇版 `semantic_version` 和 `policy_version` 从 10 统一升至 11，`save_codec`
  改为 `rustyera_envelope_v2:emuera1808`，登记 `rustyera.scene@1` 和
  `rustyera.save_state@1`，使编译缓存、快照和存档全部失效重建。
- TUI/Web 更新到最终 core 完整 SHA 和锁文件；本地联调仍不使用可发布 path
  dependency 冒充正式绑定。

### 出门条件

- 蛇版 TW 全项目解析、分析和编译为零错误。
- 所有可达 host 调用均有实现，或在启动前被目标能力拒绝。
- 缓存命中严格绑定最终 identity、源码和资源摘要。

## 子批次 4.7：真实游戏验收与交付

### 真实路线

使用专属蛇版 TW 副本依次执行：

1. 真实标题。
2. QOL/SQL 初始化。
3. 新游戏。
4. 地图与状态 UI。
5. hover、click 和 NF。
6. 自有保存。
7. 关闭客户端并重启加载。
8. 比较变量、角色、GLOBAL、RNG、SQL 和场景。

Web/Tauri 是完整图形验收目标；TUI 验证文本路线和能力边界。

### 性能与缓存

- 冻结输出后执行一次冷启动、两次暖启动和一次源码摘要失效测试。
- 记录编译缓存命中、耗时、峰值 RSS 和输出一致性。
- 暖启动必须真实命中且快于冷启动。
- 当前没有可靠 RSS 基线，不设置虚构的内存阈值；必须记录实测值，且不得触发既定
  资源配额或泄漏门禁。

### 出门条件

- Web/Tauri 完成整条路线且没有 trap。
- TUI 不虚报像素能力。
- Scene delta 前端无关，保存恢复精确。
- 完整审计和全量 E2E 各只启动一次。
- 如果 BBAS 权威资源仍缺失且零行路径不能继续，只将该最终门禁标记为外部阻塞，
  不用替代数据制造通过结果。

## 验证、提交与记录

- 每个包含产品代码的子批次，按受影响组件单独执行一次
  `$refactor-rustyera-code` 审查，并在任何测试前落实全部要求。
- 测试分别使用 `$test-rustyera-core`、`$test-rustyera-tui`、
  `$test-rustyera-web`；每条测试命令委派给 gpt-5.6-terra low。
- 每个子批次使用独立 60 分钟测试预算；先定向单元、契约和静态门禁，再执行该
  子批次唯一一次适用全量。失败后只复验最小受影响集合。
- Web/Tauri 动态测试执行每 5 秒完整 DOM/runtime 快照看门狗。
- 每项功能单独提交，跨组件分别提交。至少分离 scene 协议、HTML/查询、图形资源、
  CBG/ImageLayer、TUI 投影、Web/Tauri compositor、envelope/RNG、SQL 存档绑定、
  编译收敛和最终文档。
- 中间状态只保留在已忽略证据目录；`SNAKE_EMUERA_IMPLEMENTATION_LOG.md` 及批次
  总览只在整个批次 4 最终结束时更新。
- 最终按组件列出实际 SHA、首次全量、定向复验、真实差异和未验证项。行为变化写入
  根 `CHANGELOG_PENDING.md` 并单独提交。

## 明确不纳入批次 4

- 不实现外部蛇版/ERAZIP 存档导入、Float、完整音频、渲染器像素级提示或跨客户端
  像素一致性。
- 不复刻蛇版二进制/mtime 懒加载与 `EXISTFUNCTION` 副作用；继续使用完整静态符号
  清单和内容摘要缓存。
- Bitmap cache 指令如为源码编译所需，只提供带诊断的兼容 no-op，不伪造缓存副作用。
- 不修改蛇版 Emuera、蛇版 TW 或补造缺失 BBAS 数据。
- 不复用其他工作区的构建产物、虚拟环境或运行会话。
