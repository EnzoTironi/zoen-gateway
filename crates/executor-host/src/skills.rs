//! On-demand MCP `skills` documents (execute + passthrough search-invoke).

/// One named how-to served by the `skills` tool.
#[derive(Clone, Copy, Debug)]
pub struct Skill {
    /// Stable name (`execute`).
    pub name: &'static str,
    /// One-line summary for the catalog listing.
    pub summary: &'static str,
    /// Markdown body.
    pub body: &'static str,
}

/// How to write code for the `execute` sandbox.
pub const EXECUTE_SKILL: Skill = Skill {
    name: "execute",
    summary: "How to call integrations from the execute sandbox: search the catalog, read a tool's shape, call it, emit results, and resume paused runs.",
    body: EXECUTE_SKILL_BODY,
};

const EXECUTE_SKILL_BODY: &str = r#"# execute

Execute TypeScript in a sandboxed runtime with access to configured API tools.

## Workflow

1. `const { items: matches } = await tools.search({ query: "<intent + key nouns>", limit: 12 });`
2. `const path = matches[0]?.path; if (!path) return "No matching tools found.";`
3. `const details = await tools.describe.tool({ path });`
4. Use `details.inputTypeScript` / `details.outputTypeScript` and `details.typeScriptDefinitions` for compact shapes.
5. For live saved-connection inventory, call `tools.executor.coreTools.connections.list({})`; after checking `result.ok`, read `result.data.connections`.
6. Call the tool: `const result = await tools.<path>(input);`

## Rules

- `tools.search()` returns paginated, ranked matches: `{ items, total, hasMore, nextOffset }`. Best-first. Use short intent phrases like `github issues`, `repo details`, or `create calendar event`.
- When you already know the namespace, narrow with `tools.search({ namespace: "github", query: "issues" })`.
- `tools.executor.coreTools.connections.list({})` returns saved connections with `{ address, integration, owner, name, ... }`. The `address` field includes the leading `tools.` root.
- Tool calls return a value union: `{ ok: true, data }` for success or `{ ok: false, error: { code, message, status?, details?, retryable? } }` for expected tool/domain failures. Branch on `result.ok`.
- `data` is the upstream payload itself. HTTP-backed tools (OpenAPI) also set `http: { status, headers }` beside `data`.
- Use `emit(value)` to append user-visible output.
- If `tools.search()` returns `hasMore: true` and you didn't find what you need, fetch the next page: `tools.search({ query, offset: nextOffset, limit })`.
- Always use the full address when calling tools: `tools.<integration>.<owner>.<connection>.<tool>(args)`. The `path` returned by `tools.search()` / `tools.describe.tool()` is already the exact path under `tools` — call `tools[path]` rather than guessing segments.
- The `tools` object is a lazy proxy — enumerating it (`Object.keys(tools)`, spread, `for...in`) throws. Use `tools.search()` or `tools.executor.coreTools.connections.list({})` instead.
- Pass an object to system tools, e.g. `tools.search({ query: "..." })`, `tools.executor.coreTools.connections.list({})`, and `tools.describe.tool({ path })`.
- `tools.describe.tool()` returns compact TypeScript shapes. Use `inputTypeScript`, `outputTypeScript`, and `typeScriptDefinitions`. If the path doesn't resolve, the result carries `error: { code: "tool_not_found", suggestions }`.
- Do not use `fetch` — all API calls go through `tools.*`.
- If execution pauses for interaction, resume it with the returned `resumePayload`.
- TypeScript type syntax (`: T`, `as T`, generics, interfaces, type aliases) is stripped before execution — feel free to write idiomatic TypeScript using the shapes from `tools.describe.tool()`. Decorators and `enum` are not supported.
"#;

/// How to use passthrough `integrations` / `search` / `invoke`.
pub const SEARCH_INVOKE_SKILL: Skill = Skill {
    name: "search-invoke",
    summary: "Discover connected accounts, search for actions, and invoke tools with JSON arguments.",
    body: SEARCH_INVOKE_BODY,
};

const SEARCH_INVOKE_BODY: &str = r#"# Search and invoke

1. Call `integrations({})` to see connected integrations and accounts.
2. Call `search({ query: "create issue", integration: "github", owner: "org", connection: "main" })`. Use the exact integration, owner, and connection returned by integrations to select an account. Omit filters to search across accounts visible to you.
3. Read the matching tool's `inputSchema`. Call `invoke({ tool: <exact returned id>, arguments: <JSON object matching inputSchema> })`. Do not guess tool IDs or arguments.

## Pagination
Both integrations and search return `{ items, total, hasMore, nextOffset }`. If hasMore is true, repeat the call with the same filters and `offset: nextOffset`.

## Results and approval
Invoke forwards the tool's result. Check `isError` and any returned error before treating a call as successful. Workspace block policies still apply.
This mode accepts JSON tool arguments. It does not expose general execute or resume tools.
"#;

/// Short `execute` tool description (inventory appended by the host).
#[must_use]
pub fn execute_description(inventory: &str) -> String {
    let mut text = String::from(
        "Run TypeScript against configured API tools. Call skills({ name: \"execute\" }) for the full guide.",
    );
    if !inventory.is_empty() {
        text.push_str("\n\nConnected integrations:\n");
        text.push_str(inventory);
    }
    text
}

/// Uploaded skill listed beside the built-in guides.
#[derive(Clone, Debug)]
pub struct ExtraSkill {
    /// Slug.
    pub name: String,
    /// One-line summary.
    pub summary: String,
    /// Markdown body.
    pub body: String,
}

/// Result of `skills({ name? })` including uploaded bundles.
#[must_use]
pub fn skills_result(
    name: Option<&str>,
    passthrough: bool,
    inventory: &str,
    extras: &[ExtraSkill],
) -> serde_json::Value {
    let catalog: Vec<&Skill> = if passthrough {
        vec![&SEARCH_INVOKE_SKILL]
    } else {
        vec![&EXECUTE_SKILL]
    };
    match name {
        None | Some("") => {
            let mut skills: Vec<serde_json::Value> = catalog
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "summary": s.summary,
                    })
                })
                .collect();
            skills.extend(extras.iter().map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "summary": s.summary,
                })
            }));
            serde_json::json!({ "skills": skills })
        }
        Some(want) => {
            if let Some(skill) = catalog.iter().find(|s| s.name == want) {
                let mut body = skill.body.to_owned();
                if skill.name == "execute" && !inventory.is_empty() {
                    body.push_str("\n\n");
                    body.push_str(inventory);
                }
                return serde_json::json!({
                    "name": skill.name,
                    "summary": skill.summary,
                    "body": body
                });
            }
            if let Some(skill) = extras.iter().find(|s| s.name == want) {
                return serde_json::json!({
                    "name": skill.name,
                    "summary": skill.summary,
                    "body": skill.body
                });
            }
            let mut names: Vec<&str> = catalog.iter().map(|s| s.name).collect();
            names.extend(extras.iter().map(|s| s.name.as_str()));
            serde_json::json!({
                "error": format!("unknown skill {want}"),
                "skills": names
            })
        }
    }
}
