pub(super) fn canvas_rect(value: [i32; 4]) -> CanvasRect {
    CanvasRect {
        x: value[0],
        y: value[1],
        width: value[2],
        height: value[3],
    }
}

fn argb_bits(value: i64) -> u32 {
    u32::try_from(value & i64::from(u32::MAX)).expect("masked ARGB fits u32")
}

pub(super) fn opaque_rgb(value: i64) -> u32 {
    0xff00_0000 | (argb_bits(value) & 0x00ff_ffff)
}

fn bump_canvas(canvas: &mut CanvasSurface) {
    canvas.revision = canvas.revision.saturating_add(1);
}

fn rectangle_axis_intersects(origin: i32, extent: i32, limit: u32) -> bool {
    if extent == 0 {
        return false;
    }
    let origin = i64::from(origin);
    let end = origin.saturating_add(i64::from(extent));
    origin.min(end) < i64::from(limit) && origin.max(end) > 0
}

fn push_canvas_command(
    total_retained: &mut usize,
    canvas: &mut CanvasSurface,
    command: CanvasCommand,
) -> bool {
    push_canvas_command_with_limit(
        total_retained,
        canvas,
        command,
        MAXIMUM_CANVAS_COMMAND_BYTES,
    )
}

fn push_canvas_command_with_limit(
    total_retained: &mut usize,
    canvas: &mut CanvasSurface,
    command: CanvasCommand,
    maximum: usize,
) -> bool {
    if canvas.retained_command_bytes == 0 && !canvas.commands.is_empty() {
        // Snapshots written before the retained-byte counter was introduced decode
        // it as zero. Rebuild it lazily before accepting another command.
        canvas.retained_command_bytes = canvas
            .commands
            .iter()
            .map(CanvasCommand::retained_bytes)
            .fold(0, usize::saturating_add);
    }
    let retained = command.retained_bytes();
    if !retained_canvas_bytes_fit(
        *total_retained,
        canvas.retained_command_bytes,
        retained,
        maximum,
    ) {
        return false;
    }
    canvas.retained_command_bytes = canvas.retained_command_bytes.saturating_add(retained);
    *total_retained = total_retained.saturating_add(retained);
    canvas.commands.push(command);
    true
}

pub(super) fn retained_canvas_bytes_fit(
    total_retained: usize,
    surface_retained: usize,
    incoming: usize,
    maximum: usize,
) -> bool {
    surface_retained.saturating_add(incoming) <= maximum
        && total_retained.saturating_add(incoming) <= maximum
}
