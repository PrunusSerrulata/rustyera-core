# EraBasic Parser Notes for Rust Rewrite

本文档基于 `Emuera/Runtime/Script/Parser/`，并补充其直接调用边界：`ExpressionParser`、`VariableParser`、`ArgumentParser`、`ErbLoader`、`ErhLoader`、`StrForm`。目标是为用 Rust 重写 EraBasic 解释器时建立等价的词法、行解析、表达式解析和参数解析模型。

## 1. 源码范围

核心 Parser 目录：

| 文件 | 主要职责 |
| --- | --- |
| `Parser/Word.cs` | 定义词法 token，即 `Word` 及其子类。 |
| `Parser/SubWord.cs` | 定义格式化字符串内部的子 token，即 `{}`、`%%`、`\@?...#...\@`、三连符号。 |
| `Parser/WordCollection.cs` | token 链表与可移动指针，用于 parser 消费 token。 |
| `Parser/LexicalAnalyzer.cs` | 词法分析、字符串读取、数字读取、宏展开、格式化字符串解析。 |
| `Parser/LogicalLineParser.cs` | 将一行源码粗分为标签、指令、赋值、预处理属性等逻辑行。 |

直接相关文件：

| 文件 | 关系 |
| --- | --- |
| `Statements/Expression/ExpressionParser.cs` | 消费 `WordCollection`，构造表达式树 `AExpression`。 |
| `Statements/Expression/OperatorCode.cs` | 运算符枚举、优先级、运算符字符串映射。 |
| `Statements/Expression/OperatorMethod.cs` | 运算符对不同类型的实际规约规则。 |
| `Statements/Variable/VariableParser.cs` | 解析变量及 `:` 下标/参数。 |
| `Statements/ArgumentParser.cs` | 为 `InstructionLine` 调用对应参数构造器。 |
| `Statements/ArgumentBuilder.cs` | 不同命令的参数解析规则。 |
| `Statements/LogicalLine.cs` | `LogicalLine`、`InstructionLine`、`FunctionLabelLine` 等 Parser 输出结构。 |
| `Script/Loader/ErbLoader.cs` | `.ERB` 文件装载、预处理、按行调用 `LogicalLineParser`。 |
| `Script/Loader/ErhLoader.cs` | `.ERH` 头文件装载，处理 `#DEFINE`、`#DIM`、`#DIMS`。 |
| `Script/Data/StrForm.cs` | 将 `StrFormWord` 转换成运行期格式化字符串表达式。 |

## 2. 总体解析架构

Emuera 的 Parser 分层并不是一次性从源码生成完整 AST，而是分阶段、延迟解析：

1. `ErbLoader` 读取 `.ERB` 文件，处理 `[IF]`、`[ELSE]` 等预处理块。
2. 遇到 `#` 行时，`LogicalLineParser.ParseSharpLine` 修改当前函数标签属性。
3. 遇到 `@` 或 `$` 行时，`LogicalLineParser.ParseLabelLine` 生成函数标签或跳转标签。
4. 遇到普通行时，`LogicalLineParser.ParseLine` 粗分为指令行、赋值行、无效行。
5. `InstructionLine` 通常只保存原始参数 `CharStream`，不会马上解析全部参数。
6. `ErbLoader.setLabelsArg` 后续会解析函数标签参数。
7. `ErbLoader.ParseScript` 中的 `setArgument` 会按需调用 `ArgumentParser.SetArgumentTo`。
8. `ArgumentBuilder` 读取 `InstructionLine` 的原始参数，调用 `LexicalAnalyzer.Analyse` 得到 `WordCollection`。
9. `ExpressionParser` 消费 `WordCollection`，解析为 `AExpression` 树。
10. `VariableParser` 被 `ExpressionParser` 调用，用于变量与变量下标解析。
11. `nestCheck` 和 `setJumpTo` 后续检查 `IF/ENDIF`、循环、`SELECTCASE`、跳转目标等结构关系。

Rust 重写时建议保留这一分层：`Lexer`、`LineParser`、`ExpressionParser`、`ArgumentParser`、`Loader/Preprocessor` 分开实现。这样能较好兼容 Emuera 的“延迟参数解析”和“不同命令有不同参数词法规则”。

## 3. 输入与字符流

`CharStream` 是可变位置的字符流，核心行为如下：

| 成员 | 含义 |
| --- | --- |
| `Current` | 当前字符，越界返回 `\0`。 |
| `Next` | 下一个字符，越界返回 `\0`。 |
| `EOS` | `pointer >= source.Length`。 |
| `ShiftNext()` | 指针加一。 |
| `Jump(n)` | 指针加 `n`。 |
| `Seek(offset, origin)` | 设置或相对移动指针。 |
| `Find(str)` | 从当前位置查找字符串并返回相对偏移。 |
| `CurrentEqualTo(str)` | 判断当前位置是否匹配指定字符串。 |
| `TripleSymbol()` | 判断当前起连续 3 个字符完全相同。 |

Rust 可用以下形态替代：

```rust
struct CharStream<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
}
```

注意：C# 中 `string` 以 UTF-16 码元索引，Rust `str` 是 UTF-8 字节索引。若要精确兼容日文、全角空格和任意 Unicode 标识符，建议内部转换为 `Vec<char>` 或自定义 `CharCursor`，错误位置再映射回原始行。

## 4. Token 类型

所有普通 token 继承 `Word`，都有 `Type: char` 与 `IsMacro` 标记。

| C# 类型 | `Type` | 字段 | 语义 |
| --- | --- | --- | --- |
| `NullWord` | `\0` | 无 | 伪 token，表示行尾或链表指针为空。 |
| `IdentifierWord` | `A` | `Code: string` | 标识符、关键字、变量名、函数名。 |
| `LiteralIntegerWord` | `0` | `Int: long` | 64 位有符号整数常量。 |
| `LiteralStringWord` | `"` | `Str: string` | 双引号或特定上下文单引号字符串常量。 |
| `OperatorWord` | `=` | `Code: OperatorCode` | 运算符 token。`Type` 固定为 `=`，实际运算符看 `Code`。 |
| `SymbolWord` | 原字符 | `char` | 标点符号，如 `(`、`)`、`[`、`]`、`,`、`:`、`@`、`.`。 |
| `StrFormWord` | `F` | `Strs: string[]`、`SubWords: SubWord[]` | 格式化字符串。`Strs.len = SubWords.len + 1`。 |
| `MacroWord` | `M` | `Number: int` | 函数型宏形参占位符。当前 `.ERH` 中函数型宏定义被禁用，但类型仍存在。 |

Rust 建议定义：

```rust
enum Word {
    Null,
    Identifier { code: String, is_macro: bool },
    Integer { value: i64, is_macro: bool },
    String { value: String, is_macro: bool },
    Operator { op: OperatorCode, is_macro: bool },
    Symbol { ch: char, is_macro: bool },
    StrForm { parts: Vec<String>, subwords: Vec<SubWord>, is_macro: bool },
    MacroArg { index: usize, is_macro: bool },
}
```

`Word.Type` 在 C# 中承担快速分派作用。Rust 中不建议复刻 `char Type`，直接模式匹配 enum 更清晰，但为了移植时对照错误逻辑，可以提供一个 `kind_char()` 调试方法。

## 5. 格式化字符串 SubWord 类型

`SubWord` 只出现在 `StrFormWord` 内部。

| C# 类型 | 来源语法 | 字段 | 语义 |
| --- | --- | --- | --- |
| `TripleSymbolSubWord` | `***`、`+++`、`===`、`///`、`$$$` | `Code: char` | 特殊简写，运行时映射为角色名或称呼表达式。 |
| `CurlyBraceSubWord` | `{ expression [, width [, LEFT|RIGHT]] }` | `Words: WordCollection` | 数值插值，要求第一个表达式为整数。 |
| `PercentSubWord` | `% expression [, width [, LEFT|RIGHT]] %` | `Words: WordCollection` | 字符串插值，要求第一个表达式为字符串。 |
| `YenAtSubWord` | `\@ condition ? left # right \@` | `Words`、`Left`、`Right` | 条件格式化字符串。`Right` 可因兼容警告缺失而为 `null`。 |

`StrForm.FromWordToken` 中的三连符号映射：

| 语法 | 含义 |
| --- | --- |
| `***` | `NAME:TARGET` 格式化输出。 |
| `+++` | `CALLNAME:MASTER` 格式化输出。 |
| `===` | `CALLNAME:PLAYER` 格式化输出。 |
| `///` | `NAME:ASSI` 格式化输出。 |
| `$$$` | `CALLNAME:TARGET` 格式化输出。 |

## 6. WordCollection 行为

`WordCollection` 是 `LinkedList<Word>` 加当前指针 `Pointer`。

| 方法/属性 | 行为 |
| --- | --- |
| `PointerReset()` | 指针回到首 token，索引归零。 |
| `Add(Word)` | 尾部追加 token。 |
| `Add(WordCollection)` | 将另一个集合中的 token 依次追加。当前实现是浅拷贝 token 对象。 |
| `Current` | 当前 token；指针为空时返回静态 `NullWord`。 |
| `Next` | 下一个 token；不存在返回静态 `NullWord`。 |
| `EOL` | 指针为空。 |
| `ShiftNext()` | 指针移动到下一个 token，并递增内部 index。 |
| `Insert`、`InsertRange`、`Remove` | 为宏展开修改 token 链表。 |
| `SetIsMacro()` | 标记本集合中的 token 由宏展开而来。 |
| `Clone()` | 浅克隆 token 列表。 |

Rust 重写可用 `Vec<Word>` 加 `pos: usize`，宏展开时用 `Vec::splice`。C# 使用链表主要是为了方便插入宏展开内容，Rust 的 `Vec` 对行级 token 通常足够，且更易借用检查。

## 7. 词法终止模式与标志

`LexicalAnalyzer.Analyse` 根据调用上下文使用不同终止模式。

### 7.1 LexEndWith

| 枚举 | 终止条件 | 典型用途 |
| --- | --- | --- |
| `None` | 无特殊终止，仍会在行尾停止。 | 保留。 |
| `EoL` | 读到行尾。 | 普通表达式/参数。 |
| `Operator` | 顶层遇到运算符停止。 | 赋值语句左侧。 |
| `Question` | 顶层遇到 `?` 停止。 | `\@ condition ? ...`。 |
| `Percent` | 顶层遇到 `%` 停止。 | `% ... %`。 |
| `RightCurlyBrace` | 遇到 `}` 停止。 | `{ ... }`。 |
| `Comma` | 顶层遇到 `,` 停止。 | `TIMES` 第一参数等特殊命令。 |
| `GreaterThan` | 遇到 `>` 停止。 | HTML 标签解析。 |

顶层的含义是小括号和中括号嵌套深度为 0。`Comma` 只检查小括号深度为 0，源码中中括号条件被注释掉。

### 7.2 FormStrEndWith

| 枚举 | 终止条件 | 用途 |
| --- | --- | --- |
| `None` | 无特殊终止。 | 保留。 |
| `EoL` | 行尾。 | `FORM_STR` 到行尾。 |
| `DoubleQuotation` | `"`。 | `@"..."`。 |
| `Sharp` | `#`。 | `\@ cond ? left # right \@` 的 left 部分。 |
| `YenAt` | `\@`。 | `\@ cond ? left # right \@` 的 right 部分。 |
| `Comma` | `,`。 | `FORM_STR_ANY`、输入命令等。 |
| `LeftParenthesis_Bracket_Comma_Semicolon` | `(`、`[`、`,`、`;`。 | `CALLFORM` 类函数名部分。 |

### 7.3 StrEndWith

| 枚举 | 终止条件 | 用途 |
| --- | --- | --- |
| `None` | 无特殊终止。 | 保留。 |
| `EoL` | 行尾。 | 保留。 |
| `SingleQuotation` | `'`。 | HTML 单引号字符串。 |
| `DoubleQuotation` | `"`。 | 普通字符串字面量。 |
| `Comma` | `,`。 | `PRINTV '...'` 兼容语法。 |
| `LeftParenthesis_Bracket_Comma_Semicolon` | `(`、`[`、`,`、`;`。 | `CALL` 类函数名部分。 |

### 7.4 LexAnalyzeFlag

| 标志 | 值 | 作用 |
| --- | --- | --- |
| `None` | `0` | 默认。 |
| `AnalyzePrintV` | `1` | 允许 `PRINTV` 参数中 `'` 后面的文本作为字符串读到逗号或行尾。 |
| `AllowAssignment` | `2` | 允许 `=` 被词法解析为 `OperatorCode.Assignment`。否则裸 `=` 在表达式中报错。 |
| `AllowSingleQuotationStr` | `4` | 允许 `'...'` 单引号字符串，HTML 解析使用。 |

## 8. 基础词法规则

### 8.1 空白与注释

`SkipWhiteSpace` 用于行解析和词法解析前置跳过：

| 字符/模式 | 行为 |
| --- | --- |
| 空格 ` ` | 跳过。 |
| Tab `\t` | 跳过。 |
| 全角空格 `　` | 仅当 `Config.SystemAllowFullSpace` 为真时跳过，否则停止或报错，取决于调用点。 |
| `;` | 普通行内注释，直接跳到行尾。 |
| `;#;` | Debug 模式下跳过这 3 个字符后继续解析。 |
| `;!;` | 跳过这 3 个字符后继续解析。 |
| `;^;` | 只在 `SkipWhiteSpace` 中作为特殊注释处理。 |

`LexicalAnalyzer.Analyse` 内部也处理行内注释，但只特殊跳过 `;#;` 和 `;!;`，其他 `;` 直接截断到行尾。

`SkipHalfSpace` 只跳过半角空格，字符串赋值和格式化字符串参数中用于兼容旧行为。

### 8.2 标识符

`ReadSingleIdentifierROS` 读取到以下任意字符为止：

```text
半角空格, 全角空格, tab, ., +, -, *, /, %, =, !, <, >, |, &, ^, ~, ?, #, ), }, ], ,, :, (, {, [, $, \, ', ", @, ;
```

因此标识符不只限 ASCII 字母数字，任意未被分隔符排除的 Unicode 字符都可成为标识符的一部分。是否是合法变量名/函数名由 `IdentifierDictionary` 后续检查。

`ReadFirstIdentifierWord` 用于读取行首命令名，不展开宏。注释显示旧版本曾尝试展开行首宏，但已禁用，以禁止命令替换。

`ReadSingleIdentifierWord` 用于读取单个标识符，会尝试宏展开，但当前源码存在明显异常逻辑：`if (macro.IDWord != null) throw ...; str = macro.IDWord.Code;`。实际重写前需要结合 `DefineMacro.IDWord` 定义确认这段是否可达或为历史遗留错误。

### 8.3 数字

`ReadInt64` 支持：

| 形式 | 示例 | 说明 |
| --- | --- | --- |
| 十进制 | `123`、`-10`、`+10` | `readDigits` 允许首字符 `+` 或 `-`。在主 lexer 中只有当前字符是数字时才进入数字读取，所以表达式里的 `-1` 通常会被解析为一元负号加整数 `1`，但某些直接调用 `ReadInt64` 的上下文可读符号。 |
| 十六进制 | `0xFF`、`0Xff` | `0x` 后读取十六进制数字。 |
| 二进制 | `0b1010`、`0B1010` | 只允许 `0`/`1`，出现其他数字报错。 |
| 指数 | `10e3`、`10p3` | `e/E` 表示 10 的指数，`p/P` 表示 2 的指数。指数部分调用 `readDigits(st, fromBase)`，即在十六进制上下文中指数也按十六进制读取。 |

范围为 `long`/`i64`。指数计算通过 `double`，溢出或 NaN/Infinity 抛错。

`ReadDouble` 只用于少数特殊命令，如 `TIMES` 第二参数。它粗略读取符号、小数点、`e/E` 指数，再交给 `double.Parse(InvariantCulture)`。

### 8.4 字符串

普通双引号字符串：

```text
"abc\n"
```

转义规则：

| 转义 | 结果 |
| --- | --- |
| `\s` | 半角空格。 |
| `\S` | 全角空格。 |
| `\t` | Tab。 |
| `\n` | 换行字符。 |
| `\` 后接换行 | 忽略换行。 |
| `\` 后接其他字符 | 直接写入该字符。 |
| `\` 后到行尾 | 报“缺少转义后的字符”。 |

单引号字符串只在两个场景可用：

| 场景 | 行为 |
| --- | --- |
| `AllowSingleQuotationStr` | 读取 `'...'` 到下一个 `'`。HTML 属性解析使用。 |
| `AnalyzePrintV` | `PRINTV` 兼容语法，`'` 后内容读到逗号或行尾作为字符串。 |

### 8.5 符号

普通 lexer 可生成这些 `SymbolWord`：

| 字符 | 说明 |
| --- | --- |
| `(`、`)` | 小括号，参与嵌套计数。 |
| `[`、`]` | 中括号，参与嵌套计数。`[[...]]` 在 rename 未处理时会报错。 |
| `,` | 参数分隔符。 |
| `:` | 变量下标/参数分隔符。 |
| `@` | 当不是 `@"` 格式化字符串起始时作为普通符号。用于变量子 ID，例如 `VAR@SUB`。 |
| `.` | 命名空间预留，表达式解析中遇到 `id.` 会抛未实现。 |

`{`、`$` 在普通表达式 lexer 中直接报错。`}` 只有在 `LexEndWith.RightCurlyBrace` 下可作为终止符，否则报错。

## 9. 运算符规则

### 9.1 普通运算符读取

`ReadOperator` 识别：

| 字符串 | `OperatorCode` | 类型 |
| --- | --- | --- |
| `+` | `Plus` | 一元、二元。 |
| `-` | `Minus` | 一元、二元。 |
| `*` | `Mult` | 二元。 |
| `/` | `Div` | 二元。 |
| `%` | `Mod` | 二元。 |
| `==` | `Equal` | 二元。 |
| `=` | `Assignment` | 仅 `AllowAssignment` 时允许。 |
| `!=` | `NotEqual` | 二元。 |
| `!` | `Not` | 一元。 |
| `!&` | `Nand` | 二元。 |
| `!|` | `Nor` | 二元。 |
| `<` | `Less` | 二元。 |
| `<=` | `LessEqual` | 二元。 |
| `<<` | `LeftShift` | 二元。 |
| `>` | `Greater` | 二元。 |
| `>=` | `GreaterEqual` | 二元。 |
| `>>` | `RightShift` | 二元。 |
| `||` | `Or` | 二元。 |
| `|` | `BitOr` | 二元。 |
| `&&` | `And` | 二元。 |
| `&` | `BitAnd` | 二元。 |
| `^^` | `Xor` | 二元。 |
| `^` | `BitXor` | 二元。 |
| `~` | `BitNot` | 一元。 |
| `?` | `Ternary_a` | 三元前半。 |
| `#` | `Ternary_b` | 三元后半。 |
| `++` | `Increment` | 前置/后置一元。 |
| `--` | `Decrement` | 前置/后置一元。 |

### 9.2 赋值运算符读取

`ReadAssignmentOperator` 在赋值语句中使用，返回的是“如何更新左值”的运算符：

| 源码 | 返回 |
| --- | --- |
| `=` | `Assignment` |
| `==` | `Equal`，随后 `LogicalLineParser` 警告并改成 `Assignment`。 |
| `+=` | `Plus` |
| `-=` | `Minus` |
| `*=` | `Mult` |
| `/=` | `Div` |
| `%=` | `Mod` |
| `<<=` | `LeftShift` |
| `>>=` | `RightShift` |
| `|=` | `BitOr` |
| `&=` | `BitAnd` |
| `^=` | `BitXor` |
| `++` | `Increment` |
| `--` | `Decrement` |
| `'=` | `AssignmentStr`，实际源码匹配两个字符 `'=`, 中间无空格。 |

### 9.3 优先级

`OperatorCode` 低 8 位表示优先级，数值越大优先级越高。

| 运算符 | 优先级 | 说明 |
| --- | --- | --- |
| `*`、`/`、`%` | `0x90` | 乘除取模。 |
| `+`、`-` | `0x80` | 加减。 |
| `<<`、`>>` | `0x70` | 位移。 |
| `>`、`<`、`>=`、`<=` | `0x65` | 大小比较。 |
| `==`、`!=` | `0x60` | 等值比较，低于大小比较。 |
| `&`、`|`、`^` | `0x50` | 位运算。源码注释说 `^` 位于 `&` 与 `|` 中间，但实际三者同优先级。 |
| `&&`、`||`、`^^`、`!&`、`!|` | `0x40` | 逻辑运算。 |
| `#` | `0x10` | 三元后半。 |
| `?` | `0x05` | 三元前半。 |

一元运算符不通过优先级表统一规约，而由 `TermStack` 状态机处理。

## 10. `LexicalAnalyzer.Analyse` 主流程

输入为 `CharStream`、`LexEndWith`、`LexAnalyzeFlag`，输出 `WordCollection`。核心循环：

1. 遇到行尾或 `\0` 结束。
2. 跳过半角空格和 Tab。
3. 全角空格按配置处理。
4. 数字起始读取 `LiteralIntegerWord`。
5. 运算符起始读取 `OperatorWord`，但会先检查当前终止模式。
6. `(`、`)`、`[`、`]` 生成符号并维护嵌套深度。
7. `,` 在 `LexEndWith.Comma` 且小括号深度为 0 时终止，否则生成符号。
8. `'` 根据 flag 处理单引号字符串或报错。
9. `"` 读取普通字符串字面量。
10. `@"` 读取格式化字符串 `StrFormWord`。
11. `@` 非 `@"` 时生成符号。
12. `\@` 在普通表达式中生成仅含一个 `YenAtSubWord` 的 `StrFormWord`。
13. `{`、`$` 在普通表达式中报错。
14. `;` 处理行内注释。
15. 其他字符按标识符读取。
16. 结束后检查括号嵌套是否平衡。
17. 若 `UseMacro` 为真，执行 `expandMacro`。

关键兼容点：

| 场景 | 行为 |
| --- | --- |
| `LexEndWith.Operator` | 顶层遇到运算符不消费，由调用者读取赋值运算符。 |
| `LexEndWith.Question` | 顶层遇到 `?` 不消费，用于 `\@` 条件部分。 |
| `LexEndWith.Percent` | 顶层遇到 `%` 不消费，用于 `%...%`。 |
| `LexEndWith.RightCurlyBrace` | 遇到 `}` 不消费，用于 `{...}`。 |
| `LexEndWith.GreaterThan` | 遇到 `>` 不消费，用于 HTML 标签。 |
| `[[...]]` | 如果 rename 没有提前处理，会报不可识别或不可替换。 |

## 11. 格式化字符串解析

### 11.1 普通入口

两种方式进入格式化字符串：

| 语法 | 调用 |
| --- | --- |
| `@"..."` | 普通 lexer 遇到 `@` 且下一个字符为 `"`。 |
| 命令参数直接作为 FORM 字符串 | `ArgumentBuilder` 直接调用 `AnalyseFormattedString`。 |

`AnalyseFormattedString(st, endWith, trim)` 输出 `StrFormWord`。它维护一个文本 `buffer`，每遇到一个插值结构就将当前文本压入 `strs`，将子结构压入 `SubWords`。

### 11.2 格式化字符串内容规则

| 内容 | 行为 |
| --- | --- |
| 普通字符 | 加入当前文本 buffer。 |
| `"` | 若终止模式为 `DoubleQuotation` 则结束，否则作为普通字符。 |
| `#` | 若终止模式为 `Sharp` 则结束，否则作为普通字符。 |
| `,` | 若终止模式为 `Comma` 或 `LeftParenthesis_Bracket_Comma_Semicolon` 则结束，否则普通字符。 |
| `(`、`[`、`;` | 若终止模式为 `LeftParenthesis_Bracket_Comma_Semicolon` 则结束，否则普通字符。 |
| `%...%` | 解析为 `PercentSubWord`，内部用 `Analyse(..., LexEndWith.Percent, None)`。 |
| `{...}` | 解析为 `CurlyBraceSubWord`，内部用 `Analyse(..., LexEndWith.RightCurlyBrace, None)`。 |
| `***`、`+++`、`===`、`///`、`$$$` | 当 `SystemIgnoreTripleSymbol` 为假且连续 3 个相同符号时，解析为 `TripleSymbolSubWord`。 |
| `\s`、`\S`、`\t`、`\n` | 转义为空格、全角空格、Tab、换行。 |
| `\@...?...#...\@` | 解析为 `YenAtSubWord`。 |
| `\@` 且当前终止模式为 `YenAt` 或 `Sharp` | 作为当前格式化字符串终止，不消费为子结构。 |

如果 `trim = true`，会对 `retStr[0]` 执行 `TrimStart(' ', '\t')`，对最后一个文本段执行 `TrimEnd(' ', '\t')`，不裁剪全角空格。

### 11.3 `\@` 条件格式化字符串

语法结构：

```text
\@ condition ? left # right \@
```

解析流程：

1. `AnalyseYenAt` 假定已经消费 `\@`，当前字符是条件部分起始。
2. 用 `Analyse(..., LexEndWith.Question, None)` 读取 `condition`。
3. 当前字符必须是 `?`，否则报缺少对应字符。
4. 读取 `left = AnalyseFormattedString(..., FormStrEndWith.Sharp, trim = true)`。
5. 若遇到 `#`，继续读取 `right = AnalyseFormattedString(..., FormStrEndWith.YenAt, trim = true)`。
6. `right` 后必须遇到 `@`，此处表示 `\@` 中的 `@`，因为 `\` 已由格式化字符串 reader 处理。
7. 若 left 部分结束时遇到 `@` 而不是 `#`，会警告并返回 `Right = null`。

运行时 `StrForm.FromWordToken` 要求 `condition` 为整数，`left/right` 为字符串表达式，条件非零返回 left，否则 right。

## 12. 宏处理

### 12.1 宏定义来源

`.ERH` 中 `#DEFINE` 由 `ErhLoader.analyzeSharpDefine` 处理。

流程：

1. 读取宏名 `srcID`。
2. 检查宏名合法性和重复。
3. 判断宏名后是否紧跟 `(`，表示函数型宏。
4. 用 `LexicalAnalyzer.Analyse(..., EoL, AllowAssignment)` 读取替换目标。
5. 空替换目标允许，创建空宏。
6. 函数型宏会扫描形参并把替换体中对应标识符替换为 `MacroWord(index)`。
7. 当前版本随后直接对函数型宏抛出 `CanNotDeclaredFuncMacro`，即函数型宏定义被禁用。
8. 普通宏注册到 `IdentifierDictionary`。

### 12.2 宏展开

`LexicalAnalyzer.Analyse` 结束后，如果 `UseMacro` 为真，会调用 `expandMacro`。

普通宏展开：

1. 从头遍历 `WordCollection`。
2. 只处理 `IdentifierWord`。
3. 若标识符在宏字典中不存在，则跳过。
4. 展开次数超过 `MAX_EXPAND_MACRO = 100` 报错。
5. 无参宏：删除当前 token，将宏替换体 `InsertRange` 到当前位置。
6. 有参宏：调用 `expandFunctionlikeMacro`。

函数型宏展开逻辑仍存在：

1. 宏名后必须紧跟 `(`。
2. 读取固定数量参数，以顶层逗号分隔，以对应 `)` 结束。
3. 参数不可省略。
4. 克隆宏替换体，遇到 `MacroWord(n)` 时替换为第 n 个实参 token 列表。
5. 删除原宏调用区间，插入替换后的 token。

Rust 重写时可先实现无参宏，函数型宏可按兼容目标决定是否保留解析逻辑。若要精确复刻当前行为，`#DEFINE FOO(x)` 应在定义时报错，运行时展开函数型宏基本不可达。

## 13. 逻辑行解析

### 13.1 `.ERB` 装载级别

`ErbLoader.loadErb` 对每一行做顶层分派：

| 行首 | 处理 |
| --- | --- |
| `[` 且不是 `[[` | 预处理命令，如 `[IF]`、`[ELSE]`、`[ENDIF]`。 |
| `#` | 必须跟在函数标签后，调用 `ParseSharpLine`。 |
| `@` | 函数标签，调用 `ParseLabelLine`。 |
| `$` | 跳转标签，调用 `ParseLabelLine`。 |
| 其他 | 普通逻辑行，调用 `ParseLine`。 |

预处理支持：

| 指令 | 行为 |
| --- | --- |
| `[SKIPSTART]`、`[SKIPEND]` | 成块跳过。 |
| `[IF_DEBUG]`、`[IF_NDEBUG]` | 根据 Debug 模式启用/禁用块。 |
| `[IF MACRO]` | 若指定宏存在则启用。 |
| `[ELSEIF MACRO]` | 上一个分支未命中且宏存在则启用。 |
| `[ELSE]` | 默认分支。 |
| `[ENDIF]` | 结束条件块。 |

### 13.2 `ParseSharpLine`

`#` 行只允许作为函数标签的属性或私有变量声明。`#` 后读取一个不展开宏的标识符。

| 指令 | 行为 |
| --- | --- |
| `#SINGLE` | 事件函数属性，和 `ONLY` 互斥。 |
| `#LATER` | 事件函数属性，和 `ONLY` 互斥，与 `PRI` 有警告关系。 |
| `#PRI` | 事件函数属性，和 `ONLY` 互斥，与 `LATER` 有警告关系。 |
| `#ONLY` | 事件函数属性，设置后清除 `PRI/LATER/SINGLE`。 |
| `#FUNCTION` | 当前标签声明为返回整数的用户函数。 |
| `#FUNCTIONS` | 当前标签声明为返回字符串的用户函数。 |
| `#LOCALSIZE` | 设置局部整数数组 `LOCAL` 长度。 |
| `#LOCALSSIZE` | 设置局部字符串数组 `LOCALS` 长度。 |
| `#DIM` | 声明函数私有整数变量。 |
| `#DIMS` | 声明函数私有字符串变量。 |

`#LOCALSIZE/#LOCALSSIZE` 的参数通过 `Analyse(..., EoL, AllowAssignment)` 和 `ExpressionParser.ReduceIntegerTerm` 解析，要求能规约为正整数常量且小于 `int.MaxValue`。

`#DIM/#DIMS` 使用 `UserDefinedVariableData.Create`，支持 `STATIC`、`DYNAMIC`、`REF` 等关键字，但私有变量路径下部分关键字会报错。

### 13.3 `ParseLabelLine`

函数标签和跳转标签语法：

```text
@LabelName [subnames] (args)
$LabelName
```

解析流程：

1. 判断首字符是否 `@`，`@` 表示函数，`$` 表示跳转标签。
2. 消费 `@` 或 `$`。
3. 用 `Analyse(..., EoL, AllowAssignment)` 解析剩余内容。
4. 第一个 token 必须是 `IdentifierWord`，作为标签名。
5. 调用 `IdentifierDictionary.CheckUserLabelName` 检查名称。
6. `$` 标签不允许参数，有剩余 token 只警告，然后生成 `GotoLabelLine`。
7. `@` 标签生成 `FunctionLabelLine`，保留剩余 `WordCollection` 供之后 `ErbLoader.parseLabel` 解析参数。
8. 根据标签名判断是否事件函数或系统函数，设置 `IsEvent`、`IsSystem`、`Depth`。

函数标签参数实际由 `ErbLoader.parseLabel` 解析：

| 形式 | 行为 |
| --- | --- |
| `@FUNC` | 无参数。 |
| `@FUNC[SubNameArgs]` | 先解析中括号中的子名常量参数。 |
| `@FUNC, ARG:0, ARGS:0` | 逗号后参数列表，到行尾。 |
| `@FUNC(ARG:0 = 1, ARGS:0 = "x")` | 小括号参数列表，以 `)` 结束。 |

函数定义参数由 `ExpressionParser.ReduceArguments(..., isDefine = true)` 解析。每个形参会扩展成两项：变量表达式和默认值表达式。没有默认值时插入对应类型 `NullTerm`。

### 13.4 `ParseLine`

普通逻辑行解析顺序：

1. 跳过行首空白和注释。
2. 空行返回 `null`。
3. 若行首为 `+` 或 `-`，先尝试解析为前置 `++var` 或 `--var` 赋值行。
4. 读取行首标识符，不展开宏。
5. 若行首标识符是已知命令名，创建 `InstructionLine`。
6. 若命令是 `VARI` 或 `VARS`，走特殊路径，直接解析函数私有变量声明和可选初始值。
7. 对普通命令，命令名后必须是行尾、`;`、空格、Tab，或配置允许时全角空格。
8. 命令无参数时直接创建无参数 `InstructionLine`。
9. 命令有参数时消费一个分隔字符，将剩余 `CharStream` 原样存入 `InstructionLine`。
10. 若不是命令行，回到行首，按赋值行解析。
11. 赋值行先用 `Analyse(..., LexEndWith.Operator, None)` 读取左侧。
12. 再用 `ReadAssignmentOperator` 读取赋值运算符。
13. `==` 会警告并当作 `=`。
14. 创建 `InstructionLine(position, SETFunction, assignOP, leftWords, remainingStream)`。

关键点：`ParseLine` 并不验证赋值左侧是否变量，也不解析右侧表达式。实际验证在 `SP_SET_ArgumentBuilder` 中完成。

## 14. 表达式解析

`ExpressionParser` 是 token 到表达式树的主入口。

### 14.1 入口函数

| 函数 | 用途 |
| --- | --- |
| `ReduceArguments(wc, endWith, isDefine)` | 解析逗号分隔参数列表。 |
| `ReduceExpressionTerm(wc, endWith)` | 解析整数或字符串表达式，允许返回 `null`。 |
| `ReduceIntegerTerm(wc, endWith)` | 解析整数表达式，非整数或空时报错。 |
| `ToStrFormTerm(sfw)` | 将 `StrFormWord` 转为字符串表达式。常量格式化字符串会直接转 `SingleStrTerm`。 |
| `ReduceCaseExpressions(wc)` | 解析 `CASE` 条件列表。 |
| `ReduceVariableArgument(wc, varCode, id)` | 解析变量下标/参数。 |
| `ReduceVariableIdentifier(wc, idStr)` | 解析 `id` 或 `id@subid`，查询变量 token。 |

### 14.2 表达式终止模式

`TermEndWith` 是位标志：

| 标志 | 含义 |
| --- | --- |
| `EoL` | 行尾。 |
| `Comma` | 顶层逗号。 |
| `RightParenthesis` | `)`。 |
| `RightBracket` | `]`。 |
| `Assignment` | `=`，用于函数定义默认值或变量定义。 |
| `KeyWordPx` | HTML 参数扩展，遇到标识符 `px` 且下一个 token 是 `,` 或行尾时终止。 |

常用组合包括 `RightParenthesis_Comma`、`Comma_Assignment`、`RightParenthesis_Comma_Assignment` 等。

### 14.3 参数列表解析

`ReduceArguments` 根据 `ArgsEndWith` 决定单个表达式的终止符：

| `ArgsEndWith` | 单项终止符 |
| --- | --- |
| `EoL` | `Comma`。 |
| `RightParenthesis` | `RightParenthesis | Comma`。 |
| `RightBracket` | 源码中外层识别 `]`，但单项终止符未启用 `RightBracket_Comma`，历史注释保留。 |

解析到逗号后会消费逗号并继续。参数省略会产生 `null` 表达式，部分命令允许，部分命令报错。

`isDefine = true` 时用于函数定义参数：

1. 先解析形参表达式，终止符包含 `Assignment`。
2. 若当前 token 是赋值运算符，消费 `=` 并解析默认值。
3. 默认值类型必须与形参类型一致。
4. 若没有默认值，插入 `NullTerm(0)` 或 `NullTerm("")`。

### 14.4 表达式元素

`reduceTerm` 支持：

| Token | 行为 |
| --- | --- |
| `LiteralStringWord` | 压入 `SingleStrTerm`。 |
| `LiteralIntegerWord` | 压入 `SingleLongTerm`。 |
| `StrFormWord` | 转成 `StrFormTerm` 或常量字符串。 |
| `IdentifierWord` | 处理 `TO`、`IS` 特殊关键字，否则调用 `reduceIdentifier`。 |
| `OperatorWord` | 加入 `TermStack`。`Assignment` 只在终止符允许时结束表达式。 |
| `(` | 递归解析括号内表达式，要求非空且以 `)` 结束。 |
| `)`、`]`、`,` | 若当前终止模式允许，则结束当前表达式，否则报 unexpected symbol。 |
| `MacroWord` | 报宏未解决。 |

`TO` 只允许 `CASE a TO b` 场景作为终止关键字。`IS` 只允许 `CASE IS op expr` 语法开头。

### 14.5 标识符规约

`reduceIdentifier` 流程：

1. 消费当前标识符 token。
2. 若后接 `.`，命名空间功能未实现，报错。
3. 若后接 `(` 或 `[`，当作函数调用。`[` 函数调用未实现并报错。
4. 函数调用用 `ReduceArguments(..., RightParenthesis, false)` 解析实参。
5. 通过 `IdentifierDictionary.GetFunctionMethod` 解析函数方法。
6. 若不是函数调用，先尝试 `ReduceVariableIdentifier` 解析变量名和可选 `@subid`。
7. 若是变量，调用 `VariableParser.ReduceVariable` 解析变量下标。
8. 若不是变量，再尝试查找无参函数引用。
9. 若当前在变量下标上下文，且标识符是常量数据中的键，则当作字符串常量返回。
10. 对部分用户定义变量的命名下标，也可把标识符当作字符串常量。
11. 否则抛未知标识符错误。

### 14.6 `TermStack` 状态机

`TermStack` 用栈规约表达式，核心状态：

| `state` | 含义 |
| --- | --- |
| `0` | 期待值或前置一元运算符。 |
| `1` | 已有值，期待二元/三元运算符或后置一元运算符。 |
| `2` | 前置 `+`、`-`、`~` 后，等待值，规约可延后以允许后置处理。 |
| `3` | 前置 `++`、`--`、`!` 后，等待值，得到值后立即规约。 |

规约行为：

| 场景 | 行为 |
| --- | --- |
| `state = 0` 遇运算符 | 必须是一元运算符，压栈。 |
| `state = 1` 遇后置 `++`/`--` | 立即规约后置一元。前置和后置重复会报错。 |
| `state = 1` 遇二元/三元运算符 | 先规约等待中的前置一元，再按优先级规约栈顶。 |
| 遇值 | 压入表达式；若前面是 `state = 3` 的一元运算符则立即规约。 |
| 结束表达式 | 若仍在等待值时报语法错；规约所有剩余二元/三元。 |

三元运算符使用 `?` 和 `#`，不是 C 风格 `?:`。解析器用 `ternaryCount` 检查 `?` 与 `#` 配对。

示例语法：

```text
cond ? a # b
```

类型规则：条件必须是整数，两个分支必须同为整数或同为字符串。

### 14.7 运算符类型规则

`OperatorMethodManager` 规定：

| 运算 | 支持类型 |
| --- | --- |
| 一元 `+ - ! ~` | 整数。 |
| 前置/后置 `++ --` | 可变整数变量，不能是常量。 |
| 二元 `+ - * / %` | 整数与整数。 |
| 字符串 `+` | 字符串与字符串拼接。 |
| 字符串 `*` | 字符串与整数，或整数与字符串，表示重复字符串。负数和大于等于 10000 报错。 |
| 比较 `== != > < >= <=` | 整数与整数，或字符串与字符串，返回整数 0/1。 |
| 逻辑 `&& || ^^ !& !|` | 整数与整数，返回 0/1。 |
| 位运算 `& | ^ ~ << >>` | 整数。 |
| 三元 `? #` | 条件整数，分支同为整数或同为字符串。 |

## 15. 变量解析

变量语法不是通过 `[]` 下标，而是通过冒号：

```text
VAR
VAR:arg1
VAR:arg1:arg2
VAR:arg1:arg2:arg3
VAR@SUB:arg1
```

`ExpressionParser.ReduceVariableIdentifier` 支持 `id@subid`，用于获取带子 ID 的变量 token。

`VariableParser.ReduceVariable` 解析最多 3 个 `:` 参数。每个参数调用 `ExpressionParser.ReduceVariableArgument`，变量参数内部禁止普通运算符。

缺省下标规则依赖变量类型：

| 变量类型 | 缺省/校验规则 |
| --- | --- |
| 角色二维数组 | 要么无参返回 `VariableNoArgTerm`，要么必须提供 3 个参数。 |
| 角色一维数组 | 通常需要角色和下标两个参数；若缺角色且配置允许，默认角色为 `TARGET`。 |
| 角色零维数据 | 通常缺省角色为 `TARGET`。 |
| 普通三维数组 | 要么无参返回 `VariableNoArgTerm`，要么必须提供 3 个参数。 |
| 普通二维数组 | 要么无参返回 `VariableNoArgTerm`，要么必须提供 2 个参数。 |
| 普通一维数组 | 缺省下标为 0。`RAND` 在某些兼容配置下禁止省略或禁止 0。 |
| 零维变量 | 不允许参数。 |

若变量参数表达式是字符串，最终会包装为 `VariableStrArgTerm`，用于常量表或用户定义键名解析。

## 16. 命令参数解析

`ArgumentParser.SetArgumentTo` 根据 `InstructionLine.Function.ArgBuilder` 或 `Instruction.CreateArgument` 解析参数。多数命令的通用路径：

1. 从 `InstructionLine.PopArgumentPrimitive()` 取原始参数 `CharStream`。
2. 调用 `LexicalAnalyzer.Analyse(..., EoL, flag)`。
3. 调用 `ExpressionParser.ReduceArguments(..., EoL, false)`。
4. 用 `argumentTypeArray` 和 `minArg` 校验数量和类型。
5. 可选执行 `Restructure` 做常量折叠。
6. 构造命令专用 `Argument`。

常见 `FunctionArgType`：

| 类型 | 参数语义 |
| --- | --- |
| `VOID` | 无参数，若有多余内容警告。 |
| `INT_EXPRESSION` | 一个整数表达式，部分命令允许省略并补 0。 |
| `STR_EXPRESSION` | 一个字符串表达式。 |
| `STR` | 原始简单字符串，不走表达式 lexer。 |
| `FORM_STR` | 格式化字符串，到行尾。 |
| `FORM_STR_ANY` | 一个或多个以逗号分隔的格式化字符串。 |
| `SP_PRINTV` | 使用 `AnalyzePrintV` 的多表达式参数。 |
| `SP_SET`、`SP_SETS` | 赋值语句参数，由赋值左侧、赋值运算符、右侧原始流共同解析。 |
| `SP_CALL`、`SP_CALLFORM` | 函数名加可选 subnames 和实参列表。 |
| `SP_FOR_NEXT` | `FOR` 参数，第一项必须是可写整数变量。 |
| `CASE` | `CASE` 条件表达式列表。 |
| `SP_VAR` | 单个数组变量名，不是表达式。 |
| `EXPRESSION` | 任意类型表达式。 |

### 16.1 赋值参数 `SP_SET`

赋值行由 `ParseLine` 统一转换成 `SET` 指令，后续 `SP_SET_ArgumentBuilder` 处理。

左侧规则：

1. `destWc` 用 `ReduceArguments(..., EoL, false)` 解析。
2. 必须恰好一个表达式。
3. 必须是 `VariableTerm`。
4. 不能是常量变量。

整数变量右侧规则：

| 运算符 | 行为 |
| --- | --- |
| `=` | 右侧必须是单个整数表达式，或多个整数表达式用于数组赋值。 |
| `+=`、`-=` | 右侧单个整数表达式，可对常量右值优化成加常量。 |
| `*=`, `/=`, `%=` 等 | 构造二元运算 `op(left, right)`。 |
| `++`、`--` | 右侧必须为空，构造加一或减一。 |
| `'=` | 对整数变量非法，实际源码匹配两个字符 `'=`, 中间无空格。 |

字符串变量右侧规则：

| 运算符 | 行为 |
| --- | --- |
| `=` | 右侧按格式化字符串读到行尾，`trim = true`。 |
| `'=` | 右侧按普通表达式参数解析，可单个字符串或多个字符串用于数组赋值。实际源码匹配两个字符 `'=`, 中间无空格。 |
| `+=`、`*=` | 右侧单个表达式，构造字符串拼接或重复等二元运算。 |
| 其他 | 非法。 |

### 16.2 `CALL` 系列参数

`SP_CALL_ArgumentBuilder` 处理 `CALL`、`CALLF`、`CALLFORM`、`CALLFORMF`。

函数名部分：

| 类型 | 函数名读取方式 |
| --- | --- |
| `CALL`、`CALLF` | `ReadString(..., LeftParenthesis_Bracket_Comma_Semicolon)`，再 trim 半角空格和 Tab。 |
| `CALLFORM`、`CALLFORMF` | `AnalyseFormattedString(..., LeftParenthesis_Bracket_Comma_Semicolon, trim = true)`。 |

函数名后可接：

| 形式 | 行为 |
| --- | --- |
| `[subnames]` | 中括号参数作为 sub names。 |
| `(args)` | 小括号参数。 |
| `, args` | 逗号后到行尾参数。 |
| `[subnames](args)` | 同时指定 sub names 和 args。 |

### 16.3 `CASE` 参数

`CASE` 支持三种形式：

| 形式 | 说明 |
| --- | --- |
| `expr` | 普通匹配。 |
| `expr TO expr` | 范围匹配，两侧类型必须一致。 |
| `IS op expr` | 比较匹配，`op` 必须是二元运算符。 |

多个条件用逗号分隔。

## 17. 头文件 `.ERH` 与变量声明

`.ERH` 行必须以 `#` 开头。

| 指令 | 行为 |
| --- | --- |
| `#DEFINE` | 定义宏。 |
| `#FUNCTION`、`#FUNCTIONS` | 当前源码抛 `NotImplCodeEE`，注释中保留用户函数引用方法声明逻辑。 |
| `#DIM`、`#DIMS` | 延迟收集为 `DimLineWC`，读取完所有头文件后统一创建变量。 |

`UserDefinedVariableData.Create` 的变量声明概略语法：

```text
#DIM [CONST] [REF] [GLOBAL|SAVEDATA|CHARADATA] name [, size ...] [= initial_values]
#DIMS [CONST] [REF] [GLOBAL|SAVEDATA|CHARADATA] name [, size ...] [= initial_values]
```

私有变量 `#DIM/#DIMS` 在函数内也使用同一解析器，但允许/禁止的关键字取决于 `isPrivate`。

数组大小：

1. 若无大小且非 const，默认长度 1。
2. 大小通过整数常量表达式解析，必须大于 0 且总元素数不超过 1,000,000。
3. 引用类型 `REF` 的大小只能省略或为 0。
4. 初始化值通过 `ReduceArguments(..., EoL, false)` 解析，必须为常量且类型匹配。
5. `CONST` 必须有初始值，且不能声明多维数组。

## 18. 近似语法参考

下面是方便 Rust 重写时建模的近似语法，不是严格完整的 EraBasic 语法。

```ebnf
file            = { preprocessor_line | sharp_line | label_line | statement_line } ;

label_line      = function_label | goto_label ;
function_label  = "@" identifier [ subname_list ] [ function_params ] ;
goto_label      = "$" identifier ;
subname_list    = "[" arg_list "]" ;
function_params = "," define_arg_list | "(" define_arg_list ")" ;

sharp_line      = "#" sharp_keyword sharp_payload ;
sharp_keyword   = "SINGLE" | "LATER" | "PRI" | "ONLY" | "FUNCTION" | "FUNCTIONS" | "LOCALSIZE" | "LOCALSSIZE" | "DIM" | "DIMS" ;

statement_line  = instruction_line | assignment_line | prefix_incdec_line ;
instruction_line = instruction_name [ whitespace raw_arguments ] ;
assignment_line = expression assignment_operator raw_rhs ;
prefix_incdec_line = ("++" | "--") expression ;

arg_list        = [ expression { "," [ expression ] } ] ;
define_arg_list = [ define_arg { "," define_arg } ] ;
define_arg      = expression [ "=" expression ] ;

expression      = term { operator term } ;
term            = integer | string | strform | variable | function_call | "(" expression ")" | unary_op term | term postfix_op ;
variable        = identifier [ "@" identifier ] { ":" expression } ;
function_call   = identifier "(" arg_list ")" ;
ternary         = expression "?" expression "#" expression ;

strform         = text { strform_sub text } ;
strform_sub     = "%" arg_list "%" | "{" arg_list "}" | "\@" expression "?" strform [ "#" strform ] "\@" | triple_symbol ;
triple_symbol   = "***" | "+++" | "===" | "///" | "$$$" ;
```

## 19. Rust 重写建议

### 19.1 模块拆分

建议 Rust crate 中按以下模块拆分：

| 模块 | 职责 |
| --- | --- |
| `source` | 行读取、rename、预处理状态、位置映射。 |
| `lexer` | `CharStream`、`Word`、`SubWord`、`lex_expr`、`lex_form_string`。 |
| `macro_expander` | `#DEFINE` 宏表和 token 展开。 |
| `line_parser` | `LogicalLine` 粗解析。 |
| `expr_parser` | `AExpression` AST、运算符优先级、变量/函数引用。 |
| `var_parser` | 变量 token 查询、冒号参数规则。 |
| `arg_parser` | 命令参数构造器。 |
| `diagnostics` | 错误、警告、兼容性警告等级。 |

### 19.2 数据结构建议

| C# | Rust 建议 |
| --- | --- |
| `Word` class hierarchy | `enum Word`。 |
| `SubWord` class hierarchy | `enum SubWord`。 |
| `WordCollection` linked list | `TokenStream { tokens: Vec<Word>, pos: usize }`。 |
| `AExpression` class hierarchy | `enum Expr` 或 trait object。若要做优化和解释执行，优先 enum。 |
| `FunctionIdentifier` dictionary | `HashMap<AsciiCaseKey, FunctionInfo>` 或自定义大小写比较 key。 |
| `Config.Config.StringComparison` | 显式封装大小写敏感/不敏感比较，不要散落调用。 |
| `CodeEE`/`ParserMediator.Warn` | `Result<T, Diagnostic>` 加 `Vec<Diagnostic>` 收集警告。 |

### 19.3 兼容性优先事项

如果目标是高兼容，应优先测试以下特性：

1. 全角空格配置 `SystemAllowFullSpace`。
2. 行内注释 `;`、`;#;`、`;!;` 的区别。
3. `@"..."`、FORM 字符串命令参数、字符串赋值右侧三者的 trim 差异。
4. `%...%` 与 `{...}` 内部表达式的终止符行为。
5. `\@ condition ? left # right \@` 的嵌套和缺失 `#` 兼容警告。
6. 三元运算符使用 `?` 和 `#`，而不是 `:`。
7. `==` 在赋值语句中警告后当作 `=`。
8. 字符串变量的 `=` 右侧按 FORM 字符串解析，而 `'=` 按字符串表达式解析。实际源码匹配两个字符 `'=`, 中间无空格。
9. 标识符允许非 ASCII 和大量符号以外的字符。
10. 数字支持 `0x`、`0b`、`e/E`、`p/P`，并用 `i64` 范围检查。
11. 宏展开上限 100。
12. `CALL` 与 `CALLFORM` 对函数名的读取终止符不同。
13. 变量下标使用 `:`，并带有角色变量缺省 `TARGET` 等特殊规则。
14. 参数解析延迟执行，部分命令只有在加载配置或执行前才解析。

### 19.4 推荐测试用例方向

建立 Rust 端 golden tests 时，建议按层级组织：

| 层级 | 示例 |
| --- | --- |
| lexer | `1 + 2`, `0xFF`, `"a\n"`, `@"x{%A%}"`, `\@FLAG?yes#no\@`。 |
| macro | `#DEFINE A 1` 后表达式 `A + 2`。 |
| expression | `1 + 2 * 3`, `1 ? "a" # "b"`, `"a" * 3`, `VAR:0 + 1`。 |
| line parser | `PRINTFORM hello`, `A = 1`, `A += 2`, `++COUNT:0`, `@FUNC(ARG:0=1)`。 |
| argument parser | `CALL FOO, 1, "x"`, `CALLFORM FOO_%BAR%, 1`, `CASE 1 TO 3, IS >= 10`。 |
| compatibility | 全角空格、注释、缺少 `#` 的 `\@`、`==` 赋值警告。 |

## 20. 已发现的实现注意点

这些点在 Rust 重写时需要二次确认：

1. `ReadSingleIdentifierWord` 的宏展开分支看起来有历史遗留异常，建议结合 `DefineMacro.IDWord` 定义和实际用例确认。
2. `OperatorCode.Nor` 注释写成 `!^`，但 lexer 和字典都使用 `!|`。
3. `OperatorCode` 注释说位异或 `^` 优先级在 `&` 与 `|` 中间，但实际优先级相同。
4. `ReduceArguments` 对 `ArgsEndWith.RightBracket` 的单项终止符逻辑被注释，实际依赖外层 `]` 处理，移植时要跑标签 subnames 和 `CALL[Sub]` 测试确认。
5. `GreaterEqualStrStr` 和 `LessEqualStrStr` 的比较实现看起来都使用 `c < 0` 条件，可能是源码 bug 或历史兼容行为，重写前应确认是否要逐 bug 兼容。
6. 函数型宏解析逻辑存在，但定义阶段被禁用。不要因为看到 `MacroWord` 就默认当前语言开放函数型宏。
7. 普通 lexer 中 `{` 和 `$` 报错，但格式化字符串 lexer 中它们有特殊含义或普通字符含义，不能共用一个不带上下文的 tokenizer。
8. 参数解析高度依赖命令类型，不能只靠统一表达式列表解析全部命令。
