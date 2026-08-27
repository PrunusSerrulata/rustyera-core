# Complete Batch 1 initialization slice

Derived from the committed 1C client fixture, then runs the isolated S04 HTML/canvas functions. Expected data markers are unchanged and followed by SNAKE_HTML, SNAKE_CANVAS and SNAKE_BATCH1_READY. This source has not been executed. Pointer interactive scenarios remain in fixture-snake-services-clients. TUI validates the same A→B→C chain without advertising HTML/pointer/canvas.

The integrated service slice also checks exact single-pixel replacement: opaque blue→alpha128 pure
red→transparent0, keeping the green neighbor unchanged. Its script-visible marker is
`SNAKE_CANVAS_REPLACE=2164195328/4278255360/0/4278255360`; no prefilled image-data mock is evidence.
