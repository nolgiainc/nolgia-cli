use anyhow::{Result, bail};
use serde_json::Value;

pub(super) fn resolve<'a>(value: &'a Value, path: &str) -> Result<&'a Value> {
    let source = path.strip_prefix('.').unwrap_or(path);
    if source.is_empty() {
        bail!("invalid field path {path:?}: expected a key or array index");
    }
    let mut rest = source;
    let mut current = value;
    while !rest.is_empty() {
        let location = &source[..source.len() - rest.len()];
        let location = location.trim_end_matches('.');
        let location = if location.is_empty() { "." } else { location };
        let (segment, next, found, reason) = if let Some(index) = rest.strip_prefix('[') {
            let Some(end) = index.find(']') else {
                bail!("invalid field path {path:?}: unclosed array index");
            };
            let digits = &index[..end];
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                bail!("invalid field path {path:?}: expected a zero-based array index");
            }
            let parsed = digits.parse::<usize>().ok();
            let found = parsed.and_then(|index| current.as_array()?.get(index));
            let reason = if current.is_array() {
                "index is past the end"
            } else {
                "expected an array"
            };
            (&rest[..end + 2], &index[end + 1..], found, reason)
        } else {
            let end = rest.find(['.', '[', ']']).unwrap_or(rest.len());
            if end == 0 {
                bail!("invalid field path {path:?}: expected a key");
            }
            let key = &rest[..end];
            (
                key,
                &rest[end..],
                current.as_object().and_then(|object| object.get(key)),
                "key not found",
            )
        };
        current = match found {
            Some(found) => found,
            None => {
                let available = match current {
                    Value::Object(object) if object.is_empty() => "no keys".to_string(),
                    Value::Object(object) => object.keys().cloned().collect::<Vec<_>>().join(", "),
                    Value::Array(array) => format!("array of {} items", array.len()),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                        "scalar (no fields or indexes)".to_string()
                    }
                };
                bail!(
                    "field {path:?} not found in the output (available at {location:?}: {available}; segment {segment:?}: {reason})"
                );
            }
        };
        rest = match next.strip_prefix('.') {
            Some("") => bail!("invalid field path {path:?}: expected a key after '.'"),
            Some(next) if next.starts_with('[') => {
                bail!("invalid field path {path:?}: expected a key after '.'")
            }
            Some(next) => next,
            None if next.is_empty() || next.starts_with('[') => next,
            None => bail!("invalid field path {path:?}: expected '.' or an array index"),
        };
    }
    Ok(current)
}
