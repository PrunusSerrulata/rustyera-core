# 蛇版兼容批次 5：互操作存档与真实音频状态实施计划（修订版）

## 总结

批次 5 已完成 5.0、5.1、5.2。原计划中所有 RustyEra legacy 蛇版存档兼容内容现被废止，
包括 `RERASAV`、`OwnedSaveStateV1`、v11 identity、旧 Text 布局和
`LegacyProfileSave`。

最终生产版只接受并生成标准 Emuera 1808 存档，不读取、迁移、列举、回退或删除旧
RustyEra 私有存档。产品本身不得包含旧存档迁移能力。

- 蛇版传统存档统一读写标准 Emuera 1808 格式，直接兼容蛇版的 Binary、ERAZIP/GZip
  和 Text 格式。
- `saveNN.sav`、`global.sav`、SQL 数据库和 VM snapshot 是彼此独立的状态层，不把
  GLOBAL、RNG 或 SQL revision 嵌入普通 `.sav`。
- 标准蛇版存档加载时保持加载前的 SFMT 和 SQL 外部状态；`RANDDATA` 作为普通变量导入，
  仅显式 `INITRAND` 才改变 RNG。
- 同时完成蛇版五项音频函数、10 个音效声道、真实播放查询及浏览器/Tauri 控制；TUI
  不伪造实际音频状态。
- Float 存档标签继续留到批次 6；本批遇到 Float 或未知 codec/tag 时显式拒绝。

修订后的依赖顺序：

`5.0✓ → 5.1✓ → 5.2✓ → 5.3 → 5.4 → {5.5, 5.6}；5.6 → 5.7；{5.5, 5.7} → 5.8`

5.5（TUI）与 5.6（Web/Tauri 存档）可在不同仓库并行；5.7 与 5.6 共享 Web worktree，
因此必须在 5.6 完成后开始。core 子批次因共享 worktree 串行提交。

## 最终公共接口与行为

- 保留 Runtime 协议 `46.0` 和蛇版 compatibility identity `12`；二者尚未正式发布，
  因此直接修订当前契约，不再额外升级版本。
- 蛇版 `save_codec` 保持 `snake_emuera1808_interop_v1`，并保留
  `rustyera.audio@1` 服务契约。
- 从公共协议、Schema、C ABI 和文档中移除 `StorageNamespace::LegacyProfileSave`。
- 删除 `LEGACY_SNAKE_OWNED_SAVE_CODEC`、v11 identity 构造/识别接口以及只服务于旧
  `RERASAV` 的 `rustyera.save_state@1` 契约。
- 删除 `RERASAV` envelope parser、checksum、metadata inspection 和
  `OwnedSaveStateV1` 解码恢复能力。
- 普通与 GLOBAL 存档直接通过 Emuera 1808 codec 解码；`RERASAV` 输入作为无效 1808
  存档拒绝，并原子保持现有状态。
- 蛇版普通 `.sav` 只恢复 ordinary state，不恢复 GLOBAL、SFMT 或 SQL revision；裸
  GLOBAL 存档只恢复 global scope。
- 前端只暴露项目 `sav` 对应的 `Save/GlobalSave`，不得探测旧 profile 私有存档目录。
- 产品不得自动删除用户旧存档；这些文件保持原地，但对新版完全不可见、不可读。
- 新增并保留音频协议类型：
  - `AudioChannelV1::{Sound(0..9), Bgm}`。
  - `AudioPlaybackStateV1::{Stopped, Playing, Paused}`。
  - `AudioObservationRequest/ResponseV1`，携带目标声道、预期 revision、duration、
    position、实际状态、音量、速率、preserve-pitch 与前端单调时间戳。
  - `audio_observation@1` 服务；响应 revision 不匹配时作为 stale response 拒绝。
  - 音频 effect 增加精确声道、revision、Pause、Resume、SetRate/preserve-pitch。
- 注册并实现：
  - `GETSOUNDORBGMINFO(channel[, selector])`
  - `ISPLAYINGSOUND(channel)`
  - `SOUNDCONTROL(channel, action[, speed[, pitch_flag]])`
  - `ISPLAYINGBGM()`
  - `BGMCONTROL(action[, speed[, pitch_flag]])`
- 以固定蛇版源码为准，不实现旧计划中误列的 Seek；蛇版 ERB API 没有 Seek action。
- 不新增 RustyEra 专用 `EXPORTDATA` 等脚本命令；权威 `.sav` 本身即可由蛇版读取。

## 子批次实施

### 5.0 基准、oracle 与契约冻结（已完成）

- 已固定三仓 SHA、工具链和当前蛇版 TW 输入：
  - `sav/global.sav`：SHA-256
    `56f80b52a8a6c8fc7dd080f9a69967758fb83df966a45330123bbc3d8a1e37cf`。
  - `sav/save1000.sav`：SHA-256
    `442b1d41d3d17f2dbfdb6587ae521361bf07174f653affdc0bb82a9693dae0a2`。
  - 两者均为 ERAZIP 1808；`setting.json` 为 `UseNewRandom=true`。
- 已用蛇版 reference CLI 和隔离 Wine GUI 冻结：
  - Binary/GZip/Text 普通与 GLOBAL 存档。
  - Integer/String、角色、自定义 1D/2D/3D 数组、Map/XML/DT。
  - Float、未知 tag、截断和 zip bomb 的参考行为与 Rust 目标拒绝类别。
  - 五项音频函数的签名、返回码、暂停状态、声道分配、速率与 pitch flag 实际语义。
- 参考仓库和游戏保持只读；后续运行继续使用隔离副本。
- 固定参考的 Text writer 不写 Map/XML/DT；Text 的实际省略和不可恢复行为继续作为
  已登记 oracle 差异。

### 5.1 协议与 identity 基础（已完成，部分由 5.3 修订）

- 已加入协议 46、identity 12、音频 target/effect/observation 类型及 operation 常量。
- 已将传统存档契约改为 `snake_emuera1808_interop_v1`，明确精确 SQL/RNG 状态属于 VM
  snapshot，不属于标准传统存档。
- 已同步协议 Schema、C ABI、序列化兼容测试和测试工具说明。
- 本子批次加入的 `LegacyProfileSave`、v11 identity 和 legacy codec 常量属于被新决策
  废止的临时契约，由 5.3 删除；不改写已完成提交的历史。

### 5.2 Core 互操作存档 codec 与加载语义（已完成，部分由 5.3 修订）

- 已使蛇版 `SAVEDATA/SAVEGAME/SAVEGLOBAL` 直接编码裸 1808，不再生成 envelope；格式
  严格服从现有保存格式与压缩配置。
- 裸普通存档仅恢复 ordinary state：
  - 不恢复或替换 GLOBAL。
  - 不恢复、重播种或自动读取 RNG。
  - 不保存或恢复 SQL revision，也不因活跃 SQL reader/transaction 阻止传统保存。
  - `RANDDATA` 照常导入；只有脚本执行 `INITRAND` 才影响 SFMT。
- 裸 GLOBAL 只恢复 global scope。
- 裸存档加载发出结构化信息诊断，说明其不携带可恢复 RNG/SQL snapshot，并保持当前
  RNG 和 SQL 外部状态。
- 标准蛇版 `.sav` 可由传统存档导入、导出和验证路径直接处理；VM snapshot 行为不变。
- 本子批次加入的 v11 decoder、owned state 恢复、legacy Text dialect 与迁移诊断由 5.3
  完整删除；不得保留为隐藏或测试专用的生产能力。

### 5.3 Core legacy 存档能力清除与单一来源槽位

- 删除 `LegacyProfileSave` 枚举值、权限判断、CDDL 定义、协议测试及 runtime-tester
  支持；保留现有正式 namespace 的 wire 值不变。
- 删除整个 `RERASAV` envelope compatibility 层；metadata 检查直接调用标准 1808
  inspector，加载直接调用标准 codec。若 compatibility 模块不再承担其他职责，则删除
  该模块及其不再使用的依赖。
- 删除 `CompatibleSaveEnvelope`、`CompatibleSaveSource`、`LegacySnakeOwnedV11` 及相关
  导出，不在生产代码中保留识别或拆包入口。
- 删除 `OwnedSaveStateV1`、`DecodedOwnedSaveState`、附载 SQL revision 结构、legacy Text
  dialect、测试专用 envelope encoder 及其专用依赖。
- 删除 GLOBAL/SFMT/SQL legacy 恢复分支、迁移诊断和 legacy 专用失败路径；保留裸存档
  加载后说明外部状态保持语义的诊断。
- Load 菜单、`LOADDATA`、`LOADGLOBAL`、`DELDATA` 和系统菜单只使用项目 `Save` 或
  `GlobalSave`：
  - 不列举旧 profile 目录。
  - 不建立多来源映射。
  - 不执行 NotFound fallback。
  - 不删除、迁移或显示旧 RustyEra 槽位。
- 保留 revision precondition 和原子替换；每个逻辑槽位只有一个项目 `sav` 来源。
- 更新当前前端接口文档，明确只支持标准 Emuera 1808；历史批次 4 文档保留为历史记录。
- 将 legacy 迁移成功测试改为负向拒绝测试，不得为了负向测试保留生产 decoder。
- 作为独立 core 功能修正提交；在本子批次任何测试开始前完成一次规定的重构审查并落实
  全部要求。

### 5.4 Core 音频语言与真实查询

- 将 `PLAYSOUND` 签名收紧为资源名加可选 repeat；repeat 最小为 1。
- Web 能力可用时，`PLAYSOUND` 先查询 0–9 实际声道：
  - 选择首个非 playing 声道；paused 视为空闲。
  - 全部 playing 时覆盖声道 0。
  - effect 携带选定声道及新 revision。
- 精确实现五项函数：
  - 无效 sound channel：查询返回 0 或 `-1`，控制返回 `-1`。
  - 无效 action 返回 `-2`；有效控制返回 `1`。
  - 省略 GET selector 时写 `RESULT:0..4` 并返回 duration。
  - selector 1–5 分别返回 duration、position、实际 playing、volume、speed。
  - action 0/1/2 为 pause/resume/stop；action 3 设置速度。
  - 速度按蛇版 NAudio 的 0.1×–10×范围处理。
  - pitch flag 保持蛇版实际反向语义：省略或 0 表示 preserve，非 0 表示不 preserve。
- BGM expected state 继续可恢复；一次性 sound 只作为 transient effect，不因
  snapshot/reconnect 重播。
- 缺少 `audio_observation@1` 时：
  - GET/ISPLAYING 系列产生稳定的 `runtime.audio_observation_unavailable` 诊断和 script
    fault，不能返回伪造的 0。
  - 控制函数仍返回蛇版参数级返回码，并由 unsupported effect 产生一次设备能力警告。

### 5.5 TUI 项目存档与不支持音频契约

- 蛇版 `Save/GlobalSave` 直接映射项目 `sav`；配置独立 data dir 时仍以项目 `sav` 为
  互操作主存档。
- 不增加 legacy namespace、旧 profile 路径、fallback、迁移或删除逻辑。
- TUI 继续声明 `audio=false`，不注册 `audio_observation@1`；主动或非协商请求均返回
  `frontend.unsupported_service`。
- 更新 TUI 测试：标准蛇版传统存档不拥有 RNG、GLOBAL 或 SQL 状态；精确运行状态只由
  VM snapshot 持有。
- 更新 core pin 和锁文件，记录实际 core SHA。

### 5.6 Web/Tauri 互操作存档

- 浏览器目录项目将蛇版 `Save/GlobalSave` 指向项目文件系统的 `sav`，其他可写
  Data/Log/cache 继续保持隔离。
- 打包项目或无原目录写权限时，标准 `.sav` 保存在持久项目副本；复用现有传统存档
  导入/下载界面与蛇版交换。
- Tauri 将主存档映射项目 `sav`，不访问原 profile 私有存档目录，也不提供 legacy
  导入、迁移或删除入口。
- Browser 与 Tauri 的槽位列表、导入、验证、覆盖确认均使用裸 1808。
- 更新当前仍断言 `RERASAV` 的 Tauri 测试，改为验证裸 Binary/GZip/Text 1808。
- 更新最终 core pin、Cargo.lock/WASM 绑定，并记录 browser 与 Tauri 的实际 core SHA。

### 5.7 Web/Tauri 实际音频 provider

- 将当前匿名 BufferSource 音效池改为 10 个稳定 sound channel 加一个 BGM target。
- 使用可读取 `duration/currentTime/paused/ended/playbackRate` 的媒体元素作为声道主体，
  并接入现有音量/解锁链路：
  - pause 保存位置，resume 从原位置继续。
  - rate 变化前累计当前位置。
  - finite repeat、无限 BGM、自然结束、覆盖和 stop 均释放资源。
  - 设置标准或 WebKit `preservesPitch` 属性。
- 每个 effect 应用 revision；`audio_observation@1` 返回真实媒体状态和单调时间戳，
  revision 不符时返回 stale error。
- 仅真实 provider 就绪时广告该服务；解码、自动播放、缺失 pitch 属性和资源失败均返回
  结构化错误，不伪造成功。
- 浏览器与 Tauri 共用同一引擎语义；平台差异只保留在 capability 和错误证据中。

### 5.8 集成验收、生产门禁与收尾

- 执行真正的双向存档路径：
  1. RustyEra 读取当前蛇版 TW 的 `save1000.sav/global.sav`。
  2. RustyEra 保存新 ERAZIP。
  3. 蛇版 reference 加载、修改并再次保存。
  4. TUI、Chromium、Firefox、Safari、Tauri 分别加载修改后的文件并比较状态。
- 验证所有产品路径只使用项目 `sav`，没有旧 profile 存档请求或 fallback。
- 完成音频三浏览器和 Tauri 实际矩阵，以及 TUI 明确不支持路径。
- 使用全新隔离 target 构建 release 产物，避免旧构建缓存影响生产能力检查。
- 只在全部验收结束后写 `SNAKE_EMUERA_IMPLEMENTATION_LOG.md`、批次总览、迁移计划与
  分类表，并修正旧 Seek 描述。
- 将用户可见的 1808 互操作存档和音频行为追加到根 `CHANGELOG_PENDING.md`；删除尚未
  发布的临时 legacy 能力不单独记为用户功能变更。
- core、TUI、Web 和根仓库分别提交；不升级产品发行版本，不推送、不合并。

## 测试与验收规则

- 5.0 已按当时授权完成；5.3 及后续各代码子批次继续分别遵守一次全量、静态门禁先于
  动态测试、失败后只做最小定向复验及每批次 60 分钟预算。
- 每个代码子批次在任何测试前恰好执行一次独立 `$refactor-rustyera-code` 审查，并先落实
  全部要求。
- 每条测试命令由 `gpt-5.6-terra low` 测试子智能体按对应
  `$test-rustyera-core/tui/web` skill 执行。
- 定向测试先于一次完整套件；完整套件失败后只做受影响的定向复验。
- 静态门禁全部通过后才能启动 reference、真实 C ABI、浏览器或 Tauri 动态测试。
- Web/Tauri 动态测试执行每 5 秒完整 DOM/runtime 快照看门狗；连续相同立即失败。

关键验收场景：

- 存档：
  - 当前 `UseNewRandom=true` 的真实蛇版存档可加载，不因无法恢复 `.NET Random` 拒绝。
  - 加载前后 RustyEra RNG 的下一值与“不执行加载”的控制组一致。
  - `RANDDATA` 数组被导入，但除非执行 `INITRAND`，不会改变 RNG。
  - 普通存档不改变 GLOBAL 或现有 SQL 状态；`EVENTLOAD` 的显式数据库重建正常。
  - Binary/GZip/Text、normal/global、Integer/String、角色、自定义数组、Map/XML/DT
    双向通过。
  - Float、未知 tag、损坏 header、截断、超限解压显式拒绝且加载原子回滚。
  - 最小合成 `RERASAV` 输入作为无效标准存档拒绝，且 VM、GLOBAL、SFMT、SQL、槽位和
    文件状态均不变化。
  - 存储 trace 只出现项目 `Save/GlobalSave`，不出现旧 profile namespace、路径探测或
    fallback。
- legacy 能力缺失门禁：
  - 公共 Schema、C ABI 和协议类型中不存在 legacy save 接口。
  - 生产源码中不存在 `LegacyProfileSave`、`OwnedSaveStateV1`、
    `LegacySnakeOwnedV11`、legacy codec 常量或 envelope parser。
  - 新构建的 release 动态库、WASM 和应用产物不得包含 `RERASAV` magic、envelope
    checksum domain、legacy codec 名称或 owned-state 类型名。
  - 历史计划和负向测试样本不计入生产产物门禁。
- 音频：
  - 10 声道首空闲/全忙覆盖 0，paused 视为空闲。
  - GET omitted selector 的返回值与 `RESULT:0..4` 完全一致。
  - position 播放时单调、暂停时在容差内稳定、resume 后继续、stop 后归零或 stopped。
  - rate、volume、repeat、preserve-pitch 与蛇版返回码一致。
  - stale revision、自然结束、解码失败、自动播放失败和缺少 provider 均有稳定诊断。
  - TUI 的实际查询明确失败，不返回貌似可信的 stopped 值。

## 已确定的假设

- “legacy 蛇版存档”仅指 RustyEra 私有 `RERASAV/OwnedSaveStateV1` 及其 profile 私有
  存储入口，不包括标准 Emuera 1808、VM snapshot 或代码中其他无关的 legacy 数据结构。
- 协议 46 和 identity 12 尚未形成生产发布，因此允许原位删除临时 public variant。
- 标准蛇版 `.sav` 不拥有 RNG、GLOBAL 或 SQL 状态；这是对蛇版实际行为的兼容，不再
  沿用批次 4 的自有传统存档定义。
- 裸存档加载保持当前 SFMT，不自动 `DUMPRAND`、`INITRAND` 或重新播种。
- 旧用户存档不会被迁移或自动删除；新版只是不再发现或加载它们。
- 历史批次 4 文档可保留当时设计记录，但所有当前接口文档和批次 5 最终记录必须明确
  legacy 能力已移除。
- 当前批次不实现 Float、`.NET Random` 状态序列化、Seek 或蛇版之外的新脚本 API。
- 参考实现和游戏仓库保持只读，所有写入测试使用隔离副本。
