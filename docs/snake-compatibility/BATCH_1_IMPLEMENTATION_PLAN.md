# 批次 1 详细实施方案

用户于 2026-08-27 确认实施。实际进度和证据只记入
[实施记录](SNAKE_EMUERA_IMPLEMENTATION_LOG.md#batch-1)，本文不是验收通过证明。

## 范围与执行约束

- 目标：完整摄取、动态方法、初始化数据读取、已有展示服务接线，为批次 2–4 提供基础。
- 基线：core `45cf4fa143545ec70b757ad72de875784b7b0e20`，TUI
  `7fa8ea07b19886da547ce23e4b03b0922268f76a`，Web
  `56b43b445ae72f932bcfce34618fbd6993d410bb`；开工三个专用 worktree 干净。
  前端 core pin `8862fa957f67bae553cb8e30fa349b113745aa3f` 与当前 core 之间无产品 crate 差异。
- 四个独立实施子批次，分别且仅一次重构审查，全部要求落实后才启动测试。
- 用户明确取消本任务所有子批次的测试总时限；单套全量最多一次、静态先于动态、
  五秒卡死看门狗和测试执行者 `gpt-5.6-terra / low` 的约束保持不变。
- Browser/Tauri 实现真实服务，TUI 对缺少的 HTML 像素测量、pointer、canvas pixel
  不宣告能力并提供明确诊断；不新增终端字符格投影。
- SQL、Float、variadic、元素 REF、OUT、蛇版算术/RNG、新增 HTML 标签和 scene 不在本批。
- 不以完整游戏编译、真实标题或 GRAPH_DB_INIT 已运行作为本批验收结论。
- 用户要求 Chromium 复用本机现有可执行文件，禁止下载；浏览器 session/profile 与测试输出继续隔离。

## 1A：完整摄取与 ERD/ALS（大，S01/S02）

### 摄取

追加 FileCategory Als=6、Erd=7，既有 0–5 编号保持。ERD 是索引数据，不是声明源码，
不得进入脚本 analyzer。ALS/ERD 按 UTF-8 处理并保留路径、payload hash 和 I/O 错误。
同步 TUI、Browser、Tauri 的完整/快速扫描、延迟读取、manifest 编解码、分块传输、
source index、增量重载、compiled cache source hydration。沿用 canonical-root 及扁平
fixture 规则；ERB 子目录递归 ERD，ALS 关联同目录同 stem 数据表。

纳入 XML/TXT/数据库 seed 的只读资源清单，复用 Resource/ExternalResource；DLL 不因此
成为可执行扩展。原字节 hash 与提交 UTF-8 payload hash 分开报告。安全检查覆盖读取失败、
扫描中变化、路径归一化冲突和根外符号链接，不能静默漏文件。

### 数据语义

- 复用既有 #DIM 注册；一维 NAME，多维 NAME@1..@N，CHARADATA 角色索引不计入数据维。
- 同一 compatibility policy 透传 CSV 初始加载和 analyzer deferred resolution。
- snake 先 ERD 组、后 CSV 组，组内稳定路径排序；所有主表合并后再按顺序加载同目录 ALS。
  稳定排序是明确的跨平台确定性选择，不声称复制操作系统枚举顺序。
- 用户 ALS trim、空名跳过、不覆盖主表、重复 alias first-wins；孤立 ALS 不声明变量。
- 内置 ALS trim、重复名称和同 index 多名规则按 profile 区分，原版既有行为保持。
- 有符号 lookup 保留合法 i32 alias；负数/超维度 alias 不无符号转换，实际数组访问仍检查边界。
- 显式保存主表优先、插入序 first-match 的反向名称，防止 ERDNAME 按名字排序选到 alias。
- 尊重 UseERD，不引入脚本 rename 替换或改变其他旧文件类别的编码政策。

验收：真实蛇版 TW 的 20 ALS、2 ERD 全部摄取；BUFF、COLUMNDIV@2、SEMEN_MATRIX@2、
index 10/11/300、二维/三维/CHARADATA、重复与错误路径；ALS/ERD 增删改令 cache 失效，
cold/warm/reload 一致。S01、S02、资源清单基础按功能且按仓库分别提交。

## 1B：动态方法（大，S03）

- GETMETH(name[, fallback, args...])、GETMETHS、EXISTMETH 建立准确签名，保留 omitted/place。
- GETMETH 在通用参数预求值前专门 lowering：先名称及目标解析，缺目标只求 fallback，
  无 fallback 报错；有目标不求 fallback，先校验方法种类、返回类型和签名再求值与执行。
  错误种类/返回类型/签名不得伪装 missing。
- typed resolve/invoke 表达式方法路径复用函数查找、REF 和 frame，返回值入 caller stack。
  新路径显式 omitted，不把 i64::MIN 当省略；validator 检查 payload、分支栈及返回类型。
- EXISTMETH 不执行函数体，按零实参解析返回 Integer=1、String=2、无法解析=0。
- 仅 Integer/String、已有数组 REF；当前非 variadic policy 不扩展到批次 2 的语义。
- 可达性包含 formatted expression，不裁掉计算名称目标；memo 保守分类，查询绑定 generation。

验收覆盖惰性与副作用顺序、缺失/错误目标、省略与 REF、递归/深度、reload、优化一致性。
CAN_MOVE_*、ODEKAKEMAP_SETTING_* 使用真实调用形式的最小断面，不执行 SQL。

## 1C：列选项、GLOBAL、安全读取（中到大，S12/参考补齐）

- DT_COLUMN_OPTIONS table, column, DEFAULT, value... 专用关键字语法，仅实现固定基准 DEFAULT。
- 列保存 typed default，新行未赋值列应用 default，修改不回填旧行；Integer/String、null、
  XML schema、structured snapshot/GLOBAL 均同步；缺表/列、非法选项/类型走稳定错误。
- snake 文本读取由 runtime 明确 Data -> 仅 NotFound -> 清单 Resource；其他错误不 fallback。
  SAVETEXT 只写 Data，integer 文本编号仍 Save，Resource 不可写/删。原版行为保持。
- EXISTFILE/递归 ENUMFILES 使用同规则，稳定合并，Data 覆盖同路径 Resource；检查限额、
  symlink 逃逸/循环、归一化冲突及扫描后资源变化。
- GLOBAL 复用已有实现，验证 missing/roundtrip/普通变量隔离/损坏/profile/structured restore。
- 使用实际存在的 schema.xml、bbas_dataset.xml 和最小 MAP/XML/DT fixture。缺失的
  bbas_map_schema.xml、bbas_map.xml 作为后续资源阻塞，不生成假文件令真实初始化通过。

列选项和安全资源读取分别提交；小型修复和 GLOBAL 验证共享本子批次审查/测试。

## 1D：已有服务与集成（大，S04）

- HTML 三 operation 升 v2；core erabasic-html 规范树、源映射、样式、projection context
  交前端按现有 HtmlNode 测量，返回实际测量及合法边界。禁止另写 EraHTML parser/innerHTML。
- core 负责切分、RESULTS[0/1]、实体及标签闭合输出；避免通用序列化改变脚本可见字符串。
  明确 UTF-8/DOM UTF-16/节点坐标，不拆有效 Unicode。STRINGLEN 首显示行、半角/像素规则；
  STRINGLINES 空串/换行/无进展切分；无进展明确失败，不无限循环。新增标签留批次 4。
- 补 core pointer_state 协商；Web/Tauri 宣告已实现服务。MOUSEB 为规范按钮脚本值，
  MOUSEY 为客户区左下原点；覆盖 scroll/resize/blur/leave/no-hover。
- canvas 服务复用 replay renderer，独立受限画布重放指定 revision 返回 ARGB，不依赖 DOM
  可见窗口；查询前 flush，覆盖刚打印/绘制即查询。
- 校验 request ID/epoch/三 projection revision/canvas revision，取消/切项目/异步解码
  不接受旧回复；区分 unsupported/invalid/stale/resource-limit/backend-failure。
- TUI 不宣告上述缺少服务，验 profile、operation、version 的准确诊断。

HTML、pointer、canvas 分别提交。最终组合 fixture 在三端验证
ALS/ERD -> 动态方法 -> 资源读取 -> MAP/XML/DT -> GLOBAL，图形服务只在 Browser/Tauri。

## 公共契约、覆盖与门禁

- FileCategory 变更升级 runtime protocol 并同步全部 CBOR/TS/Python/host；ProjectData、
  structured state、ISA/VM/compiler 格式变更升级相应版本，旧 cache/snapshot 明确拒绝。
- snake identity 体现 ALS 新 policy，继续实验标记，不提前改变 arithmetic/RNG/layout/save。
  HTML v2 复用 ServiceKind；未变 payload 的 pointer/canvas 保持原 operation version。
  无 C 函数表布局变化不无故升级 C ABI。
- core 契约验证提交后机械同步前端完整 SHA、rev、lock，重建本组 C ABI/WASM/Tauri。
- 流式覆盖保留全部出现点、未调用函数、动态名称候选及未知原因；新清单包含变量/ERD/alias/
  方法，标题及 GRAPH 引用；每 API 标 analyzer/compiler/VM/service/frontend 及后续批次。
  源码字符串命中和注册不代表执行；无效 span 不升级有效证据。
- 每子批次全部实质代码完成 -> 唯一 refactor skill 独立审查 -> 落实全部要求 -> 静态。
  测试执行由 gpt-5.6-terra low 只读代码的子智能体负责。单套全量最多启动一次，失败只定向复验。
- core fmt/check/clippy/minimal -> full；工具独立 workspace 另验。TUI minimal pytest -> full、
  Ruff、真实 C ABI、pin 变化的打包/加载。Web minimal Vitest -> full、typecheck/lint/format/
  build/WASM、适用 Rust workspace 和 core rev 检查。
- 全部相关静态及共享 core 门禁通过后，双 oracle smoke/同输入差分、真实 Chromium/原生
  Firefox/Safari/真实 Tauri/TUI。原版与 snake 分开记录；不要求跨字体平台像素完全一致。
- Web/Tauri 五秒完整 DOM/runtime 看门狗不变，TUI 稳定等待观察，core/oracle 遵循自身规则。

## 调度、磁盘、交付

1A -> 1C；1B 可独立开发；1D 最终集成等待 1A–1C。共享协议/版本/lock 由主智能体串行整合，
不修改正在验证的输入；最终集成属于 1D，不另建测试批次重置次数。

开工可用 35 GiB，本组 target 15 GiB、批次 0 evidence 2.1 GiB。最多两路构建，优先串行复用
本组经核验缓存；真正并行才隔离 target，绝不使用主工作区产物。每子批次及大型步骤前后查盘，
低于 20 GiB 减并行清本任务可再生旧产物，低于 10 GiB 暂停新增高写入任务并释放空间。
报告直接流式 gzip，保留双摘要；不删批次 0 证据、用户数据或其他任务产物；无法安全释放时
报告阻塞。保留合理单命令超时，不以延时掩盖无进展。

各子批次回写 SHA/hash、审查、首次全量、定向复验、动态证据和未完成项。最终根 changelog
只记已完成产品行为；各功能各仓库单独提交，不 push、不 merge、不顺带改产品版本。
全部必要门禁满足才标记批次 1 完成。
