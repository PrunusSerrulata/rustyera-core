# Emuera runtime reference mapping

Compatibility is pinned to reference commit
`26a35dc9334bb67590b96f7b8efbefbf199e391e`. This inventory records what shaped the
interface and the current runtime. A row describes the target contract, not a claim
that every reference system state or Host operation is already implemented.

Reference behavior is interpreted under the project priority order documented in
[Design principles](design-principles.md). Portable script-visible rules remain compatibility
targets; WinForms/GDI implementation details are represented as semantic intent or explicitly
unsupported portable operations.

| Reference area | Observed behavior | Contract representation |
| --- | --- | --- |
| `Runtime/Script/Process.cs` and `Process.SystemProc.cs` | Central script/system state machine covers title, opening, train, ablup, shop, save/load, normal execution and ERB reload. | `RuntimePhase`, lifecycle messages, project revisions and reload transactions. |
| `Runtime/Script/Process.State.cs` | System states determine where BEGIN, save and load are legal; call frames are owned by the process state. | Runtime remains aggregate authority; VM frames are inspected only through `VmDebugInspect`. |
| `Runtime/InputRequest.cs` | Nine effective input kinds, defaults, one-input, message-skip, system/mouse flags and optional timeout state. | `WaitKind`, `InputWait`, normalized `InputIntent`, opaque interaction tokens and `WaitChange` retain the observable contract without exposing game values to the frontend. |
| `UI/Game/EmueraConsole.cs` | Console transitions among initialization, running, input wait, sleep, quit and error; UI timers race with input and stale buttons are generation-checked. | Ordered monotonic time/input messages, wait IDs, opaque epoch-scoped interaction tokens, transient QTE waits and explicit shutdown/fault states. |
| `UI/Game/EmueraConsole.Print.cs` | Text/style/buttons, HTML, images, shapes, backgrounds, window title and log buffer are observable. | Revisioned `PresentationSnapshot` synchronization baselines followed by `PresentationDelta` updates. Frontend render objects are projections. |
| `Statements/Instraction.Child.cs` and `Function/Creator.Method.cs` | Sound, dynamic graphics, font metrics, files, network update checks and external URLs directly use platform APIs. | Asynchronous storage/platform service messages; no VM instruction or runtime layer performs OS I/O. |
| `Statements/Variable/VariableEvaluator.cs` | Save slots, global saves and DAT files are listed/read/written directly, with game/version compatibility checks. | Runtime-owned in-memory codecs, validation and transactions over correlated frontend storage namespaces. |
| `UI/Game/PrintStringBuffer.cs` and console line types | `PRINTC`, alignment, separators, automatic buttons, HTML and physical history combine semantic output with GDI/WinForms measurement. | Runtime-owned logical lines, `ColumnCell`, `Separator`, history operations and tokens express intent. Client pixel layout is not canonical state; explicit queries of realized layout use revision-bound typed frontend services, and missing frontend operations are reported as unsupported. |
| GDI/canvas/resource classes | Image decoding, raster mutation, font measurement and animation scheduling use Windows device objects. | Canonical resource/canvas replay plus typed frontend services. Content-addressed source-image facts are distinct from frontend-dependent text/canvas raster observations. Physical GDI/CBG object APIs remain unsupported where no portable contract exists. |
| `Process.CalledFunction.cs` | Frames retain function identity, return address, event state, arguments and local scope. | Generation/frame-aware VM debug descriptors and source-mapped call stacks. |
| `EmueraConsole.DebugCommand` and `UI/DebugDialog.cs` | Watches evaluate current expressions; debug commands clone/restore control state and reject flow, waits, partial and unsafe instructions. | Separate debug channel, coherent stop tokens and the reference-safe console subset. |

Architectural differences are intentional: the reference console owns timers and much
presentation state, whereas RustyEra makes them runtime-authoritative messages; reference
save/resource code performs I/O directly, whereas the Rust contract never does; the new
permission model, breakpoints and stepping are debugger extensions rather than claimed
Emuera behavior.

Runtime authority covers semantic presentation and the game-state transition that follows
an observation; it does not make runtime a renderer. Where a script explicitly asks for a
font-, viewport- or raster-dependent result, the target contract uses the selected
authoritative frontend's revision-bound response. Using that value to determine persistent
gameplay is diagnosed as a portability hazard and may become deprecated after a portable
replacement and real-script review exist.
