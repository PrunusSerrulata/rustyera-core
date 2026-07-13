# 06 CSV文件格式规范

本文整理本仓库 `CSV/` 目录下各类 CSV / ALS / 配置文件的实际格式、引擎加载方式与脚本侧用途。结论同时来自：

- Emuera 源码中的 CSV/配置加载器（`ConstantData`、`GameBase`、`ConfigData`、`ParserMediator` 等）
- `CSV/` 目录现有文件内容
- `ERB/` 中对这些名字表、常量表和扩展数组的实际引用

> 说明：能在源码中直接确认的规则，会标明“代码可确认”；仅能从文件内容与脚本使用方式归纳的，会标明“根据文件内容/ERB 用法推断”或“需进一步验证”。

---

## 1. CSV文件总览

### 1.1 当前仓库中的相关文件范围

- `CSV/` 根目录文件：61 个
- `CSV/Chara/` 角色模板文件：167 个 `Chara*.csv`

### 1.2 编码规则

代码可确认：Emuera 读取 CSV 时会先做编码探测，优先支持：

1. UTF-8 BOM
2. UTF-8
3. 解析失败时回退 Shift-JIS（CP932）

对应源码：

- `source/Emuera/MinorShift.Emuera.Runtime.Utils/EncodingHandler.cs`
- `source/Emuera/MinorShift.Emuera.Sub/EraStreamReader.cs`

本仓库现状（`file -I CSV/* CSV/Chara/*` 实测）：当前 `CSV/` 与 `CSV/Chara/` 下文件均识别为 **UTF-8**。很多文件带 BOM，适合按 `utf-8-sig` 处理。

### 1.3 通用加载规则

#### 1）普通“名字表”CSV（代码可确认）

`ConstantData.LoadData()` 会自动加载以下内建文件：

- `Abl.csv`
- `exp.csv`
- `Talent.csv`
- `Palam.csv`
- `Train.csv`
- `Mark.csv`
- `Item.csv`
- `Base.csv`
- `source.csv`
- `ex.csv`
- `Str.csv`
- `Equip.csv`
- `Tequip.csv`
- `FLAG.csv`
- `TFLAG.csv`
- `CFLAG.csv`
- `TCVAR.csv`
- `CSTR.csv`
- `Stain.csv`
- `Strname.csv`
- `DAY.csv`
- `TIME.csv`
- `VariableSize.csv`
- `Chara*.csv`
- `VarExt*.csv`

这些文件大多由 `ConstantData.loadDataTo()` 解析，基本格式是：

```csv
编号,名称
编号,名称,;说明
```

解析特点：

- 第 1 列：必须是整数编号
- 第 2 列：名称字符串
- 第 3 列及之后：**通常忽略**，因此常被用来放说明
- 空行会跳过
- 文件不存在时直接忽略，不报致命错

#### 2）`Item.csv` 的特殊点（代码可确认）

`Item.csv` 第 3 列会被解析成价格，写入 `ITEMPRICE`：

```csv
编号,物品名,价格
```

第 4 列及之后仍然只是附加说明，不参与引擎内建解析。

#### 3）`Str.csv` 的特殊点（代码可确认）

`Str.csv` 虽然也走 `loadDataTo()`，但它不是“名字表变量”，而是 **STR 字符串数组的初始内容来源**。初始化时，`VariableData` 会把 `STR.CSV` 读到 `STR` 数组里。

也就是说：

- `Str.csv`：给 `STR` 提供初始字符串内容
- `Strname.csv`：给 `STRNAME` 提供“索引名称”

#### 4）注释与空行（部分可确认，部分需进一步验证）

- `_default.config` / `_fixed.config` / `_Rename.csv` / `_Replace.csv`：代码中**明确**支持 `;` 开头整行注释
- `VarExt*.csv`：源码未显式跳过 `;`，但仓库文件确实大量使用 `;` 说明行
- 普通名字表 CSV：源码 `loadDataTo()` 本身没有专门的“跳过 `;` 注释行”逻辑，因此**最稳妥的写法**是：
  - 真实数据写成 `编号,名称,;注释`
  - 纯注释行虽然本仓库大量使用，但从加载器源码看，是否在所有类别都完全无警告，**需进一步验证**

#### 5）行续接（代码可确认，但本仓库几乎未实际使用）

所有这些文件底层都经 `EraStreamReader.ReadEnabledLine()` 读取，因此理论上支持：

```text
{
多行内容
}
```

的行续接语法；但本仓库 CSV 几乎都采用单行写法，文档中不建议依赖这一特性。

### 1.4 内建 CSV 与项目自定义 CSV 的区别

`CSV/` 下的文件大致分两类：

#### A. 引擎内建名字表 / 配置表

由 Emuera 源码直接加载，例如：

- `Abl.csv` → `ABLNAME`
- `Base.csv` → `BASENAME`
- `Item.csv` → `ITEMNAME` / `ITEMPRICE`
- `CSTR.csv` → `CSTRNAME`
- `DAY.csv` → `DAYNAME`
- `VariableSize.csv` → 变量尺寸定义
- `GameBase.csv` → 游戏元信息
- `Chara/*.csv` → 角色模板
- `VarExtGameData.csv` → 扩展存档登记

#### B. 项目自定义“常量索引表”

这些文件**不是** Emuera 内建加载列表的一部分，主要被本仓库脚本、头文件生成工具和自定义数组使用，例如：

- `BUFF.csv`
- `Juel.csv`
- `M_FLAG.csv`
- `M_CFLAG.csv`
- `M_TFLAG.csv`
- `Setting_FLAG.csv`
- `Setting_CFLAG.csv`
- `EI_BASE.csv`
- `EI_FLAG.csv`
- `M_SPELL@1.csv`
- `M_SPELL@2.csv`

它们的作用更接近：

- 为自定义 `#DIM` / `#DIM SAVEDATA` 数组提供**索引命名表**
- 由 `tool/Generate_ERH.py` 合并 `.csv + .als` 生成 `ERB/Headers/AutoConst_*.ERH`
- 让脚本里能用可读名字代替裸数字

---

## 2. `.als` 文件说明

### 2.1 `.als` 与同名 `.csv` 的关系

代码可确认：普通名字表 CSV 加载完成后，Emuera 会尝试读取同名 `.als` 文件，例如：

- `Abl.csv` → `Abl.als`
- `FLAG.csv` → `FLAG.als`
- `Equip.csv` → `Equip.als`

`.als` 的格式与主 `.csv` 基本相同：

```csv
编号,别名
```

其作用是把“别名/同义词”映射到同一个编号。引擎会把这些别名加入对应名字字典中，之后脚本可用别名访问同一索引。

### 2.2 `.als` 的规则（代码可确认）

- 第 1 列：整数编号
- 第 2 列：别名
- 同一个编号可以出现多个别名
- 如果两个不同编号定义了同名别名，会给出警告

### 2.3 实例：`Abl.csv` 与 `Abl.als`

`Abl.csv`：

```csv
;感覚
0,Ｃ感覚
1,Ｖ感覚
2,Ａ感覚
3,Ｂ感覚
4,Ｍ感覚
;基本
9,親密
10,従順
11,欲望
12,技巧
```

`Abl.als`：

```csv
;感觉
0,Ｃ感觉
1,Ｖ感觉
2,Ａ感觉
3,Ｂ感觉
4,Ｍ感觉
;基本
9,亲密
10,从顺
10,顺从
11,欲望
```

可见：

- 主表提供“正式名称”
- `.als` 追加汉化别名/常用同义词
- 如 `従順` 与 `从顺` / `顺从` 最终都指向同一编号 `10`

### 2.4 本仓库工具链中的附加作用

`tool/Generate_ERH.py` 也会同时读取 `.csv` 和 `.als`，生成 `AutoConst_*.ERH` 常量头。也就是说，`.als` 不仅被引擎用于别名解析，也被项目工具用于生成常量名。

---

## 3. 角色能力/属性类 CSV

这一组大多是 Emuera **内建名字表**，对应 05 变量文档中的“CSV 名称对照变量”。

### 3.1 `Abl.csv`（能力名表）

- 对应内建变量：`ABL` / `ABLNAME`
- 用途：定义角色能力槽位名称
- 格式：`编号,名称[,说明]`

示例：

```csv
0,Ｃ感覚
1,Ｖ感覚
2,Ａ感覚
3,Ｂ感覚
4,Ｍ感覚
9,親密
10,従順
11,欲望
12,技巧
```

### 3.2 `Base.csv`（基础数值名表）

- 对应内建变量：`BASE` / `BASENAME`
- 用途：定义体力、气力、法力等基础条目名称
- 格式：`编号,名称[,说明]`

示例：

```csv
0,体力
1,気力
2,射精
3,母乳
4,尿意
5,勃起
6,精力
7,法力
8,TSP
10,ムード
```

### 3.3 `Talent.csv`（素质名表）

- 对应内建变量：`TALENT` / `TALENTNAME`
- 用途：定义角色素质/特性开关与枚举位
- 格式：`编号,名称[,说明列]`

示例：

```csv
0,処女,;(1= 処女 2=再生処女 -1=無自覚非処女）
1,非童貞,;(0= 童貞 ビット0膣性交経験済み ビット1肛門性交経験済み)
2,性別,;(1=女性器（bit0）　2=男性器（bit1）　3=扶她（bit0&bit1）)
3,恋慕,;愛情に似た感情を抱いている状態。=2で上位互換
4,淫乱,;色欲に乱れきった状態。=2で上位互換
```

### 3.4 `Mark.csv`（刻印名表）

- 对应内建变量：`MARK` / `MARKNAME`
- 用途：定义苦痛、快感、反发等刻印槽位
- 格式：`编号,名称`

示例：

```csv
0,苦痛刻印
1,快楽刻印
2,不埒刻印
3,反発刻印
4,反発取得履歴
5,時姦刻印
8,成長
```

### 3.5 `exp.csv`（经验名表）

- 对应内建变量：`EXP` / `EXPNAME`
- 用途：定义经验类项目
- 格式：`编号,名称`

示例：

```csv
0,Ｃ経験
1,Ｖ経験
2,Ａ経験
3,Ｂ経験
4,Ｍ経験
5,Ｃ絶頂経験
6,Ｖ絶頂経験
7,Ａ絶頂経験
8,Ｂ絶頂経験
9,Ｍ絶頂経験
```

### 3.6 `Palam.csv`（欲情/参数名表）

- 对应内建变量：`PALAM` / `PALAMNAME`
- 用途：定义 PALAM 参数名称
- 格式：`编号,名称`

示例：

```csv
0,快Ｃ
1,快Ｖ
2,快Ａ
3,快Ｂ
4,快Ｍ
9,潤滑
10,恭順
11,欲情
12,屈服
13,習得
```

### 3.7 `source.csv`（来源增减名表）

- 对应内建变量：`SOURCE` / `SOURCENAME`
- 用途：定义 `SOURCE` 各来源槽位名称
- 格式：`编号,名称`

示例：

```csv
0,快Ｃ
1,快Ｖ
2,快Ａ
3,快Ｂ
4,快Ｍ
9,液体
10,情愛
11,性行動
12,達成
13,苦痛
14,恐怖
```

### 3.8 `ex.csv`（状态/绝顶扩展名表）

- 对应内建变量：`EX` / `EXNAME`
- 用途：定义绝顶与特殊状态索引
- 格式：`编号,名称`

示例：

```csv
0,Ｃ絶頂
1,Ｖ絶頂
2,Ａ絶頂
3,Ｂ絶頂
4,Ｍ絶頂
6,二重絶頂
7,三重絶頂
8,四重絶頂
9,五重絶頂
10,噴乳
```

---

## 4. 旗标/开关类 CSV

### 4.1 内建 FLAG 系：`FLAG.csv` / `TFLAG.csv` / `CFLAG.csv`

这三类文件都是 Emuera 内建名字表：

- `FLAG.csv` → `FLAG` / `FLAGNAME`（全局整数数组）
- `TFLAG.csv` → `TFLAG` / `TFLAGNAME`（全局临时标记）
- `CFLAG.csv` → `CFLAG` / `CFLAGNAME`（角色整数数组）

格式统一为：

```csv
编号,名称[,说明列]
```

#### `FLAG.csv` 示例

```csv
0,休憩FLAG
1,SYS_EVENT起動FLAG
2,長筒襪着用
3,宴会的有無
4,難易度
5,游戏模式
6,情景文本設定
7,口上文本設定
9,每日变更事件
```

#### `TFLAG.csv` 示例

```csv
1,射精部位,; (ビット 1=膣内 2=肛門 3=手淫 4=口淫 5=乳交 6=素股 7=足交 8=体表 9=肛門奉仕
2,破瓜抑制FLAG,;（処女で快Vを得た場合、経験を変動させない)
3,SELECTCOM保存
4,破瓜FLAG
5,推倒
10,Ｖ挿入継続
11,Ａ挿入継続
```

#### `CFLAG.csv` 示例

```csv
1,既成事実
2,好感度
3,異常経験
4,信頼度
6,態度
7,基本服装セット
8,服装オプション
10,弱み握り
11,弱み握られ
```

### 4.2 `M_FLAG.csv` / `M_CFLAG.csv` / `M_TFLAG.csv`

这一组**不是 Emuera 内建名字表**，而是本仓库“魔法 DLC / 魔改系统”的自定义索引表。

`ERB/DLC/魔法DLC/魔法相关参数.ERH` 可确认：

```erb
#DIM SAVEDATA M_FLAG,100
#DIM SAVEDATA M_TFLAG,100
#DIM SAVEDATA CHARADATA M_CFLAG,100
```

也就是说：

- `M_FLAG.csv`：自定义全局魔法状态索引表
- `M_CFLAG.csv`：自定义角色魔法状态索引表
- `M_TFLAG.csv`：自定义临时/流程状态索引表

这些 CSV 的主要作用是：

1. 给人看得懂的索引命名
2. 供 `tool/Generate_ERH.py` 生成 `AutoConst_M_FLAG.ERH` / `AutoConst_M_CFLAG.ERH` / `AutoConst_M_TFLAG.ERH`
3. 让 ERB 可以写出更可读的访问形式

例如 ERB 里实际出现：

```erb
IF M_FLAG:魔力来源 == 1
SETBIT M_CFLAG:TARGET:魔法奖励获得, 0
```

#### `M_FLAG.csv` 示例

```csv
1,魔力来源
2,法强
3,TSP使用奖励
4,魔法强度修正
5,理解时间
10,隙间标记
11,标记睡眠
12,标记合意
13,标记手动
14,魔力结晶数
```

#### `M_CFLAG.csv` 示例

```csv
1,魔法奖励获得
2,魔法奖励EXP
3,手动标记
5,初期记录
6,精力储存器_上限
7,精力储存器_当前值
8,变形魔法_魅力
9,变形魔法_谜之魅力
10,变形魔法_魅惑
11,约会道中延迟
```

#### `M_TFLAG.csv` 示例

```csv
1,锻炼魔法CD
2,转运魔法CD
3,神清气爽
4,瞬移中
5,魔法换装
6,往日之隙
7,无垢
8,不眠
9,时间循环
10,侘寂计时器
```

### 4.3 `Setting_FLAG.csv` / `Setting_CFLAG.csv`

这一组同样是**项目自定义数组索引表**，来自“实用设置补丁”。

`ERB/DLC/实用设置补丁/实用设置.ERH` 可确认：

```erb
#DIM SAVEDATA CHARADATA Setting_CFLAG,100
#DIM SAVEDATA Setting_FLAG,100
```

因此：

- `Setting_FLAG.csv`：全局设置项索引表
- `Setting_CFLAG.csv`：角色级设置/居住状态索引表

ERB 中实际用法：

```erb
IF Setting_FLAG:背景图片
Setting_CFLAG:LOCAL:后宫居住标记 = 1
```

#### `Setting_FLAG.csv` 示例

```csv
0,常量更新
1,五重绝顶
2,满足终了
3,主动权转移禁止
4,外出时可以带出去
5,单独一人吃饭
6,即时重载
7,逆推口上找补
8,跟随型后宫
9,背景图片
```

#### `Setting_CFLAG.csv` 示例

```csv
1,后宫居住标记
```

> 说明：仓库中已提交 `ERB/Headers/AutoConst_Setting_CFLAG.ERH`，但未见对应的 `AutoConst_Setting_FLAG.ERH`。从 ERB 用法看，`Setting_FLAG.csv` 仍被作为索引名表使用；自动生成链路是否漏提交，需进一步验证。

### 4.4 `EI_FLAG.csv`

`EI_FLAG.csv` 是“永琳医疗系统”使用的自定义索引表，不属于 Emuera 内建 CSV。

`ERB/DLC/MEDICINE/EIRIN.ERH` 可确认：

```erb
#DIM EI_FLAG,200
```

因此它定义的是自定义整数数组 `EI_FLAG` 的各病症/症状位名称。

示例：

```csv
0,栄養
1,感冒
2,疼痛
3,精神
4,血液
5,火熱
6,腹痛
7,関節
8,眼科
9,挫傷
10,咽痛
11,解毒
```

### 4.5 `M_` 前缀与 `Setting_` 前缀的区别

可归纳为：

- `M_`：魔法 DLC / 魔改系统的专用数组索引表
- `Setting_`：设置系统 / 选项系统的专用数组索引表
- `EI_`：永琳医疗系统的专用数组索引表

它们共同点：

- **都不是** Emuera 内建 `ConstantData.LoadData()` 自动读取的“名字表变量”
- 都依赖 ERB 中的 `#DIM` / `#DIM SAVEDATA` 自定义数组
- 都适合作为“项目级常量表”维护

---

## 5. 物品/装备类 CSV

### 5.1 `Item.csv`

- 对应内建变量：`ITEMNAME` / `ITEMPRICE`
- 用途：定义物品名与默认价格
- 格式：`编号,名称,价格[,附加说明]`

示例：

```csv
0,跳蛋,3000,
1,電動按摩棒,9000,
2,陰蒂夾,3000,
3,飛機杯,12000,
4,振動棒,4500,
5,肛用振動棒,4500,
10,乳頭夾,3000,
11,搾乳器,4500,
```

说明：

- 第 3 列会进入 `ITEMPRICE`
- 名称进入 `ITEMNAME`
- `Item.als` 可以补中文/日文别名

### 5.2 `Equip.csv`

- 对应内建变量：`EQUIP` / `EQUIPNAME`
- 用途：定义角色装备槽位名称
- 格式：`编号,名称[,说明]`

示例：

```csv
1,飾品,;1:めがね
2,帽子,;1:帽子 2:发饰 3:睡衣帽
3,靴,;1:靴 2:木屐
4,襪子,;1:襪子 2:長筒襪
5,下半身内衣１,;ずらし不可
6,下半身内衣２,;ずらし可
```

### 5.3 `Tequip.csv`

- 对应内建变量：`TEQUIP` / `TEQUIPNAME`
- 用途：定义“穿着状态/临时装备状态”索引
- 格式：`编号,名称[,说明]`

示例：

```csv
0,下半身着衣状況
;(ビット0=裙子 1=ずらし可 2=ずらし不可 3=ずらせない下身衣服 4=突っ込み不可)
1,上半身着衣状況
;(0=無 1=はだけ可 2=はだけ不可 3=突っ込み不可)
3,上半身裸露状態
5,上衣脱衣完毕
```

说明：该表经常把“状态编码说明”写在后续注释行中，实际脚本侧要结合注释理解位含义。

### 5.4 `Juel.csv`

- 对应项目变量：`JUEL`（名称表为项目侧补充）
- 用途：给 `JUEL` 索引提供可读命名
- 状态：**不是 Emuera 内建自动加载的 CSV 名字表**

代码可确认：Emuera 内建只把 `PALAMNAME` 与 `JUEL` 的尺寸联动处理，但不会自动读取 `Juel.csv`。因此本仓库的 `Juel.csv` 主要用于：

- 项目脚本常量化
- 头文件生成
- 与 `Palam.csv` 保持同一编号语义

示例：

```csv
0,快Ｃ
1,快Ｖ
2,快Ａ
3,快Ｂ
4,快Ｍ
9,潤滑
10,恭順
11,欲情
12,屈服
13,習得
```

### 5.5 `Stain.csv`

- 对应内建变量：`STAIN` / `STAINNAME`
- 用途：定义污浊部位索引
- 格式：`编号,名称[,说明]`

示例：

```csv
0,口
1,手
2,Ｐ
3,Ｖ
4,Ａ
5,Ｂ
6,膣内
7,腸内
;ビット　1=愛液 2=阴茎 4=精液 8=肛門 16=母乳 32=黏液 64=破瓜之血 128=巧克力
```

### 5.6 `BUFF.csv`

- 对应项目变量：`BUFF`
- 用途：角色 Buff/临时修正槽位常量表
- 状态：项目自定义，不是 Emuera 内建 CSV 名字表

`ERB/DIM.ERH` 可确认：

```erb
#DIM CHARADATA SAVEDATA BUFF,50
```

因此 `BUFF.csv` 的作用是：

- 给自定义 `BUFF` 数组的编号命名
- 供 `AutoConst_BUFF.ERH` 生成常量

示例：

```csv
0,体力
1,気力
2,射精
3,母乳
4,尿意
5,勃起
6,精力
7,法力
8,TSP
10,ムード
```

---

## 6. 字符串/名称类 CSV

### 6.1 `Str.csv`

- 对应内建变量：`STR`
- 用途：给 `STR` 字符串数组提供**初始内容**
- 不是 `STRNAME`
- 格式：`编号,初始字符串`

示例：

```csv
1,鳥居
2,境内
3,賽銭箱
4,本殿
5,手水舎
6,庫房
7,土間
8,廚房
9,居間
10,走廊
```

### 6.2 `Strname.csv`

- 对应内建变量：`STRNAME`
- 用途：给 `STR` 的索引提供命名
- 当前文件几乎全是说明注释，未正式列出大量命名项

示例：

```csv
;我在需要初始化字符串常量时尝试了一些方法，不过STR似乎是唯一的预留给字符串常量的空间，尽管STR也不是作为字符串常量设计的，其实可以运行时修改它。
;1,鸟居的名字（有意义吗）
;2,境内的名字
```

说明：从当前仓库看，`Strname.csv` 更像是“预留说明文件”。若要真正使用 `STRNAME`，应补充正式的 `编号,名称` 行。

### 6.3 `CSTR.csv`

- 对应内建变量：`CSTR` / `CSTRNAME`
- 用途：定义角色字符串变量槽位名称
- 格式：`编号,名称[,说明]`

示例：

```csv
0,FIRSTTIME
1,ONCE
2,職場
3,工作情報
4,SELECTCOM_DEFINITION
```

在角色 CSV 中，它会这样被实际赋值：

```csv
CSTR,工作情報,清扫神社　每天早上　6时～8时
CSTR,職場,境内周边
CSTR,10,～乐园的巫女～　●种族：人类　●能力：主要是在空中飞行的程度的能力
```

### 6.4 `TCVAR.csv`

- 对应内建变量：`TCVAR` / `TCVARNAME`
- 用途：定义角色临时计数/中间状态槽位名称
- 格式：`编号,名称[,说明]`

示例：

```csv
2,射精部位
3,避孕套
4,射精快感強度
12,已射精部位FLAG
15,破瓜
```

说明：`TCVAR` 常用于流程状态、阶段信息、临时判定结果。

---

## 7. 时间、魔法与其他 CSV

### 7.1 `DAY.csv`

- 对应内建变量：`DAY` / `DAYNAME`
- 用途：定义全局日期相关变量名
- 格式：`编号,名称[,说明]`

示例：

```csv
0,総日数
2,季節,;（1=春 2=夏 3=秋 4=冬）
3,日期,;（1～31）
10,醸造
41,依頼期限１
42,依頼期限２
43,依頼期限３
44,依頼期限４
```

### 7.2 `TIME.csv`

- 对应内建变量：`TIME` / `TIMENAME`
- 用途：定义时间段、天气等全局时间变量名
- 格式：`编号,名称[,说明]`

示例：

```csv
0,時間
1,時間進行管理
2,時間帯
3,MASTERの起床予定時刻
5,天気
7,虹
```

### 7.3 `EI_BASE.csv`

- 对应项目变量：`EI_BASE`
- 用途：永琳医疗系统的角色级基础数据槽位命名
- 代码佐证：`ERB/DLC/MEDICINE/EIRIN.ERH` 中有

```erb
#DIM CHARADATA SAVEDATA EI_BASE,100
```

示例：

```csv
38,今日问诊
39,近期问诊
40,明日再来
41,住院中
42,患病
43,诊断完成
44,患者体力
45,症状类型
46,病症大小
47,病症残余
48,疾病防御
```

### 7.4 `M_SPELL@1.csv` / `M_SPELL@2.csv`

- 对应项目变量：`M_SPELL`
- 代码佐证：`ERB/DLC/魔法DLC/魔法相关参数.ERH`

```erb
#DIM SAVEDATA M_SPELL,100,3
```

- 用途：给二维数组 `M_SPELL` 提供两层索引名称

根据文件内容与 ERB 实际用法可推断：

- `M_SPELL@1.csv`：第 1 维（符咒/法术编号）
- `M_SPELL@2.csv`：第 2 维（字段编号）

ERB 中存在：

```erb
M_SPELL:恢复符咒:符咒等级 = 1
```

这与两个文件的分工正好对应。

#### `M_SPELL@1.csv` 示例

```csv
1,恢复符咒,;50,lv1
2,追踪符咒,;50,lv1
3,千里眼符咒,;150,lv3
4,旅行符咒,;250,lv5
5,幸运符咒,
6,奇迹符咒,
```

#### `M_SPELL@2.csv` 示例

```csv
0,符咒数量
1,符咒等级
2,符咒配方
```

> `@` 命名规则不是 Emuera 内建 CSV 约定，而是本仓库为二维数组配套索引表采用的项目约定。

### 7.5 `Train.csv`

- 对应内建变量：`TRAIN` / `TRAINNAME`
- 用途：定义训练/指令编号与名称
- 格式：`编号,指令名`

示例：

```csv
0,愛撫
1,舐陰
2,给对方口交
3,手指挿入
4,舐肛
5,肛門愛撫
6,胸愛撫
7,玩弄乳頭
8,張開陰唇
9,自慰
```

### 7.6 `VarExtGameData.csv`

- 对应引擎功能：扩展变量（VarExt）存档登记
- 代码可确认：`ConstantData.loadGlobalVarExSetting()` 会搜索 `VarExt*.csv`
- 本仓库文件：`VarExtGameData.csv`

格式：

```csv
SAVE_DTS, DataTable名称1, DataTable名称2, ...
SAVE_XMLS, Xml名称1, Xml名称2, ...
SAVE_MAPS, Map名称1, Map名称2, ...
GLOBAL_DTS, ...
GLOBAL_XMLS, ...
GLOBAL_MAPS, ...
STATIC_DTS, ...
STATIC_XMLS, ...
STATIC_MAPS, ...
```

当前文件示例：

```csv
SAVE_DTS, DT_UFUFU_LOG
SAVE_DTS, DT_UFUFU_M_LOG
```

说明：

- `SAVE_*`：写入普通存档
- `GLOBAL_*`：写入 `global.sav`
- `STATIC_*`：更偏“全局静态持久化”
- 仅登记名字，真正对象必须在脚本里已创建

### 7.7 `VariableSize.csv`（重点）

- 对应引擎功能：修改数组尺寸
- 代码可确认：`ConstantData.loadVariableSizeData()` + `changeVariableSizeData()`
- 作用极其关键：决定大量数组长度上限

#### 基本格式

一维数组：

```csv
变量名,长度
```

二维数组：

```csv
变量名,长度1,长度2
```

三维数组（源码支持）：

```csv
变量名,长度1,长度2,长度3
```

示例：

```csv
DAY,100
MONEY,100
TIME,100
ITEM,1000
FLAG,10000
TFLAG,1000
SAVESTR,100
RESULTS,200
DITEMTYPE,100,100
DA,100,100
```

#### 规则与限制（代码可确认）

1. **第 1 列必须是变量名**，例如 `FLAG`、`CFLAG`、`TCVAR`、`DA`
2. **后续列必须是整数尺寸**
3. 0 维变量（如 `NAME`、`NO`）不可改尺寸
4. 计算值（如 `RAND`、`CHARANUM`）不可改尺寸
5. 一维数组：
   - 普通内建数组尺寸不能小于 100
   - `LOCAL` 等局部数组不能小于 1
   - 单维上限 1,000,000
6. 二维/三维数组：每一维都要给出，且每维上限 1,000,000
7. 一些变量彼此联动：
   - `ITEMNAME` 与 `ITEMPRICE` 尺寸联动
   - `PALAMNAME`、`PALAM`、`JUEL` 尺寸会做一致性校验
   - `CDFLAG` 与 `CDFLAGNAME1/2` 尺寸必须匹配

#### 当前文件示例片段

```csv
ITEMNAME,1000
ABLNAME,100
TALENTNAME,1000
EXPNAME,200
MARKNAME,100
PALAMNAME,200
TRAINNAME,1000
BASENAME,100
SOURCENAME,250
EXNAME,100
```

#### 实务建议

- 改 `CFLAG` / `TCVAR` / `FLAG` 前先检查现有脚本是否存在硬编码上限
- 减小尺寸可能导致旧存档溢出数据丢失
- 自定义数组（如 `M_FLAG`、`Setting_FLAG`）若长度由 `#DIM` 决定，则**不由** `VariableSize.csv` 控制

### 7.8 `GameBase.csv`

- 对应引擎功能：游戏元数据 / 窗口信息 / 兼容性检查
- 代码可确认：`GameBase.LoadGameBaseCsv()`
- 格式：`关键字,值`

源码中能直接确认的关键字包括：

- `コード`
- `バージョン`
- `バージョン違い認める`
- `最初からいるキャラ`
- `アイテムなし`
- `タイトル`
- `作者`
- `製作年`
- `追加情報`
- `ウィンドウタイトル`
- `動作に必要なEmueraのバージョン`
- `バージョン情報URL`
- `バージョン名`

当前文件示例：

```csv
コード,7153
バージョン,proto
タイトル,eraThe World【画蛇添足版】
弄ってみた人,まだ名前は無い人
製作年,2013～2026
バージョン違い認める,0012
追加情報,※游玩中遇到的问题可以去问一下单独指导里的哆来咪（游玩时注意温馨提示哦）
```

注意：

- 源码识别的是 `作者`，而当前文件写的是 `弄ってみた人`
- 代码中未发现对 `弄ってみた人` 的额外处理，因此这一行**很可能不会被引擎当作作者字段读取**
- 如果希望稳定填充作者信息，建议改回 `作者,xxx`

---

## 8. 角色数据文件（`CSV/Chara/`）

### 8.1 角色文件的定位

- 对应引擎功能：角色模板加载
- 代码可确认：`ConstantData.loadCharacterData()` / `loadCharacterDataFile()`
- 文件匹配：`CHARA*.CSV`
- 当前 `_fixed.config` 又启用了“搜索子目录”，因此角色文件可以分散在 `CSV/Chara/` 下

### 8.2 角色文件的两种“编号”

这一点非常重要：

1. **文件名中的 CHARA 数字** → `csvNo`
   - 例如 `Chara1 霊夢.csv` 的 `1`
   - 会被 `AddCharacterFromCsvNo()` / `EXISTCSV()` 等以“角色 CSV 编号”引用
2. **文件正文里的 `番号` / `NO`** → 角色实际 `No`
   - 例如 `番号,1,`
   - 这是角色模板本体的角色号

二者通常相同，但源码上它们是两个字段。

### 8.3 基本格式

角色文件按行写“分类,键,值”：

```csv
番号,1,
名前,博丽 灵梦,
呼び名,灵梦,
基礎,体力,2000
能力,技巧,2
素質,処女,1
フラグ,基本服装セット,101
CSTR,工作情報,清扫神社　每天早上　6时～8时
```

### 8.4 支持的分类（代码可确认）

源码 `toCharacterTemplate()` 明确支持：

- `NO` / `番号`
- `NAME` / `名前`
- `CALLNAME` / `呼び名`
- `NICKNAME` / `あだ名`
- `MASTERNAME` / `主人の呼び方`
- `MARK` / `刻印`
- `EXP` / `経験`
- `ABL` / `能力`
- `BASE` / `基礎`
- `TALENT` / `素質`
- `RELATION` / `相性`
- `CFLAG` / `フラグ`
- `EQUIP` / `装着物`
- `JUEL` / `珠`
- `CSTR`

### 8.5 第二列和第三列的规则

#### 整数类（如 `基礎/能力/素質/経験/刻印/フラグ/...`）

格式：

```csv
分类,索引名或数字,值
```

- 第 2 列可以写数字索引，也可以写名字表中定义的名称/别名
- 第 3 列若省略或无法解析，整数类默认写入 `1`

例如：

```csv
素質,処女,1
能力,清掃技能,2
フラグ,来訪時間,540
```

#### `CSTR`

格式：

```csv
CSTR,索引名或数字,字符串内容
```

例如：

```csv
CSTR,工作情報,清扫神社　每天早上　6时～8时
CSTR,職場,境内周边
CSTR,10,～乐园的巫女～　●种族：人类　●能力：主要是在空中飞行的程度的能力
```

### 8.6 真实示例 1：`Chara0.csv`

```csv
番号,0,
名前,你,
呼び名,你,
基礎,体力,2000
基礎,気力,2000
基礎,射精,10000
基礎,勃起,1500
基礎,情緒,1500
基礎,理性,1000
基礎,憤怒,1000
能力,技巧,2
素質,性別,2
```

### 8.7 真实示例 2：`Chara1 霊夢.csv`

```csv
番号,1,
名前,博丽 灵梦,
呼び名,灵梦,
基礎,体力,2000
基礎,気力,1500
基礎,勃起,1500
基礎,精力,10000
基礎,法力,4000
素質,処女,1
素質,性別,1
能力,清掃技能,2
フラグ,基本服装セット,101;服装
フラグ,初期位置,15;開始位置
CSTR,工作情報,清扫神社　每天早上　6时～8时
```

### 8.8 角色 CSV 的注意事项

1. `第二列` 若写名称，必须能在对应名字表里查到
2. `CSTR` 第 3 列是字符串，不是数值
3. `JUEL` 在角色 CSV 中可使用，但其命名依赖项目侧 `Juel.csv`
4. `;` 注释在角色文件中也被大量使用，但从通用读取器源码看，整行注释兼容性建议以实际运行日志复核

---

## 9. 辅助配置文件

### 9.1 `_Rename.csv`

- 对应功能：EraEx 风格的文本替换/重命名表
- 代码可确认：`ParserMediator.LoadEraExRenameFile()`
- 生效前提：`_fixed.config` / 用户配置中启用了“`_Rename.csvを利用する`”

#### 格式

```csv
实际值,别名
```

加载后会建立：

```text
[[别名]] -> 实际值
```

例如当前文件：

```csv
0 , 你
1 , 灵梦
2 , 留琴
3 , 卡娜
4 , 魅魔
```

表示脚本里写：

```text
[[灵梦]]
```

会被替换成：

```text
1
```

#### 额外规则（代码可确认）

- `;` 开头整行注释会跳过
- 分隔逗号支持“未转义逗号”拆分，理论上可用 `\,` 保留字面逗号
- 最终字典键名格式是 `[[别名]]`

### 9.2 `_Replace.csv`

- 对应功能：界面文字/单位等替换配置
- 代码可确认：`ConfigData.LoadReplaceFile()`
- 生效前提：启用了“`_Replace.csvを利用する`”

#### 格式

```csv
键,值
```

也支持：

```csv
键:值
```

当前文件示例：

```csv
お金の単位 , ￥
単位の位置 , 前
```

#### 当前代码中可识别的部分键（代码可确认）

包括但不限于：

- `お金の単位`
- `単位の位置`
- `起動時簡略表示`
- `販売アイテム数`
- `DRAWLINE文字`
- `BAR文字1`
- `BAR文字2`
- `システムメニュー0`
- `システムメニュー1`
- `COM_ABLE初期値`
- `汚れの初期値`
- `時間切れ表示`
- `EXPLVの初期値`
- `PALAMLVの初期値`

但**本仓库当前实际只用了金钱单位相关项**。

### 9.3 `_default.config`

- 对应功能：项目默认配置
- 代码可确认：`ConfigData.LoadConfig()` 会优先加载 `CSV/_default.config`
- 格式：

```text
配置项:值
```

当前文件示例：

```text
ウィンドウ幅:1000
ウィンドウ高さ:800
PRINTCを並べる数:5
PRINTCの文字数:25
フォント名:ＭＳ ゴシック
フォントサイズ:14
一行の高さ:14
描画インターフェース:TEXTRENDERER
```

作用：给项目提供一套默认显示/界面/行为参数。

### 9.4 `_fixed.config`

- 对应功能：强制配置 / 锁定配置
- 代码可确认：`LoadConfig()` 会在用户配置之后再加载 `_fixed.config`，并把相应项标记为 `Fixed=true`
- 格式同样是：

```text
配置项:值
```

当前文件示例：

```text
大文字小文字の違いを無視する:YES
_Rename.csvを利用する:YES
_Replace.csvを利用する:YES
ボタンの途中で行を折りかえさない:YES
サブディレクトリを検索する:YES
読み込み順をファイル名順にソートする:YES
セーブデータをバイナリ形式で保存する:YES
ERD機能を利用する:YES
```

其作用是：

1. 覆盖 `_default.config` 和用户配置
2. 把关键项目“锁死”，避免用户在运行时改掉

### 9.5 配置加载顺序（代码可确认）

`ConfigData.LoadConfig()` 的顺序是：

1. `CSV/_default.config`
2. 用户配置文件（`configPath`）
3. `CSV/_fixed.config`

因此：

- `_default.config` = 默认值
- `_fixed.config` = 最终强制值

---

## 10. 文件类型速查表

| 文件 | 主要作用 | 对应变量/系统 | 是否引擎内建直读 |
|---|---|---|---|
| Abl.csv | 能力名表 | `ABL` / `ABLNAME` | 是 |
| Base.csv | 基础值名表 | `BASE` / `BASENAME` | 是 |
| BUFF.csv | 自定义 Buff 索引表 | `BUFF` | 否 |
| CFLAG.csv | 角色旗标名表 | `CFLAG` / `CFLAGNAME` | 是 |
| CSTR.csv | 角色字符串名表 | `CSTR` / `CSTRNAME` | 是 |
| DAY.csv | 日期变量名表 | `DAY` / `DAYNAME` | 是 |
| EI_BASE.csv | 医疗角色数据索引表 | `EI_BASE` | 否 |
| EI_FLAG.csv | 医疗状态索引表 | `EI_FLAG` | 否 |
| Equip.csv | 装备槽名表 | `EQUIP` / `EQUIPNAME` | 是 |
| ex.csv | EX 状态名表 | `EX` / `EXNAME` | 是 |
| exp.csv | 经验名表 | `EXP` / `EXPNAME` | 是 |
| FLAG.csv | 全局旗标名表 | `FLAG` / `FLAGNAME` | 是 |
| GameBase.csv | 游戏元信息 | GameBase | 是 |
| Item.csv | 物品名+价格 | `ITEMNAME` / `ITEMPRICE` | 是 |
| Juel.csv | JUEL 索引表 | `JUEL` | 否（项目侧） |
| M_CFLAG.csv | 魔法角色标记表 | `M_CFLAG` | 否 |
| M_FLAG.csv | 魔法全局标记表 | `M_FLAG` | 否 |
| M_SPELL@1.csv | `M_SPELL` 第1维名表 | `M_SPELL` | 否 |
| M_SPELL@2.csv | `M_SPELL` 第2维名表 | `M_SPELL` | 否 |
| M_TFLAG.csv | 魔法流程标记表 | `M_TFLAG` | 否 |
| Mark.csv | 刻印名表 | `MARK` / `MARKNAME` | 是 |
| Palam.csv | PALAM 名表 | `PALAM` / `PALAMNAME` | 是 |
| Setting_CFLAG.csv | 设置角色标记表 | `Setting_CFLAG` | 否 |
| Setting_FLAG.csv | 设置全局标记表 | `Setting_FLAG` | 否 |
| source.csv | 来源名表 | `SOURCE` / `SOURCENAME` | 是 |
| Stain.csv | 污浊部位名表 | `STAIN` / `STAINNAME` | 是 |
| Str.csv | STR 初始字符串 | `STR` | 是 |
| Strname.csv | STR 索引名表 | `STRNAME` | 是 |
| Talent.csv | 素质名表 | `TALENT` / `TALENTNAME` | 是 |
| TCVAR.csv | 角色临时变量名表 | `TCVAR` / `TCVARNAME` | 是 |
| Tequip.csv | 临时装备状态名表 | `TEQUIP` / `TEQUIPNAME` | 是 |
| TFLAG.csv | 临时旗标名表 | `TFLAG` / `TFLAGNAME` | 是 |
| TIME.csv | 时间变量名表 | `TIME` / `TIMENAME` | 是 |
| Train.csv | 指令名表 | `TRAIN` / `TRAINNAME` | 是 |
| VarExtGameData.csv | 扩展存档登记 | VarExt | 是 |
| VariableSize.csv | 数组尺寸配置 | VariableSize | 是 |
| Chara/*.csv | 角色模板 | CharacterTemplate | 是 |
| `_Rename.csv` | `[[别名]]` 替换表 | RenameDic | 是 |
| `_Replace.csv` | 界面/单位替换表 | Replace config | 是 |
| `_default.config` | 默认配置 | Config | 是 |
| `_fixed.config` | 强制配置 | Config | 是 |

---

## 11. 结论与维护建议

1. **内建名字表**（如 `Abl/Base/Talent/...`）应继续保持“`编号,名称[,说明]`”的稳定格式。  
2. **需要脚本可读性的自定义数组**（如 `M_FLAG`、`BUFF`、`EI_BASE`）适合继续用 CSV 维护索引，并配合 `Generate_ERH.py` 生成常量头。  
3. `Juel.csv`、`BUFF.csv`、`Setting_*.csv`、`M_*.csv` 这类文件，本质上是**项目规范层**而不是引擎规范层。  
4. `VariableSize.csv` 和 `Chara/*.csv` 是最容易引发存档兼容/越界问题的两类文件，修改前应优先检查脚本硬编码与旧存档兼容性。  
5. `GameBase.csv` 当前存在 `弄ってみた人` 这种非源码标准键，若要完全依赖引擎原生读取，建议改成 `作者`。  
6. 若后续要让 `Setting_FLAG.csv` / `M_SPELL@*.csv` 的自动常量化更稳定，建议补齐对应的生成产物或在工具脚本中正式加入二维数组支持。  

