# 蛇版 Emuera、蛇版 TW 与 RustyEra 兼容性详查

> 调研日期：2026-08-25\
> 性质：源码与资源的只读静态审计，不是运行通过证明\
> 目标：回答蛇版 TW 的实际依赖、蛇版 Emuera 相对参考实现的增量，以及 RustyEra 的对应缺口

文中的源码/资源路径以多组件工作区根目录（`rustyera-core` 的上一级）为基准；省略游戏前缀的 `ERB/` 等路径相对于所述游戏目录，不相对于本文目录。审计日期、revision 与“当前”状态均指本次历史审计，不代表后续实现进度。

## 0. 结论先行

当前 RustyEra **不能装载并运行蛇版 TW**。最先遇到的不是少数画面差异，而是多层硬阻塞：

1. 蛇版 TW 在标题函数中就无条件调用 `QOL_DB_INIT`，连接 SQLite 并导入翻译 MAP；新游戏、读档公共初始化又调用 `GRAPH_DB_INIT`。RustyEra 的分析器/编译器没有这些 `SQL_*`、`SQL_P_*`、`SQL_READER_*` 方法与指令，项目在编译阶段即会失败。
2. 同一条初始化链会使用参考实现已有的动态方法调用 `GETMETH`；蛇版 TW 其他主流程还大量使用 `GETMETH/GETMETHS/EXISTMETH`。RustyEra 虽在分析器目录中接受这些名字，却没有 compiler/runtime 实现，会生成 unsupported trap。即使先移除 SQL，仍不能完成初始化。
3. 蛇版 TW 的主要地图/UI 使用 `TINPUTSNF`、`MOUSEB/MOUSEX/MOUSEY`、`HTML_STRINGLEN/HTML_SUBSTRING/HTML_STRINGLINES`、CBG/位图和蛇版 HTML 布局扩展。RustyEra 对其中一部分完全不识别；另一部分在 core 有请求模型，但 Web/TUI 没有协商或实现所需服务，调用会 fault。
4. 蛇版 TW 开启 `USELAZYLOADING:YES`，把 2,370 个、约 155 MiB 的个性口上文件放入延迟目录。RustyEra 有自己的项目编译选择与缓存机制，但没有蛇版的 `lazyloading.cfg`、两份二进制索引、首次调用加载及 `EXISTFUNCTION` 触发加载语义。它不一定是语义上的第一条错误，却是该 176 MiB ERB 项目的现实可用性门槛。
5. 蛇版 TW 使用 20 个 `.als`、两份 `.ERD`，并有自定义变量 `BUFF` 的 `BUFF.csv`/`BUFF.als`。RustyEra core 能处理部分 CSV deferred index 和内置 ALS，但 TUI、Web 浏览器、Tauri 的项目扫描器均不提交 `.als` 或 `.erd`；core 也未实现蛇版“用户 ERD 同名 ALS”的完整语义。

需要特别避免两个误判：

- 蛇版 Emuera 的全部 83 个新增表达式方法和 23 个新增命令，并不都是当前蛇版 TW 的必需项。Float、`VARIADIC`、`OUT`、元素 `#REF` 等是引擎能力，但在当前游戏脚本中没有找到对应声明语法。
- RustyEra 中出现了某个名字、协议类型或 host 注册项，不代表功能可用。本报告把 analyzer、compiler、VM/native、runtime service 和最终前端逐层核验。

## 1. 范围、基线与判定方法

### 1.1 审计对象

| 对象 | 审计基线 | 说明 |
|---|---|---|
| 蛇版 Emuera | `emuera_lazyloading_selfmodified_version`，HEAD `fc4fb21416768c17256d0e82f997e5f99c9bba91` | 版本签名 `1824+v24+EMv18+EEv56+Skiav12` |
| 参考实现 | `emuera.em`，HEAD `af9886061ba420d530581e7975c4db735c391d03` | 产品版本 `1.824+v24+EMv18+EEv56`；EEv56 产品基线 commit 为 `26a35dc9334bb67590b96f7b8efbefbf199e391e` |
| 蛇版 TW | `games/eratw-sub-modding`，HEAD `667b9cd0...` | 4,100 个 ERB/ERH、229 个 CSV 类数据文件、约 176 MiB ERB、约 566 MiB 图像资源 |
| RustyEra core | `rustyera-core`，HEAD `637a084...` | README 的兼容目标仍是固定参考实现，不是蛇版；当前工作树含用户未提交改动，本次只读 |
| RustyEra TUI | `rustyera-tui`，HEAD `8c6b0e3...` | 当前工作树含用户未提交改动，本次只读 |
| RustyEra Web/Tauri | `rustyera-web`，HEAD `af5d54f...` | 当前工作树含用户未提交改动，本次只读 |

参考仓库 HEAD 在 EEv56 之后的主要变化是本工作区 reference CLI/headless 适配。蛇版没有可用于单次 `git diff` 的连续共同历史，因此本报告采用三重证据：当前活动注册表集合差、双方具体实现差、蛇版提交史/说明交叉确认。

### 1.2 “必须”的四级定义

| 等级 | 含义 | 本报告如何认定 |
|---|---|---|
| P0 启动硬依赖 | 新游戏/读档或全项目编译必经；缺失即无法进入可玩状态 | 事件调用链、无条件初始化、分析器/编译器对全项目处理 |
| P1 主玩法依赖 | 地图、交互、常用 UI、存读档等正常游玩可达；缺失会很快卡死或破坏主要功能 | 活跃调用及其上层系统用途 |
| P2 条件功能依赖 | DLC、调试、某菜单、某角色/口上或配置开启时才需要 | 活跃源码存在，但不是启动必经 |
| P3 体验/规模依赖 | 不必然改变脚本结果，但影响启动耗时、内存、画面、音频或操作体验 | 资源规模、配置和引擎实现 |

“静态词法命中”只说明源码中出现了名字。行首 `;` 注释、说明文字、演示函数、永不可达分支不自动升级为依赖。动态 `CALLFORM`、运行时字符串、宏和 `STRFORM` 又可能令静态搜索漏报，因此所有“未发现”都表示当前树静态审计未发现，而不是形式化证明不存在。

### 1.3 没有执行的操作

本次没有启动蛇版 Emuera、参考实现或 RustyEra，也没有运行构建、测试或端到端游戏流程；没有修改三个实现仓库和游戏仓库。因而：

- “实现存在”是源码链路结论，不等于像素/时序/存档的行为差分已经通过；
- “P0/P1”来自调用可达性和实现层审计，不是动态覆盖率；
- 运行期才拼出的函数名、具体 XPath/SQL 数据边界仍需后续差分测试。

## 2. 蛇版 TW 实际需要什么

### 2.1 标题与初始化调用链：SQL 是确定性第一阻塞项

SQL 在进入新游戏之前就已执行：

- `ERB/TITLE.ERB:27` 无条件 `CALL QOL_DB_INIT`；
- `ERB/魔改内容/qol/qol_db.ERB:49-58` 初始化天赋、物品、药品、料理、虫、木材、寻路图模块，随后 `SQL_CONNECT "TR_DB"`，并用两次 `SQL_IMPORT_MAP_XML` 导入 `plugins/tw_csv_chs.xml` 与 `plugins/tw_taste_chs.xml`；
- `TITLE.ERB:12-15` 还会执行 `LOADGLOBAL/SAVEGLOBAL/LOADGLOBAL`；
- `TITLE.ERB:60,80,85,87` 分别涉及标题 WebP、扩展 `<div>`、`GETPLATFORM` 和桌面路径的 `TINPUTNF`。

因此标题首屏本身已要求：SQL、GLOBAL storage、递归 resource CSV、WebP/HTML 布局，以及平台判断/NF 输入的至少一个可用分支。

`ERB/SYSTEM.ERB` 的新游戏和读档事件都会进入 `INIT_NG_OR_LOAD`：

- `@EVENTFIRST` 在 `SYSTEM.ERB:8-10` 调用它；
- `@EVENTLOAD` 路径在 `SYSTEM.ERB:33-73` 调用它；
- `@INIT_NG_OR_LOAD` 在 `SYSTEM.ERB:160-175` 无条件调用 `GRAPH_DB_INIT` 和 `CREATE_BBAS_DATABASE()`。

`ERB/魔改内容/qol/qol_graph_init.ERB:34-55` 的 `GRAPH_DB_INIT` 随即执行：

- `SQL_CONNECT`；
- 多次 `SQL_EXECUTE_NONQUERY` 建表；
- `SQL_P_EXECUTE_SCALAR_STRING` 检查 schema 版本；
- 版本不一致时进入事务，重建图、全源 BFS、跨地图边及地点属性；
- 重建过程还使用参数化 nonquery/reader、reader 遍历、long/string 读取及 close。

这一数据库不是可有可无的调试缓存：`qol_graph_query.ERB` 的移动距离、可达点、视野、跨地图耗时等主要地图查询都读取它。`plugins/qol_data.db` 是游戏随附资源；另有药品、料理、木材、虫类等 QOL 模块复用同一 SQLite 文件。

`CREATE_BBAS_DATABASE` 则在 `ERB/BODY_INFO/BBAS_DATASET.ERB:1-17` 读取 `plugins/schema.xml`、`bbas_dataset.xml`、`bbas_map_schema.xml`、`bbas_map.xml`，使用参考系的 `LOADTEXT`/XML/MAP/DT 能力建立身体数据集。它不是蛇版新增 API，但必须保留其数据语义。当前仓库没有找到后两个 `bbas_map_*.xml`；原引擎对缺失 `LOADTEXT`/空字符串及随后 `DT_FROMXML` 的实际处理需要动态核验，不能在静态报告中假设成功。

结论：要让原样游戏显示并操作标题，再完成首次初始化，至少必须实现游戏实际使用的 SQL 子集，并保证 GLOBAL storage、`GETPLATFORM/TINPUTNF`、`GETMETH/GETMETHS`、MAP/XML/DT、文本读取与标题图像/HTML 同时可用。

### 2.2 蛇版专有 API 的游戏使用矩阵

下表优先使用排除行首注释后的活动代码行数；同一行多次调用只计一行，且 `[SKIPSTART]` 中的代码仍可能被高估。代表位置另行核对了上下文。零活动命中项目移至 2.8，完整新增 API 见第 3 节。

| 蛇版能力 | 活动代码行 | 活动证据/用途 | 必要度 |
|---|---:|---|---|
| `SQL_CONNECT` | 7 | `qol_db.ERB:56`、`qol_graph_init.ERB:37`；另见 dish、wood、mushi、pharmacy | P0 |
| `SQL_EXECUTE_NONQUERY` | 42 | 图数据库建表、事务、清表、VACUUM | P0 |
| `SQL_EXECUTE_READER` | 9 | 图及 QOL 表遍历 | P1 |
| `SQL_EXECUTE_SCALAR_STRING` | 20 | schema 和业务查询；另有参数化版本 | P0/P1 |
| `SQL_P_EXECUTE_NONQUERY` | 18 | 图边、距离、属性以及 QOL 数据写入 | P0/P1 |
| `SQL_P_EXECUTE_READER` | 8 | 寻路图与 QOL 数据读取 | P1 |
| `SQL_P_EXECUTE_SCALAR_LONG/STRING` | 32 / 38 | 版本、距离、配方、物品等查询 | P0/P1 |
| `SQL_READER_READ/CLOSE` | 17 / 18 | 所有 reader 生命周期 | P1 |
| `SQL_READER_GET_LONG/STRING/ISNULL` | 53 / 26 / 5 | 图与业务表列读取 | P1 |
| `SQL_IMPORT_MAP_XML` | 2 | `qol_db.ERB:57-58` 导入翻译映射 | P0 |
| `TINPUTSNF` | 10 | `qol_MAP.ERB:40-42,394-396,677-679,...` 地图动画/悬停循环 | P1 |
| `TINPUTNF` | 1 | `TITLE.ERB:87` 的桌面标题定时输入 | P0 |
| `GETPLATFORM` | 1 | `TITLE.ERB:85` 选择桌面 NF 输入或普通输入 | P0 |
| `HTML_PRINTC/HTML_PRINTLC` | 3 / 1 | `HTML_PRINT_Components.ERB:127-129`、`QOL_USERCOM.ERB:115` | P1 |
| `TEXT_BGC_ON/OFF` | 2 / 1 | 全行文本背景组件 | P2 |
| `BITMAP_CACHE_ENABLE` | 6 | QOL 图片、彩色地图与口上颜色 | P1/P3 |
| `SETANIMETIMER` | 19 | 地图/界面动画节拍；蛇版使用命令语法 | P1/P2 |
| `GETANIMETIMER` | 4 | 动画状态读取 | P1/P2 |
| `SEQUENCEINPUT` | 1 | 快速施法开启时向下一次输入等待注入序列 | P2 |
| `DISABLE_INPUT_MACRO` / `ENABLE_INPUT_MACRO` | 1 / 1 | 录入宏文本时局部禁用、恢复输入宏 | P2 |
| `GETSOUNDORBGMINFO` | 2 | 音乐补丁的播放状态、时长与进度 | P2 |
| `COS` | 1 | 局部图形/动画几何 | P2 |
| `UNCHECKED_ADD/UNCHECKED_MUL` | 1 / 2 | `COMMON.ERB:3001-3002 @NOISE` 明确依赖溢出哈希，人物体型计算可达 | P1 |
| `EXISTVAR(name,1)` | 4 | `Misc.ERB:694,721,741`、`QOL_能力表示.ERB:300` 动态存储单元/排序 | P1 |
| `GETDISPLAYLINE(负数)` | 1 | `Misc.ERB` 从显示历史尾部取行 | P2 |
| `TRYCCALLSTR` | 1 | `SHOP関連/TEST.ERB:13` 隐藏开发测试 | P3 |

注意：蛇版的 `COS` 等数学方法可接受 Integer 并动态决定结果类型；仅有普通数学调用不能推出游戏需要 `#DIMF` 或 Float 存档。

### 2.3 参考实现已有、但蛇版 TW 同样依赖的高风险能力

兼容目标不能只做“蛇版新增接口”。蛇版 TW 已经依赖若干 RustyEra 当前也不完整的参考能力：

| 能力 | 游戏证据 | 用途/可达性 | RustyEra 后果 |
|---|---|---|---|
| `GETMETH/GETMETHS` | `qol_graph_init.ERB:355,369`；`COMMON.ERB:4364-4391`；地图修复、口上、事件等大量调用 | 启动时构图会动态调用 `CAN_MOVE_*`、`ODEKAKEMAP_SETTING_*` | analyzer 接受，compiler trap；P0 |
| `EXISTMETH` | `QOL_USERCOM.ERB:166` | 动态命令选项探测 | compiler trap；P1 |
| `HTML_STRINGLEN` | 标题、地图、状态、通用布局等大量调用，例如 `TITLE.ERB:75` | 像素宽度、按钮布局、居中、折行 | core 发 service，但 TUI/Web 未协商，运行 fault；P0/P1 |
| `HTML_SUBSTRING` | `Toolkits.ERB:621`、`String_Layout_&_Format.ERB:166,489` | 保持标签闭合的像素宽度截断 | 两前端缺 service；P1 |
| `HTML_STRINGLINES` | `Flan_UI.ERB:131,215` | 弹窗高度与换行 | 两前端缺 service；P1/P2 |
| `MOUSEB` | `qol_MAP.ERB:50,404,687,901,...` | 地图悬停/点击循环 | core 请求 `pointer_state`，两前端未协商；P1 |
| `MOUSEX/MOUSEY` | `QOL_USERCOM.ERB:243-249`、开锁 DLC | 弹出菜单定位、开锁角度 | 同上；P1/P2 |
| CBG | `QOL_IMAGE.ERB:1747,1773` 的 `CBGCLEAR/CBGSETG` | 角色/场景图像 | RustyEra runtime 仅真实处理 `CBGCLEAR`；P1/P2 |
| `GGETCOLOR` | `ステータス表示関連/縁取り.ERB:173` | 像素 alpha 检测和描边 | core 需要前端 `sample_canvas_pixel`，两前端缺；P2 |
| `DT_COLUMN_OPTIONS` | `MOVEMENTS/物件関連/MAP_NODE_TO_XML.ERB:26` | DataTable 列设置 | 编译为 native，VM dispatcher 无实现；P2 |

这也说明“SQL 是第一阻塞项”不等于“加一个 SQLite 包即可启动”。图数据库重建函数内部的动态方法调用本身就是另一个 P0。

#### 动态语言和结构化数据的规模

蛇版 TW 广泛依赖动态调用，而不是少量孤立调用：排除行首注释后约有 `CALLFORM` 248 行、`TRYCALLFORM` 1,245 行、`TRYCCALLFORM` 149 行、`GETMETH` 26 行、`GETMETHS` 11 行、`GETVAR/GETVARS/SETVAR` 16/7/4 行。lazy 口上入口 `SYSTEM.ERB:140` 和更新菜单 `UPDATE_MAIN.ERB:919` 都用 `TRYCCALLFORM M_KOJO...`。

声明/预处理规模同样很大：约有 `#DIM` 22,469 行、`#DIMS` 9,077 行、`#FUNCTION` 2,660 行、`#FUNCTIONS` 1,871 行、`[SKIPSTART]` 3,144 处、`[IF_DEBUG]` 41 处。兼容器必须在预处理之后判断活动代码，且让动态名称与 lazy 函数索引协同。

新游戏/读档公共初始化还会无条件建立标准 MAP、DataTable 和 XML 数据。静态活动代码中，标准 MAP 的 create/get/set/keys/has/size/release 等有数十行；DataTable 的 `DT_CELL_GET` 约 85 行、`DT_COLUMN_ADD` 27 行、`DT_SELECT` 19 行；XML 的 `XML_ADDNODE` 约 68 行、`XML_GET` 30 行。这些大多不是蛇版新增 API，但其数据语义是启动和主玩法必需。

### 2.4 HTML、图像、字体与资源格式

蛇版 TW 的资源与 UI 设计明显以蛇版的 Skia 渲染栈为基线：

- 3,681 个资源文件中约 2,924 个为 WebP，另有约 594 个 PNG、35 个 MP3；
- `emuera.config` 选择 `RENDERING BACKEND:OpenGL`、Skia 高质量图像、subpixel antialias，并设置 SimHei；
- 脚本使用 `<font size=...>`；该 `size` 是蛇版新增的像素字号语义；
- 脚本使用 `<img>`/`<div>` 的 `xpos`、`ypos`、`width`、`height` 等像素布局，例如 `TITLE.ERB:80`、`TEMP_CHARA_LIST_BY_DT_FUNC.ERB:103`；
- `Flan_UI.ERB:163` 等使用 ARGB 颜色；`ARGB_TO_HTML_COLOR` 在当前游戏中是 ERB helper，不是当前蛇版引擎 API；
- 游戏活动玩法使用 sprite、G 位图、CBG、HTML img/div；`SETIMAGELAYERL` 的 4 条真实指令仅位于 `Flan_UI.ERB:153,296,314,325` 的测试/未来 UI 函数，未发现文件外调用；
- `M_KOJO_K140_イベント.ERB:264` 的 `SPRITECREATE` 是 6 参数形式，参考实现已经支持，不能据此要求蛇版新增的 8/10 参数形式。

要达到“能玩”，Web/Tauri 是比 TUI 更现实的承载端：Web 已有 Canvas2D、WebP/PNG、普通 sprite 和音频链路；TUI 会忽略图片且没有 graphics/audio service。但 Web 现有 HTML 模型也只覆盖固定标签集合，并未实现蛇版 font/img/div 的全部属性和 CBG 语义。ImageLayer 可放在主玩法之后，但若 RustyEra 全量编译未调用测试函数，仍需至少识别其语法或按可达性安全跳过。

### 2.5 Lazyloading 是规模门槛，也是可观察语义

游戏根目录的 `lazyloading.cfg` 只指定：

```text
口上・メッセージ関連\個人口上
```

该目录包含 2,370 个 ERB/ERH，约 155 MiB；整个 ERB 树约 176 MiB。随附索引大小约为：

- `lazyloading.bin`：14,392,411 bytes，约 96,425 条函数→文件索引；
- `lazyloadingfiles.bin`：273,227 bytes，约 2,205 个文件项。

蛇版启动时跳过索引中的 ERB，首次 `CALL` 或 `EXISTFUNCTION` 时按函数→文件映射加载；文件变动时增量更新索引；事件函数和包含 `#FUNCTION` 的文件不能延迟。这同时影响：

1. 启动时间和峰值内存；
2. “尚未加载但确实存在”的函数可见性；
3. `EXISTFUNCTION` 的副作用；
4. 修改口上文件后的索引一致性。

游戏有 9 行活动 `EXISTFUNCTION`，用于更新菜单口上选项、立绘版本、Sex Modules、宴会和 OBJ；lazy 口上又通过 `TRYCCALLFORM M_KOJO...` 动态进入。因此 lazy-aware `EXISTFUNCTION` 不是单纯性能优化。

RustyEra 的 compiled cache、analysis selection 或 ignore-uncalled 能减少部分工作量，但不是同一协议。若选择“一次性编译全部口上”，必须用真实性能数据证明在浏览器、Tauri 和 TUI 都可接受；若选择兼容蛇版 lazyloading，则需实现等价的函数发现与按需编译，而不能只读取两个旧二进制索引。

### 2.6 ERD/ALS 与数据名

当前游戏：

- `USE ERD:YES`；
- 有 20 个 `.als`；
- 有两份位于 ERB 子目录的 `.ERD`：`ERB/カラム機能/COLUMNDIV@2.ERD`、`ERB/魔改内容/SEMEN_MATRIX@2.ERD`；
- 有 `CSV/BUFF.csv` 和 `CSV/BUFF.als`；
- `ERB/DIM.ERH:567` 声明 `#DIM CHARADATA SAVEDATA BUFF,50`；
- 游戏还提交了 `ERB/Headers/AutoConst_BUFF.ERH` 等生成常量，能覆盖部分用途，但不能把它等同于完整的运行时别名解析。

蛇版相对参考实现增加“用户定义 ERD CSV 旁的同名 `.als`”加载，并支持多维别名字典。RustyEra core 的 deferred-index loader 能把 CSV 顶层未知表按 `#DIM` 注册解析，也能加载部分内置 ALS；但：

- TUI `project_scan.py:196-206` 不分类 `.als`/`.erd`；
- Web browser `browserProjectFilesystem.ts:21-42` 不分类 `.als`/`.erd`；
- Tauri `src-tauri/src/project/scan.rs:337-386` 不分类 `.als`/`.erd`；
- core 的内置 ALS 加载不等价于蛇版用户 ERD ALS 扩展。

这些 ALS 不是只有前十项：例如 `Item.als` 约 324 项、`CFLAG.als` 约 305 项、`FLAG.als` 约 244 项；蛇版还修复了参考实现对序号 10 之后字符串指针/别名读取的问题。`CFLAG.als:70` 的 `300,现在位置` 在 `UFUFU_LOG.ERB`、自室描写等文件中有活动使用。

因此应把“先让前端提交文件”“正确处理较大 ALS”“再补用户 ERD ALS 语义”作为独立任务。两份 `.ERD` 使 `.erd` 扫描也是当前快照的直接装载需求。

### 2.7 存档与路径约定

游戏配置使用 `sav/`、二进制存档和压缩存档；仓库随附 7 个 `.sav`，文件头均为 `\x89ERAZIP\n`。标题还会处理 GLOBAL，QOL 存档 UI 调用 `LOADDATA/SAVEDATA`，并有 `CHARADATA SAVEDATA BUFF` 等用户变量。

因此应区分两个里程碑：

- “新游戏进入可玩状态”可以先不承诺导入现有 `.sav`，但仍需 GLOBAL 和自身存档闭环；
- “完整运行现有蛇版 TW”需要 ERAZIP 压缩二进制、用户 SAVEDATA/CHARADATA/ERD 数组、槽位约定和蛇版 RNG 状态的兼容声明。

### 2.8 当前未发现为游戏必需的蛇版能力

对 4,100 个 ERB/ERH 的静态搜索未发现以下能力的正常玩法依赖；其中一部分完全零命中，一部分仅出现在注释、说明或测试函数：

- `#DIMF`、`#FUNCTIONF`、`#REFF`；
- `LOCALF`、`ARGF`、`RESULTF`；
- `VARIADIC`、`ARGF` 可变参数；
- 元素 `#REF/#REFS/#REFF`；
- `#DIM/#DIMS/#DIMF OUT`；
- Float 存档数据的直接证据；
- 蛇版 `SPRITECREATE` 8/10 参数形式；
- 蛇版新增的 `MAP_VALUES/MERGE/REMOVEIF/FINDKEY/TOSTRING/FROMSTRING`：未发现活动调用，词法命中来自注释/说明；
- `SETIMAGELAYER`：未发现活动指令；`SETIMAGELAYERL` 只有 4 条测试/未来 UI 指令，未发现正常玩法调用；
- `SIN/TAN/FLOOR/ROUND/GETLINEY`：未发现活动调用，原始词法命中来自注释、说明或样例；
- `EVAL/EVALS`、strict font fallback、运行时 Skia quality/text drawing control；
- `SOUNDCONTROL/BGMCONTROL/ISPLAYINGSOUND/ISPLAYINGBGM`；
- `CALLSHARP` 的活动调用。游戏虽然随附 `plugins/MathParser.org-mXparser.dll`，但没有发现静态 `CALLSHARP` 使用，不能仅凭 DLL 文件宣称 CLR 插件是必需项。

这些仍是“完整兼容蛇版 Emuera”的缺口，但不应进入当前蛇版 TW 的运行期 P0 实现序列。需要注意：RustyEra 若不按可达性跳过未调用函数，测试函数内的未知指令也可能成为全项目编译阻塞。动态构造与未来游戏更新仍可能改变结论。

## 3. 蛇版 Emuera 相对参考实现新增了什么

### 3.1 精确 API 集合

以当前活动注册表比较，蛇版新增 **83 个表达式方法**、**23 个命令式指令**。参考实现注册为表达式的 `SETANIMETIMER`、`BITMAP_CACHE_ENABLE` 在蛇版被迁移为命令，不是功能删除。

#### 新增的 23 个命令

- 动态调用：`CALLSTR`、`JUMPSTR`、`TRYCALLSTR`、`TRYJUMPSTR`、`TRYCCALLSTR`、`TRYCJUMPSTR`
- NF 输入：`TINPUTNF`、`TINPUTSNF`、`TONEINPUTNF`、`TONEINPUTSNF`
- 图像图层：`SETIMAGELAYER`、`SETIMAGELAYERL`、`CLEARIMAGELAYER`、`CLEARIMAGELAYER_ALL`
- HTML/文字：`HTML_PRINTC`、`HTML_PRINTLC`、`TEXT_BGC_ON`、`TEXT_BGC_OFF`、`STRICT_FONT_FALLBACK`
- 渲染：`SET_SKIA_QUALITY`、`SET_TEXT_DRAWING_MODE`
- 动画/缓存：`SETANIMETIMER`、`BITMAP_CACHE_ENABLE`

#### 新增的 83 个表达式方法

- Float/数学：`TOFLOAT`、`TOSTRF`、`SIN`、`COS`、`TAN`、`ASIN`、`ACOS`、`ATAN`、`FLOOR`、`CEIL`、`ROUND`、`UNCHECKED_ADD`、`UNCHECKED_SUB`、`UNCHECKED_MUL`、`UNCHECKED_NEG`、`GETVARF`、`GETMETHF`、`DT_CELL_GETF`
- 动态求值：`EVAL`、`EVALF`、`EVALS`、`ARGLEN`、`STRFORMCHECK`
- 数组/角色 CSV：`MATCHALL`、`MATCHALLEX`、`GETCSVNOBYNAME`、`GETCSVNOBYNICKNAME`、`GETCSVNOBYCALLNAME`、`GETCSVNOBYMASTERNAME`
- bit：`BITSET`、`BITGET`、`BITTOGGLE`、`BITINDEXOFFIRST`
- 图形/平台：`SPRITECREATEFROMFILE`、`G_POLYGON_DRAW`、`G_POLYGON_FILL`、`G_POLYGON_POINT_ADD`、`G_POLYGON_POINT_CLEAR`、`EXISTSIMAGELAYER`、`GETLINEY`、`GETANIMETIMER`、`GETPLATFORM`、`GET_TEXT_DRAWING_MODE`、`GET_SKIA_QUALITY`
- 输入：`SEQUENCEINPUT`、`DISABLE_INPUT_MACRO`、`ENABLE_INPUT_MACRO`
- 音频：`GETSOUNDORBGMINFO`、`ISPLAYINGSOUND`、`SOUNDCONTROL`、`ISPLAYINGBGM`、`BGMCONTROL`
- MAP：`MAP_VALUES`、`MAP_MERGE`、`MAP_REMOVEIF`、`MAP_FINDKEY`、`MAP_TOSTRING`、`MAP_FROMSTRING`
- SQL：`SQL_CONNECTION_OPEN`、`SQL_CONNECT`、`SQL_DISCONNECT`、`SQL_EXECUTE_NONQUERY`、`SQL_EXECUTE_READER`、`SQL_READER_READ`、`SQL_READER_GET_LONG`、`SQL_READER_GET_FLOAT`、`SQL_READER_GET_STRING`、`SQL_READER_ISNULL`、`SQL_READER_CLOSE`、`SQL_EXECUTE_SCALAR_LONG`、`SQL_EXECUTE_SCALAR_FLOAT`、`SQL_EXECUTE_SCALAR_STRING`、`SQL_IMPORT_MAP_XML`、`SQL_IMPORT_DT_XML`、`SQL_EXPORT_MAP_XML`、`SQL_EXPORT_DT_XML`、`SQL_IMPORT_XML_CUSTOM`、`SQL_ESCAPE`、`SQL_P_EXECUTE_NONQUERY`、`SQL_P_EXECUTE_READER`、`SQL_P_EXECUTE_SCALAR_LONG`、`SQL_P_EXECUTE_SCALAR_FLOAT`、`SQL_P_EXECUTE_SCALAR_STRING`

注册证据：

- 蛇版表达式：`Emuera/Runtime/Script/Statements/Function/Creator.cs:1-455`
- 蛇版命令：`FunctionIdentifier.cs:201,282-292,364-369,472-478`
- 参考表达式注册：参考仓库同路径 `Creator.cs`

### 3.2 Float 是第三种完整 ERB 类型

蛇版将参考实现的 Integer/String 模型扩展为 Integer/String/Float：

- `#DIMF`、`#FUNCTIONF`、`#REFF`；
- `LOCALF`、`ARGF`、`RESULTF`；
- Float 字面量、算术、比较、一元和三元表达式；
- Integer 到 Float 的提升；
- 既有数学/数组函数的 Float 分支；
- Float 变量、数组和角色变量；
- 存档新增 `Float=0x04`、`FloatArray=0x05`、`FloatArray2D=0x06`、`FloatArray3D=0x07`。

因此 Float 兼容不能只加 `double` parser：还必须贯通类型推导、bytecode/runtime、变量 storage、REF、存档版本与调试显示。当前蛇版 TW 没有使用 Float 声明，所以这是完整蛇版兼容项，不是当前 P0。

主要证据：`EraType.cs`、`LogicalLineParser.cs:143-178`、`UserDefinedVariable.cs:41,74,129`、`VariableDescriptor.cs:108,154-155`、`OperatorMethod.cs`、`EraBinaryDataReader.cs:19-32`、`EraBinaryDataWriter.cs:132-164`。

### 3.3 Variadic、元素引用、OUT 与调用兼容

蛇版增加：

- `VARIADIC ARG/ARGS/ARGF`，最后一个形参捕获剩余实参；
- `ARGLEN()`；
- 标量元素引用 `#REF/#REFS/#REFF`；
- 原 `#DIM REF/#DIMS REF/#DIMF REF` 继续表示数组引用；
- 可省略输出 `#DIM OUT/#DIMS OUT/#DIMF OUT`，省略时写入黑洞；
- 无 variadic 时，多余实参被静默丢弃；参考实现会报 `TooManyFuncArgs`。

这一最后一项是可观察兼容变化：即使游戏不声明新参数类型，某个“实参多于形参”的旧脚本也可能只在蛇版成功。当前审计没有完成全项目调用签名的形式化匹配，后续 compiler 接入时应记录并比较多余参数诊断。

证据：`ErbLoader.cs:610-775`、`VariadicArgTerm.cs`、`ElementRefInfo.cs`、`NullRefTerm.cs`、蛇版与参考 `Process.CalledFunction.cs`。

### 3.4 既有指令/方法的语义修改

| 功能 | 参考实现 | 蛇版 |
|---|---|---|
| `EXISTVAR` | 一个字符串参数，报告变量类别/维度 | 可选第二参数；非 0 时解析具体 storage cell；Float 增加 bit 32 |
| `EXISTFUNCTION` | 只查已装载 label dictionary | 若函数在 lazy index 中则加载文件后再判断，具有副作用 |
| `SPRITECREATE` | 2/6 参数 | 2/6/8/10 参数，增加目标尺寸和源矩形/偏移 |
| `CBGSETSPRITE` | 固定 4 参数 | 扩至最多 8 参数，增加尺寸、opacity、ColorMatrix，后续参数可省 |
| `GETDISPLAYLINE` | 负数返回空串 | `-1` 为最后一行、`-2` 为倒数第二行 |
| `SETANIMETIMER` | 表达式 `SETANIMETIMER(x)` | 命令 `SETANIMETIMER x`，另有 `GETANIMETIMER()` |
| `BITMAP_CACHE_ENABLE` | 表达式 | 命令 |
| `PRINTC/PRINTFORMC` | 依 Shift-JIS 字节数补列宽 | 依实际字体像素宽度补齐 |
| `INITRAND/DUMPRAND/RANDOMIZE` | 新 RNG 配置下有门控/忽略 | 始终操作 MTRandom 状态 |
| 普通整数算术 | 参考行为 | 溢出 warning 并饱和；除/模零 warning 后 0；`UNCHECKED_*` 明确提供回环 |

游戏已经活动使用 `EXISTVAR(...,1)`（如 `ERB/魔改内容/Misc.ERB:694,721,741`）、`GETDISPLAYLINE(-LOCAL)`（同文件约 `:756`）和命令式 `SETANIMETIMER`，这些不是纯引擎内部差异。

其他活动修正包括：`XML_ADDNODE` 多目标 clone、字符串 `>=/<=`、`TOINT` 整数读取异常返回 0、字体样式跨平台、鼠标键 latch、`SELECTCASE` 跳转表。这些应在行为差分套件中覆盖，但不能仅凭游戏含同名普通操作就全部升级为 P0。

2026-08-27 批次 0 源码复核：两版本 `Emuera/Runtime/Script/Statements/Function/Creator.Method.cs`
的 `ToIntMethod.GetIntValue` 都对空串及普通非数字字符串返回 0；蛇版另外捕获
`LexicalAnalyzer.ReadInt64` 的异常并返回 0，原版直接调用并传播异常。此前笼统的“原版非法
TOINT 报错”不能作为所有非法输入的期望；蛇版源码该路径也没有 warning。

### 3.5 Lazyloading

蛇版新增真正的函数级按需载入：

- `UseLazyLoading` 配置；
- `lazyloading.cfg` 目录清单；
- `lazyloading.bin` 函数→文件索引；
- `lazyloadingfiles.bin` 文件/mtime 索引；
- 启动跳过、首次 CALL/`EXISTFUNCTION` 加载；
- 文件增删改的索引更新；
- 事件和 `#FUNCTION` 文件排除规则。

参考实现也有 Preload 文件缓存，但不是上述功能。主要证据：`Process.LazyLoading.cs:16-428`、`ErbLoader.cs:44-173,391-540`、`Process.CalledFunction.cs:170`。

### 3.6 HTML、ImageLayer 与渲染栈

蛇版默认 SkiaSharp，并增加：

- `Auto/OpenGL/CPU` backend，OpenGL 失败回退 CPU；
- 统一逻辑坐标的 F11 比例缩放与鼠标逆映射；
- `<font size/valign/render/edging/hinting>` 及继承/重建；
- `<img xpos display ColorMatrix>`；
- `<div>` absolute display、可省高度、ARGB、自动布局；
- ImageLayer：多 sprite、depth、opacity、ColorMatrix、缩放、随滚动、离屏动画暂停；
- SharedBitmapCache、按需解码、WebP/GIF 动画、负尺寸翻转；
- GDI 光栅字体 fallback、strict fallback、CJK 字体和高 DPI 修复；
- `HTML_PRINTC/LC` 的像素单元格布局与整行文字背景。

它们是互相耦合的渲染模型，不宜逐条做成互不知晓的 host stub。特别是 ImageLayer、div、CBG 和文本共享 depth 排序，简单“把图片画出来”仍会产生遮挡顺序错误。

主要证据：`HtmlManager.cs:232-335,841-900,1118-1305,1533-1689`、`ConsoleImagePart.cs`、`ConsoleDivPart.cs`、`UI/Game/ImageLayerManager.cs:24-143`、`EmueraConsole.cs:1932-1978`。

### 3.7 输入、音频、SQL/MAP/bit 和运行时

- NF 输入：定时输入时保留上滚位置，不强制滚回底部；
- `SEQUENCEINPUT`：把字符串排入下一次 WaitInput，沿用宏、`\n`、`\e` 处理；
- 宏开关：局部关闭/开启 input macro；
- 音频：pause/resume/stop、长度/进度、seek、速度、preserve pitch，桌面用 SoundTouch；
- SQL：多连接、reader handle、parameter binding、scalar、MAP/DT/XML import/export；
- MAP：values、merge、remove-if、find-key、string round-trip；
- bit：以 `long[]` 为 storage 的 set/get/toggle/find-first；
- 每次函数调用独立 ExecutionContext，修复递归 LOCAL/ARG 覆盖；
- 1D int/string/float 稀疏数组；不能夸大为所有多维数组都稀疏；
- 用户 ERD 同名 ALS；
- ALS 序号 10 之后的字符串指针/别名读取修复；
- `BEFORE_THROW`、`BEFORE_ERROR` 事件及禁用配置。

### 3.8 配置、桌面体验与调试

蛇版新增活动配置：`UseLazyLoading`、`SkiaSharpImageQuality`、`SkiaSharpFontHinting`、`SkiaSharpFontEdging`、`RenderingBackend`、`DisableBeforeErrorThrow`、`MemoryDiagnosticEnabled`。此外有调试 watch 锁定/改值、tooltip 生命周期修复、F11、多显示器、内存诊断等。

这些多数不是蛇版 TW 的脚本 P0，但决定蛇版桌面体验。RustyEra 跨平台实现不必复制 WinForms 内部结构，却应定义等价的用户可观察行为和降级策略。

### 3.9 历史说明中已不存在的 API

以下名字出现在旧说明/changelog，但当前蛇版 HEAD 没有活动注册或实现，不能列为现有 API：

- `RM_RESOURCECHECK_LOAD`
- `RM_RELEASE_ALL`
- `RM_RESOURCE_EXIST`
- `SPRITEANIMEFRAME`
- `HOVER_PAUSE`
- `ARGB_TO_HTML_COLOR`

另外，`GCREATEFROMFILE` 的相对路径参数、`PluginAvailableWarn`、BREAKBUTTON、ENUMFILES 路径修复、FORCE_QUIT 等已存在于当前 EEv56 参考实现，不属于蛇版独有增量。

## 4. RustyEra 相对蛇版 Emuera 缺什么

### 4.1 分层状态矩阵

| 能力 | Analyzer/catalog | Compiler | VM/core runtime | Web/Tauri | TUI | 对蛇版 TW |
|---|---|---|---|---|---|---|
| 蛇版 `SQL_*` 全套 | 缺 | 缺 | 缺 SQLite manager/ABI | 缺持久化策略 | 缺 | P0 |
| `GETMETH/GETMETHS/EXISTMETH` | 名字存在 | unsupported trap | 无可执行路径 | — | — | P0/P1 |
| `TINPUTNF/SNF` | 缺蛇版命令 | 缺 | 无 NoFocus wait 语义 | 缺滚动保持语义 | 缺 | P1 |
| `MOUSEX/Y/B` | 参考名字存在 | host import | core 会请求 `pointer_state` | 未协商 | 未协商 | P1 |
| `HTML_STRING*` | 参考名字存在 | host import | core 会请求 presentation query | 未协商 | 未协商 | P0/P1 |
| ImageLayer 命令 | 缺 | 缺 | 缺统一 layer/depth model | 缺 | 缺 graphics | 当前仅测试函数；语法/编译 P3 |
| 蛇版 HTML 属性 | parser 仅固定标签/属性子集 | — | canonical HTML 不完整 | 布局不等价 | TUI 只文本近似 | P1/P2 |
| `HTML_PRINTC/LC`、BGC | 缺蛇版指令 | 缺 | 缺像素 cell/BGC 语义 | 缺 | 缺 | P1/P2 |
| `SETANIMETIMER` 蛇版命令 | 目录口径需迁移 | 参考形式与蛇版不同 | 动画时钟不等价 | 部分动画 | 无图形 | P1/P2 |
| 蛇版 MAP 扩展 | 多数名字缺 | 缺 | 标准 MAP 主体已有，新增 6 项缺 | — | — | 当前无活动调用 |
| Float 全栈 | 缺蛇版语法/类型 | 缺 | 缺 Float variable/save tags | — | — | 当前游戏非 P0 |
| VARIADIC/OUT/元素 REF | 缺或主动拒绝 | 缺 | 局部数组 REF 已有，但不是蛇版新增语义 | — | — | 当前游戏未发现 |
| 用户 ERD ALS | 部分 deferred index | 部分 | 不完整 | 扫描器不提交 `.als/.erd` | 同左 | P0 装载/别名 |
| lazyloading | 无蛇版索引协议 | 有不同的编译/选择机制 | 无首次函数加载和存在性副作用 | 无 | 无 | P3，可能成为实际阻断 |
| CBG | 名字注册 | host imports | 除 `CBGCLEAR` 外无 handler | 不完整 | 无 | P1/P2 |
| Canvas pixel/PNG | 有 host 模型 | 有 | 需前端 service | `sample_canvas_pixel`、`encode_canvas_png` 未协商 | 未协商 | P2 |
| 扩展音频控制 | 蛇版名字缺 | 缺 | 普通 BGM/effect 有，speed/seek/pitch 缺 | 普通音频部分可用 | 无 audio | P2 |

### 4.2 蛇版新增项基本都在 catalog 之前缺失

对 RustyEra analyzer/catalog、compiler registry、VM native 与 host 表做符号交叉后，蛇版 TW 活动使用的以下族并非“实现有 bug”，而是根本没有完整注册链：

- SQL 全族；
- NF 输入；
- `HTML_PRINTC/LC`、`TEXT_BGC_*`；
- `GETANIMETIMER` 等活动使用的蛇版图形查询；
- `SEQUENCEINPUT`、input macro 开关；
- 蛇版 trig/round/unchecked 方法。

ImageLayer、蛇版 MAP 新增六方法等也缺注册链，但当前游戏只有测试/说明命中或无活动调用，应列为完整蛇版兼容项，而不是当前玩法 P1。

这意味着不能先依赖 runtime fallback“稍后实现”：全项目编译时就会收到 unknown/unsupported diagnostics。兼容工作需要从 analyzer signature、类型规则、lowering、ABI 和 runtime 一起设计。

### 4.3 RustyEra 现有参考能力也存在“注册不等于实现”

已核实的典型例子：

- `GETMETH` 测试本身期望 Unsupported；普通 lowering 最终生成 `Opcode::Trap`；
- `DT_COLUMN_OPTIONS` 因 `dt_` 前缀被 compiler 视为 native，但 VM DataTable dispatcher 没有分支，运行时报 `unsupported data-table native`；
- CBG host registry 列出多个名称，runtime 只有 `CBGCLEAR` handler；
- `SAVEVAR/LOADVAR` 编译到 host，runtime 主动返回 Unsupported；
- `CALLSHARP` 能生成 extension ABI，但 declaration 与 builtin 同名校验互相冲突，而且前端没有 extension registry/service，实际不可达；当前游戏未活动使用它；
- `HTML_STRINGLEN/SUBSTRING/STRINGLINES` core 能发 query，但前端 hello 不协商对应 operation，能力检查直接 fault；
- `MOUSEX/Y/B` 同理依赖 `pointer_state`；
- `GGETCOLOR` 依赖 `sample_canvas_pixel`；`GSAVE` 缺 `encode_canvas_png` 时恒失败。

实现状态报告必须沿完整链路给出，不能把 analyzer 收录、compiler host 表、protocol 类型或单元测试 ABI 当作功能完成。

### 4.4 已有能力与可复用基础

RustyEra 并非从零开始，下列能力已经有真实实现，可以作为蛇版兼容的基础：

- 静态 `CALL/CALLF/JUMP/TRY*` 和动态 `CALLFORM/JUMPFORM` 专用 lowering；
- 函数参数 `#DIM/#DIMS REF` 的 frame alias；
- 普通输入、定时等待、按钮输入、超时、message skip；
- `GETCONFIG(S)`、`VARSIZE`、`EXISTFUNCTION/EXISTVAR` 等参考功能的 runtime handler，但蛇版扩展语义仍缺；
- 普通/全局/角色存档、文本、文件枚举；
- 标准 MAP/XML/DataTable 的主体状态机；
- Web 的常规 HTML、Canvas replay、sprite metadata、WebP/PNG 和普通音频；
- 项目 content hash、compiled cache、runtime/frontend 分层协议。

但已有结构存在明确子集：

- XML/XPath 是 portable、无 namespace 的 XPath 1.0 子集；
- DataTable 只有若干整数和 String、单比较 filter、单列 sort；
- 动态 `STRFORM` 不支持全部三连符、增减、host callable 和带 REF 的用户方法；
- TUI 字体宽度按 cell 近似，高度恒 1，不能满足像素 UI；
- TUI 不声明 graphics/audio，图片标签被忽略；
- Web 的 Canvas2D 与蛇版 Skia 不会天然像素一致。

### 4.5 存档兼容

若目标包含“直接读取蛇版 Emuera 的现有存档”，还需单独审计：

1. 蛇版 Float save tag `0x04..0x07`；
2. 自定义 `CHARADATA SAVEDATA BUFF` 等用户变量排序、shape 和 alias；
3. 稀疏 storage materialize 后的二进制布局；
4. SQLite `plugins/qol_data.db` 是衍生缓存还是需要随存档迁移；
5. 压缩存档配置 `USE COMPRESSED SAVE DATA:YES`；
6. RNG 状态在蛇版取消门控后的 `DUMPRAND/INITRAND` 行为。

当前游戏没有 Float 声明，故 Float tag 不是这份快照新存档的直接必要项；但 RustyEra 不能据此宣称兼容任意蛇版存档。

## 5. 推荐实现顺序

### P0-A：先建立可重复的“静态装载到首屏”门禁

1. 固定当前游戏 commit、配置和资源清单；保留用户 `emuera.config`，测试用复制项目。
2. 让项目扫描器提交 `.als`、`.erd`，并记录每类文件数量/hash。
3. 生成“游戏实际出现的指令/表达式签名清单”，把 unknown、compiler trap、runtime unsupported 分开。
4. 先补 `GETMETH/GETMETHS/EXISTMETH`，因为 SQL 图构建依赖动态方法。
5. 设计 SQL ABI 后实现游戏使用子集：连接、nonquery、reader、long/string scalar、参数绑定、MAP XML import、reader 生命周期；明确路径沙箱、浏览器持久化、事务和关闭策略。
6. 完成 `EVENTFIRST`/`EVENTLOAD` → `INIT_NG_OR_LOAD` → `GRAPH_DB_INIT` 与 BBAS 初始化的定向验证。

不建议一开始实现未被游戏使用的 SQL Float scalar、全部 custom XML import/export 或 Float 类型；但 API 设计应为它们保留类型扩展空间。

### P0-B/P1：打通实际交互与像素布局

1. 前端协商并实现 `HTML_STRINGLEN/SUBSTRING/STRINGLINES`；以同一字体、字号和标签模型测量。
2. 实现 `pointer_state`，补 `MOUSEX/Y/B` 真实坐标、按钮/悬停及 Web/Tauri focus 行为。
3. 增加 NF timed input，定义“保持滚动位置、超时、按钮点击、输入宏”的状态机。
4. 实现 `HTML_PRINTC/LC`、BGC、动画计时和游戏实际使用的 HTML 属性；`GETLINEY` 可随测试/未来 UI 后补。
5. 设计统一 scene/depth model，先接活动使用的 div、CBG、sprite，再为测试/未来 UI 接 ImageLayer，而不是各自直接操作 DOM/Canvas。
6. Web/Tauri 先达到可玩；TUI 明确提供文本降级或声明该游戏不受支持，不应伪装为图形等价。

### P1/P2：数据、地图与功能完整性

- 补游戏用到的 `MAP_*` 扩展；
- 对当前 XML、XPath、DT 输入做数据驱动差分，补 `DT_COLUMN_OPTIONS`；
- 补 `EXISTVAR(...,1)`、负索引 `GETDISPLAYLINE`、命令式 `SETANIMETIMER`；
- 补游戏可达的 CBG、canvas pixel、图像保存；
- 补音频查询/控制、速度和 pitch，或提供明确降级；
- 明确 custom ERD ALS 与生成 AutoConst headers 的权威来源。

### P3：规模与完整蛇版语言

- 以 4,100 个脚本、176 MiB ERB、2,370 个 lazy 口上做真实启动/内存 profile；
- 选择兼容 lazyloading 或以 RustyEra compiled cache/分区编译提供等价效果；
- 再实现当前游戏未用的 Float、variadic、OUT、元素 REF、Float save tags、全部 SQL/XML/MAP/bit API；
- 做蛇版 Emuera 与 RustyEra 的脚本级、存档级、像素级和输入时序级差分套件。

## 6. 建议的验收断面

后续实现不应只用“项目编译成功”作为完成条件，建议至少有以下断面：

1. **项目摄取**：4,100 个 ERB/ERH、229 个 CSV 类文件、20 个 ALS、资源清单数量与 hash 正确；无静默漏文件。
2. **静态编译**：蛇版 TW 全项目无 unknown instruction、unsupported construct 或 trap；动态构造目标有运行时检查。
3. **首次新游戏**：图数据库建表/版本/重建事务完成；BBAS XML 数据完成；首个可交互画面出现。
4. **读档**：`EVENTLOAD` 走相同初始化后恢复角色、自定义变量、RNG 和 UI。
5. **地图**：动画/悬停、鼠标点击、NF timed input、寻路距离、跨地图移动一致。
6. **布局**：标题、QOL 命令、状态条、Flan UI、角色列表在指定字体和窗口尺寸下做结构与像素基准。
7. **资源**：WebP/PNG、动画、CBG、音频均有真实输出或明确受支持平台声明；ImageLayer 测试函数至少能安全编译，承诺完整蛇版兼容时再验 depth 行为。
8. **口上按需路径**：首次调用未编译口上、`EXISTFUNCTION`、返回标题、再次调用和文件更新的语义一致。
9. **存档互操作**：若承诺读取蛇版存档，使用真实存档做双向/单向兼容声明，而不是仅测 RustyEra 自己生成的存档。

## 7. 证据索引

### 蛇版 Emuera

- API 注册：`emuera_lazyloading_selfmodified_version/Emuera/Runtime/Script/Statements/Function/Creator.cs`
- 命令注册：`.../FunctionIdentifier.cs`
- Float/运算：`.../EraType.cs`、`.../OperatorMethod.cs`
- variadic/REF/OUT：`.../ErbLoader.cs`、`.../Process.CalledFunction.cs`
- lazyloading：`.../Process.LazyLoading.cs`
- EXISTVAR/SPRITECREATE/GETDISPLAYLINE/SQL 等：`.../Creator.Method.cs`
- HTML：`.../HtmlManager.cs`、`.../ConsoleImagePart.cs`、`.../ConsoleDivPart.cs`
- ImageLayer：`.../UI/Game/ImageLayerManager.cs`
- SQL：`.../Runtime/Utils/尊尼获加/SqlManager.cs`
- 音频：`.../Runtime/Utils/Sound.cs`、`Sound.NAudio.cs`
- Float 存档：`.../EraBinaryDataReader.cs`、`EraBinaryDataWriter.cs`
- 用户 ERD ALS：`.../ConstantData.cs:793-850`

### 蛇版 TW

- 启动链：`games/eratw-sub-modding/ERB/SYSTEM.ERB`
- SQL 图初始化/查询：`.../ERB/魔改内容/qol/qol_graph_init.ERB`、`qol_graph_query.ERB`
- SQLite 业务模块：同目录 `qol_db.ERB`、`qol_dish.ERB`、`qol_wood.ERB`、`qol_mushi.ERB`、`qol_PHARMACY.ERB`
- 地图交互：`.../qol_MAP.ERB`
- HTML/UI：`.../ERB/魔改内容/PRINT系/`、`QOL_USERCOM.ERB`、`QOL_IMAGE.ERB`
- BBAS：`.../ERB/BODY_INFO/BBAS_DATASET.ERB`
- 自定义 BUFF：`.../ERB/DIM.ERH:567`、`CSV/BUFF.csv`、`CSV/BUFF.als`
- Lazy：`lazyloading.cfg`、`lazyloading.bin`、`lazyloadingfiles.bin`

### RustyEra

- compiler registry：`rustyera-core/crates/erabasic-compiler/src/registry.rs` 及 `registry/hosts.rs`
- unsupported lowering：`.../lowering/builder/imports.rs`、`statements.rs`
- DataTable VM：`.../erabasic-vm/src/structured/data_calls.rs`
- HTML parser：`.../erabasic-html/src/markup/model.rs`
- runtime dispatch：`.../era-runtime/src/session/host_dispatch/`
- 前端 capability hello：`rustyera-core/crates/era-web-bridge/src/lib.rs`、`rustyera-tui/src/rustyera_tui/runtime_transport.py`
- TUI 扫描：`rustyera-tui/src/rustyera_tui/project_scan.py`
- Web 浏览器扫描：`rustyera-web/src/platform/browserProjectFilesystem.ts`
- Tauri 扫描：`rustyera-web/src-tauri/src/project/scan.rs`
- CSV deferred index：`rustyera-core/crates/erabasic-csv/src/deferred.rs`
- 内置 ALS：`rustyera-core/crates/erabasic-csv/src/tables.rs`

## 8. 最终判断

若目标是“让当前蛇版 TW 原样在 RustyEra 中进入可玩状态”，正确的最小闭环不是完整复刻蛇版 Emuera，而是：

1. 文件摄取完整；
2. 动态方法调用可执行；
3. 游戏所用 SQL 子集可执行且跨平台持久化；
4. 首次/读档初始化成功；
5. HTML 宽度、鼠标与 NF 输入可用；
6. 地图和核心 UI 的 HTML 布局、CBG/sprite 与 depth 可用；
7. 在大规模口上项目上达到可接受的启动和内存表现。

完成这一闭环后，再扩展 Float、variadic/OUT/元素 REF、完整蛇版 SQL/音频/渲染和存档格式，才能逐步从“蛇版 TW 可玩”推进到“蛇版 Emuera 兼容实现”。
