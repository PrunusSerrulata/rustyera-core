# RustyEra

RustyEra 是用 Rust 重新实现的 EraBasic 语言工具链与运行环境，兼容目标固定为
Emuera 参考实现提交 `26a35dc9334bb67590b96f7b8efbefbf199e391e`（Emuera 1.824
系列）。项目覆盖从 UTF-8 源码、静态数据、语义分析、字节码、虚拟机到可移植
runtime 协议和 C ABI 的完整链路，并提供一个 Python/Textual TUI 作为集成验证前端。

项目仍处于开发阶段，尚未覆盖 Emuera 的全部指令、系统流程以及依赖
WinForms/GDI/CBG 的客户端能力。协议中存在类型或参考 CLI 能够执行某项操作，不代表
Rust 实现已经支持对应能力。

## 项目目标

发生目标冲突时，优先级依次为：

1. 跨客户端、跨平台支持；
2. 架构纯净；
3. 与固定 Emuera 参考实现严格保持行为一致。

这一顺序只用于解决冲突。只要不违背更高优先级，游戏规则、输入判定、状态变化以及
其他脚本可观察行为仍必须与参考实现一致。依赖具体窗口系统、字体、设备状态或像素
布局的行为，应提炼为 runtime 持有的可移植语义，再由不同前端投影。

RustyEra 只接受 UTF-8 源码和配置内容，不负责识别或转换 Shift-JIS、GBK 等传统编码。
详细原则见[设计原则](docs/design-principles.zh-CN.md)。

## 项目结构

### EraBasic 语言与执行链路

| 模块 | 职责 |
| --- | --- |
| `erabasic-ast` | 公共 AST、UTF-8 byte span 与稳定诊断结构。 |
| `erabasic-lexer` | 上下文相关词法分析、终止规则、宏与 FORM 字符串拆分。 |
| `erabasic-parser` | 表达式、逻辑行、ERH、ERB、预处理器与块结构解析。 |
| `erabasic-config` | Emuera 配置项的可序列化模型与规范化处理。 |
| `erabasic-data` | 项目静态数据、初始化数据与存档加载契约。 |
| `erabasic-csv` | 只处理前端提交内容的内存 CSV 加载器，不执行文件 I/O。 |
| `erabasic-hir` | 稳定、可序列化的类型化高级中间表示。 |
| `erabasic-analyzer` | 项目级符号、类型、声明、指令参数与控制流分析。 |
| `erabasic-bytecode` | 版本化 VM 指令、Host ABI、自包含容器、源码映射与补丁。 |
| `erabasic-compiler` | 确定性、可并行且支持函数级缓存的 HIR 到字节码编译器。 |
| `erabasic-validator` | HIR 与不可信字节码的结构、类型、控制流和 ABI 验证。 |
| `erabasic-vm` | 确定性解释器、协作式多 fiber 调度、Host/Native 边界、快照与热替换。 |
| `erabasic-html` | EraBasic HTML 子集的规范化与安全文本处理。 |
| `erabasic-repl` | 用于人工检查 lexer、parser 和 analyzer 的开发工具。 |

### Runtime、协议与边界

| 模块 | 职责 |
| --- | --- |
| `era-protocol` | 确定性 CBOR 信封、版本协商、标识符、限制与诊断投影。 |
| `era-runtime-protocol` | 正常运行时的生命周期、项目、输入、展示、存储与服务消息。 |
| `era-debug-protocol` | 独立版本、按能力授权的调试协议。 |
| `era-runtime-save` | 不执行文件 I/O 的传统存档编解码、迁移与恢复。 |
| `era-runtime` | caller-pumped 权威 runtime，驱动 VM 并持有游戏、展示、交互和存档状态。 |
| `era-source-extractor` | 从 runtime 项目编译缓存中精确提取嵌入的 UTF-8 源码快照。 |
| `era-runtime-ffi` | 安全 Rust FFI 函数表与经过检查的结构声明。 |
| `era-runtime-capi` | C ABI 动态库实现；这是 workspace 中唯一包含原始指针 `unsafe` 边界的 crate。 |

### 前端、参考实现与工具

| 路径 | 用途 |
| --- | --- |
| `frontends/era-tui` | Python 3.12/Textual TUI，通过公共 C ABI 驱动 runtime。主要用于验证参考行为、真实游戏脚本和 C ABI 可用性，不是排版或性能标杆。 |
| `reference/emuera.em` | 固定版本的 C# Emuera 兼容性参考实现。 |
| `reference/emuera.em/emuera-reference-cli` | 无窗口 NDJSON oracle，用于差分测试。 |
| `reference/eraTW` | 真实游戏 eraTW 的 CSV、ERH 与 ERB 输入；不纳入版本控制。 |
| `tools/runtime-tester` | runtime、C ABI 与 TUI 的人工/长流程测试工具。 |
| `tools/protocol-smoke.ps1`、`tools/test-macos-wine.sh` | Windows 与 macOS/Wine 参考 CLI 冒烟测试。 |

## 模块关系

```mermaid
flowchart LR
    SRC[UTF-8 ERH/ERB] --> LEX[erabasic-lexer]
    LEX --> PARSER[erabasic-parser]
    PARSER --> AST[erabasic-ast]

    CSVFILES[前端提交的 CSV 内容] --> CSV[erabasic-csv]
    CSV --> DATA[erabasic-data]
    AST --> ANALYZER[erabasic-analyzer]
    DATA --> ANALYZER
    ANALYZER --> HIR[erabasic-hir]
    HIR --> COMPILER[erabasic-compiler]
    COMPILER --> BYTECODE[erabasic-bytecode]
    BYTECODE --> VALIDATOR[erabasic-validator]
    VALIDATOR --> VM[erabasic-vm]

    TUI[Python Textual TUI] <--> CAPI[era-runtime-capi]
    CAPI --> FFI[era-runtime-ffi]
    FFI --> RUNTIME[era-runtime]
    RUNTIME <--> VM
    RUNTIME <--> RPROTO[era-runtime-protocol]
    RUNTIME <--> DPROTO[era-debug-protocol]
    RUNTIME <--> SAVE[era-runtime-save]
```

应用前端负责文件扫描、UTF-8 解码、终端或 GUI 投影、平台输入与实际 I/O。runtime
只接收版本化消息和前端提交的数据，不直接访问文件系统、系统时钟或设备 API。

## 环境要求

- 当前稳定版 Rust 工具链，支持 workspace 使用的 Rust 2024 edition；
- 构建 TUI 时需要 Python 3.12 或更高版本及 `uv`；
- 运行 C# 参考 CLI 时需要 .NET 10 Windows Desktop 工具链；
- macOS 上运行参考 CLI 还需要 Wine、`jq` 和 Perl。

所有命令默认从仓库根目录执行。

## 编译

编译全部 Rust crate：

```sh
cargo build --workspace
```

编译 release C ABI 动态库：

```sh
cargo build --release -p era-runtime-capi
```

动态库位于 `target/release/`，文件名分别为：

- macOS：`libera_runtime_capi.dylib`
- Linux：`libera_runtime_capi.so`
- Windows：`era_runtime_capi.dll`

## 使用方法

### EraBasic REPL

```sh
cargo run -p erabasic-repl
```

REPL 支持：

```text
:lex SOURCE
:expr SOURCE
:line SOURCE
:file PATH
:analyze PATH...
:help
:quit
```

`:file` 将 `.erh` 作为声明头处理，其他文件作为 ERB 处理。REPL 会保留同一个 parser
上下文，因此先加载的宏和声明会影响后续输入。它只是开发检查工具，不是游戏运行前端。

### Python TUI

先安装依赖：

```sh
uv sync --project frontends/era-tui
```

TUI 接收一个可选资源目录，默认使用当前工作目录。资源目录中应包含 `CSV/`、`ERB/`
以及当前平台的 `era-runtime-capi` release 动态库；存档、日志、快照和编译缓存默认也
写入该目录。

```sh
uv --project frontends/era-tui run rustyera-tui /path/to/resource-directory
```

也可通过 `--runtime-library PATH` 或 `ERA_RUNTIME_LIBRARY=PATH` 单独指定动态库。
仓库开发环境可以在 `frontends/era-tui` 下建立指向 `reference/eraTW/{CSV,ERB}` 和
`target/release` 动态库的相对符号链接；这些本地链接已被 `.gitignore` 排除。

TUI 的功能、按键和存储约定见
[TUI 说明](frontends/era-tui/README.md)。

### 项目源码提取器

成功编译的项目缓存内包含前端提交的完整 UTF-8 源码快照。源码提取器直接恢复该快照，
不会从字节码反推源码，因此不属于传统意义上的反编译器：

```sh
cargo run -p era-source-extractor -- \
  /path/to/compiled-project-v7.bin.zst [/path/to/output]
```

省略输出目录时写入当前工作目录。默认拒绝覆盖已有文件；显式传入 `--force` 才会覆盖
普通文件。工具恢复 CSV、ERH、ERB、配置和 UTF-8 资源清单并保留相对目录，跳过图片、
音频等二进制资源。它不支持旧版项目缓存或通用 `.erbc` 字节码容器。

仓库维护者可构建提取器后，对 `reference/` 下发现的全部 Era 游戏执行编译缓存往返：

```sh
cargo build -p era-source-extractor
cargo run --manifest-path tools/runtime-tester/Cargo.toml -- source-extractor-all
```

### 作为 Rust 库使用

解析 ERH 与 ERB：

```rust
use erabasic_parser::{DefaultParserContext, parse_erb, parse_erh};

let mut context = DefaultParserContext::default();
let header = parse_erh("#DEFINE TEN 10\n", &mut context);
assert!(!header.has_errors());

let script = parse_erb("@TEST\nRESULT = TEN + 1\n", &mut context);
assert!(!script.has_errors());
```

加载前端已读取的 CSV：

```rust
use erabasic_csv::{CsvLoadOptions, FilePayload, FrontendFile, ProjectFiles, load_project};

let report = load_project(
    &ProjectFiles {
        csv: vec![FrontendFile {
            relative_path: "ABL.csv".into(),
            payload: FilePayload::Utf8("0,力量\n".into()),
        }],
        erb: vec![],
    },
    &CsvLoadOptions::default(),
);
assert!(report.data.is_some());
```

## 验证与测试

开发流程和测试职责以 [AGENTS.md](AGENTS.md) 为准。修改 Rust 实现时，先完成格式化，
处理全部编译器错误和 Clippy 警告，再执行全量测试：

```sh
cargo fmt --all
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

兼容性修改还必须运行当前平台的参考 CLI 冒烟测试，并使用相同输入比较 Rust 与 C#
结果：

```powershell
tools/protocol-smoke.ps1
```

```sh
tools/test-macos-wine.sh
```

## 文档

- [设计原则](docs/design-principles.zh-CN.md)
- [输入与等待兼容性](docs/input-wait-compatibility.zh-CN.md)
- [Emuera runtime 参考映射](docs/runtime-reference-mapping.zh-CN.md)
- [参考 CLI](reference/emuera.em/emuera-reference-cli/README.md)
- [参考实现 headless 修改记录](reference/emuera.em/emuera-reference-cli/REFERENCE_CHANGES.md)
- [运行时测试工具](tools/runtime-tester/AGENTS.md)

## 许可证

RustyEra 自有代码和文档采用
[GNU 通用公共许可证第 3 版](LICENSE)（SPDX：`GPL-3.0-only`）。
`reference/emuera.em` 及其他第三方内容仍分别遵循其随附许可证，GPLv3 不改变这些
第三方材料的原有授权条款。

## 致谢

RustyEra 受益于 era 生态长期积累的工具、实现和创作内容，谨向下列作者与贡献者致谢：

- **佐藤敏**（サークル獏）：eramaker 的开发者
- **MinorShift** 与 **妊）|дﾟ)の中の人**：Emuera 的著作者。RustyEra 以仓库中固定版本的
  Emuera 为兼容性参考实现。
- **まだ名前は無い人**：`eraThe World`（eraTW）项目在 `GameBase.csv` 中署名的修改／制作
  者；以及该项目列出的咨询协作者 **哆来咪**。
- **eraTW 的口上与内容作者、改编者**：所有为角色口上、事件、数据与文档作出贡献的
  创作者。完整署名、改编记录与各自的使用条件均保留在
  `reference/eraTW/ERB/口上・メッセージ関連/個人口上/` 的随附说明和许可文件中。
- **所有开源依赖、工具维护者和 era 社区的脚本／内容作者**。

本项目的致谢不改变 `reference/` 内第三方材料的著作权、署名、许可或使用条件；使用或再分发
这些材料时，请以其随附文件为准。
