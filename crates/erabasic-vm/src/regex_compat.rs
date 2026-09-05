use std::collections::{HashMap, VecDeque};

use fancy_regex::{CompileError, Error, ParseError, Regex, RegexBuilder, RuntimeError};

const MAXIMUM_CACHED_PATTERNS: usize = 128;
const BACKTRACK_LIMIT: usize = 1_000_000;

#[derive(Clone, Default)]
pub(crate) struct RegexCache {
    entries: HashMap<String, Result<Regex, crate::ExecutionFailure>>,
    insertion_order: VecDeque<String>,
}

impl RegexCache {
    pub(crate) fn get_or_compile(
        &mut self,
        pattern: &str,
    ) -> Result<Regex, crate::ExecutionFailure> {
        if let Some(cached) = self.entries.get(pattern) {
            return cached.clone();
        }
        let compiled = compile(pattern);
        if self.entries.len() == MAXIMUM_CACHED_PATTERNS
            && let Some(oldest) = self.insertion_order.pop_front()
        {
            self.entries.remove(&oldest);
        }
        self.insertion_order.push_back(pattern.to_owned());
        self.entries.insert(pattern.to_owned(), compiled.clone());
        compiled
    }
}

/// Compile the supported intersection between .NET and Rust regex syntax.
///
/// `fancy-regex` supplies the four zero-width look-around forms used by the reference runtime.
/// An explicit backtracking limit keeps hostile or accidental exponential expressions bounded.
pub(crate) fn compile(pattern: &str) -> Result<Regex, crate::ExecutionFailure> {
    build(pattern).map_err(|error| {
        let message = format!("unsupported or invalid regex: {error}");
        vm_failure(&error, message)
    })
}

pub(crate) fn build(pattern: &str) -> Result<Regex, Error> {
    reject_unsupported(pattern)?;
    RegexBuilder::new(pattern)
        .backtrack_limit(BACKTRACK_LIMIT)
        .build()
}

#[derive(Clone, Copy)]
enum ErrorClass {
    Script,
    ResourceLimit,
    InternalInvariant,
}

fn error_class(error: &Error) -> ErrorClass {
    match error {
        Error::ParseError(_, _) => ErrorClass::Script,
        Error::CompileError(error) => match error.as_ref() {
            CompileError::InnerError(error) if error.size_limit().is_some() => {
                ErrorClass::ResourceLimit
            }
            CompileError::InnerError(error) if error.syntax_error().is_some() => ErrorClass::Script,
            CompileError::LookBehindNotConst
            | CompileError::VariableLookBehindRequiresFeature
            | CompileError::InvalidGroupName
            | CompileError::InvalidGroupNameBackref(_)
            | CompileError::InvalidBackref(_)
            | CompileError::NamedBackrefOnly
            | CompileError::FeatureNotYetSupported(_)
            | CompileError::SubroutineCallTargetNotFound(_, _)
            | CompileError::LeftRecursiveSubroutineCall(_)
            | CompileError::NeverEndingRecursion => ErrorClass::Script,
            CompileError::InnerError(_)
            | CompileError::DfaBuildError(_, _)
            | CompileError::UnexpectedGeneralError(_)
            | CompileError::PatternCanNeverMatch
            | CompileError::UnresolvedAstNode(_, _)
            | _ => ErrorClass::InternalInvariant,
        },
        Error::RuntimeError(RuntimeError::StackOverflow | RuntimeError::BacktrackLimitExceeded) => {
            ErrorClass::ResourceLimit
        }
        Error::RuntimeError(_) | _ => ErrorClass::InternalInvariant,
    }
}

fn vm_failure(error: &Error, message: String) -> crate::ExecutionFailure {
    let (category, code) = match error_class(error) {
        ErrorClass::Script => (
            crate::FaultCategory::Script(crate::ScriptFaultKind::Parse),
            crate::VmFaultCode::TypeMismatch,
        ),
        ErrorClass::ResourceLimit => (
            crate::FaultCategory::ResourceLimit,
            crate::VmFaultCode::ResourceLimit,
        ),
        ErrorClass::InternalInvariant => (
            crate::FaultCategory::InternalInvariant,
            crate::VmFaultCode::Native,
        ),
    };
    crate::ExecutionFailure::classified(category, code, message)
}

pub(crate) fn core_failure(error: &Error, message: String) -> crate::ExecutionFailure {
    let category = match error_class(error) {
        ErrorClass::Script => crate::FaultCategory::Script(crate::ScriptFaultKind::Parse),
        ErrorClass::ResourceLimit => crate::FaultCategory::ResourceLimit,
        ErrorClass::InternalInvariant => crate::FaultCategory::InternalInvariant,
    };
    crate::ExecutionFailure::classified(category, crate::VmFaultCode::Native, message)
}

pub(crate) fn runtime_error(error: &Error) -> crate::ExecutionFailure {
    vm_failure(error, format!("regex execution failed: {error}"))
}

/// Match the portable subset `(<one character atom>)\1{N}`.
///
/// Rust's linear-time regex engine deliberately omits backreferences. Era games
/// nevertheless use this bounded shape to detect long separator lines. Handling
/// it directly retains deterministic linear behavior without enabling arbitrary
/// backtracking.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RepeatedCharacterMatch {
    Unsupported,
    NoMatch,
    Match(std::ops::Range<usize>),
}

pub(crate) fn find_repeated_character(pattern: &str, input: &str) -> RepeatedCharacterMatch {
    let Some(body) = pattern.strip_prefix('(') else {
        return RepeatedCharacterMatch::Unsupported;
    };
    let Some((atom, repetition)) = body.split_once(r")\1{") else {
        return RepeatedCharacterMatch::Unsupported;
    };
    let Some(repetition) = repetition.strip_suffix('}') else {
        return RepeatedCharacterMatch::Unsupported;
    };
    let Ok(repetitions) = repetition.parse::<usize>() else {
        return RepeatedCharacterMatch::Unsupported;
    };
    let Some(required) = repetitions.checked_add(1) else {
        return RepeatedCharacterMatch::Unsupported;
    };
    let Some(predicate) = CharacterPredicate::parse(atom) else {
        return RepeatedCharacterMatch::Unsupported;
    };

    let mut characters = input.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        // System.Text.RegularExpressions operates on UTF-16 code units. A
        // supplementary scalar therefore cannot repeat immediately in this
        // one-code-unit capture shape because its low surrogate intervenes.
        if character.len_utf16() != 1 || !predicate.matches(character) {
            continue;
        }
        let mut count = 1usize;
        let mut end = start + character.len_utf8();
        while count < required
            && let Some(&(next, candidate)) = characters.peek()
            && candidate == character
        {
            characters.next();
            count += 1;
            end = next + candidate.len_utf8();
        }
        if count == required {
            return RepeatedCharacterMatch::Match(start..end);
        }
    }
    RepeatedCharacterMatch::NoMatch
}

enum CharacterPredicate {
    AnyNonNewline,
    Equal(char),
    NotEqual(char),
}

impl CharacterPredicate {
    fn parse(atom: &str) -> Option<Self> {
        if atom == "." {
            return Some(Self::AnyNonNewline);
        }
        if let Some(value) = atom
            .strip_prefix("[^")
            .and_then(|value| value.strip_suffix(']'))
            .and_then(single_character)
        {
            return Some(Self::NotEqual(value));
        }
        atom.strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .and_then(single_character)
            .map(Self::Equal)
    }

    fn matches(&self, character: char) -> bool {
        match self {
            Self::AnyNonNewline => character != '\n',
            Self::Equal(expected) => character == *expected,
            Self::NotEqual(excluded) => character != *excluded,
        }
    }
}

fn single_character(source: &str) -> Option<char> {
    let mut characters = source.chars();
    let character = characters.next()?;
    (character.len_utf16() == 1 && characters.next().is_none()).then_some(character)
}

fn reject_unsupported(pattern: &str) -> Result<(), Error> {
    let bytes = pattern.as_bytes();
    let mut index = 0;
    let mut in_class = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                let escaped = bytes.get(index + 1).copied();
                if escaped.is_some_and(|value| value.is_ascii_digit() && value != b'0')
                    || matches!(escaped, Some(b'k' | b'K'))
                {
                    return Err(unsupported_error(
                        ".NET backreferences are not supported by the common regex subset",
                    ));
                }
                index = index.saturating_add(2);
                continue;
            }
            b'[' => in_class = true,
            b']' => in_class = false,
            b'(' if !in_class && bytes.get(index + 1) == Some(&b'?') => {
                let suffix = &pattern[index..];
                if suffix.starts_with("(?>")
                    || suffix.starts_with("(?(")
                    || suffix.starts_with("(?'")
                {
                    return Err(unsupported_error(
                        ".NET atomic, conditional, and quoted-group constructs are not supported by the common regex subset",
                    ));
                }
            }
            _ => {}
        }
        index += 1;
    }
    Ok(())
}

fn unsupported_error(message: &str) -> Error {
    Error::ParseError(0, ParseError::GeneralParseError(message.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_failure_classification_survives_cache() {
        let mut cache = RegexCache::default();
        let invalid = cache.get_or_compile("[").unwrap_err();
        assert_eq!(
            invalid.category,
            crate::FaultCategory::Script(crate::ScriptFaultKind::Parse)
        );
        assert_eq!(invalid.code, crate::VmFaultCode::TypeMismatch);
        assert!(invalid.message.starts_with("unsupported or invalid regex:"));
        assert_eq!(cache.get_or_compile("[").unwrap_err(), invalid);
        assert!(compile(r"\q").unwrap_err().is_script());
        let too_large = cache.get_or_compile("a{1000000000}").unwrap_err();
        assert_eq!(too_large.category, crate::FaultCategory::ResourceLimit);
        assert_eq!(too_large.code, crate::VmFaultCode::ResourceLimit);
        assert_eq!(
            cache.get_or_compile("a{1000000000}").unwrap_err(),
            too_large
        );
    }

    #[test]
    fn runtime_limits_keep_the_vm_boundary_contract() {
        let error = Error::RuntimeError(RuntimeError::BacktrackLimitExceeded);
        let failure = runtime_error(&error);
        assert_eq!(failure.category, crate::FaultCategory::ResourceLimit);
        assert_eq!(failure.code, crate::VmFaultCode::ResourceLimit);
        assert!(failure.message.starts_with("regex execution failed:"));
    }

    #[test]
    fn accepts_dotnet_named_groups() {
        let regex = compile(r"(?<word>a+)").unwrap();
        assert_eq!(&regex.captures("aaa").unwrap().unwrap()["word"], "aaa");
    }

    #[test]
    fn rejects_unsupported_backreferences_but_accepts_all_lookarounds() {
        assert!(compile(r"(a)\1").is_err());
        for (pattern, input, expected_start, expected) in [
            (r"foo(?=bar)", "foobar fooqux", 0, "foo"),
            (r"foo(?!bar)", "foobar fooqux", 7, "foo"),
            (r"(?<=USD)\d+", "USD10 EUR20", 3, "10"),
            (r"(?<!AU)\$\d+", "AU$10, $20", 7, "$20"),
        ] {
            let regex = compile(pattern).unwrap();
            let matched = regex.find(input).unwrap().unwrap();
            assert_eq!(matched.start(), expected_start);
            assert_eq!(matched.as_str(), expected);
        }
    }

    #[test]
    fn captures_adjacent_values_between_positive_boundaries() {
        let regex = compile(r"(?<=\[\$TOKEN:).*?(?=\])").unwrap();
        let captures = regex
            .captures_iter("[$TOKEN:A][$TOKEN:B]")
            .map(|captures| captures.unwrap()[0].to_owned())
            .collect::<Vec<_>>();
        assert_eq!(captures, vec!["A".to_owned(), "B".to_owned()]);
    }

    #[test]
    fn finds_bounded_repeated_character_backreferences_without_backtracking() {
        let pattern = r"([^ ])\1{15}";
        assert_eq!(
            find_repeated_character(pattern, "<p>----------------</p>"),
            RepeatedCharacterMatch::Match(3..19)
        );
        assert_eq!(
            find_repeated_character(pattern, "---------------"),
            RepeatedCharacterMatch::NoMatch
        );
        assert_eq!(
            find_repeated_character(pattern, "                "),
            RepeatedCharacterMatch::NoMatch
        );
        assert_eq!(
            find_repeated_character(pattern, "😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀"),
            RepeatedCharacterMatch::NoMatch
        );
        assert_eq!(
            find_repeated_character(r"(ab)\1", "abab"),
            RepeatedCharacterMatch::Unsupported
        );
    }

    #[test]
    fn cache_reuses_successes_and_errors_with_a_fixed_bound() {
        let mut cache = RegexCache::default();
        assert!(cache.get_or_compile("a+").unwrap().is_match("aaa").unwrap());
        assert!(cache.get_or_compile("a+").unwrap().is_match("aaa").unwrap());
        assert_eq!(cache.entries.len(), 1);

        assert!(cache.get_or_compile("(a)\\1").is_err());
        assert!(cache.get_or_compile("(a)\\1").is_err());
        assert_eq!(cache.entries.len(), 2);

        for index in 0..=MAXIMUM_CACHED_PATTERNS {
            cache.get_or_compile(&format!("pattern-{index}")).unwrap();
        }
        assert_eq!(cache.entries.len(), MAXIMUM_CACHED_PATTERNS);
        assert!(!cache.entries.contains_key("a+"));
    }
}
