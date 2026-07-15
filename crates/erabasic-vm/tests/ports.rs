use erabasic_bytecode::{BytecodeType, SymbolKey};
use erabasic_vm::{
    FiberId, FrameId, GenerationId, PlaceDescriptor, VmDebugVariableRef, VmDriveMode,
    VmHostCompletion, VmStepKind, VmStopToken, VmValue,
};

#[test]
fn prospective_host_completion_is_data_not_a_callback() {
    let completion = VmHostCompletion::Ready(erabasic_vm::HostReady {
        value: Some(VmValue::default_for(BytecodeType::Integer)),
        writes: Vec::new(),
    });
    assert!(matches!(completion, VmHostCompletion::Ready(_)));
    assert_eq!(
        VmDriveMode::SelectedFiber(FiberId(2)),
        VmDriveMode::SelectedFiber(FiberId(2))
    );
}

#[test]
fn local_debug_targets_are_generation_and_frame_scoped() {
    let target = VmDebugVariableRef {
        target: PlaceDescriptor {
            variable: SymbolKey([1; 16]),
            indices: vec![3],
            character: None,
            fiber: Some(FiberId(4)),
            frame: Some(FrameId(5)),
        },
        generation: GenerationId(2),
    };
    assert_eq!(target.target.frame, Some(FrameId(5)));
    assert_eq!(
        VmStopToken {
            pause_epoch: 9,
            generation: GenerationId(2),
        }
        .generation,
        target.generation
    );
    assert_eq!(VmStepKind::Over, VmStepKind::Over);
}
