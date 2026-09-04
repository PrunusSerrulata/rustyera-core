use super::*;
pub(super) fn map_owner(slot: usize) -> MapLeaseOwner {
    MapLeaseOwner {
        fiber: crate::FiberId(1),
        frame: crate::FrameId(1),
        generation: crate::GenerationId(1),
        function: SymbolKey::derive("test.map", b"owner"),
        origin: MapLeaseOrigin::Bytecode { begin: slot },
    }
}
pub(super) fn map_test_call(
    state: &mut StructuredState,
    name: &str,
    request: &NativeCallRequest,
) -> Result<NativeReady, ExecutionFailure> {
    let Some(operation) = MapOperation::from_name(name) else {
        return state.call(name, request);
    };
    let Some(lease) = state.capture_map(string_argument(request, 0)?, map_owner(1))? else {
        return Ok(NativeReady::value(VmValue::default_for(
            operation.result_type(),
        )));
    };
    let result = state.call_leased_map(
        operation,
        lease,
        request,
        &mut crate::compat_text::TextBudget::new(1_000_000, 1_000_000_000),
    );
    state.release_map_lease(lease)?;
    result
}

#[test]
fn map_capture_survives_release_recreate_and_snapshot_without_aliasing() {
    let mut state = map_with_entries(&[("a", "old")]);
    let first = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    state.retire_map_binding("m");
    state.maps.insert("m".into(), OrderedMap::default());
    let second = state.capture_map("m", map_owner(2)).unwrap().unwrap();
    state
        .leased_map_mut(second)
        .unwrap()
        .set("a".into(), "new".into());
    state
        .leased_map_mut(first)
        .unwrap()
        .set("b".into(), "detached".into());
    assert_eq!(state.maps["m"].entries, vec![("a".into(), "new".into())]);
    let decoded = StructuredState::decode(&state.encode().unwrap()).unwrap();
    assert_eq!(
        decoded.leased_map(first).unwrap().entries,
        vec![("a".into(), "old".into()), ("b".into(), "detached".into())]
    );
    decoded
        .validate_map_lease_owners(&[first, second].into_iter().collect())
        .unwrap();
    assert!(
        decoded
            .validate_map_lease_owners(&[second].into_iter().collect())
            .is_err()
    );
    state.release_map_lease(first).unwrap();
    assert!(state.leased_map(first).is_err());
    state.release_map_lease(second).unwrap();
    assert!(state.all_map_leases().is_empty());
    assert_eq!(state.maps["m"].entries, vec![("a".into(), "new".into())]);
}

#[test]
fn map_reachability_releases_only_abandoned_captures() {
    let mut state = map_with_entries(&[("a", "old")]);
    let outer = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    let inner = state.capture_map("m", map_owner(2)).unwrap().unwrap();
    state.retire_map_binding("m");
    state
        .retain_map_leases(&[outer].into_iter().collect())
        .unwrap();
    assert!(state.leased_map(inner).is_err());
    assert_eq!(state.leased_map(outer).unwrap().entries.len(), 1);
    state.retain_map_leases(&BTreeSet::new()).unwrap();
    assert!(state.all_map_leases().is_empty());
}

#[test]
fn map_reset_and_import_retire_captured_identity_without_aliasing() {
    let mut state = map_with_entries(&[("a", "old")]);
    let lease = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    let declarations = ExtensionData {
        save_maps: ["m".into()].into_iter().collect(),
        ..ExtensionData::default()
    };
    state
        .clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetNewGame,
        )
        .unwrap();
    assert!(state.maps["m"].entries.is_empty());
    assert_eq!(
        state.leased_map(lease).unwrap().entries,
        vec![("a".into(), "old".into())]
    );
    state
        .import_extensions(
            &declarations,
            StructuredScope::Ordinary,
            &[StructuredExtension::Map {
                key: "m".into(),
                entries: vec![("b".into(), "new".into())],
            }],
        )
        .unwrap();
    assert_eq!(state.maps["m"].entries, vec![("b".into(), "new".into())]);
    assert_eq!(
        state.leased_map(lease).unwrap().entries,
        vec![("a".into(), "old".into())]
    );
}

#[test]
fn map_revision_exhaustion_rejects_stamp_and_batch_reclaim_atomically() {
    let mut state = map_with_entries(&[("a", "old")]);
    let first = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    let second = state.capture_map("m", map_owner(2)).unwrap().unwrap();
    state.map_leases.revision = u64::MAX;
    assert!(state.map_lease_stamp().is_err());
    assert!(state.retain_map_leases(&BTreeSet::new()).is_err());
    assert_eq!(
        state.all_map_leases(),
        [first, second].into_iter().collect()
    );
    assert!(state.leased_map(first).is_ok());
    assert!(state.leased_map(second).is_ok());
}

mod icu72_raw_ce_candidates {
    use crate::compat_collation::{
        FixedIcu72Root,
        ce::{CeError, CeLimits},
        raw_off::{RawRootData, raw_off_elements},
    };
    use zerovec::{ZeroSlice, ZeroVec};

    #[derive(Clone, Copy)]
    enum Mapping {
        Plain,
        Expansion,
        Contraction,
        Discontiguous,
    }
    struct Data {
        mapping: Mapping,
        contexts: ZeroVec<'static, u16>,
    }
    impl Data {
        fn new(mapping: Mapping) -> Self {
            // UCharsTrie one-unit linear match ('b'), final 32-bit CE32.
            // Layout matches ICU72 root_standard_data context examples:
            // default high/low, 0x30, char, 0xffff, result high/low.
            let suffix = if matches!(mapping, Mapping::Discontiguous) {
                0x308
            } else {
                0x62
            };
            Self {
                mapping,
                contexts: ZeroVec::alloc_from_slice(&[
                    0x2a00, 0x0505, 0x30, suffix, 0xffff, 0x2c00, 0x0505,
                ]),
            }
        }
    }
    impl RawRootData for Data {
        fn ce32(&self, cp: u32) -> Result<u32, CeError> {
            Ok(match cp {
                0 => 0,
                0x61 if matches!(self.mapping, Mapping::Expansion) => 0x02c6,
                0x61 if matches!(self.mapping, Mapping::Discontiguous) => 0x06c9,
                0x61 if matches!(self.mapping, Mapping::Contraction) => 0x00c9,
                0x61 => 0x2a00_0505,
                0x62 => 0x2c00_0505,
                0x6f => 0x4600_0505,
                0x308 => 0x0000_9605,
                0x301 => 0x0000_8805,
                0x316 => 0x0000_8a05,
                _ => 0xffff_ffff,
            })
        }
        fn ce32_at(&self, _: usize) -> Result<u32, CeError> {
            Err(CeError::MalformedProvider)
        }
        fn ce_at(&self, index: usize) -> Result<u64, CeError> {
            [0x2a00_0000_0500_0500, 0x2c00_0000_0500_0500]
                .get(index)
                .copied()
                .ok_or(CeError::MalformedProvider)
        }
        fn contexts(&self) -> &ZeroSlice<u16> {
            &self.contexts
        }
        fn jamo_ce32_at(&self, _: usize) -> Result<u32, CeError> {
            Err(CeError::MalformedProvider)
        }
        fn fcd16(&self, cp: u32) -> Result<u16, CeError> {
            Ok(match cp {
                0x308 | 0x301 => 0xe6e6,
                0x316 => 0xdcdc,
                _ => 0,
            })
        }
    }
    fn limits() -> CeLimits {
        CeLimits {
            utf16_units: 64,
            ce64: 128,
            context_depth: 64,
        }
    }
    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn raw_ce_expansion_and_legacy_continuation_keep_native_forward_offsets() {
        let expansion =
            raw_off_elements(&Data::new(Mapping::Expansion), &units("a"), limits()).unwrap();
        assert_eq!(
            expansion
                .elements
                .iter()
                .map(|e| (e.forward_low, e.forward_high))
                .collect::<Vec<_>>(),
            [(0, 1), (1, 1)]
        );
        let implicit =
            raw_off_elements(&Data::new(Mapping::Plain), &units("😀"), limits()).unwrap();
        let legacy = implicit.legacy_elements().unwrap();
        assert_eq!(legacy.len(), 2);
        assert_eq!((legacy[0].forward_low, legacy[0].forward_high), (0, 2));
        assert_eq!((legacy[1].forward_low, legacy[1].forward_high), (2, 2));
        assert!(legacy[1].continuation_half);
    }

    #[test]
    fn raw_ce_lookahead_consumes_work_even_before_any_output() {
        use crate::compat_collation::{ce::TextBudget, raw_off::raw_off_elements_bounded};
        let data = Data::new(Mapping::Contraction);
        let mut budget = TextBudget::new(3, 100_000);
        let failure =
            raw_off_elements_bounded(&data, &units("ab"), limits(), &mut budget).unwrap_err();
        assert_eq!(failure, CeError::WorkLimit);
        assert_eq!(budget.remaining_work(), 0);
    }

    #[test]
    fn raw_ce_contraction_commits_only_longest_matching_suffix() {
        let data = Data::new(Mapping::Contraction);
        let matched = raw_off_elements(&data, &units("abx"), limits()).unwrap();
        assert_eq!(matched.elements[0].value, 0x2c00_0000_0500_0500);
        assert_eq!(
            (
                matched.elements[0].forward_low,
                matched.elements[0].forward_high
            ),
            (0, 2)
        );
        assert_eq!(matched.elements[1].forward_low, 2);
        let failed = raw_off_elements(&data, &units("ax"), limits()).unwrap();
        assert_eq!(failed.elements[0].value, 0x2a00_0000_0500_0500);
        assert_eq!(
            (
                failed.elements[0].forward_low,
                failed.elements[0].forward_high
            ),
            (0, 1)
        );
    }

    #[test]
    fn simple_affix_keeps_zero_ce_and_prefix_only_combining_guard() {
        let root = FixedIcu72Root::from_validated_data(Data::new(Mapping::Plain));
        assert!(
            !root
                .starts_with_utf16(&units("o\u{308}"), &units("o"), limits())
                .unwrap()
        );
        assert!(
            root.starts_with_utf16(&units("o\0\u{308}"), &units("o"), limits())
                .unwrap()
        );
        assert!(
            root.ends_with_utf16(&units("o\u{308}"), &units("\u{308}"), limits())
                .unwrap()
        );
        assert!(
            !root
                .starts_with_utf16(&units("\u{308}"), &[0], limits())
                .unwrap()
        );
        assert!(
            root.starts_with_utf16(&units("\u{308}"), &[], limits())
                .unwrap()
        );
    }

    #[test]
    fn raw_ce_lone_surrogates_do_not_become_replacement_character() {
        let data = Data::new(Mapping::Plain);
        let lead = raw_off_elements(&data, &[0xd800], limits()).unwrap();
        let trail = raw_off_elements(&data, &[0xdc00], limits()).unwrap();
        let replacement = raw_off_elements(&data, &[0xfffd], limits()).unwrap();
        assert_ne!(lead.elements[0].value, trail.elements[0].value);
        assert_ne!(lead.elements[0].value, replacement.elements[0].value);
    }
    #[test]
    fn raw_ce_discontiguous_match_buffers_skipped_marks_and_respects_blocking() {
        let data = Data::new(Mapping::Discontiguous);
        let allowed = raw_off_elements(&data, &units("a\u{316}\u{308}"), limits()).unwrap();
        assert_eq!(
            allowed.elements.iter().map(|e| e.value).collect::<Vec<_>>(),
            [0x2c00_0000_0500_0500, 0x0000_0000_8a00_0500]
        );
        assert_eq!(
            allowed
                .elements
                .iter()
                .map(|e| (e.forward_low, e.forward_high))
                .collect::<Vec<_>>(),
            [(0, 3), (3, 3)]
        );
        let blocked = raw_off_elements(&data, &units("a\u{301}\u{308}"), limits()).unwrap();
        assert_eq!(blocked.elements[0].value, 0x2a00_0000_0500_0500);
        assert_eq!(blocked.elements[0].forward_high, 1);
        assert_eq!(blocked.elements.len(), 3);
    }
}

// These assertions are fixed-source-derived candidates, not captured oracle goldens.
