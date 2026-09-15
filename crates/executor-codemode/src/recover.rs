//! Recover a runnable guest body from model-shaped source.
//!
//! Mirrors `packages/kernel/core/src/code-recovery.ts` without Babel: fences,
//! `export default`, arrow / function wrappers. Valid JS that needs a real
//! parser still runs in the isolate after this pass.

const CALLABLE_ERROR: &str = "Code must evaluate to a function";

/// Strip markdown fences / wrappers so a JS isolate can `Function()`-run it.
#[must_use]
pub fn recover_execution_body(code: &str) -> String {
    let source = extract_candidate(code);
    if source.is_empty() {
        return String::new();
    }
    render_heuristic(&source)
}

fn extract_candidate(code: &str) -> String {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    fenced_body(trimmed).map_or_else(|| trimmed.to_owned(), |body| body.trim().to_owned())
}

fn fenced_body(source: &str) -> Option<String> {
    let start = source.find("```")?;
    let after_ticks = source.get(start + 3..)?;
    let newline = after_ticks.find('\n')?;
    let rest = after_ticks.get(newline + 1..)?;
    let end = rest.find("```")?;
    Some(rest[..end].to_owned())
}

fn render_heuristic(source: &str) -> String {
    let stripped_comments = strip_leading_comments(source);
    let (after_export, had_export) = strip_export_default(&stripped_comments);
    let after_export = if had_export {
        unwrap_one_paren_if_arrow(&after_export)
    } else {
        after_export
    };

    if let Some(head) = function_head(&after_export) {
        let wrap_src = if had_export {
            after_export
        } else {
            source.to_owned()
        };
        return match head {
            FnHead::Named(name) => wrap_named(&wrap_src, &name),
            FnHead::Anonymous => wrap_anonymous(&wrap_src),
        };
    }

    if looks_like_arrow(&after_export) {
        let wrap_src = if had_export {
            after_export
        } else {
            source.to_owned()
        };
        return wrap_callable(&wrap_src);
    }

    if had_export {
        after_export
    } else {
        source.to_owned()
    }
}

fn strip_leading_comments(source: &str) -> String {
    let mut s = source.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("//") {
            s = rest
                .split_once('\n')
                .map_or("", |(_, rest)| rest)
                .trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("/*") {
            match rest.split_once("*/") {
                Some((_, rest)) => {
                    s = rest.trim_start();
                    continue;
                }
                None => return String::new(),
            }
        }
        break;
    }
    s.to_owned()
}

fn strip_export_default(source: &str) -> (String, bool) {
    let trimmed = source.trim_start();
    let Some(rest) = trimmed.strip_prefix("export") else {
        return (source.to_owned(), false);
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix("default") else {
        return (source.to_owned(), false);
    };
    if !rest.starts_with(|c: char| c.is_whitespace() || c == '(') {
        return (source.to_owned(), false);
    }
    (rest.trim_start().to_owned(), true)
}

fn unwrap_one_paren_if_arrow(source: &str) -> String {
    let trimmed = source.trim();
    let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return source.to_owned();
    };
    let inner = inner.trim();
    if looks_like_arrow(inner) {
        inner.to_owned()
    } else {
        source.to_owned()
    }
}

fn looks_like_arrow(source: &str) -> bool {
    let trimmed = source.trim_start();
    (trimmed.starts_with("async") || trimmed.starts_with('(')) && trimmed.contains("=>")
}

enum FnHead {
    Named(String),
    Anonymous,
}

fn function_head(source: &str) -> Option<FnHead> {
    let mut rest = source.trim_start();
    if let Some(after) = rest.strip_prefix("async") {
        if !after.starts_with(|c: char| c.is_whitespace()) {
            return None;
        }
        rest = after.trim_start();
    }
    rest = rest.strip_prefix("function")?;
    if rest.starts_with('(') {
        return Some(FnHead::Anonymous);
    }
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    rest = rest.trim_start();
    if rest.starts_with('(') {
        return Some(FnHead::Anonymous);
    }
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(rest.len());
    if name_len == 0 {
        return None;
    }
    let name = rest[..name_len].to_owned();
    let after_name = rest[name_len..].trim_start();
    after_name.starts_with('(').then_some(FnHead::Named(name))
}

fn wrap_callable(source: &str) -> String {
    format!(
        "const __fn = (\n{source}\n);\nif (typeof __fn !== \"function\") throw new Error({error});\nreturn await __fn();",
        error = serde_json::to_string(CALLABLE_ERROR)
            .unwrap_or_else(|_| "\"Code must evaluate to a function\"".into()),
    )
}

fn wrap_named(source: &str, name: &str) -> String {
    format!("{source}\nreturn await {name}();")
}

fn wrap_anonymous(source: &str) -> String {
    format!("return await ({source})();")
}

#[cfg(test)]
mod tests {
    use super::{recover_execution_body, wrap_callable};

    #[test]
    fn wraps_async_arrow() {
        let src = "async () => { return 1; }";
        assert_eq!(recover_execution_body(src), wrap_callable(src));
    }

    #[test]
    fn wraps_async_arrow_with_params() {
        let src = "async (x) => x + 1";
        assert_eq!(recover_execution_body(src), wrap_callable(src));
    }

    #[test]
    fn strips_export_default_arrow() {
        assert_eq!(
            recover_execution_body("export default async () => 42"),
            wrap_callable("async () => 42")
        );
    }

    #[test]
    fn leaves_bare_expression() {
        assert_eq!(recover_execution_body("1 + 2"), "1 + 2");
    }

    #[test]
    fn wraps_named_function() {
        let src = "function hello() { return 42; }";
        assert_eq!(
            recover_execution_body(src),
            "function hello() { return 42; }\nreturn await hello();"
        );
    }

    #[test]
    fn wraps_async_function_declaration() {
        let src = "async function run() { return 'ok'; }";
        assert_eq!(
            recover_execution_body(src),
            "async function run() { return 'ok'; }\nreturn await run();"
        );
    }

    #[test]
    fn wraps_exported_named_async_function() {
        assert_eq!(
            recover_execution_body("export default async function run() { return 'ok'; }"),
            "async function run() { return 'ok'; }\nreturn await run();"
        );
    }

    #[test]
    fn wraps_exported_anonymous_async_function() {
        assert_eq!(
            recover_execution_body("export default async function () { return 'ok'; }"),
            "return await (async function () { return 'ok'; })();"
        );
    }

    #[test]
    fn strips_markdown_fences() {
        assert_eq!(
            recover_execution_body("```js\nasync () => 42\n```"),
            wrap_callable("async () => 42")
        );
        assert_eq!(
            recover_execution_body("```typescript\nasync () => 42\n```"),
            wrap_callable("async () => 42")
        );
        assert_eq!(
            recover_execution_body("```TypeScript\nasync () => 42\n```"),
            wrap_callable("async () => 42")
        );
    }

    #[test]
    fn empty_and_whitespace() {
        assert_eq!(recover_execution_body(""), "");
        assert_eq!(recover_execution_body("   \n  "), "");
    }

    #[test]
    fn keeps_multi_statement() {
        let code = "const a = 1;\nconst b = 2;\na + b";
        assert_eq!(recover_execution_body(code), code);
    }

    #[test]
    fn parenthesized_arrow_from_fence() {
        assert_eq!(
            recover_execution_body("```ts\n(async () => 42)\n```"),
            wrap_callable("(async () => 42)")
        );
    }

    #[test]
    fn strips_export_default_parenthesized_arrow() {
        assert_eq!(
            recover_execution_body("export default (async () => 42)"),
            wrap_callable("async () => 42")
        );
    }

    #[test]
    fn wraps_anonymous_function_from_fence() {
        assert_eq!(
            recover_execution_body("```javascript\nfunction () { return 42; }\n```"),
            "return await (function () { return 42; })();"
        );
    }

    #[test]
    fn keeps_body_that_already_returns() {
        let code =
            "const query = \"github issues\";\nreturn await tools.search({ query, limit: 5 });";
        assert_eq!(recover_execution_body(code), code);
    }

    #[test]
    fn extracts_fenced_body_from_prose() {
        let code = "Sure, here's the code:\n\n```ts\nasync () => 42\n```";
        assert_eq!(
            recover_execution_body(code),
            wrap_callable("async () => 42")
        );
    }

    #[test]
    fn named_function_with_leading_comments() {
        let code = "// top-level helper the model wanted to keep\nasync function run() {\n  return \"ok\";\n}";
        assert_eq!(
            recover_execution_body(code),
            format!("{code}\nreturn await run();")
        );
    }

    #[test]
    fn leading_comments_before_arrow_wrap_original() {
        let code = "// calling tools.search\nasync () => await tools.search({ query: \"x\" })";
        assert_eq!(recover_execution_body(code), wrap_callable(code));
    }

    #[test]
    fn block_comment_before_export_default() {
        let code = "/* generated by the model */\nexport default async () => 42";
        assert_eq!(
            recover_execution_body(code),
            wrap_callable("async () => 42")
        );
    }

    #[test]
    fn extracts_prose_before_and_after_fence() {
        let code = [
            "Use this exact snippet.",
            "",
            "```javascript",
            "return await tools.search({ query: \"github issues\", limit: 5 });",
            "```",
            "",
            "That should do it.",
        ]
        .join("\n");
        assert_eq!(
            recover_execution_body(&code),
            "return await tools.search({ query: \"github issues\", limit: 5 });"
        );
    }
}
