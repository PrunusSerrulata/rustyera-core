# Emuera runtime reference mapping

Compatibility is pinned to reference commit
`26a35dc9334bb67590b96f7b8efbefbf199e391e`. This inventory records what shaped the
interface; it does not claim that a Rust runtime implements the behavior yet.

| Reference area | Observed behavior | Contract representation |
| --- | --- | --- |
| `Runtime/Script/Process.cs` and `Process.SystemProc.cs` | Central script/system state machine covers title, opening, train, ablup, shop, save/load, normal execution and ERB reload. | `RuntimePhase`, lifecycle messages, project revisions and reload transactions. |
| `Runtime/Script/Process.State.cs` | System states determine where BEGIN, save and load are legal; call frames are owned by the process state. | Runtime remains aggregate authority; VM frames are inspected only through `VmDebugInspect`. |
| `Runtime/InputRequest.cs` | Nine effective input kinds, defaults, one-input, message-skip, system/mouse flags and optional timeout state. | `WaitKind`, `InputWait`, `FrontendInput` and `WaitChange` retain these fields and stable IDs. |
| `UI/Game/EmueraConsole.cs` | Console transitions among initialization, running, input wait, sleep, quit and error; UI timers race with input and stale buttons are generation-checked. | Ordered monotonic time/input messages, wait IDs, button generations, transient QTE waits and explicit shutdown/fault states. |
| `UI/Game/EmueraConsole.Print.cs` | Text/style/buttons, HTML, images, shapes, backgrounds, window title and log buffer are observable. | Revisioned `PresentationSnapshot`/`PresentationDelta`; frontend render objects are projections. |
| `Statements/Instraction.Child.cs` and `Function/Creator.Method.cs` | Sound, dynamic graphics, font metrics, files, network update checks and external URLs directly use platform APIs. | Asynchronous storage/platform service messages; no VM instruction or runtime layer performs OS I/O. |
| `Statements/Variable/VariableEvaluator.cs` | Save slots, global saves and DAT files are listed/read/written directly, with game/version compatibility checks. | Frontend storage namespaces and correlated operations; future runtime owns compatible serialization and validation. |
| `Process.CalledFunction.cs` | Frames retain function identity, return address, event state, arguments and local scope. | Generation/frame-aware VM debug descriptors and source-mapped call stacks. |
| `EmueraConsole.DebugCommand` and `UI/DebugDialog.cs` | Watches evaluate current expressions; debug commands clone/restore control state and reject flow, waits, partial and unsafe instructions. | Separate debug channel, coherent stop tokens and the reference-safe console subset. |

Architectural differences are intentional: the reference console owns timers and much
presentation state, whereas RustyEra makes them runtime-authoritative messages; reference
save/resource code performs I/O directly, whereas the Rust contract never does; the new
permission model, breakpoints and stepping are debugger extensions rather than claimed
Emuera behavior.
