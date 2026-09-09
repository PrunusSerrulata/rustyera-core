use super::*;
use erabasic_bytecode::{
    BytecodeFunctionKind, BytecodeType, EncodedInstruction, SymbolKey, UserArgumentSpec,
    UserCallMode, opcode,
};

fn call(arguments: usize) -> EncodedInstruction {
    opcode::resolve_user_call(&UserCallSpec {
        mode: UserCallMode::Procedure,
        allow_missing: false,
        missing_target: 0,
        arguments: vec![UserArgumentSpec::Value(BytecodeType::Integer); arguments],
    })
}

fn function(code: Vec<EncodedInstruction>) -> BytecodeFunction {
    BytecodeFunction {
        key: SymbolKey([0; 16]),
        name: String::new(),
        kind: BytecodeFunctionKind::Normal,
        parameters: Vec::new(),
        result: None,
        labels: Vec::new(),
        imports: Vec::new(),
        code,
        max_stack: 16,
    }
}

#[test]
fn decoded_user_call_limits_are_cumulative_and_progress_includes_skipped_functions() {
    let functions = [
        function(vec![call(2), call(2)]),
        function(vec![call(2), call(0), call(1)]),
        function(vec![call(1)]),
    ];
    let mut progress = 0;
    let cache = DecodedUserCallSpecs::with_limits(&functions, 2, 3, 4, || progress += 1);
    assert_eq!(progress, functions.len());
    assert!(cache.get(0, 0).is_some());
    assert!(cache.get(0, 1).is_some());
    assert!(cache.get(1, 0).is_none());
    assert!(cache.get(1, 1).is_some());
    assert!(cache.get(1, 2).is_none());
    assert!(cache.get(2, 0).is_none());
    assert_eq!(cache.functions.len(), 2);
    assert_eq!(cache.functions.capacity(), 2);
    let entries: usize = cache.functions.iter().map(Vec::len).sum();
    let capacity: usize = cache.functions.iter().map(Vec::capacity).sum();
    assert_eq!(entries, 3);
    // The cap counts entries, not Vec allocation slots. Small vectors reserve
    // four slots; geometric growth remains bounded by four times inserted entries.
    assert!(capacity > entries);
    assert!(capacity <= 4 * entries);
    assert_eq!(
        cache
            .functions
            .iter()
            .flatten()
            .map(|(_, spec)| spec.arguments.capacity())
            .sum::<usize>(),
        4
    );

    let cache = DecodedUserCallSpecs::with_limits(&functions, 3, 10, 5, || {});
    assert!(cache.get(1, 0).is_none());
    assert!(cache.get(1, 1).is_some());
    assert!(cache.get(1, 2).is_some());
    assert!(cache.get(2, 0).is_none());
    assert_eq!(
        cache
            .functions
            .iter()
            .flatten()
            .map(|(_, spec)| spec.arguments.capacity())
            .sum::<usize>(),
        5
    );

    let calls = DecodedUserCallSpecs::with_limits(&functions, 3, 2, 100, || {});
    assert!(calls.get(0, 1).is_some());
    assert!(calls.get(1, 0).is_none()); // Call limit alone; argument budget remains.
    let directories = DecodedUserCallSpecs::with_limits(&functions, 2, 100, 100, || {});
    assert!(directories.get(1, 2).is_some());
    assert!(directories.get(2, 0).is_none()); // Function limit alone.

    let mut progress = 0;
    let empty = DecodedUserCallSpecs::with_limits(&functions, 0, 10, 10, || progress += 1);
    assert_eq!(progress, functions.len());
    assert_eq!(empty.functions.capacity(), 0);
    assert!(empty.get(0, 0).is_none());
}

#[test]
fn decoded_user_call_cache_skips_invalid_headers_and_oversized_argument_counts() {
    let mut huge = vec![UserCallMode::Procedure as u8, 0, 0, 0, 0, 0, 0xff, 0xff];
    huge.resize(8 + usize::from(u16::MAX), 0); // Valid omitted slots, but over the cache budget.
    let mut invalid = call(1).payload.to_vec();
    invalid[0] = 0xff;
    let functions = [function(vec![
        EncodedInstruction::new(Opcode::ResolveUserCall, vec![0; 7]),
        EncodedInstruction::new(Opcode::ResolveUserCall, invalid),
        EncodedInstruction::new(Opcode::ResolveUserCall, huge),
        call(2),
    ])];
    let mut progress = 0;
    let cache = DecodedUserCallSpecs::with_limits(&functions, 1, 1, 2, || progress += 1);
    assert_eq!(progress, 1);
    for instruction in 0..3 {
        assert!(cache.get(0, instruction).is_none());
    }
    assert_eq!(cache.get(0, 3).unwrap().arguments.len(), 2);
    assert_eq!(cache.functions[0].len(), 1);
    assert_eq!(cache.functions[0][0].1.arguments.capacity(), 2);
}
