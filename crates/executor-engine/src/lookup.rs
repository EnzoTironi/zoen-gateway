//! Resolve a CLI / sandbox path to a catalog or static tool.

use executor_core::{ExecutorError, Tool, ToolListFilter, parse_tool_address};

use crate::Inner;

impl Inner {
    pub(crate) fn all_tools(&self) -> Result<Vec<Tool>, ExecutorError> {
        let mut tools = self.store.list_tools()?;
        tools.extend(self.static_tools.read().values().cloned());
        Ok(tools)
    }

    pub(crate) fn list_filtered(
        &self,
        filter: &ToolListFilter,
    ) -> Result<Vec<Tool>, ExecutorError> {
        let policies = self.store.list_policies()?;
        let mut out = Vec::new();
        for tool in self.all_tools()? {
            if let Some(ref slug) = filter.integration
                && tool.integration.as_str() != slug.as_str()
            {
                continue;
            }
            if let Some(owner) = filter.owner
                && tool.owner != owner
            {
                continue;
            }
            if let Some(ref conn) = filter.connection
                && tool.connection.as_str() != conn.as_str()
            {
                continue;
            }
            if let Some(ref q) = filter.query {
                let q = q.to_ascii_lowercase();
                let hay = format!("{} {}", tool.name, tool.description).to_ascii_lowercase();
                if !hay.contains(&q) {
                    continue;
                }
            }
            if !filter.include_blocked {
                let id = tool.cli_path();
                let effective = executor_core::effective_policy(
                    &id,
                    &policies,
                    executor_core::Owner::outer_rank,
                    tool.annotations
                        .as_ref()
                        .and_then(|a| a.requires_approval)
                        .unwrap_or(false),
                );
                if effective.action == executor_core::PolicyAction::Block {
                    continue;
                }
            }
            out.push(tool);
            if out.len() >= self.limits.max_search_results && filter.query.is_some() {
                break;
            }
        }
        Ok(out)
    }

    pub(crate) fn resolve_path(&self, path: &str) -> Result<Resolved, ExecutorError> {
        {
            let guard = self.static_tools.read();
            if let Some(tool) = guard.get(path).cloned() {
                return Ok(Resolved::Static(tool));
            }
        }
        if let Some(addr) = parse_tool_address(path) {
            return self
                .store
                .get_tool(&addr)?
                .map(Resolved::Dynamic)
                .ok_or_else(|| ExecutorError::missing_address(&addr));
        }
        let prefixed = format!("tools.{path}");
        if let Some(addr) = parse_tool_address(&prefixed)
            && let Some(tool) = self.store.get_tool(&addr)?
        {
            return Ok(Resolved::Dynamic(tool));
        }
        let tools = self.all_tools()?;
        let exact: Vec<Tool> = tools
            .iter()
            .filter(|t| t.cli_path() == path || t.address.to_string() == path)
            .cloned()
            .collect();
        if let [tool] = exact.as_slice() {
            return Ok(classify(tool.clone()));
        }
        let prefixed_matches: Vec<Tool> = tools
            .iter()
            .filter(|t| t.cli_path() == path || t.cli_path().starts_with(&format!("{path}.")))
            .cloned()
            .collect();
        if let [tool] = prefixed_matches.as_slice() {
            return Ok(classify(tool.clone()));
        }
        let suggestions: Vec<String> = tools
            .iter()
            .map(Tool::cli_path)
            .filter(|p| p.contains(path))
            .take(5)
            .collect();
        Err(ExecutorError::tool_not_found(path.to_owned(), suggestions))
    }
}

const fn classify(tool: Tool) -> Resolved {
    if tool.static_tool {
        Resolved::Static(tool)
    } else {
        Resolved::Dynamic(tool)
    }
}

/// A path resolved to either a built-in or a connection-backed tool.
pub enum Resolved {
    /// Built-in configurator.
    Static(Tool),
    /// Catalog tool with a connection address.
    Dynamic(Tool),
}

impl Resolved {
    pub const fn tool(&self) -> &Tool {
        match self {
            Self::Static(t) | Self::Dynamic(t) => t,
        }
    }
}
