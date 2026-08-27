# 批次 0：双 oracle 实测比较汇总

以下为同 fixture、seed=123456 的实际结果，不是完整蛇版兼容验收。原始输入、输出、诊断、布局及逐字段差异保存在本组 batch-0-work/results/recompared-*.json；原始 oracle 请求/响应在 batch-0-work/oracle-evidence-*/evidence.json。使用当前 fixture 的最终有效采集，旧 hash 初次失败材料不参与汇总。

matched_observables 仅指本例共同结构的观测相同；incomparable 保留实际错误但诊断没有共同语义 schema；blocked 表示 Rust 不能执行或完整消费输入 trace；different 为已观察差异。所有例子 snakeTargetStatus 仍为 deferred_semantics。

| Case / 组 | Rust / 原版 | Rust / 蛇版 | 后续批次 |
|---|---|---|---|
| printc-layout / PRINTC | different | different | 4 |
| arithmetic-constant-add / arithmetic | matched_observables | different | 2 |
| arithmetic-constant-subtract / arithmetic | matched_observables | different | 2 |
| arithmetic-constant-multiply / arithmetic | matched_observables | different | 2 |
| arithmetic-variable / arithmetic | matched_observables | different | 2 |
| arithmetic-divide-zero / arithmetic | incomparable | different | 2 |
| arithmetic-modulo-zero / arithmetic | incomparable | different | 2 |
| rng-roundtrip / RNG | matched_observables | different | 2 |
| rng-invalid-state / RNG | different | different | 2 |
| array-ref / REF | matched_observables | matched_observables | 6 |
| arity-normal / extra_args | matched_observables | matched_observables | 2 |
| arity-extra-value / extra_args | incomparable | different | 2 |
| arity-trailing-omission / extra_args | matched_observables | matched_observables | 2 |
| arity-extra-effect / extra_args | incomparable | different | 2 |
| toint-integer / TOINT | matched_observables | matched_observables | 2 |
| toint-decimal / TOINT | matched_observables | matched_observables | 2 |
| toint-empty / TOINT | matched_observables | matched_observables | 2 |
| toint-invalid / TOINT | matched_observables | matched_observables | 2 |
| toint-overflow / TOINT | incomparable | different | 2 |
| key-inactive / GETKEY | matched_observables | matched_observables | 2 |
| key-held-interleaved / GETKEY | matched_observables | different | 2 |
| key-same-pump-click / GETKEY | blocked | blocked | 2 |
| key-clear-old-latch / GETKEY | blocked | blocked | 2 |
| key-pump-down / GETKEY | blocked | blocked | 2 |
| key-resume / GETKEY | different | different | 2 |
| key-validation-and-reset / GETKEY | blocked | blocked | 2 |
| toint-runtime-overflow / TOINT | incomparable | different | 2 |

## 结果与边界

- original：blocked 4，different 3，incomparable 6，matched_observables 14。
- snake：blocked 4，different 15，matched_observables 8。
- 原版与 Rust 的除零、模零、TOINT 溢出均实际失败；oracle envelope.ok=true 只代表请求处理成功，不能误算脚本成功。诊断结构不同仍为 incomparable。
- 蛇版常量/变量溢出使用饱和及告警，与当前 wrapping_i64_v1 不同；超量实参、TOINT 溢出和按键 latch 差异归批次 2/6。
- 蛇版 RNG 的 DumpRanddata 将状态写入临时 ToArray 副本，随后 INITRAND 恢复零；固定输出 192905、520548、0、0 以 observed 记录，未更改参考算法，未伪报 roundtrip 通过。
- 原版非法 RANDDATA 也与 Rust 的拒绝行为不同，记录为批次 2 待决策；额外实参双方拒绝但错误结构不等价。
- GETKEY 三个 AWAIT pump case 在 Rust 未消费完整 trace，validation/reset 缺少等价 runtime 原语，明确 blocked；两个 oracle 的输入原子性、无消费观察、reset 隔离实际断言通过，不能替代 Rust 能力。
- key-resume 输入回显（Rust 空行、oracle 0）仍为输出差异，蛇版另有 latch watches 差异，不按 setup 噪声过滤。
- PRINTC 两引擎与当前列布局的文本/排版均不同。蛇版 SKIASHARP 和 TEXTRENDERER 额外观察均完成；provider 从实际缓存读取，不按配置名猜测。字体输入 hash 和 family/fallback 已记录，实际安装字节来源未验证，不宣称像素等价。

## 比较器修复与来源

精确扣除该 case load 响应中完整的输出前缀；相似脚本文字不会被过滤，前缀变化则 incomparable。仅将 exact code/level/configuration context/identity/source 匹配的实验 profile warning 另列 setup。原始响应保持不变。recompare.py 校验 semantic baseline、fixture hash、seed、profile、顺序请求，离线重算并记录父证据 hash，不重新执行引擎、不覆盖原记录。

| 最终比较文件（本组 results/） | SHA-256 |
|---|---|
| recompared-original-arithmetic-outcome.json | 32dbaaaa2436af8248799c914b4b6db6e5d60024149d82fc928243c620467039 |
| recompared-original-rng.json | 341b6561ab89b22b8afdd9658731b9f4f10b40f499b0511523685e7ca879f068 |
| recompared-original-ref.json | 225f7733b9f7abe15c2b89401ebab12bf71deddb3716fe43faf5ba64ea9addfd |
| recompared-original-extra-args.json | f8caa9a973d1f2cb849e6ea38a70e952e940349d4d84d6c928d2ac3677bd82da |
| recompared-original-toint-outcome.json | 7c186baacce4206d0243b3ea1059b4346cd75d1e9d4e5868dceeae8866cdb7f6 |
| recompared-original-getkey.json | 2ba12de777792fe2a1aaf19e565d66aef0658232a076701921c44195803a9f81 |
| recompared-original-printc-current.json | 4544727d77143ae792f46c982584deaa9c9c5a196df0831bfcc7362261162a09 |
| recompared-snake-arithmetic.json | 4cd09e40bc7e36e994cf39115ed1727775f4f97cda7c9ebdf04c7682c2c0634d |
| recompared-snake-rng.json | 30e5451383e0a07d00a4d1fce51d3eae19e8535951a4504313db6e8c7a3cfebf |
| recompared-snake-ref.json | d3a9617cebe3f5df345e0b42bd84481b5d8b0bd72dbba6088e5c1869135abd24 |
| recompared-snake-extra-args.json | 501becf7da8910654b06e843bbedeaa808469bdc24cf90f110d88d797f8fb52c |
| recompared-snake-toint.json | 04a4bd076df8b8db8111a4fab7db1aefc89078be93e39ad27c8a0c7337fac99d |
| recompared-snake-getkey.json | 91278b3318a3255fcf2ca23929dbe34b703da11a2da65f95248223c6375318ff |
| recompared-snake-printc-skiasharp-current.json | e67283e71bd48b67af0adb8fe8909428223e0f76111a7f6c3b4ae099d2832a47 |
