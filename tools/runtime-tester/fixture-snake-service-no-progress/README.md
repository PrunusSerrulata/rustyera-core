# Isolated S04 non-progress hazard

Prepared, never executed. Fixed HtmlStringLinesMethod loops until its tail is
empty and has no progress guard. For `HTML_STRINGLINES("A", 0)`, if the actual
measured A is wider than zero, HtmlSubString returns an unchanged tail. The
reference can remain inside that method until its wrapper or outer watchdog
terminates it. Do not place this entry in the normal service smoke/default run.

Run one selected case in its own process and new isolated output/game directory,
only after D review and static gates. Keep the existing request timeout and
5-second complete oracle-state watchdog. A request timeout/unchanged-state kill
is **failed/incomplete reference evidence**, never a successful return value,
never an expectedRejection boolean that bypasses the missing response. Save the
pending request, last full response, diagnostics and process termination.

Rust Browser/Tauri must use actual measurements and return the explicit
`runtime.html_query_no_progress` failure when the cut cannot advance. Preserve
RESULT:10=777 when observable and mark it missing if the debugger cannot inspect
fault state. The stronger bounded failure is a documented intentional safety
difference, not equality to the reference's nontermination. TUI without HTML v2
must instead report its capability failure; that is not this runtime behavior.

No total batch deadline is reinstated. The per-command/request limits and
watchdog remain mandatory. Do not increase waits to hide repeated no-progress.
