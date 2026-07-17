# Input and wait compatibility

This document fixes the behavior expected from the runtime for the input
operations audited against Emuera commit
`26a35dc9334bb67590b96f7b8efbefbf199e391e`. The analyzer, compiler, protocol and
VM capability checks and the basic runtime wait/service paths exist today.

## Meaning of transient

`Transient` is the VM snapshot category for a pending Host operation that cannot
be rebound from an exact VM snapshot. It includes a live deadline, a fresh
frontend service query and a non-resumable Void wait. It does not imply that the
operation is guaranteed to finish. A stable wait has no deadline and can resume
only from a versioned user-input message carrying the same wait identity.

## Reference behavior

| Operation | Reference behavior | Wait classification |
| --- | --- | --- |
| `TINPUT` | Takes `time, default[, display, message, mouse, canskip]`; produces integer input and uses `mouse == 1`. | No wait when the sixth argument is present and message skip is active. Otherwise transient when `time > 0`, stable when `time <= 0`. |
| `TONEINPUTS` | Uses the string form of the same six slots and sets `OneInput`; long defaults are retained. | Same as `TINPUT`. |
| `TWAIT` | Takes exactly `time, flag`; `flag == 0` requests Enter and any other value requests Void. | Transient when `time > 0`; stable for `time <= 0 && flag == 0`; snapshot-ineligible transient for `time <= 0 && flag != 0`. |
| `FORCEWAIT` | Takes no arguments, requests Enter and sets `StopMesskip`, ending the current message-skip run. | Stable input wait. |
| `GETKEY` | Takes one integer. Inactive frontend and codes outside `0..=255` return zero; otherwise the pressed bit determines zero or one. | Invalid codes return immediately. A valid code uses a fresh frontend query and is transient until its response arrives. |
| `TINPUTNF` | Not defined by the pinned implementation. | Analyzer error; no wait is created. |

The `canskip` value itself is not evaluated as a Boolean by the reference
instruction. Supplying the sixth slot grants the shortcut whenever message skip
is already active. Arguments used to construct the request are evaluated before
that shortcut is selected. Display defaults to enabled and the timeout message
defaults to the configured time-up label.

## Runtime status

The runtime decides `WaitStability` before publishing an
`InputWait`. A positive millisecond limit is converted to a monotonic deadline;
zero and negative values create no deadline. Timeout commits the typed default
and applies the configured display/message behavior before resuming the fiber.

For `GETKEY`, an in-range call issues the versioned `InputState/get_key_state`
service and completes the VM Host request only after the correlated response.
The runtime owns the per-key toggle observation shared with `GETKEYTRIGGERED`.
It must return zero for an inactive frontend and must never sample an OS API in
the VM or runtime library itself.

Exact snapshots remain legal only at stable input waits. Traditional Era saves
do not serialize VM stacks or pending input/service requests.

The runtime implements typed waits, positive monotonic deadlines, defaults, timeout messages,
FORCEWAIT flags, TWAIT Void classification, fresh GETKEY queries, the shared
GETKEYTRIGGERED toggle observation and the sixth-slot message-skip shortcut. Primitive mouse/key
input intentionally arrives as frontend-normalized EraBasic-shaped result fields; runtime still
validates the wait, token, epoch and ordering and alone synthesizes timeouts. This keeps platform
event interpretation outside the runtime while preserving authoritative game results.

One-input maximum-length validation is not yet fully enforced by the runtime. That remaining
gap is tracked in [Runtime compatibility status](runtime-compatibility-status.zh-CN.md).
