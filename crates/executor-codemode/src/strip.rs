//! Strip TypeScript type syntax so `QuickJS` can eval model-written code.
//!
//! Mirrors `packages/kernel/core/src/strip-types.ts` (sucrase `typescript`
//! transform) without a JS toolchain: annotations, `as`/`satisfies`, generics
//! on calls, `interface` / `type` declarations. Invalid leftover syntax is
//! rejected so callers get a parse error here instead of `Unexpected token ':'`.

use super::CodeError;

/// Remove TypeScript type syntax from `source`.
///
/// # Errors
///
/// `const =` / `let =` with no binding, or an unclosed type construct.
pub fn strip_typescript(source: &str) -> Result<String, CodeError> {
    if looks_like_bare_assignment(source) {
        return Err(CodeError::new("invalid TypeScript/JavaScript syntax"));
    }
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut out = String::with_capacity(source.len());
    while i < chars.len() {
        if starts_with(&chars, i, "//") {
            let end = chars[i..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(chars.len(), |n| i + n + 1);
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if starts_with(&chars, i, "/*") {
            let end = find_end(&chars, i + 2, "*/").map_or(chars.len(), |n| n + 2);
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if chars[i] == '\'' || chars[i] == '"' || chars[i] == '`' {
            let (chunk, next) = take_string(&chars, i)?;
            out.push_str(&chunk);
            i = next;
            continue;
        }
        if is_word_at(&chars, i, "interface") && is_decl_start(&chars, i) {
            i = skip_declaration(&chars, i + "interface".len())?;
            continue;
        }
        if is_word_at(&chars, i, "type")
            && is_decl_start(&chars, i)
            && looks_like_type_alias(&chars, i)
        {
            i = skip_type_alias(&chars, i)?;
            continue;
        }
        if is_word_at(&chars, i, "as") || is_word_at(&chars, i, "satisfies") {
            let kw_len = if is_word_at(&chars, i, "satisfies") {
                "satisfies".len()
            } else {
                2
            };
            if is_type_operator(&chars, i, kw_len) {
                i = skip_type(&chars, skip_ws(&chars, i + kw_len))?;
                continue;
            }
        }
        if chars[i] == ':' && !starts_with(&chars, i, "::") && !is_object_prop_colon(&chars, i) {
            let prev = prev_code(&chars, i);
            if prev.is_some_and(|c| {
                c == ')' || c.is_ascii_alphanumeric() || c == '_' || c == '?' || c == ']'
            }) {
                i = skip_type(&chars, skip_ws(&chars, i + 1))?;
                continue;
            }
        }
        if chars[i] == '<' && looks_like_call_generic(&chars, i) {
            i = skip_type_args(&chars, i)?;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    Ok(out)
}

const fn is_object_prop_colon(chars: &[char], colon: usize) -> bool {
    let mut i = colon;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return false;
    }
    i -= 1;
    if chars[i] == '"' || chars[i] == '\'' {
        let quote = chars[i];
        while i > 0 {
            i -= 1;
            if chars[i] == quote {
                break;
            }
        }
    } else if is_ident(chars[i]) {
        while i > 0 && is_ident(chars[i - 1]) {
            i -= 1;
        }
    } else {
        return false;
    }
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i > 0 && matches!(chars[i - 1], '{' | ',')
}

fn looks_like_bare_assignment(source: &str) -> bool {
    let t = source.trim_start();
    t.starts_with("const =") || t.starts_with("let =") || t.starts_with("var =")
}

fn starts_with(chars: &[char], i: usize, s: &str) -> bool {
    s.chars()
        .enumerate()
        .all(|(n, c)| chars.get(i + n) == Some(&c))
}

fn is_word_at(chars: &[char], i: usize, word: &str) -> bool {
    if !starts_with(chars, i, word) {
        return false;
    }
    let before_ok = i == 0 || !is_ident(chars[i - 1]);
    let after = i + word.len();
    let after_ok = after >= chars.len() || !is_ident(chars[after]);
    before_ok && after_ok
}

const fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

fn is_decl_start(chars: &[char], i: usize) -> bool {
    i == 0
        || chars[..i]
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .is_none_or(|c| matches!(c, ';' | '{' | '}' | '\n' | '('))
}

fn looks_like_type_alias(chars: &[char], i: usize) -> bool {
    let mut j = skip_ws(chars, i + 4);
    if j >= chars.len() || !is_ident(chars[j]) {
        return false;
    }
    while j < chars.len() && is_ident(chars[j]) {
        j += 1;
    }
    j = skip_ws(chars, j);
    chars.get(j) == Some(&'=') || chars.get(j) == Some(&'<')
}

fn is_type_operator(chars: &[char], i: usize, kw_len: usize) -> bool {
    let prev = prev_code(chars, i);
    let next = skip_ws(chars, i + kw_len);
    prev.is_some_and(|c| is_ident(c) || c == ')' || c == ']' || c == '>' || c == '?')
        && next < chars.len()
        && (is_ident(chars[next]) || chars[next] == '{' || chars[next] == '(')
}

fn prev_code(chars: &[char], i: usize) -> Option<char> {
    chars[..i]
        .iter()
        .rev()
        .find(|c| !c.is_whitespace())
        .copied()
}

const fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

fn find_end(chars: &[char], start: usize, needle: &str) -> Option<usize> {
    let n: Vec<char> = needle.chars().collect();
    chars[start..]
        .windows(n.len())
        .position(|w| w == n.as_slice())
        .map(|p| start + p)
}

fn take_string(chars: &[char], start: usize) -> Result<(String, usize), CodeError> {
    let quote = chars[start];
    let mut i = start + 1;
    let mut out = String::new();
    out.push(quote);
    while i < chars.len() {
        let c = chars[i];
        out.push(c);
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if quote == '`' && c == '$' && chars.get(i + 1) == Some(&'{') {
            i += 2;
            out.push('{');
            let mut depth = 1;
            while i < chars.len() && depth > 0 {
                if chars[i] == '\'' || chars[i] == '"' || chars[i] == '`' {
                    let (chunk, next) = take_string(chars, i)?;
                    out.push_str(&chunk);
                    i = next;
                    continue;
                }
                if chars[i] == '{' {
                    depth += 1;
                } else if chars[i] == '}' {
                    depth -= 1;
                }
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == quote {
            return Ok((out, i + 1));
        }
        i += 1;
    }
    Err(CodeError::new("unterminated string"))
}

fn skip_declaration(chars: &[char], mut i: usize) -> Result<usize, CodeError> {
    i = skip_ws(chars, i);
    while i < chars.len() && is_ident(chars[i]) {
        i += 1;
    }
    i = skip_ws(chars, i);
    if chars.get(i) == Some(&'<') {
        i = skip_type_args(chars, i)?;
        i = skip_ws(chars, i);
    }
    if chars.get(i) == Some(&'{') {
        return skip_balanced(chars, i, '{', '}');
    }
    Err(CodeError::new("invalid interface declaration"))
}

fn skip_type_alias(chars: &[char], start: usize) -> Result<usize, CodeError> {
    let mut i = skip_ws(chars, start + 4);
    while i < chars.len() && is_ident(chars[i]) {
        i += 1;
    }
    i = skip_ws(chars, i);
    if chars.get(i) == Some(&'<') {
        i = skip_type_args(chars, i)?;
        i = skip_ws(chars, i);
    }
    if chars.get(i) != Some(&'=') {
        return Err(CodeError::new("invalid type alias"));
    }
    i = skip_type(chars, skip_ws(chars, i + 1))?;
    if chars.get(i) == Some(&';') {
        i += 1;
    }
    Ok(i)
}

fn skip_type_args(chars: &[char], i: usize) -> Result<usize, CodeError> {
    skip_balanced(chars, i, '<', '>')
}

fn skip_balanced(
    chars: &[char],
    start: usize,
    open: char,
    close: char,
) -> Result<usize, CodeError> {
    if chars.get(start) != Some(&open) {
        return Err(CodeError::new("expected type delimiter"));
    }
    let mut depth = 0;
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '\'' || chars[i] == '"' || chars[i] == '`' {
            let (_, next) = take_string(chars, i)?;
            i = next;
            continue;
        }
        if chars[i] == open {
            depth += 1;
        } else if chars[i] == close {
            depth -= 1;
            if depth == 0 {
                return Ok(i + 1);
            }
        }
        i += 1;
    }
    Err(CodeError::new("unclosed type construct"))
}

fn skip_type(chars: &[char], mut i: usize) -> Result<usize, CodeError> {
    i = skip_ws(chars, i);
    if i >= chars.len() {
        return Ok(i);
    }
    if starts_with(chars, i, "typeof")
        || starts_with(chars, i, "keyof")
        || starts_with(chars, i, "infer")
    {
        i = skip_ws(chars, i + 6);
    }
    match chars.get(i) {
        Some('{') => i = skip_balanced(chars, i, '{', '}')?,
        Some('[') => i = skip_balanced(chars, i, '[', ']')?,
        Some('(') => {
            i = skip_balanced(chars, i, '(', ')')?;
            i = skip_ws(chars, i);
            if starts_with(chars, i, "=>") {
                i = skip_type(chars, skip_ws(chars, i + 2))?;
            }
        }
        Some('<') => i = skip_type_args(chars, i)?,
        Some(_) if chars[i] == '\'' || chars[i] == '"' => {
            let (_, next) = take_string(chars, i)?;
            i = next;
        }
        Some(c) if is_ident(*c) => {
            while i < chars.len() && (is_ident(chars[i]) || chars[i] == '.') {
                i += 1;
            }
            i = skip_ws(chars, i);
            if chars.get(i) == Some(&'<') {
                i = skip_type_args(chars, i)?;
            }
        }
        _ => {}
    }
    i = skip_ws(chars, i);
    if chars.get(i) == Some(&'[') && chars.get(i + 1) == Some(&']') {
        i += 2;
        i = skip_ws(chars, i);
    }
    if chars.get(i) == Some(&'|') || chars.get(i) == Some(&'&') {
        i = skip_type(chars, skip_ws(chars, i + 1))?;
    }
    Ok(i)
}

fn looks_like_call_generic(chars: &[char], i: usize) -> bool {
    let prev = prev_code(chars, i);
    if !prev.is_some_and(is_ident) {
        return false;
    }
    let Ok(end) = skip_type_args(chars, i) else {
        return false;
    };
    let j = skip_ws(chars, end);
    chars.get(j) == Some(&'(') || chars.get(j) == Some(&'`')
}

#[cfg(test)]
mod tests {
    use super::strip_typescript;

    #[test]
    fn removes_variable_annotations() {
        let out = strip_typescript(r#"const x: string = "hello"; return x;"#).unwrap();
        assert!(!out.contains(": string"));
        assert!(out.contains("const x"));
        assert!(out.contains("\"hello\""));
    }

    #[test]
    fn removes_function_param_and_return_types() {
        let out = strip_typescript("function f(x: number): number { return x + 1; } return f(5);")
            .unwrap();
        assert!(!out.contains(": number"));
    }

    #[test]
    fn removes_as_casts() {
        let out = strip_typescript("const x = (1 as number) + 2; return x;").unwrap();
        assert!(!out.contains("as number"));
    }

    #[test]
    fn removes_generic_type_arguments_on_calls() {
        let out = strip_typescript("const arr = Array.from<string>([]); return arr;").unwrap();
        assert!(!out.contains("<string>"));
    }

    #[test]
    fn removes_interface_declarations() {
        let out =
            strip_typescript("interface User { name: string; } const u = { name: 'a' }; return u;")
                .unwrap();
        assert!(!out.contains("interface User"));
        assert!(!out.contains(": string"));
    }

    #[test]
    fn removes_type_aliases() {
        let out = strip_typescript("type Foo = string; const x = 'a'; return x;").unwrap();
        assert!(!out.contains("type Foo"));
    }

    #[test]
    fn preserves_object_literal_values() {
        let out = strip_typescript("return { n: 4, doubled: ping.n * 2 };").unwrap();
        assert!(out.contains("n: 4"), "{out}");
        assert!(out.contains("ping.n * 2"), "{out}");
    }

    #[test]
    fn preserves_plain_javascript() {
        let out = strip_typescript("const x = 5; return x * 2;").unwrap();
        assert!(out.contains("const x = 5"));
    }

    #[test]
    fn rejects_bare_const() {
        assert!(strip_typescript("const = 5;").is_err());
    }

    #[test]
    fn regression_promise_annotation() {
        let code = r"
      const fetchResources = async (sourceId: string): Promise<Array<{ id: string }>> => {
        const result = await tools.executor.integrations.list();
        return result.items;
      };
      return fetchResources('example-source');
    ";
        let out = strip_typescript(code).unwrap();
        assert!(!out.contains(": string"));
        assert!(!out.contains("Promise<"));
    }
}
