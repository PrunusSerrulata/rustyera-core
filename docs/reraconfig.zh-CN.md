# `reraconfig.toml` 设计

`reraconfig.toml` 是 RustyEra 项目唯一的外部配置文件。它采用 UTF-8 编码，所有字段均可
省略；读取时，缺失字段使用 `era-config` 目录中的统一默认值。客户端必须接受 LF、CRLF
和单独 CR 行尾，写入时使用所在操作系统的原生行尾。

## 格式原则

- 文件使用 TOML，设置按用途放入浅层表中，例如 `text.font_size` 和
  `save.auto_save`。设置名称使用小写、完整且直观的英语单词。
- `[meta]` 只保存格式元数据。`schema_version` 当前为整数 `1`；
  `locked_settings` 是不可由客户端设置面板修改的规范路径数组，用来承接旧
  `_fixed.config` 的语义。
- 未知表、未知字段、错误类型、越界整数和无效枚举值均被拒绝，
  防止拼写错误被静默忽略。
- 修改单项设置时保留文件中其他字段、排序、空行和注释。配置文档层不执行文件 I/O，
  由客户端负责读取、原子写入和选择原生行尾。
- 配置目录为每项分配稳定的正整数 ID，并记录旧 Emuera code、适用客户端和热应用策略。
  新项目配置和未来客户端共用同一目录，不允许客户端另设隐藏的项目配置文件。

完整中文注释示例位于
`crates/era-config/schema/reraconfig.example.toml`，机器可读 schema 位于
`crates/era-config/schema/reraconfig.schema.json`。两者由
`cargo run -p era-config --example generate_reraconfig_artifacts` 从代码目录确定性生成。

## 新增的通用设置

- `audio.volume`：整数 `0..=100`，默认 `100`，表示游戏主音量百分比；支持热应用。
- `text.replace_full_width_spaces`：布尔值，默认 `false`。启用后仅由前端把游戏输出中的
  一个全角空格显示为两个半角空格，不改变 core 中的原始文本；支持热应用。
- `text.character_width_mode`：字符串枚举，默认 `"automatic"`。当前提供
  `"automatic"`、`"ambiguous_narrow"` 和 `"ambiguous_wide"` 三个占位策略；支持热应用，
  但本阶段不实现具体列宽算法。

TUI 不暴露音量设置，因为终端音频不由 TUI 控制；浏览器和 Tauri 暴露全部三项。

## 统一默认值

过去各客户端对部分缺失设置使用不同默认值。`reraconfig.toml` 统一采用：历史日志
`1000` 行、每行 `5` 个 PRINTC 项、每项宽度 `24` 列。其他设置使用配置目录列出的默认值。

## 旧项目迁移

仅当项目不存在 `reraconfig.toml` 时，客户端才收集旧设置文件并调用迁移器。迁移器按下列
顺序合并内容，后者覆盖前者：

1. `CSV/_default.config` 或 `CSV/default.config`；
2. `emuera.config`；
3. `setting.json` 中的 `UseNewRandom`；
4. `CSV/_fixed.config` 或 `CSV/fixed.config`，并把这些设置写入
   `meta.locked_settings`；
5. `debug.config`；
6. 启用 `replacement.enabled` 时读取 `_Replace.csv` 中的固定替换设置。

迁移结果只写出不同于 RustyEra 统一默认值或被锁定的字段。旧文件保留在项目中但不再作为
后续配置来源；生成成功后立即按普通 `reraconfig.toml` 路径读取。迁移器对每个无法解析的
旧行产生诊断，避免静默丢失用户设置。

## 设置范围

配置目录覆盖参考实现的 `emuera.config`、默认/固定配置、`debug.config`、
`setting.json` 和 `_Replace.csv` 所描述的固定设置，以及 RustyEra 通用客户端设置。
`_Rename.csv` 仍属于游戏数据映射，不是固定配置，因此不并入本文件。
