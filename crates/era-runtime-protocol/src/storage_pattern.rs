//! Portable, bounded filename patterns for snake Data and Resource enumeration.
//!
//! The reference delegates to platform-dependent `Directory.EnumerateFiles`. This
//! contract deliberately uses NFC and Unicode lowercase, scalar `?`, and only `*`
//! and `?` metacharacters. Brackets are literal. An absent or empty pattern does
//! not filter. Original-profile hosts retain their existing matching policies.

use unicode_normalization::UnicodeNormalization;

const MAXIMUM_BYTES: usize = 4096;
const MAXIMUM_STEPS: usize = 1_048_576;

/// A malformed, oversized, or computationally excessive storage pattern input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoragePatternError {
    InvalidInput,
    WorkLimit,
}

impl std::fmt::Display for StoragePatternError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput => "storage pattern/name contains NUL or exceeds 4096 UTF-8 bytes",
            Self::WorkLimit => "storage pattern matching exceeds its operation limit",
        })
    }
}

impl std::error::Error for StoragePatternError {}

fn normalized(value: &str) -> Result<Vec<char>, StoragePatternError> {
    if value.len() > MAXIMUM_BYTES || value.contains('\0') {
        return Err(StoragePatternError::InvalidInput);
    }
    let value = value.nfc().collect::<String>().to_lowercase();
    if value.len() > MAXIMUM_BYTES {
        return Err(StoragePatternError::InvalidInput);
    }
    Ok(value.chars().collect())
}

/// Validate a pattern even when the selected directory contains no entries.
///
/// # Errors
/// Returns `InvalidInput` for NUL or an input exceeding the UTF-8 size limit before
/// or after normalization.
pub fn validate_snake_storage_pattern(pattern: Option<&str>) -> Result<(), StoragePatternError> {
    normalized(pattern.unwrap_or_default()).map(|_| ())
}

/// Match a basename, never a relative path. Hosts map any error to `InvalidData`.
///
/// # Errors
/// Returns `InvalidInput` for NUL or oversized input and `WorkLimit` when matching
/// exceeds the bounded number of greedy steps.
pub fn matches_snake_storage_pattern(
    pattern: Option<&str>,
    name: &str,
) -> Result<bool, StoragePatternError> {
    let pattern = normalized(pattern.unwrap_or_default())?;
    let name = normalized(name)?;
    if pattern.is_empty() {
        return Ok(true);
    }
    let (mut p, mut n, mut star, mut retry) = (0, 0, None, 0);
    let mut steps = 0;
    // Retain only the most recent star; unlike a regex this never builds an
    // exponential backtracking tree. The explicit budget also bounds quadratic
    // retries of long literal suffixes. Input normalization is separately bounded.
    while n < name.len() {
        steps += 1;
        if steps > MAXIMUM_STEPS {
            return Err(StoragePatternError::WorkLimit);
        }
        if p < pattern.len() && pattern[p] != '*' && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            p += 1;
            retry = n;
        } else if let Some(index) = star {
            retry += 1;
            n = retry;
            p = index + 1;
        } else {
            return Ok(false);
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        steps += 1;
        if steps > MAXIMUM_STEPS {
            return Err(StoragePatternError::WorkLimit);
        }
        p += 1;
    }
    Ok(p == pattern.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_storage_patterns_follow_shared_vectors() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tools/runtime-tester/fixtures/snake-storage-patterns.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let result = matches_snake_storage_pattern(
                case["pattern"].as_str(),
                case["name"].as_str().unwrap(),
            );
            if let Some(expected) = case["expected"].as_bool() {
                assert_eq!(result, Ok(expected), "{}", case["id"]);
            } else {
                assert!(result.is_err(), "{}", case["id"]);
            }
        }
    }
}
