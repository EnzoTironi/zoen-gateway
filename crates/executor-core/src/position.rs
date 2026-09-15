//! Lexicographic keys for policy ordering. Lower key = higher precedence.

/// Generate a key strictly between `prev` and `next` (`None` is ±infinity).
///
/// Keys sort with ordinary byte order. Compatible with the original's
/// fractional-index *usage* (not bit-identical to the JS `generateKeyBetween`).
#[must_use]
pub fn generate_key_between(prev: Option<&str>, next: Option<&str>) -> String {
    match (prev, next) {
        (None, None) => "a0".to_owned(),
        (Some(prev), None) => increment(prev),
        (None, Some(next)) => before(next),
        (Some(prev), Some(next)) => between(prev, next),
    }
}

fn increment(key: &str) -> String {
    let mut bytes = key.as_bytes().to_vec();
    if let Some(last) = bytes.last_mut()
        && *last < b'z'
    {
        *last += 1;
        return String::from_utf8(bytes).unwrap_or_else(|_| format!("{key}0"));
    }
    format!("{key}0")
}

fn before(next: &str) -> String {
    if next.is_empty() || next == "a0" {
        return "Zz".to_owned();
    }
    let mut bytes = next.as_bytes().to_vec();
    if let Some(last) = bytes.last_mut()
        && *last > b'0'
    {
        *last -= 1;
        return String::from_utf8(bytes).unwrap_or_else(|_| "Zz".to_owned());
    }
    format!("Zz{next}")
}

fn between(prev: &str, next: &str) -> String {
    if prev < next {
        let candidate = increment(prev);
        if candidate.as_str() < next {
            return candidate;
        }
        return format!("{prev}0");
    }
    increment(prev)
}

#[cfg(test)]
mod tests {
    use super::generate_key_between;

    #[test]
    fn orders_around_neighbors() {
        let a = generate_key_between(None, None);
        let b = generate_key_between(Some(a.as_str()), None);
        let mid = generate_key_between(Some(a.as_str()), Some(b.as_str()));
        assert!(a < mid);
        assert!(mid < b);
        assert_eq!(a, "a0");
    }
}
