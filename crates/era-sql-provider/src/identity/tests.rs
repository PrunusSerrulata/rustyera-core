use super::*;

fn options() -> BTreeSet<String> {
    REQUIRED_OPTIONS
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
}

#[test]
fn fixed_contract_accepts_only_matching_engine() {
    assert!(require("3.53.4", SOURCE_ID, &options()).is_ok());
    for version in ["3.53.3", "3.53.5", "3.51.1"] {
        assert!(require(version, SOURCE_ID, &options()).is_err());
    }
    assert!(require("3.53.4", "different source", &options()).is_err());
}

#[test]
fn every_required_option_is_enforced() {
    for option in REQUIRED_OPTIONS {
        let mut actual = options();
        actual.remove(*option);
        assert!(require("3.53.4", SOURCE_ID, &actual).is_err(), "{option}");
    }
}

#[test]
fn incompatible_dependency_engine_options_are_rejected() {
    for option in ["OMIT_DESERIALIZE", "DEFAULT_FOREIGN_KEYS", "HAS_CODEC"] {
        let mut actual = options();
        actual.insert(option.to_owned());
        assert!(require("3.53.4", SOURCE_ID, &actual).is_err());
    }
}

#[test]
fn actual_linked_engine_matches_contract() {
    verify().expect("linked engine must satisfy the contract even when dependency features merge");
}
