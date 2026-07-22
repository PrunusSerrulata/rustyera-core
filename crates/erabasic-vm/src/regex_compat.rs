use std::collections::{HashMap, VecDeque};

use regex::Regex;

const MAXIMUM_CACHED_PATTERNS: usize = 128;

#[derive(Clone, Default)]
pub(crate) struct RegexCache {
    entries: HashMap<String, Result<Regex, String>>,
    insertion_order: VecDeque<String>,
}

impl RegexCache {
    pub(crate) fn get_or_compile(&mut self, pattern: &str) -> Result<Regex, String> {
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

/// Compile the deliberately small intersection between .NET and Rust regex syntax.
///
/// The pinned runtime uses `System.Text.RegularExpressions`. Accepting syntax that Rust's
/// engine interprets differently would be worse than a stable runtime error, so constructs
/// with backtracking-dependent semantics are rejected before compilation. Named captures use
/// the only common spelling that needs a mechanical translation.
pub(crate) fn compile(pattern: &str) -> Result<Regex, String> {
    reject_unsupported(pattern)?;
    let translated = translate_named_captures(pattern)?;
    Regex::new(&translated).map_err(|error| format!("unsupported or invalid regex: {error}"))
}

fn reject_unsupported(pattern: &str) -> Result<(), String> {
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
                    return Err(
                        ".NET backreferences are not supported by the common regex subset".into(),
                    );
                }
                index = index.saturating_add(2);
                continue;
            }
            b'[' => in_class = true,
            b']' => in_class = false,
            b'(' if !in_class && bytes.get(index + 1) == Some(&b'?') => {
                let suffix = &pattern[index..];
                if suffix.starts_with("(?=")
                    || suffix.starts_with("(?!")
                    || suffix.starts_with("(?<=")
                    || suffix.starts_with("(?<!")
                    || suffix.starts_with("(?>")
                    || suffix.starts_with("(?(")
                    || suffix.starts_with("(?'")
                {
                    return Err(".NET lookaround, atomic, conditional, and quoted-group constructs are not supported by the common regex subset".into());
                }
            }
            _ => {}
        }
        index += 1;
    }
    Ok(())
}

fn translate_named_captures(pattern: &str) -> Result<String, String> {
    let bytes = pattern.as_bytes();
    let mut result = String::with_capacity(pattern.len());
    let mut index = 0;
    let mut in_class = false;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let end = (index + 2).min(bytes.len());
            result.push_str(&pattern[index..end]);
            index = end;
            continue;
        }
        if bytes[index] == b'[' {
            in_class = true;
        } else if bytes[index] == b']' {
            in_class = false;
        }
        if !in_class && pattern[index..].starts_with("(?<") {
            let name_start = index + 3;
            let Some(relative_end) = pattern[name_start..].find('>') else {
                return Err("unterminated .NET named capture".into());
            };
            let name_end = name_start + relative_end;
            let name = &pattern[name_start..name_end];
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|value| value.is_ascii_alphanumeric() || value == b'_')
            {
                return Err("named capture contains unsupported characters".into());
            }
            result.push_str("(?P<");
            result.push_str(name);
            result.push('>');
            index = name_end + 1;
            continue;
        }
        let character = pattern[index..]
            .chars()
            .next()
            .expect("index remains at a character boundary");
        result.push(character);
        index += character.len_utf8();
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_dotnet_named_groups() {
        let regex = compile(r"(?<word>a+)").unwrap();
        assert_eq!(&regex.captures("aaa").unwrap()["word"], "aaa");
    }

    #[test]
    fn rejects_backtracking_only_constructs() {
        assert!(compile(r"(a)\1").is_err());
        assert!(compile(r"a(?=b)").is_err());
    }

    #[test]
    fn cache_reuses_successes_and_errors_with_a_fixed_bound() {
        let mut cache = RegexCache::default();
        assert!(cache.get_or_compile("a+").unwrap().is_match("aaa"));
        assert!(cache.get_or_compile("a+").unwrap().is_match("aaa"));
        assert_eq!(cache.entries.len(), 1);

        assert!(cache.get_or_compile("a(?=b)").is_err());
        assert!(cache.get_or_compile("a(?=b)").is_err());
        assert_eq!(cache.entries.len(), 2);

        for index in 0..=MAXIMUM_CACHED_PATTERNS {
            cache.get_or_compile(&format!("pattern-{index}")).unwrap();
        }
        assert_eq!(cache.entries.len(), MAXIMUM_CACHED_PATTERNS);
        assert!(!cache.entries.contains_key("a+"));
    }
}
