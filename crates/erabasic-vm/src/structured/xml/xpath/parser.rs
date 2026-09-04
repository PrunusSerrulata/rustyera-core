//! Parser and UTF-8-safe scanner for the supported `XPath` subset.

use super::{
    XPathAxis, XPathComparison, XPathExpression, XPathPath, XPathPredicate, XPathStep, XPathTest,
};
use crate::ExecutionFailure;
use crate::structured::parse_failure;

pub(super) fn parse_xpath(value: &str) -> Result<XPathExpression, ExecutionFailure> {
    let parts = split_xpath_top_level(value, '|')?;
    let paths = parts
        .into_iter()
        .map(|part| parse_xpath_path(part.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Err(parse_failure(
            "native.xpath.unsupported: empty XPath expression",
        ));
    }
    Ok(XPathExpression { paths })
}

fn parse_xpath_path(value: &str) -> Result<XPathPath, ExecutionFailure> {
    let value = value.trim();
    if value.is_empty() {
        return Err(parse_failure(
            "native.xpath.unsupported: empty location path",
        ));
    }
    if value == "." {
        return Ok(XPathPath {
            absolute: false,
            steps: Vec::new(),
        });
    }

    let mut cursor = 0;
    let mut absolute = false;
    let mut next_axis = XPathAxis::Child;
    if value.starts_with("//") {
        absolute = true;
        cursor = 2;
        next_axis = XPathAxis::Descendant;
    } else if value.starts_with('/') {
        absolute = true;
        cursor = 1;
    } else if value.starts_with(".//") {
        cursor = 3;
        next_axis = XPathAxis::Descendant;
    } else if value.starts_with("./") {
        cursor = 2;
    }

    let mut steps = Vec::new();
    while cursor < value.len() {
        let start = cursor;
        let mut bracket_depth = 0usize;
        let mut quote = None;
        while cursor < value.len() {
            let ch = value[cursor..]
                .chars()
                .next()
                .ok_or("native.xpath.unsupported: malformed UTF-8 cursor")?;
            if let Some(delimiter) = quote {
                if ch == delimiter {
                    quote = None;
                }
            } else {
                match ch {
                    '\'' | '"' => quote = Some(ch),
                    '[' => bracket_depth += 1,
                    ']' => {
                        bracket_depth = bracket_depth.checked_sub(1).ok_or_else(|| {
                            parse_failure("native.xpath.unsupported: malformed predicate")
                        })?;
                    }
                    '/' if bracket_depth == 0 => break,
                    _ => {}
                }
            }
            cursor += ch.len_utf8();
        }
        if quote.is_some() || bracket_depth != 0 {
            return Err(parse_failure(
                "native.xpath.unsupported: malformed predicate",
            ));
        }
        let part = value[start..cursor].trim();
        if part.is_empty() {
            return Err(parse_failure(
                "native.xpath.unsupported: empty location step",
            ));
        }
        steps.push(parse_xpath_step(part, next_axis)?);
        if cursor == value.len() {
            break;
        }
        if value[cursor..].starts_with("//") {
            next_axis = XPathAxis::Descendant;
            cursor += 2;
        } else {
            next_axis = XPathAxis::Child;
            cursor += 1;
        }
    }
    if steps.is_empty() {
        return Err(parse_failure(
            "native.xpath.unsupported: empty location path",
        ));
    }
    Ok(XPathPath { absolute, steps })
}

fn parse_xpath_step(value: &str, mut axis: XPathAxis) -> Result<XPathStep, ExecutionFailure> {
    let first_predicate = find_xpath_top_level(value, '[')?;
    let (test, mut rest) =
        first_predicate.map_or((value, ""), |index| (&value[..index], &value[index..]));
    let test = test.trim();
    if let Some(name) = test.strip_prefix("descendant::") {
        if !matches!(axis, XPathAxis::Child) {
            return Err(parse_failure(
                "native.xpath.unsupported: combined explicit axes are unsupported",
            ));
        }
        axis = XPathAxis::Descendant;
        ensure_xpath_name(name)?;
    } else if test.contains("::") || test.contains(':') {
        return Err(parse_failure(
            "native.xpath.unsupported: namespace and axis expressions are unsupported",
        ));
    }
    let test = if test == "text()" {
        XPathTest::Text
    } else if let Some(name) = test.strip_prefix('@') {
        ensure_xpath_name(name)?;
        axis = match axis {
            XPathAxis::Descendant => XPathAxis::DescendantOrSelfAttribute,
            _ => XPathAxis::Attribute,
        };
        XPathTest::Attribute(name.to_owned())
    } else {
        let name = test.strip_prefix("descendant::").unwrap_or(test);
        ensure_xpath_name(name)?;
        XPathTest::Element(name.to_owned())
    };

    let mut predicates = Vec::new();
    while !rest.is_empty() {
        if !rest.starts_with('[') {
            return Err(parse_failure(
                "native.xpath.unsupported: malformed predicate",
            ));
        }
        let close = matching_xpath_delimiter(rest, 0, '[', ']')?;
        predicates.push(parse_xpath_predicate(&rest[1..close])?);
        rest = rest[close + 1..].trim_start();
    }
    Ok(XPathStep {
        axis,
        test,
        predicates,
    })
}

fn parse_xpath_predicate(value: &str) -> Result<XPathPredicate, ExecutionFailure> {
    let value = strip_xpath_parentheses(value.trim())?;
    if value.is_empty() {
        return Err(parse_failure("native.xpath.unsupported: empty predicate"));
    }
    let parts = split_xpath_keyword(value, "or")?;
    if parts.len() > 1 {
        return Ok(XPathPredicate::Or(
            parts
                .into_iter()
                .map(parse_xpath_predicate)
                .collect::<Result<Vec<_>, _>>()?,
        ));
    }
    let parts = split_xpath_keyword(value, "and")?;
    if parts.len() > 1 {
        return Ok(XPathPredicate::And(
            parts
                .into_iter()
                .map(parse_xpath_predicate)
                .collect::<Result<Vec<_>, _>>()?,
        ));
    }
    for (operator, comparison) in [
        ("!=", XPathComparison::NotEqual),
        ("<=", XPathComparison::LessOrEqual),
        (">=", XPathComparison::GreaterOrEqual),
        ("=", XPathComparison::Equal),
        ("<", XPathComparison::Less),
        (">", XPathComparison::Greater),
    ] {
        if let Some(index) = find_xpath_top_level_operator(value, operator)? {
            let left = parse_xpath_predicate(&value[..index])?;
            let right = parse_xpath_predicate(&value[index + operator.len()..])?;
            return Ok(XPathPredicate::Compare(
                comparison,
                Box::new(left),
                Box::new(right),
            ));
        }
    }
    if let Some(argument) = xpath_function_argument(value, "not")? {
        return Ok(XPathPredicate::Not(Box::new(parse_xpath_predicate(
            argument,
        )?)));
    }
    if let Some(arguments) = xpath_function_argument(value, "contains")? {
        let arguments = split_xpath_top_level(arguments, ',')?;
        let [value, fragment] = arguments.as_slice() else {
            return Err(parse_failure(
                "native.xpath.unsupported: contains() requires two arguments",
            ));
        };
        return Ok(XPathPredicate::Contains(
            Box::new(parse_xpath_predicate(value)?),
            Box::new(parse_xpath_predicate(fragment)?),
        ));
    }
    if let Some(arguments) = xpath_function_argument(value, "concat")? {
        let arguments = split_xpath_top_level(arguments, ',')?;
        if arguments.len() < 2 {
            return Err(parse_failure(
                "native.xpath.unsupported: concat() requires at least two arguments",
            ));
        }
        return Ok(XPathPredicate::Concat(
            arguments
                .into_iter()
                .map(parse_xpath_predicate)
                .collect::<Result<Vec<_>, _>>()?,
        ));
    }
    if value == "last()" {
        return Ok(XPathPredicate::Last);
    }
    if value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
    {
        return Ok(XPathPredicate::String(value[1..value.len() - 1].to_owned()));
    }
    if let Ok(number) = value.parse::<f64>() {
        return Ok(XPathPredicate::Number(number));
    }
    let parts = split_xpath_top_level(value, '|')?;
    if parts.len() > 1 {
        return Ok(XPathPredicate::Union(
            parts
                .into_iter()
                .map(|part| parse_xpath_path(part.trim()))
                .collect::<Result<Vec<_>, _>>()?,
        ));
    }
    Ok(XPathPredicate::Path(parse_xpath_path(value)?))
}

fn ensure_xpath_name(value: &str) -> Result<(), ExecutionFailure> {
    if value.is_empty()
        || value != value.trim()
        || value.contains([':', '/', '[', ']', '(', ')', '@', '|'])
    {
        Err(parse_failure(
            "native.xpath.unsupported: namespace and axis expressions are unsupported",
        ))
    } else {
        Ok(())
    }
}

fn xpath_function_argument<'a>(
    value: &'a str,
    name: &str,
) -> Result<Option<&'a str>, ExecutionFailure> {
    let Some(rest) = value.strip_prefix(name) else {
        return Ok(None);
    };
    if !rest.starts_with('(') {
        return Ok(None);
    }
    let close = matching_xpath_delimiter(rest, 0, '(', ')')?;
    if close + 1 != rest.len() {
        return Ok(None);
    }
    Ok(Some(&rest[1..close]))
}

fn strip_xpath_parentheses(mut value: &str) -> Result<&str, ExecutionFailure> {
    loop {
        if !value.starts_with('(') {
            return Ok(value);
        }
        let close = matching_xpath_delimiter(value, 0, '(', ')')?;
        if close + 1 != value.len() {
            return Ok(value);
        }
        value = value[1..close].trim();
    }
}

fn split_xpath_keyword<'a>(
    value: &'a str,
    keyword: &str,
) -> Result<Vec<&'a str>, ExecutionFailure> {
    let mut output = Vec::new();
    let mut start = 0;
    for (cursor, _) in value.char_indices() {
        if cursor >= start
            && value[cursor..].starts_with(keyword)
            && xpath_is_top_level(value, cursor)?
            && value[..cursor]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
            && value[cursor + keyword.len()..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            output.push(value[start..cursor].trim());
            start = cursor + keyword.len();
        }
    }
    output.push(value[start..].trim());
    Ok(output)
}

fn split_xpath_top_level(value: &str, delimiter: char) -> Result<Vec<&str>, ExecutionFailure> {
    let mut output = Vec::new();
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if ch == delimiter && xpath_is_top_level(value, index)? {
            output.push(value[start..index].trim());
            start = index + ch.len_utf8();
        }
    }
    output.push(value[start..].trim());
    if output.iter().any(|part| part.is_empty()) {
        return Err(parse_failure(
            "native.xpath.unsupported: malformed union expression",
        ));
    }
    Ok(output)
}

fn find_xpath_top_level(value: &str, needle: char) -> Result<Option<usize>, ExecutionFailure> {
    for (index, ch) in value.char_indices() {
        if ch == needle && xpath_is_top_level(value, index)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn find_xpath_top_level_operator(
    value: &str,
    operator: &str,
) -> Result<Option<usize>, ExecutionFailure> {
    for (cursor, _) in value.char_indices() {
        if value[cursor..].starts_with(operator) && xpath_is_top_level(value, cursor)? {
            return Ok(Some(cursor));
        }
    }
    Ok(None)
}

fn xpath_is_top_level(value: &str, index: usize) -> Result<bool, ExecutionFailure> {
    xpath_depths(value, index)
        .map(|(brackets, parentheses, quote)| brackets == 0 && parentheses == 0 && quote.is_none())
}

fn xpath_depths(value: &str, end: usize) -> Result<(usize, usize, Option<char>), ExecutionFailure> {
    let mut brackets = 0usize;
    let mut parentheses = 0usize;
    let mut quote = None;
    for ch in value[..end].chars() {
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' => brackets += 1,
            ']' => {
                brackets = brackets.checked_sub(1).ok_or_else(|| {
                    parse_failure("native.xpath.unsupported: malformed predicate")
                })?;
            }
            '(' => parentheses += 1,
            ')' => {
                parentheses = parentheses.checked_sub(1).ok_or_else(|| {
                    parse_failure("native.xpath.unsupported: malformed function call")
                })?;
            }
            _ => {}
        }
    }
    Ok((brackets, parentheses, quote))
}

fn matching_xpath_delimiter(
    value: &str,
    open_index: usize,
    open: char,
    close: char,
) -> Result<usize, ExecutionFailure> {
    let mut depth = 0usize;
    let mut quote = None;
    for (relative, ch) in value[open_index..].char_indices() {
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            candidate if candidate == open => depth += 1,
            candidate if candidate == close => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    parse_failure("native.xpath.unsupported: malformed expression")
                })?;
                if depth == 0 {
                    return Ok(open_index + relative);
                }
            }
            _ => {}
        }
    }
    Err(parse_failure(
        "native.xpath.unsupported: unclosed expression",
    ))
}
