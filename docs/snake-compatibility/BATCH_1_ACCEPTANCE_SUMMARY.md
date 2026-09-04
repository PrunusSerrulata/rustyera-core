# 蛇版兼容批次 1：验收汇总

状态：**批次 1（1A–1D）已完成；以下参考差异与后续阻塞保持明确。**

各子批次最终结果、首次全量与定向复验结论及证据入口见[实施记录](SNAKE_EMUERA_IMPLEMENTATION_LOG.md#batch-1)，
范围与门槛见[实施方案](BATCH_1_IMPLEMENTATION_PLAN.md)。本页只汇总当前有效结论，
不把首次失败改写成通过。

## 已实现范围

| 子批次 | 产品范围 | 当前结论 |
|---|---|---|
| 1A | 三端 ALS/ERD 摄取、稳定合并、用户别名、反向名称、缓存失效及资源清单 | 已完成，保留参考差异与边界 |
| 1B | GETMETH/GETMETHS 惰性调用、EXISTMETH、Integer/String 返回及数组 REF | 已完成；后置实参/Float/元素 REF/OUT 不在本批 |
| 1C | 列 DEFAULT、XML/structured 数据、GLOBAL、Data→Resource 安全读取与枚举 | 已完成，已分项提交 |
| 1D | HTML v2、真实 pointer、独立 canvas replay 采样、过期回复拒绝 | 四端基础、生命周期、普通服务与无进展断面均已验收；差异见下文 |

TUI 本批明确拒绝未实现的 HTML 像素测量、pointer、canvas pixel 服务，不宣告支持。
未实现 SQL、蛇版算术/RNG、Float、新标签或 scene；不以真实标题或 GRAPH_DB_INIT 已运行
作为结论。缺少 `bbas_map_schema.xml`、`bbas_map.xml` 仍是后续初始化资源阻塞。

## 首次全量与定向修复

| 范围（1D） | 首次全量 | 后续有效定向结果 |
|---|---|---|
| core workspace | 通过 | b8b5bee 只读故障后调试等定向通过 |
| 独立 runtime-tester | 57 通过 | coverage/看门狗修复及 TW v3 实际报告另列 |
| Oracle Python | 34 通过 | typed identity、失败观察比较及省略/null边界定向通过 |
| TUI pytest | 480 通过，5 跳过 | opt-in 缺能力5/5、debug3项、源码及打包数据断面通过 |
| Web Vitest | 1209 通过，1 失败 | mediaImage 隔离修复后定向20通过；不称修复后全量通过 |
| Web Rust | 130 通过，1 ignored | 对应类型、lint、格式、构建与core pin门禁通过 |

后续每次修复均先恢复受影响静态门禁，再运行定向动态。1D 唯一重构审查 R1–R6 已在
首次测试前落实，未再启动审查；没有借修复重置全量次数。用户授权的TW重跑单独逐次记录，
最新repeat4完成后未再运行。测试无批次总时限，五秒完整快照及单命令限制保持有效。

## 当前客户端动态证据

本地证据根为同组 `batch-1-work/1D/`；原始输入、结果与摘要保留，不提交本机产物。

| 客户端 | 基础/组合断面 | 最新生命周期 | 普通服务与双Oracle对照 |
|---|---|---|---|
| Chromium | repair62当前源码组合通过 | repair58，exit0，17.62s | 34项完成 |
| Firefox（安装的原生应用） | 数据/服务组合通过 | repair57，exit0，22.01s | repair51/52，34项完成 |
| Safari（安装的原生应用） | 数据/服务组合通过 | repair57，exit0，10.58s | repair53，34项完成 |
| Tauri | repair56 services/batch1均exit0 | repair56，exit0，11.13s | repair76对照34项完成，18份修复后+16份有效旧证据 |
| TUI | repair36源码/打包组合及能力缺失通过 | 不要求周期DOM快照 | 不伪装为像素服务对照 |

生命周期均验证6组独立pointer观察与2组真实图片解码竞态；不得用返回值反推预期。
Tauri repair56的未挂载显示canvas数量为0、blocked=[]。修复了原生测试cursor不移动、
resize旧pointer前置、浮层遮挡视口中心和测试环境导致隐式重建等问题。测试输入修复不代表
修改产品滚动语义；任何PTY退出回报回收都不是重复构建或重测理由。

三个已完成浏览器的普通服务对照各为 **22 matched_observables、6 incomparable、6 different**。
12项非匹配逐项核对实际watches与终止状态，保留原机器结论；涉及错误呈现观察限制、
显式资源限额、字体/平台像素差异及不切断有效Unicode字符的有意差异。
普通参考34次执行已完成并固定复用，不重复启动Wine。
Tauri普通对照原始分类为 **7 matched_observables、21 incomparable、6 different**；
额外不可比来自缓存就绪info，未过滤或改写。混合样式像素为85（浏览器84、原版104、蛇版88），
其余副作用与错误边界保留。逐项结果见repair76-tauri-offline-result.json。

无进展危险断面8/8已验证runtime明确html.query.NoProgress与RESULT:10=777。两个参考均成功
加载后在相同run输入停滞，五秒完整快照看门狗终止，均exit1/无可比返回值，保留失败；
不称oracle通过。Rust按本批要求有界报错，这是已登记的安全差异。未更改参考实现或重跑。

## API与执行证据映射

以下为执行断面入口，不以源码命中代替运行。文件SHA256明确标为单文件摘要，完整fixture
及实际构建清单保留在各次capture/trace中。core执行绑定b8b5bee；浏览器WASM摘要为
`bbe455923aca722a49c8f4dde3cc35498393455eae7f3a301b2f9d9d439205bf`。

| API范围 | core链路 / 前端契约 | 最小执行入口与有效证据 | 边界 |
|---|---|---|---|
| ALS/ERD、ERDNAME、GETMETH | 摄取→索引→analyzer/compiler→VM；无需像素服务 | `fixture-snake-batch1-clients/ERB/main.erb`；三浏览器、Tauri repair56及TUI repair36组合 | 1B完整签名/惰性/REF证据仍见其35项fixture |
| LOADTEXT/ENUMFILES、MAP/XML/DT、DEFAULT、GLOBAL | storage命名空间与structured数据；三端实际存储 | 同一main.erb；读取overlay、默认值与普通变量不恢复均有实际输出 | SQL、缺少地图文件未执行 |
| HTML_STRINGLEN | `presentation_query/html_string_len/2.0`；compiler/runtime→规范树→DOM测量 | `fixture-snake-batch1-clients/ERB/services.erb`；Chromium repair62、Firefox/Safari及Tauri repair56 | 字体/平台像素差异保留 |
| HTML_SUBSTRING | `presentation_query/html_substring/2.0`；core保持原文本切分与RESULTS | 同一services.erb；Unicode、实体、标签断面对照见普通34项 | 有效Unicode不切半；Tauri矩阵已收齐，原差异保留 |
| HTML_STRINGLINES | `presentation_query/html_string_lines/2.0`；core逐行推进 | 同一services.erb；空串/多行及求值次数已执行 | 8个host/profile均明确NoProgress与777；参考停滞失败单列 |
| MOUSEX/MOUSEY/MOUSEB | `input_state/pointer_state/1.0`；实际viewport与脚本按钮值 | `fixture-snake-service-lifecycle/ERB/main.erb`；repair56/57/58四端6样本、2解码竞态 | TUI明确缺能力，不提供伪坐标 |
| GGETCOLOR | `canvas/sample_canvas_pixel/1.0`；指定revision独立replay | services.erb；红/蓝ARGB及未挂载canvas、过期解码拒绝 | TUI明确缺能力；错误与资源限额保留 |

上述单文件SHA256：

- batch1 `main.erb`：`22fcde4c6014a6c3b7cc25905ffc944dc89fd14b2dccb5de40f968388a056eeb`
- batch1 `services.erb`：`16c6f1ef87f8378706aa5018e426423454b67bf205a7688127d0aa82741de831`
- lifecycle `main.erb`：`86c1de8601a4a1ee69d1b237bdfb950a414dd8813135661a7db2adce3b242274`

## 蛇版 TW 覆盖报告

repeat4全量审计2012.55s，随后流式处理89.49s；15,761个输入、0读取失败、20 ALS、2 ERD，
5,069,815行、112,727函数、44,431引用。原始报告4,668,806,077 bytes，gzip150,730,066 bytes。

- 原始SHA256：`b4dd4441e7f0e731fd1434fecfb54a24f3f706640b4a54ab6960f362a57700db`
- gzip SHA256：`d365704edff41c8e47b3179b52db8a7b0239d469d001765efa337f9d9d45c88b`

默认binary=false审计保留8198个CHARADATA错误；正确配置的最小binary=true断面编译0错误、
别名执行通过。这里是完整摄取和可追溯覆盖报告，不是完整游戏编译/运行通过。

标题静态切片保留321个静态闭包函数、41,885条引用和611个目标解析记录；
GRAPH_DB_INIT切片保留11个静态闭包函数、475条引用和21个目标解析记录。
两者均为`static_slice_not_execution`；动态目标保守保留，不转称已执行或已证明可达。
报告区分运行时整数插值、未解析表达式、局部标签解析及无符号候选等原因；
无效parser span不作为有效执行证据。原始输入排除901项、用户变量7,983个的清单仍在压缩报告。

## 绑定与收尾

发布core pin为 `b8b5bee45d1a7d3fc31f4df42dcbe0048422794a`；TUI最终提交
`ad5c018b7c73bac441a9064d3339a174eff7dcfa`，Web最终提交`e3633311233df4a502faa41b32d8807c8c38de33`。
core收尾工具提交`e919d3719a2b0f5394c545783caa27289dcd7f7d`未改产品crate，后续记录提交不移动前端pin。
本次31个分项提交的标题、动机、改动与验证绑定见[交付提交](BATCH_1_DELIVERY_COMMITS.md)。
捕获时HEAD为6d378650且有dirty源码；最终Web提交的文件内容与该已验证组合按hash对应，
不以旧HEAD单独代表测试源码，不为仅Git提交重复构建。

repair59–62修复仅测试会话的诊断路径与Worker转移后证据读取，147项定向及相关静态门禁
通过；repair74窄窗口Grid修复后当前Tauri二进制53,903,424 bytes，SHA256
`e6355b5425d1e25926871b1f4f9cecea4a3d80147dd81a966c3b3f58e77bc6be`，provider52来源不变。
矩阵使用严格cache-only入口，不允许隐式重建；旧fe664…生命周期证据仍记录原绑定。
repair63归档和脚本已运行，但调试响应先到导致typed检查超时；未产出capture，不能记为通过。
repair64修复通用提交登记竞争，4项state与33项store断面分首次/定向复验通过，相关静态及构建通过。

分项提交、源码/证据绑定和根更新日志已完成；根提交`40fdea805c2fac3065a69fe541b8dd6265046efb`。
repair77来源审计与repair78定向修复检查通过。14个补丁空白context行保持原SHA，
仅通过精确文件属性说明合法补丁语法，不修改provider或重建产物。
本批无未处理的必要实施项；SQL、缺失地图资源、后置语言/存档/标签与真实可玩性仍归后续批次。
没有推送、合并主线或调整产品版本；磁盘低于20GiB时串行构建，只清理本任务已替换的
可再生缓存，所有批次0证据与续做材料保留。
