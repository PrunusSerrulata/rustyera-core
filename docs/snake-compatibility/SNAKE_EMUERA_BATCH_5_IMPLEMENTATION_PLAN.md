# 蛇版兼容批次 5：互操作存档与真实音频状态实施计划

## 总结

批次 5 废弃“蛇版 profile 仍以 RustyEra 私有 envelope 为权威存档”的方向：

- 蛇版传统存档统一读写标准 Emuera 1808 格式，直接兼容蛇版的 Binary、ERAZIP/GZip 和 Text 格式。
- `saveNN.sav`、`global.sav`、SQL 数据库和 VM snapshot 恢复为彼此独立的状态层，不再把 GLOBAL、RNG、SQL 修订嵌入普通 `.sav`。
- 标准蛇版存档加载时保持加载前的 SFMT 状态；`RANDDATA` 作为普通变量导入，仅显式 `INITRAND` 才改变 RNG。
- 批次 4 的 `RERASAV/OwnedSaveStateV1` 降为只读迁移格式，继续精确加载，但不再生成。
- 同时完成蛇版五项音频函数、10 音效声道、真实播放查询及浏览器/Tauri 控制；TUI 不伪造实际音频状态。
- Float 存档标签继续留到批次 6；本批遇到 Float 或未知 codec/tag 时显式拒绝。

依赖顺序：

`5.0 → 5.1 → 5.2 → 5.3 → 5.4 → {5.5, 5.6} → 5.7 → 5.8`

其中 5.5（TUI）与 5.6（Web/Tauri 存档）可在不同仓库并行；core 子批次因共享 worktree 串行提交。

## 公共接口与行为变更

- Runtime 协议从 `45.0` 升至 `46.0`。
- 蛇版 compatibility identity 从 11 升至 12；`save_codec` 改为蛇版 1808 互操作契约，并新增 `rustyera.audio@1` 服务契约。旧 cache/snapshot 按现有规则拒绝或重建，旧 v11 存档仅由专用迁移解码器接受。
- `StorageNamespace` 新增 `LegacyProfileSave`：
  - 指向原 `.rustyera/profiles/emuera.skia.snake/sav`。
  - 允许读取、列举、Stat、ReadRange 和删除；禁止写入。
  - 新 `Save/GlobalSave` 在蛇版 profile 下指向项目 `sav`；非蛇版 profile 行为不变。
- 新增音频协议类型：
  - `AudioChannelV1::{Sound(0..9), Bgm}`。
  - `AudioPlaybackStateV1::{Stopped, Playing, Paused}`。
  - `AudioObservationRequest/ResponseV1`，携带目标声道、预期 revision、duration、position、实际状态、音量、速率、preserve-pitch 与前端单调时间戳。
  - `audio_observation@1` 服务；响应 revision 不匹配时作为 stale response 拒绝。
  - 音频 effect 增加精确声道、revision、Pause、Resume、SetRate/preserve-pitch。
- 注册并实现：
  - `GETSOUNDORBGMINFO(channel[, selector])`
  - `ISPLAYINGSOUND(channel)`
  - `SOUNDCONTROL(channel, action[, speed[, pitch_flag]])`
  - `ISPLAYINGBGM()`
  - `BGMCONTROL(action[, speed[, pitch_flag]])`
- 以固定蛇版源码为准，不实现旧计划中误列的 Seek；蛇版 ERB API 没有 Seek action。
- 不新增 RustyEra 专用 `EXPORTDATA` 等脚本命令：新的权威 `.sav` 本身即可由蛇版读取。

## 子批次实施

### 5.0 基准、oracle 与最终契约冻结

- 固定三仓 SHA、工具链和当前蛇版 TW 输入：
  - `sav/global.sav`：SHA-256 `56f80b52a8a6c8fc7dd080f9a69967758fb83df966a45330123bbc3d8a1e37cf`。
  - `sav/save1000.sav`：SHA-256 `442b1d41d3d17f2dbfdb6587ae521361bf07174f653affdc0bb82a9693dae0a2`。
  - 两者均为 ERAZIP 1808；`setting.json` 为 `UseNewRandom=true`。
- 用现有蛇版 reference CLI 和一次隔离 Wine GUI 执行冻结：
  - Binary/GZip/Text 普通与 GLOBAL 存档。
  - Integer/String、角色、自定义 1D/2D/3D 数组、Map/XML/DT。
  - 固定蛇版参考接受 Float，而 RustyEra 批次 5 显式拒绝 Float；另冻结未知 tag、截断、zip bomb 的参考实际结果和 Rust 目标拒绝类别。
  - 五项音频函数的签名、返回码、暂停状态、声道分配、速率与 pitch flag 实际语义。
- 参考仓库和游戏保持只读；所有运行使用隔离副本。
- GUI 只使用任务隔离的 Wine prefix；允许从 `~/.wine` 复制必要组件与设置，但不得直接复用该 prefix，不得跟随或复制其 `drive_c/eratw-sub-modding` 链接。启动前持久化 symlink 审计，并断言没有链接解析到蛇版 TW 或用户日常游戏目录。
- NAudio GUI 必须把固定本地蛇版客户端配套的 `SoundTouch_x64.dll` 作为普通文件放在 `Emuera.exe` 同目录，并在证据中绑定来源与目标 SHA-256；不得从日常游戏链接复制或保留链接。
- Binary 与 ERAZIP 冻结 Map/XML/DT；固定参考的 Text writer 不写这些扩展对象，Text 的实际省略/不可恢复结果单独进入 oracle。

### 5.1 协议与 identity 基础

- 在 core 一次性加入协议 46、`LegacyProfileSave`、音频 target/effect/observation 类型和 operation 常量。
- identity 升至 12，明确：
  - 传统存档为 `snake_emuera1808_interop_v1`。
  - `RERASAV v2` 仅为 v11 迁移输入。
  - 精确 SQL/RNG 仍属于 VM snapshot，不属于标准传统存档。
- 同步协议 schema、C ABI、序列化兼容测试和测试工具说明。
- 单独提交 core 公共契约；此后下游只绑定该系列最终 core SHA。

### 5.2 Core 互操作存档 codec 与加载语义

- 蛇版 `SAVEDATA/SAVEGAME/SAVEGLOBAL` 直接编码裸 1808，不再调用 envelope writer；格式严格服从现有保存格式与压缩配置。
- 裸普通存档仅恢复 ordinary state：
  - 不恢复或替换 GLOBAL。
  - 不恢复、重播种或自动读取 RNG。
  - 不保存/恢复 SQL revision，也不因活跃 SQL reader/transaction 阻止传统保存。
  - `RANDDATA` 照常导入；只有脚本执行 `INITRAND` 才影响 SFMT。
- 裸 GLOBAL 只恢复 global scope。
- 增加专用 v11 legacy decoder：
  - 只接受精确已知的 snake identity 11 和 `OwnedSaveStateV1`。
  - 继续原子恢复其 GLOBAL、SFMT、SQL revision。
  - 加载成功发出一次迁移诊断；下一次保存输出裸 1808。
- 裸存档每次加载发出结构化信息诊断，说明其不携带可恢复 RNG/SQL snapshot，本次保持当前 RNG 和 SQL 外部状态。
- 更新传统存档导入/导出：标准蛇版 `.sav` 可直接验证；VM snapshot 行为不变。

### 5.3 Core 多来源槽位与旧存档迁移

- Load 菜单依次列举项目 `Save` 和 `LegacyProfileSave`，建立显式 `path → source` 映射：
  - 项目 `sav` 同槽始终优先，即使其损坏；不得静默回退旧文件。
  - 仅项目文件 NotFound 时才读取 legacy 同槽。
  - 菜单标签区分标准蛇版存档与待迁移旧 RustyEra 存档。
- `LOADDATA`、系统菜单及 `LOADGLOBAL` 实现同样的 primary-first、NotFound-only fallback。
- 保存永远写项目 `sav`；成功后可最佳努力删除被遮蔽的同槽 legacy 文件，失败只警告，不回滚已成功的标准存档。
- `DELDATA` 删除逻辑槽位的所有来源：先删 legacy，再删 primary，避免删除 primary 后旧槽重新出现。
- 把槽位来源状态纳入菜单热重载/状态转移或在恢复时安全清空重扫；不得凭路径猜测来源。
- 保留 revision precondition 和原子替换；现在只提交一个权威 `.sav`，不再存在双写事务。

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
- BGM expected state 继续可恢复；一次性 sound 只作为 transient effect，不因 snapshot/reconnect 重播。
- 缺少 `audio_observation@1` 时：
  - GET/ISPLAYING 系列产生稳定的 `runtime.audio_observation_unavailable` 诊断和 script fault，不能返回伪造的 0。
  - 控制函数仍返回蛇版参数级返回码，并由 unsupported effect 产生一次设备能力警告。

### 5.5 TUI 存档与不支持音频契约

- 蛇版 `Save/GlobalSave` 映射项目 `sav`；`LegacyProfileSave` 映射原隔离 profile 目录。配置了独立 data dir 时仍以项目 `sav` 为互操作主存档。
- 实现 legacy 的读取/列举/删除与 Write 拒绝。
- TUI 继续声明 `audio=false`，不注册 `audio_observation@1`；主动或非协商请求均返回 `frontend.unsupported_service`。
- 更新 TUI 测试 skill：标准蛇版传统存档不拥有 RNG；legacy envelope 与 VM snapshot 才拥有精确 RNG。
- 更新 core pin 和锁文件，记录实际 core SHA。

### 5.6 Web/Tauri 互操作存档

- 浏览器目录项目将蛇版 `Save/GlobalSave` 指向项目文件系统的 `sav`，其他可写 Data/Log/cache 仍保持隔离。
- 打包项目或无原目录写权限时，标准 `.sav` 保存在持久项目副本；复用现有传统存档导入/下载界面与蛇版交换。
- Tauri 将主存档映射项目 `sav`，legacy 映射原 profile 目录，并实施相同的只读/删除策略。
- Browser 与 Tauri 的槽位列表、导入、验证、覆盖确认均使用裸 1808。
- 更新最终 core pin、Cargo.lock/WASM 绑定，并记录 browser 与 Tauri 的实际 core SHA。

### 5.7 Web/Tauri 实际音频 provider

- 将当前匿名 BufferSource 音效池改为 10 个稳定 sound channel 加一个 BGM target。
- 使用可读取 `duration/currentTime/paused/ended/playbackRate` 的媒体元素作为声道主体，并接入现有音量/解锁链路：
  - pause 保存位置，resume 从原位置继续。
  - rate 变化前累计当前位置。
  - finite repeat、无限 BGM、自然结束、覆盖和 stop 均释放资源。
  - 设置标准或 WebKit `preservesPitch` 属性。
- 每个 effect 应用 revision；`audio_observation@1` 返回真实媒体状态和单调时间戳，revision 不符时返回 stale error。
- 仅真实 provider 就绪时广告该服务；解码、自动播放、缺失 pitch 属性和资源失败均返回结构化错误，不伪造成功。
- 浏览器与 Tauri 共用同一引擎语义；平台差异只保留在能力和错误证据中。

### 5.8 集成验收、文档与提交收尾

- 执行真正的双向存档路径：
  1. RustyEra 读取当前蛇版 TW 的 `save1000.sav/global.sav`。
  2. RustyEra 保存新 ERAZIP。
  3. 蛇版 reference 加载、修改并再次保存。
  4. TUI、Chromium、Firefox、Safari、Tauri 分别加载修改后的文件并比较状态。
- 验证旧 v11 `RERASAV` 可加载、迁移后输出为裸 1808，旧文件不会覆盖项目同槽。
- 完成音频三浏览器和 Tauri 实际矩阵，以及 TUI 明确不支持路径。
- 只在全部验收结束后写 `SNAKE_EMUERA_IMPLEMENTATION_LOG.md`、批次总览、迁移计划与分类表；修正旧 Seek 描述。
- 将用户可见行为追加到根 `CHANGELOG_PENDING.md`；纯测试、文档和流程调整不写 changelog。
- core、TUI、Web 和根仓库分别提交；不升级产品发行版本，不推送、不合并。

## 测试与验收规则

- 子批次 5.0 按用户明确要求，任务全过程取消 60 分钟测试墙钟上限；仍保留一次全量、静态门禁先于动态测试、失败后只做最小定向复验及看门狗等其余门禁。
- 每个代码子批次在任何测试前恰好执行一次独立 `$refactor-rustyera-code` 审查，并先落实全部要求。
- 每条测试命令由 `gpt-5.6-terra low` 测试子智能体按对应 `$test-rustyera-core/tui/web` skill 执行。
- 每个子批次独立 60 分钟测试预算；定向测试先于一次完整套件。完整套件失败后只做受影响的定向复验。
- 静态门禁全部通过后才能启动 reference、真实 C ABI、浏览器或 Tauri 动态测试。
- Web/Tauri 动态测试执行每 5 秒完整 DOM/runtime 快照看门狗；连续相同立即失败。

关键验收场景：

- 存档：
  - 当前 `UseNewRandom=true` 的真实蛇版存档可加载，不因无法恢复 `.NET Random` 拒绝。
  - 加载前后 RustyEra RNG 的下一值与“不执行加载”的控制组一致。
  - `RANDDATA` 数组被导入，但除非执行 `INITRAND`，不会改变 RNG。
  - 普通存档不改变 GLOBAL 或现有 SQL 状态；`EVENTLOAD` 的显式数据库重建正常。
  - Binary/GZip/Text、normal/global、Integer/String、角色、自定义数组、Map/XML/DT 双向通过。
  - Float、未知 tag、损坏 header、截断、超限解压显式拒绝且加载原子回滚。
  - 项目槽位优先、legacy fallback、覆盖迁移和双来源删除无幽灵槽位。
- 音频：
  - 10 声道首空闲/全忙覆盖 0、paused 视为空闲。
  - GET omitted selector 的返回值与 `RESULT:0..4` 完全一致。
  - position 播放时单调、暂停时在容差内稳定、resume 后继续、stop 后归零或 stopped。
  - rate、volume、repeat、preserve-pitch 与蛇版返回码一致。
  - stale revision、自然结束、解码失败、自动播放失败和缺少 provider 均有稳定诊断。
  - TUI 的实际查询明确失败，不返回貌似可信的 stopped 值。

## 已确定的假设

- 标准蛇版 `.sav` 不拥有 RNG、GLOBAL 或 SQL 状态；这是对蛇版实际行为的兼容，不再沿用批次 4 的自有传统存档定义。
- 裸存档加载保持当前 SFMT，不自动 `DUMPRAND`、`INITRAND` 或重新播种。
- 旧 v11 envelope 仅作为迁移输入继续精确恢复；新写入永远是裸 1808。
- 当前批次不实现 Float、`.NET Random` 状态序列化、Seek 或蛇版之外的新脚本 API。
- 参考实现和游戏仓库保持只读，所有写入测试使用隔离副本。
