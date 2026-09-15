//! RFC 7235 §2.1 `WWW-Authenticate` parser.
//!
//! Fail closed: any malformed header returns `None`. A miss is benign; a false
//! positive on `insufficient_scope` would strip a valid re-authenticate recovery.

use std::collections::BTreeMap;

/// One challenge: `scheme` plus auth-params. `scheme` is lowercased.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthChallenge {
    /// HTTP token, lowercased (`bearer`, `basic`, …).
    pub scheme: String,
    /// Lowercased parameter names. Signal params (`error`, `scope`) reject duplicates.
    pub params: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum State {
    Boundary,
    Scheme,
    Token68,
    Params,
}

/// Parse authentication challenges without joining parameters across schemes.
///
/// Returns `None` when the header is malformed.
#[must_use]
pub fn parse_challenges(header: &str) -> Option<Vec<AuthChallenge>> {
    Parser::new(header).run()
}

struct Parser<'a> {
    bytes: &'a [u8],
    i: usize,
    challenges: Vec<AuthChallenge>,
    state: State,
    saw_comma: bool,
}

impl<'a> Parser<'a> {
    const fn new(header: &'a str) -> Self {
        Self {
            bytes: header.as_bytes(),
            i: 0,
            challenges: Vec::new(),
            state: State::Boundary,
            saw_comma: true,
        }
    }

    fn run(mut self) -> Option<Vec<AuthChallenge>> {
        let len = self.bytes.len();
        while self.i < len {
            self.skip_bws();
            if self.i >= len {
                break;
            }
            if self.bytes[self.i] == b',' {
                self.saw_comma = true;
                self.i += 1;
                continue;
            }
            if !is_word_byte(self.bytes[self.i]) {
                return None;
            }
            if !self.consume_token()? {
                return None;
            }
        }
        Some(self.challenges)
    }

    const fn skip_bws(&mut self) {
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    const fn skip_htab_sp(&mut self) {
        while self.i < self.bytes.len() && matches!(self.bytes[self.i], b' ' | b'\t') {
            self.i += 1;
        }
    }

    fn read_word(&mut self) -> String {
        let start = self.i;
        while self.i < self.bytes.len() && is_word_byte(self.bytes[self.i]) {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.i]).into_owned()
    }

    fn consume_token(&mut self) -> Option<bool> {
        let word = self.read_word();
        if self.is_padding_run() {
            return self.take_token68_padding(&word);
        }
        let equals_at = self.peek_equals_after_bws();
        if equals_at {
            return self.take_auth_param(&word).map(|()| true);
        }
        self.take_bare_word(&word).map(|()| true)
    }

    const fn peek_equals_after_bws(&self) -> bool {
        let mut j = self.i;
        while j < self.bytes.len() && matches!(self.bytes[j], b' ' | b'\t') {
            j += 1;
        }
        j < self.bytes.len() && self.bytes[j] == b'='
    }

    const fn is_padding_run(&self) -> bool {
        if self.i >= self.bytes.len() || self.bytes[self.i] != b'=' {
            return false;
        }
        let mut k = self.i;
        while k < self.bytes.len() && self.bytes[k] == b'=' {
            k += 1;
        }
        while k < self.bytes.len() && matches!(self.bytes[k], b' ' | b'\t') {
            k += 1;
        }
        k >= self.bytes.len() || self.bytes[k] == b','
    }

    fn take_token68_padding(&mut self, word: &str) -> Option<bool> {
        if self.state != State::Scheme || self.saw_comma {
            return None;
        }
        if !is_token68(word) {
            return None;
        }
        while self.i < self.bytes.len() && self.bytes[self.i] == b'=' {
            self.i += 1;
        }
        self.state = State::Token68;
        self.saw_comma = false;
        Some(true)
    }

    fn take_auth_param(&mut self, word: &str) -> Option<()> {
        if !is_http_token(word) {
            return None;
        }
        if self.challenges.is_empty() {
            return None;
        }
        if self.state == State::Token68 {
            return None;
        }
        if self.state == State::Scheme && self.saw_comma {
            return None;
        }
        if self.state == State::Params && !self.saw_comma {
            return None;
        }
        self.skip_htab_sp();
        if self.i >= self.bytes.len() || self.bytes[self.i] != b'=' {
            return None;
        }
        self.i += 1;
        self.skip_htab_sp();
        let value = self.read_param_value(word)?;
        let key = word.to_ascii_lowercase();
        let current = self.challenges.last_mut()?;
        let is_signal = key == "error" || key == "scope";
        match current.params.entry(key) {
            std::collections::btree_map::Entry::Occupied(_) if is_signal => return None,
            std::collections::btree_map::Entry::Occupied(_) => {}
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(value);
            }
        }
        self.state = State::Params;
        self.saw_comma = false;
        Some(())
    }

    fn read_param_value(&mut self, name: &str) -> Option<String> {
        if self.i < self.bytes.len() && self.bytes[self.i] == b'"' {
            return self.read_quoted();
        }
        let start = self.i;
        while self.i < self.bytes.len()
            && !matches!(self.bytes[self.i], b' ' | b'\t' | b'\n' | b'\r' | b',')
        {
            self.i += 1;
        }
        let value = String::from_utf8_lossy(&self.bytes[start..self.i]).into_owned();
        if value.is_empty() {
            return None;
        }
        let lower = name.to_ascii_lowercase();
        if is_http_token(&value) || (lower == "resource_metadata" && is_urlish(&value)) {
            Some(value)
        } else {
            None
        }
    }

    fn read_quoted(&mut self) -> Option<String> {
        self.i += 1;
        let mut value = String::new();
        while self.i < self.bytes.len() {
            let ch = self.bytes[self.i];
            if ch == b'"' {
                self.i += 1;
                let ok = self.i >= self.bytes.len()
                    || matches!(self.bytes[self.i], b' ' | b'\t' | b'\n' | b'\r' | b',');
                return if ok { Some(value) } else { None };
            }
            if ch == b'\\' && self.i + 1 < self.bytes.len() {
                value.push(self.bytes[self.i + 1] as char);
                self.i += 2;
                continue;
            }
            value.push(ch as char);
            self.i += 1;
        }
        None
    }

    fn take_bare_word(&mut self, word: &str) -> Option<()> {
        if self.saw_comma {
            if !is_http_token(word) {
                return None;
            }
            self.challenges.push(AuthChallenge {
                scheme: word.to_ascii_lowercase(),
                params: BTreeMap::new(),
            });
            self.state = State::Scheme;
            self.saw_comma = false;
            return Some(());
        }
        if self.state == State::Scheme {
            if !is_token68(word) {
                return None;
            }
            self.state = State::Token68;
            return Some(());
        }
        None
    }
}

const fn is_word_byte(b: u8) -> bool {
    is_http_token_byte(b) || b == b'/'
}

const fn is_http_token_byte(b: u8) -> bool {
    matches!(
        b,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
    )
}

const fn is_token68_byte(b: u8) -> bool {
    matches!(
        b,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'~' | b'+' | b'/' | b'-'
    )
}

fn is_http_token(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(is_http_token_byte)
}

fn is_token68(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(is_token68_byte)
}

fn is_urlish(word: &str) -> bool {
    let bytes = word.as_bytes();
    let Some((scheme_end, _)) = bytes.iter().copied().enumerate().find(|(_, b)| *b == b':') else {
        return false;
    };
    if scheme_end == 0 || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    if !bytes[1..scheme_end]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
    {
        return false;
    }
    let Some(rest) = bytes.get(scheme_end..) else {
        return false;
    };
    if rest.len() < 3 || rest[1] != b'/' || rest[2] != b'/' {
        return false;
    }
    rest.len() > 3
        && !rest[3..]
            .iter()
            .any(|b| b.is_ascii_whitespace() || *b == b',' || *b == b'"')
}

#[cfg(test)]
mod tests {
    use super::parse_challenges;

    #[test]
    fn malformed_returns_none() {
        assert!(parse_challenges("Bearer error=\"insufficient_scope").is_none());
    }
}
