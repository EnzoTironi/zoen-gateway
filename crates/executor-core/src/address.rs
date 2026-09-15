//! Tool and connection addresses.
//!
//! Callable tool: `tools.<integration>.<owner>.<connection>.<tool>`.
//! The four leading segments never contain `.`. The tool remainder may.

use std::fmt::{Display, Formatter, Result as FmtResult};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{ConnectionName, IntegrationSlug, InvalidId, Owner, ToolName};

const PREFIX: &str = "tools";

/// `tools.<integration>.<owner>.<connection>`.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ConnectionAddress(String);

impl ConnectionAddress {
    /// Borrow the wire form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ConnectionAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(&self.0)
    }
}

/// Full callable address. Internally structured so illegal shapes cannot be built.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ToolAddress {
    integration: IntegrationSlug,
    owner: Owner,
    connection: ConnectionName,
    tool: ToolName,
}

impl ToolAddress {
    /// Assemble from validated parts.
    #[must_use]
    pub const fn new(
        integration: IntegrationSlug,
        owner: Owner,
        connection: ConnectionName,
        tool: ToolName,
    ) -> Self {
        Self {
            integration,
            owner,
            connection,
            tool,
        }
    }

    /// Integration slug.
    #[must_use]
    pub const fn integration(&self) -> &IntegrationSlug {
        &self.integration
    }

    /// Owner segment.
    #[must_use]
    pub const fn owner(&self) -> Owner {
        self.owner
    }

    /// Connection name.
    #[must_use]
    pub const fn connection(&self) -> &ConnectionName {
        &self.connection
    }

    /// Tool name (may contain dots).
    #[must_use]
    pub const fn tool(&self) -> &ToolName {
        &self.tool
    }

    /// Sandbox path: address without the `tools.` prefix.
    #[must_use]
    pub fn sandbox_path(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            self.integration, self.owner, self.connection, self.tool
        )
    }
}

impl Display for ToolAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "{PREFIX}.{}.{}.{}.{}",
            self.integration, self.owner, self.connection, self.tool
        )
    }
}

impl Serialize for ToolAddress {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ToolAddress {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

impl FromStr for ToolAddress {
    type Err = InvalidId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_tool_address(s).ok_or_else(|| {
            InvalidId::new(
                "tool address",
                s,
                "expected tools.<integration>.<owner>.<connection>.<tool>",
            )
        })
    }
}

/// Parsed form of a well-shaped address (same as [`ToolAddress`] parts).
pub type ParsedToolAddress = ToolAddress;

/// Parse `tools.<integration>.<owner>.<connection>.<tool>`.
///
/// Walks to the 4th `.`; everything after is the tool name.
#[must_use]
pub fn parse_tool_address(address: &str) -> Option<ToolAddress> {
    let mut cut = None;
    let mut from = 0;
    for _ in 0..4 {
        let rel = address[from..].find('.')?;
        cut = Some(from + rel);
        from += rel + 1;
    }
    let cut = cut?;
    let head = &address[..cut];
    let tool = &address[cut + 1..];
    let mut segs = head.split('.');
    let prefix = segs.next()?;
    let integration = segs.next()?;
    let owner = segs.next()?;
    let connection = segs.next()?;
    if segs.next().is_some() || prefix != PREFIX {
        return None;
    }
    let owner = Owner::parse(owner)?;
    if integration.is_empty() || connection.is_empty() || tool.is_empty() {
        return None;
    }
    Some(ToolAddress {
        integration: IntegrationSlug::new(integration).ok()?,
        owner,
        connection: ConnectionName::new(connection).ok()?,
        tool: ToolName::new(tool).ok()?,
    })
}

/// `tools.<integration>.<owner>.<connection>`.
#[must_use]
pub fn connection_address(
    owner: Owner,
    integration: &IntegrationSlug,
    connection: &ConnectionName,
) -> ConnectionAddress {
    ConnectionAddress(format!("{PREFIX}.{integration}.{owner}.{connection}"))
}

/// Full tool address from parts.
#[must_use]
pub const fn tool_address(
    owner: Owner,
    integration: IntegrationSlug,
    connection: ConnectionName,
    tool: ToolName,
) -> ToolAddress {
    ToolAddress::new(integration, owner, connection, tool)
}

#[cfg(test)]
mod tests {
    use super::{PREFIX, parse_tool_address};

    #[test]
    fn parses_dotted_tool_remainder() {
        let parsed = parse_tool_address("tools.vercel.org.prod.aliases.deleteAlias").unwrap();
        assert_eq!(parsed.integration().as_str(), "vercel");
        assert_eq!(parsed.owner().as_str(), "org");
        assert_eq!(parsed.connection().as_str(), "prod");
        assert_eq!(parsed.tool().as_str(), "aliases.deleteAlias");
        assert_eq!(
            parsed.to_string(),
            "tools.vercel.org.prod.aliases.deleteAlias"
        );
        assert_eq!(PREFIX, "tools");
    }

    #[test]
    fn rejects_bad_owner() {
        assert!(parse_tool_address("tools.vercel.team.prod.list").is_none());
    }

    #[test]
    fn rejects_missing_tool() {
        assert!(parse_tool_address("tools.vercel.org.prod").is_none());
        assert!(parse_tool_address("tools.vercel.org.prod.").is_none());
    }
}
