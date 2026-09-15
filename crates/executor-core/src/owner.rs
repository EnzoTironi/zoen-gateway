//! Placement of a catalog record: org-shared or user-owned.

use std::fmt::{Display, Formatter, Result as FmtResult};

use serde::{Deserialize, Serialize};

/// Who owns a connection or policy. The `<owner>` segment of a tool address.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    /// Tenant-shared. Everyone in the partition uses it.
    Org,
    /// This subject's own credential or rule.
    User,
}

impl Owner {
    /// Parse `org` or `user`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "org" => Some(Self::Org),
            "user" => Some(Self::User),
            _ => None,
        }
    }

    /// Wire form used in addresses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Org => "org",
            Self::User => "user",
        }
    }

    /// Outer-wins rank: org is the guardrail, user is inner.
    #[must_use]
    pub const fn outer_rank(self) -> u8 {
        match self {
            Self::User => 0,
            Self::Org => 1,
        }
    }
}

impl Display for Owner {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(self.as_str())
    }
}
