# Runtime presentation command intent notes

本文暂存对 `GETDISPLAYLINE`、Emuera HTML 辅助函数、`PRINTC`、`GETLINESTR` 和
`DRAWLINE` 的参考实现调研，以及面向 Runtime—前端分离架构的后续实现方案。

## 调研结论

| 指令或函数 | 参考实现的主要行为 | eraTW 脚本使用情况 |
| --- | --- | --- |
| `GETDISPLAYLINE` | 按当前日志中的绝对物理行下标返回纯文本，越界返回空串。 | `real-erb` 和 `eraTW-minimal` 均未检出。 |
| `HTML_GETPRINTEDSTR` | 从最新行向前取得指定逻辑行，将其物理折行、样式和按钮规范化为 Emuera 伪 HTML。 | 均未检出。 |
| `HTML_POPPRINTINGSTR` | 清空尚未提交的打印缓冲区并将其转换为伪 HTML，不弹出已提交日志。 | 均未检出。 |
| `HTML_STRINGLEN` | 以参考实现的字体度量计算伪 HTML 第一物理行的宽度，而不是计算字符数。 | 均未检出。 |
| `HTML_SUBSTRING` | 按显示宽度切分伪 HTML，平衡样式标签，并把前段和剩余段写入 `RESULTS`。 | 均未检出。 |
| `HTML_STRINGLINES` | 反复按显示宽度切分伪 HTML并返回行数。非正宽度存在不取得进展的风险。 | 均未检出。 |
| `PRINTC` 家族 | 将内容作为独立排版单元追加到当前行；`C` 右对齐，`LC` 左对齐，参考实现依赖 `PrintCLength`、CP932 字节数和字体像素度量。 | 高频用于多列菜单、分页按钮、占位单元和角色/指令选择。完整脚本文本命中约 251 次。 |
| `GETLINESTR` | 重复参数字符串并按默认字体截短，返回填满可绘制宽度的装饰字符串；空参数报错。 | 均未检出。其明显意图是生成满宽装饰文本。 |
| `DRAWLINE` | 重复配置的分隔符至可绘制宽度，使用常规字体打印并换行。 | 高频用于菜单、状态区和事件消息分隔；完整脚本文本命中约 979 次。 |

命中数包含注释，只用于衡量相对重要性，不代表实际执行次数。代表性用法包括：

- `reference/eraTW-minimal/ERB/SHOP関連/BONUS2.ERB` 使用 `PRINTFORMC` 构造分页菜单。
- `reference/eraTW-minimal/ERB/SHOP関連/SHOP.ERB` 使用 `PRINTLC` 构造主菜单网格。
- `reference/eraTW-minimal/ERB/コマンド関連/USERCOM_コマンド表示処理.ERB` 使用
  `PRINTFORMC` 构造指令选择网格。
- `reference/eraTW-minimal/ERB/NEWGAME/NEWGAME_UTILS.ERB` 和事件脚本使用
  `DRAWLINE` 分隔交互区域与消息。

当前编译器目录已经认识这些名称，但首阶段 Runtime 仅把所有 `PRINT*` 和
`DRAWLINE` 当作普通文本追加，并为每次追加创建独立逻辑行。因此当前实现不能表达
连续 `PRINTC` 单元构成的一行菜单，无参数的 `DRAWLINE` 也只会产生空文本行；上述
查询和 HTML 函数则会成为不支持的 Host import。

## 已选择的设计方向

这些指令主要表达 UI 意图。Runtime 保存权威、规范化的展示语义以及所有交互 Token，
前端负责将这些语义投影为 GUI、TUI 或其他平台的具体像素布局。字体、窗口和前端能力
不得反向影响游戏规则或输入判定。

### `PRINTC` 家族

在展示协议中增加语义单元格节点，例如：

```rust
DisplayRun::ColumnCell {
    content: Vec<DisplayRun>,
    alignment: CellAlignment,
    preferred_columns: u32,
}
```

- `PRINTC`/`PRINTFORMC` 产生右对齐提示，`PRINTLC`/`PRINTFORMLC` 产生左对齐提示。
- 连续单元格留在同一个 Runtime 逻辑行缓冲区，直至 `PRINTL` 等行为提交该行。
- 不在 Runtime 中填充依赖字体的空格；`preferred_columns` 只是前端布局提示。
- 单元格中的按钮与交互 Token 仍由 Runtime 创建、保存和验证。
- 支持语义节点的前端可使用 grid、flex 或固定列；纯文本投影在单元格之间插入稳定的
  简单间隔。

### `DRAWLINE`

在展示协议中增加 `Separator` 节点，保存可选 pattern 和语义 role。

- 若当前逻辑行已有内容，先提交该行，再产生独立分隔线并结束其逻辑行。
- GUI 可以绘制真正的水平线，TUI 可以按视口重复 pattern。
- 不支持该节点的投影使用确定性的固定长度文本分隔线。
- 具体像素长度不进入 Runtime 权威状态。

### `GETLINESTR`

该函数必须把普通字符串返回 VM，不能完全交给前端，否则不同设备会改变脚本结果。
采用跨平台、确定性的逻辑列近似：

- Runtime 配置固定 `logical_line_columns`，并将其纳入会话和持久化状态。
- 使用 Unicode grapheme 与显示列宽重复 pattern，不拆分 grapheme cluster。
- 空 pattern 保持报错。
- 优先考虑 `unicode-segmentation` 与 `unicode-width`。
- 结果不依赖前端字体、像素宽度或 viewport。若以后发现真实脚本依赖精确返回值，再
  增加独立的参考布局兼容模式。

### 暂缓的查询与 HTML 函数

`GETDISPLAYLINE`、`HTML_GETPRINTEDSTR`、`HTML_POPPRINTINGSTR`、
`HTML_STRINGLEN`、`HTML_SUBSTRING` 和 `HTML_STRINGLINES` 暂不实现。执行到这些
函数时应产生带命令名和源码位置的稳定 `UnsupportedRuntimeFeature` 故障，不得静默
返回空串并改变脚本分支。语言前端和编译器仍可保留其名称与签名。

## 建议实施顺序

1. 扩展展示协议，加入 `ColumnCell` 和 `Separator`，并同步更新协议版本和 C ABI。
2. 将 Runtime 展示模型改为真正的逻辑行缓冲区。
3. 实现 `PRINTC` 家族及 `DRAWLINE` 的 Host 调用分派。
4. 实现确定性的 `GETLINESTR` 逻辑列算法。
5. 实现按前端能力生成的语义投影和纯文本降级投影。
6. 添加连续单元格、按钮 Token、Unicode pattern、分隔线、能力降级和不支持函数故障测试。

这是一项明确的跨平台展示策略：不承诺复刻 Windows GDI 的像素度量，但必须保持菜单
结构、交互关系、逻辑行顺序、Runtime 权威状态以及确定性。
