use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeFunction, BytecodeGlobal, BytecodePersistence,
    BytecodeStorage, BytecodeType, Digest, Opcode, SourceMap, SymbolKey, opcode,
};
use erabasic_validator::{ValidationCode, ValidationContext, validate_bytecode};

use super::support::{fixture_runtime_variables, project_data};

fn method_artifact(
    code: Vec<erabasic_bytecode::EncodedInstruction>,
    result: BytecodeType,
    globals: Vec<BytecodeGlobal>,
) -> BytecodeArtifact {
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        runtime_builtins: Vec::new(),
        runtime_native_authorizations: Vec::new(),
        runtime_host_authorizations: Vec::new(),
        runtime_staged_authorizations: Vec::new(),
        runtime_variables: globals
            .iter()
            .map(|global| erabasic_bytecode::RuntimeVariableSymbol {
                match_name_rejection: None,
                character_disposal: erabasic_bytecode::CharacterArrayDisposal::Preserve,
                key: global.key,
                reference: false,
                reference_semantics: erabasic_bytecode::RuntimeReferenceSemantics {
                    is_const: false,
                    can_restructure: false,
                },
            })
            .collect(),
        project_data: project_data(),
        globals,
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: SymbolKey::derive("test.function", b"dynamic-method"),
            name: "DYNAMIC_METHOD".into(),
            kind: erabasic_bytecode::BytecodeFunctionKind::Method,
            parameters: Vec::new(),
            result: Some(result),
            labels: Vec::new(),
            imports: Vec::new(),
            code,
            max_stack: 16,
        }],
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    fixture_runtime_variables(&mut artifact);
    artifact.refresh_ids().unwrap();
    artifact
}

fn method_spec(
    arguments: Vec<erabasic_bytecode::UserArgumentSpec>,
) -> erabasic_bytecode::UserCallSpec {
    erabasic_bytecode::UserCallSpec {
        mode: erabasic_bytecode::UserCallMode::MethodInteger,
        allow_missing: false,
        missing_target: 0,
        arguments,
    }
}

fn method_validation_codes(
    code: Vec<erabasic_bytecode::EncodedInstruction>,
) -> Vec<ValidationCode> {
    let artifact = method_artifact(code, BytecodeType::Integer, Vec::new());
    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    report
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn accepts_method_omission_minimum_integer_and_nested_call_captures() {
    use erabasic_bytecode::UserArgumentSpec::{Omitted, Value};
    let outer = method_spec(vec![
        Omitted,
        Value(BytecodeType::Integer),
        Value(BytecodeType::Integer),
    ]);
    let inner = method_spec(Vec::new());
    let codes = method_validation_codes(vec![
        opcode::push_string("OUTER"),
        opcode::resolve_user_call(&outer),
        opcode::advance_user_argument(1, 0, erabasic_bytecode::UserArgumentAdvance::Omitted),
        opcode::push_integer(i64::MIN),
        opcode::capture_user_argument(1, 1, false),
        opcode::push_string("INNER"),
        opcode::resolve_user_call(&inner),
        opcode::invoke_user_call(6),
        opcode::capture_user_argument(1, 2, false),
        opcode::invoke_user_call(1),
        opcode::return_value(true),
    ]);
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn accepts_method_fallback_branch_with_the_declared_result_type() {
    for result in [
        erabasic_bytecode::MethodResult::Integer,
        erabasic_bytecode::MethodResult::String,
    ] {
        let spec = erabasic_bytecode::UserCallSpec {
            mode: result.into(),
            allow_missing: true,
            missing_target: 4,
            arguments: Vec::new(),
        };
        let fallback = match result {
            erabasic_bytecode::MethodResult::Integer => opcode::push_integer(17),
            erabasic_bytecode::MethodResult::String => opcode::push_string("fallback"),
        };
        let artifact = method_artifact(
            vec![
                opcode::push_string("OPTIONAL"),
                opcode::resolve_user_call(&spec),
                opcode::invoke_user_call(1),
                opcode::jump(Opcode::Jump, 6),
                opcode::abandon_user_call(1),
                fallback,
                opcode::return_value(true),
            ],
            result.bytecode_type(),
            Vec::new(),
        );
        let report = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &ValidationContext::for_artifact(&artifact),
        );
        assert!(report.is_valid(), "{:#?}", report.diagnostics);
    }
}

#[test]
fn accepts_variable_value_and_reference_branches_joining_as_one_method_slot() {
    let key = SymbolKey::derive("test.variable", b"A");
    let spec = method_spec(vec![erabasic_bytecode::UserArgumentSpec::Variable(key)]);
    let artifact = method_artifact(
        vec![
            opcode::push_string("CHOOSE_FORMAL"),
            opcode::resolve_user_call(&spec),
            opcode::guard_user_argument(1, 0, 10),
            opcode::select_user_argument(1, 0, 7),
            opcode::variable(Opcode::LoadVariable, key, 0, 0),
            opcode::capture_user_argument(1, 0, false),
            opcode::jump(Opcode::Jump, 9),
            opcode::variable(Opcode::MakePlace, key, 0, 0),
            opcode::capture_user_argument(1, 0, true),
            opcode::jump(Opcode::Jump, 11),
            opcode::advance_user_argument(1, 0, erabasic_bytecode::UserArgumentAdvance::Discarded),
            opcode::invoke_user_call(1),
            opcode::return_value(true),
        ],
        BytecodeType::Integer,
        vec![BytecodeGlobal {
            key,
            name: "A".into(),
            value_type: BytecodeType::Integer,
            dimensions: vec![4],
            mutable: true,
            storage: BytecodeStorage::Project,
            persistence: BytecodePersistence::GameSave,
            initial_values: Vec::new(),
            owner: None,
        }],
    );
    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(report.is_valid(), "{:#?}", report.diagnostics);
}

#[test]
fn rejects_opaque_method_tokens_consumed_by_ordinary_instructions() {
    for instruction in [
        erabasic_bytecode::EncodedInstruction::new(Opcode::Dup, Vec::new()),
        erabasic_bytecode::EncodedInstruction::new(Opcode::ToString, Vec::new()),
        erabasic_bytecode::EncodedInstruction::new(Opcode::SelectStart, Vec::new()),
        opcode::unary(1),
        erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new()),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&method_spec(Vec::new())),
            instruction,
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&ValidationCode::TypeMismatch), "{codes:?}");
    }
}

#[test]
fn rejects_opaque_captures_being_discarded_or_recaptured() {
    let spec = method_spec(vec![erabasic_bytecode::UserArgumentSpec::Value(
        BytecodeType::Integer,
    )]);
    for instruction in [
        erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new()),
        erabasic_bytecode::EncodedInstruction::new(Opcode::Dup, Vec::new()),
        opcode::capture_user_argument(1, 0, false),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&spec),
            opcode::push_integer(1),
            opcode::capture_user_argument(1, 0, false),
            instruction,
            opcode::invoke_user_call(1),
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&ValidationCode::TypeMismatch), "{codes:?}");
    }
}

#[test]
fn rejects_method_capture_wrong_type_omitted_slot_and_out_of_order_slot() {
    use erabasic_bytecode::UserArgumentSpec::{Omitted, Value};
    for (spec, value, slot, expected) in [
        (
            method_spec(vec![Value(BytecodeType::Integer)]),
            opcode::push_string("wrong"),
            0,
            ValidationCode::TypeMismatch,
        ),
        (
            method_spec(vec![Omitted]),
            opcode::push_integer(1),
            0,
            ValidationCode::InvalidOperand,
        ),
        (
            method_spec(vec![Value(BytecodeType::Integer)]),
            opcode::push_integer(1),
            1,
            ValidationCode::InvalidOperand,
        ),
        (
            method_spec(vec![
                Value(BytecodeType::Integer),
                Value(BytecodeType::Integer),
            ]),
            opcode::push_integer(1),
            1,
            ValidationCode::StackMismatch,
        ),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&spec),
            value,
            opcode::capture_user_argument(1, slot, false),
            opcode::invoke_user_call(1),
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&expected), "{codes:?}");
    }
}

#[test]
fn rejects_method_invocation_with_incomplete_arguments_or_forged_origin() {
    use erabasic_bytecode::UserArgumentSpec::Value;
    for (spec, resolve, expected) in [
        (
            method_spec(vec![Value(BytecodeType::Integer)]),
            1,
            ValidationCode::StackMismatch,
        ),
        (method_spec(Vec::new()), 0, ValidationCode::InvalidOperand),
        (
            method_spec(Vec::new()),
            u32::MAX,
            ValidationCode::InvalidOperand,
        ),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&spec),
            opcode::invoke_user_call(resolve),
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&expected), "{codes:?}");
    }
}

#[test]
fn rejects_method_consumers_before_their_resolve_even_when_control_flow_reaches_it_first() {
    use erabasic_bytecode::UserArgumentSpec::{Value, Variable};
    let key = SymbolKey::derive("test.variable", b"A");
    for code in [
        vec![
            opcode::jump(Opcode::Jump, 3),
            opcode::invoke_user_call(4),
            opcode::return_value(true),
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&method_spec(Vec::new())),
            opcode::jump(Opcode::Jump, 1),
        ],
        vec![
            opcode::jump(Opcode::Jump, 5),
            opcode::push_integer(1),
            opcode::capture_user_argument(6, 0, false),
            opcode::invoke_user_call(6),
            opcode::return_value(true),
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&method_spec(vec![Value(BytecodeType::Integer)])),
            opcode::jump(Opcode::Jump, 1),
        ],
        vec![
            opcode::jump(Opcode::Jump, 9),
            opcode::select_user_argument(10, 0, 5),
            opcode::variable(Opcode::LoadVariable, key, 0, 0),
            opcode::capture_user_argument(10, 0, false),
            opcode::jump(Opcode::Jump, 7),
            opcode::variable(Opcode::MakePlace, key, 0, 0),
            opcode::capture_user_argument(10, 0, true),
            opcode::invoke_user_call(10),
            opcode::return_value(true),
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&method_spec(vec![Variable(key)])),
            opcode::jump(Opcode::Jump, 1),
        ],
    ] {
        let artifact = method_artifact(
            code,
            BytecodeType::Integer,
            vec![BytecodeGlobal {
                key,
                name: "A".into(),
                value_type: BytecodeType::Integer,
                dimensions: vec![4],
                mutable: true,
                storage: BytecodeStorage::Project,
                persistence: BytecodePersistence::GameSave,
                initial_values: Vec::new(),
                owner: None,
            }],
        );
        let report = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &ValidationContext::for_artifact(&artifact),
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == ValidationCode::InvalidOperand),
            "{:#?}",
            report.diagnostics
        );
    }
}

#[test]
fn rejects_method_missing_branch_that_does_not_discard_its_token_first() {
    let spec = erabasic_bytecode::UserCallSpec {
        allow_missing: true,
        missing_target: 2,
        ..method_spec(Vec::new())
    };
    for first in [
        opcode::invoke_user_call(1),
        opcode::push_integer(0),
        erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, vec![0]),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("OPTIONAL"),
            opcode::resolve_user_call(&spec),
            first,
            opcode::return_value(true),
        ]);
        assert!(
            codes.contains(&ValidationCode::InvalidControlFlow),
            "{codes:?}"
        );
    }
}

#[test]
fn method_variable_specs_require_the_callers_frame_local_owner() {
    let caller = SymbolKey::derive("test.function", b"dynamic-method");
    let other = SymbolKey::derive("test.function", b"other-method");
    let key = SymbolKey::derive("test.variable", b"LOCAL_ARRAY");
    for (storage, owner, accepted) in [
        (BytecodeStorage::FunctionLocal, caller, true),
        (BytecodeStorage::FunctionLocal, other, false),
        (BytecodeStorage::FunctionStatic, other, true),
    ] {
        let spec = method_spec(vec![erabasic_bytecode::UserArgumentSpec::Variable(key)]);
        let mut artifact = method_artifact(
            vec![
                opcode::push_string("METHOD"),
                opcode::resolve_user_call(&spec),
                opcode::push_integer(1),
                opcode::capture_user_argument(1, 0, false),
                opcode::invoke_user_call(1),
                opcode::return_value(true),
            ],
            BytecodeType::Integer,
            vec![BytecodeGlobal {
                key,
                name: "LOCAL_ARRAY".into(),
                value_type: BytecodeType::Integer,
                dimensions: vec![4],
                mutable: true,
                storage,
                persistence: BytecodePersistence::None,
                initial_values: Vec::new(),
                owner: Some(owner),
            }],
        );
        let mut other_function = artifact.functions[0].clone();
        other_function.key = other;
        other_function.name = "OTHER_METHOD".into();
        other_function.code = vec![opcode::push_integer(0), opcode::return_value(true)];
        artifact.functions.push(other_function);
        fixture_runtime_variables(&mut artifact);
        artifact.refresh_ids().unwrap();
        let report = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &ValidationContext::for_artifact(&artifact),
        );
        if accepted {
            assert!(report.is_valid(), "{:#?}", report.diagnostics);
        } else {
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == ValidationCode::MissingReference),
                "{:#?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn rejects_control_flow_join_of_tokens_from_distinct_resolves() {
    let spec = method_spec(Vec::new());
    let codes = method_validation_codes(vec![
        opcode::push_integer(1),
        opcode::jump(Opcode::JumpIfFalse, 5),
        opcode::push_string("FIRST"),
        opcode::resolve_user_call(&spec),
        opcode::jump(Opcode::Jump, 7),
        opcode::push_string("SECOND"),
        opcode::resolve_user_call(&spec),
        erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new()),
        opcode::push_integer(0),
        opcode::return_value(true),
    ]);
    assert!(codes.contains(&ValidationCode::StackMismatch), "{codes:?}");
}

#[test]
fn rejects_method_fallback_and_invoke_result_type_mismatches() {
    let spec = erabasic_bytecode::UserCallSpec {
        allow_missing: true,
        missing_target: 4,
        ..method_spec(Vec::new())
    };
    let codes = method_validation_codes(vec![
        opcode::push_string("OPTIONAL"),
        opcode::resolve_user_call(&spec),
        opcode::invoke_user_call(1),
        opcode::jump(Opcode::Jump, 6),
        opcode::abandon_user_call(1),
        opcode::push_string("wrong result"),
        opcode::return_value(true),
    ]);
    assert!(codes.contains(&ValidationCode::StackMismatch), "{codes:?}");
    let string_spec = erabasic_bytecode::UserCallSpec {
        mode: erabasic_bytecode::UserCallMode::MethodString,
        ..method_spec(Vec::new())
    };
    let codes = method_validation_codes(vec![
        opcode::push_string("STRING_METHOD"),
        opcode::resolve_user_call(&string_spec),
        opcode::invoke_user_call(1),
        opcode::return_value(true),
    ]);
    assert!(codes.contains(&ValidationCode::TypeMismatch), "{codes:?}");
}

#[test]
fn rejects_malformed_method_payloads_and_unresolved_variable_shapes() {
    let encoded = method_spec(Vec::new()).encode();
    let mut payloads = vec![Vec::new(), encoded[..7].to_vec()];
    let mut trailing = encoded.clone();
    trailing.push(0);
    payloads.push(trailing);
    for (offset, tag) in [(0, 5), (1, 2)] {
        let mut payload = encoded.clone();
        payload[offset] = tag;
        payloads.push(payload);
    }
    for payload in payloads {
        let codes = method_validation_codes(vec![
            opcode::push_string("METHOD"),
            erabasic_bytecode::EncodedInstruction::new(Opcode::ResolveUserCall, payload),
            opcode::invoke_user_call(1),
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&ValidationCode::InvalidOperand), "{codes:?}");
    }
    let missing = method_spec(vec![erabasic_bytecode::UserArgumentSpec::Variable(
        SymbolKey([42; 16]),
    )]);
    let codes = method_validation_codes(vec![
        opcode::push_string("METHOD"),
        opcode::resolve_user_call(&missing),
        opcode::return_value(true),
    ]);
    assert!(
        codes.contains(&ValidationCode::MissingReference),
        "{codes:?}"
    );
}

#[test]
fn rejects_invalid_method_capture_flags_and_selection_operands() {
    let spec = method_spec(vec![erabasic_bytecode::UserArgumentSpec::Value(
        BytecodeType::Integer,
    )]);
    let mut bad_capture = opcode::capture_user_argument(1, 0, false).payload.to_vec();
    bad_capture[6] = 2;
    for instruction in [
        opcode::select_user_argument(1, 0, 4),
        opcode::capture_user_argument(1, 0, true),
        erabasic_bytecode::EncodedInstruction::new(Opcode::CaptureUserArgument, bad_capture),
        erabasic_bytecode::EncodedInstruction::new(Opcode::SelectUserArgument, vec![0; 9]),
        erabasic_bytecode::EncodedInstruction::new(Opcode::InvokeUserCall, vec![1; 3]),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("METHOD"),
            opcode::resolve_user_call(&spec),
            opcode::push_integer(1),
            instruction,
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&ValidationCode::InvalidOperand), "{codes:?}");
    }
    let spec = erabasic_bytecode::UserCallSpec {
        allow_missing: true,
        missing_target: u32::MAX,
        ..method_spec(Vec::new())
    };
    let codes = method_validation_codes(vec![
        opcode::push_string("METHOD"),
        opcode::resolve_user_call(&spec),
        opcode::invoke_user_call(1),
        opcode::return_value(true),
    ]);
    assert!(
        codes.contains(&ValidationCode::InvalidControlFlow),
        "{codes:?}"
    );
}

#[test]
fn user_argument_guard_capture_discard_and_omission_join_one_token() {
    use erabasic_bytecode::{UserArgumentAdvance, UserArgumentSpec};
    let spec = method_spec(vec![
        UserArgumentSpec::Value(BytecodeType::Integer),
        UserArgumentSpec::Omitted,
    ]);
    let codes = method_validation_codes(vec![
        opcode::push_string("TARGET"),
        opcode::resolve_user_call(&spec),
        opcode::guard_user_argument(1, 0, 6),
        opcode::push_integer(9),
        opcode::capture_user_argument(1, 0, false),
        opcode::jump(Opcode::Jump, 7),
        opcode::advance_user_argument(1, 0, UserArgumentAdvance::Discarded),
        opcode::advance_user_argument(1, 1, UserArgumentAdvance::Omitted),
        opcode::invoke_user_call(1),
        opcode::return_value(true),
    ]);
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn user_argument_slots_cannot_skip_repeat_or_merge_different_progress() {
    use erabasic_bytecode::{UserArgumentAdvance, UserArgumentSpec};
    let spec = method_spec(vec![UserArgumentSpec::Omitted, UserArgumentSpec::Omitted]);
    for advances in [vec![1], vec![0, 0], vec![0]] {
        let mut code = vec![
            opcode::push_string("TARGET"),
            opcode::resolve_user_call(&spec),
        ];
        code.extend(
            advances
                .into_iter()
                .map(|slot| opcode::advance_user_argument(1, slot, UserArgumentAdvance::Omitted)),
        );
        code.extend([opcode::invoke_user_call(1), opcode::return_value(true)]);
        let codes = method_validation_codes(code);
        assert!(codes.contains(&ValidationCode::StackMismatch), "{codes:?}");
    }
    let spec = method_spec(vec![UserArgumentSpec::Value(BytecodeType::Integer)]);
    let codes = method_validation_codes(vec![
        opcode::push_string("TARGET"),
        opcode::resolve_user_call(&spec),
        opcode::guard_user_argument(1, 0, 6),
        opcode::push_integer(9),
        opcode::capture_user_argument(1, 0, false),
        opcode::jump(Opcode::Jump, 7),
        erabasic_bytecode::EncodedInstruction::new(Opcode::Nop, Vec::new()),
        opcode::invoke_user_call(1),
        opcode::return_value(true),
    ]);
    assert!(codes.contains(&ValidationCode::StackMismatch), "{codes:?}");
}

#[test]
fn user_call_missing_branch_requires_matching_abandon_not_pop_or_other_origin() {
    let spec = erabasic_bytecode::UserCallSpec {
        allow_missing: true,
        missing_target: 4,
        ..method_spec(Vec::new())
    };
    for abandon in [
        opcode::abandon_user_call(0),
        opcode::abandon_user_call(2),
        erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new()),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("TARGET"),
            opcode::resolve_user_call(&spec),
            opcode::invoke_user_call(1),
            opcode::jump(Opcode::Jump, 6),
            abandon,
            opcode::push_integer(0),
            opcode::return_value(true),
        ]);
        assert!(
            codes.contains(&ValidationCode::InvalidControlFlow),
            "{codes:?}"
        );
    }
}

#[test]
fn user_call_procedure_discard_and_jump_modes_have_explicit_result_effects() {
    use erabasic_bytecode::UserCallMode;
    for mode in [
        UserCallMode::Procedure,
        UserCallMode::MethodDiscard,
        UserCallMode::JumpProcedure,
    ] {
        let spec = erabasic_bytecode::UserCallSpec {
            mode,
            ..method_spec(Vec::new())
        };
        let mut code = vec![
            opcode::push_string("TARGET"),
            opcode::resolve_user_call(&spec),
            opcode::invoke_user_call(1),
        ];
        if !mode.unwinds_caller() {
            code.extend([opcode::push_integer(0), opcode::return_value(true)]);
        }
        let mut artifact = method_artifact(code, BytecodeType::Integer, Vec::new());
        if mode.unwinds_caller() {
            artifact.functions[0].kind = erabasic_bytecode::BytecodeFunctionKind::Normal;
            artifact.functions[0].result = None;
            fixture_runtime_variables(&mut artifact);
            artifact.refresh_ids().unwrap();
        }
        let report = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &ValidationContext::for_artifact(&artifact),
        );
        assert!(report.is_valid(), "{mode:?}: {:?}", report.diagnostics);
    }
}

#[test]
fn call_text_has_equal_empty_successor_stacks_and_rejects_bad_wire_targets() {
    use erabasic_bytecode::{CallTextMode, CallTextSpec};
    let spec = CallTextSpec {
        mode: CallTextMode::CatchCall,
        catch_target: 4,
    };
    let codes = method_validation_codes(vec![
        opcode::push_string("TARGET(1)"),
        opcode::invoke_call_text(spec),
        opcode::push_integer(1),
        opcode::jump(Opcode::Jump, 5),
        opcode::push_integer(0),
        opcode::return_value(true),
    ]);
    assert!(codes.is_empty(), "{codes:?}");
    for payload in [
        vec![6, 0, 0, 0, 0],
        vec![0, 1, 0, 0, 0],
        vec![0; 4],
        vec![0; 6],
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string(""),
            erabasic_bytecode::EncodedInstruction::new(Opcode::InvokeCallText, payload),
            opcode::push_integer(0),
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&ValidationCode::InvalidOperand), "{codes:?}");
    }
    let codes = method_validation_codes(vec![
        opcode::push_string(""),
        opcode::invoke_call_text(CallTextSpec {
            mode: CallTextMode::CatchJump,
            catch_target: u32::MAX,
        }),
        opcode::push_integer(0),
        opcode::return_value(true),
    ]);
    assert!(
        codes.contains(&ValidationCode::InvalidControlFlow),
        "{codes:?}"
    );
}

#[test]
fn retired_eager_user_call_wire_is_unknown() {
    for old_opcode in [36, 37] {
        let mut instruction = opcode::push_integer(0);
        instruction.opcode = old_opcode;
        let codes = method_validation_codes(vec![instruction, opcode::return_value(true)]);
        assert!(codes.contains(&ValidationCode::UnknownOpcode), "{codes:?}");
    }
}

#[test]
fn rejects_guard_and_advance_invalid_slot_shape_flags_and_payload_lengths() {
    use erabasic_bytecode::{EncodedInstruction, UserArgumentAdvance, UserArgumentSpec};
    let omitted = method_spec(vec![UserArgumentSpec::Omitted]);
    let value = method_spec(vec![UserArgumentSpec::Value(BytecodeType::Integer)]);
    let mut invalid_reason =
        erabasic_bytecode::opcode::advance_user_argument(1, 0, UserArgumentAdvance::Omitted);
    let mut payload = invalid_reason.payload.to_vec();
    payload[6] = 2;
    invalid_reason.payload = payload.into();
    for (spec, instruction) in [
        (omitted.clone(), opcode::guard_user_argument(1, 0, 3)),
        (
            omitted.clone(),
            opcode::advance_user_argument(1, 0, UserArgumentAdvance::Discarded),
        ),
        (
            value.clone(),
            opcode::advance_user_argument(1, 0, UserArgumentAdvance::Omitted),
        ),
        (value, opcode::guard_user_argument(1, 1, 3)),
        (omitted.clone(), invalid_reason),
        (
            omitted.clone(),
            EncodedInstruction::new(Opcode::AdvanceUserArgument, vec![0; 8]),
        ),
        (
            omitted,
            EncodedInstruction::new(Opcode::GuardUserArgument, vec![0; 9]),
        ),
    ] {
        let codes = method_validation_codes(vec![
            opcode::push_string("TARGET"),
            opcode::resolve_user_call(&spec),
            instruction,
            opcode::invoke_user_call(1),
            opcode::return_value(true),
        ]);
        assert!(codes.contains(&ValidationCode::InvalidOperand), "{codes:?}");
    }
}
