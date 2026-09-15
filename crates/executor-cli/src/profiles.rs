//! Named remote server profiles (`~/.executor/server-connections.json`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// One saved connection profile.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Profile {
    /// Profile name.
    pub name: String,
    /// Connection.
    pub connection: Connection,
}

/// HTTP origin + optional auth.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Connection {
    /// Origin URL.
    pub origin: String,
    /// Auth block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
}

/// Stored credential.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Auth {
    /// Static bearer.
    Bearer {
        /// Token.
        token: String,
    },
    /// Device-login OAuth.
    Oauth {
        /// Access token.
        #[serde(rename = "accessToken")]
        access_token: String,
        /// Refresh token.
        #[serde(
            rename = "refreshToken",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        refresh_token: Option<String>,
        /// Email for `whoami`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        email: Option<String>,
    },
}

/// Versioned on-disk store.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Store {
    /// Schema version.
    pub version: u32,
    /// Default profile name.
    #[serde(rename = "defaultProfile")]
    pub default_profile: Option<String>,
    /// Profiles.
    pub profiles: Vec<Profile>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            version: 1,
            default_profile: None,
            profiles: Vec::new(),
        }
    }
}

/// Validate a profile name.
///
/// # Errors
///
/// Illegal characters.
pub fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(
            "Server profile names may contain only letters, numbers, dots, underscores, and dashes."
                .into(),
        );
    }
    Ok(trimmed.to_owned())
}

/// Path of the JSON store.
#[must_use]
pub fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join("server-connections.json")
}

/// Load the store, or empty if missing.
///
/// # Errors
///
/// Illegal JSON.
pub fn load(data_dir: &Path) -> Result<Store, String> {
    let path = store_path(data_dir);
    if !path.is_file() {
        return Ok(Store::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Persist owner-only.
///
/// # Errors
///
/// IO.
pub fn save(data_dir: &Path, store: &Store) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let path = store_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(store).unwrap_or_else(|_| json!({}).to_string())
    );
    fs::write(&tmp, body).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&tmp).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&tmp, perms).map_err(|e| e.to_string())?;
    }
    fs::rename(tmp, path).map_err(|e| e.to_string())
}

/// Upsert a profile.
///
/// # Errors
///
/// Name / IO.
pub fn upsert(
    data_dir: &Path,
    name: &str,
    origin: &str,
    auth: Option<Auth>,
    make_default: bool,
) -> Result<Store, String> {
    let name = validate_name(name)?;
    let mut store = load(data_dir)?;
    store.profiles.retain(|p| p.name != name);
    store.profiles.push(Profile {
        name: name.clone(),
        connection: Connection {
            origin: origin.trim_end_matches('/').to_owned(),
            auth,
        },
    });
    store.profiles.sort_by(|a, b| a.name.cmp(&b.name));
    if make_default || store.default_profile.is_none() {
        store.default_profile = Some(name);
    }
    save(data_dir, &store)?;
    Ok(store)
}

/// Select the default profile.
///
/// # Errors
///
/// Missing name.
pub fn set_default(data_dir: &Path, name: &str) -> Result<Store, String> {
    let name = validate_name(name)?;
    let mut store = load(data_dir)?;
    if !store.profiles.iter().any(|p| p.name == name) {
        return Err(format!("No server profile named \"{name}\"."));
    }
    store.default_profile = Some(name);
    save(data_dir, &store)?;
    Ok(store)
}

/// Remove a profile.
///
/// # Errors
///
/// IO.
pub fn remove(data_dir: &Path, name: &str) -> Result<Store, String> {
    let name = validate_name(name)?;
    let mut store = load(data_dir)?;
    store.profiles.retain(|p| p.name != name);
    if store.default_profile.as_deref() == Some(name.as_str()) {
        store.default_profile = None;
    }
    save(data_dir, &store)?;
    Ok(store)
}

/// Active profile (default, else first).
#[must_use]
pub fn active(store: &Store) -> Option<&Profile> {
    store
        .default_profile
        .as_ref()
        .and_then(|n| store.profiles.iter().find(|p| &p.name == n))
        .or_else(|| store.profiles.first())
}

#[cfg(test)]
mod tests {
    use super::{upsert, validate_name};

    #[test]
    fn rejects_spaces() {
        assert!(validate_name("bad name").is_err());
        assert_eq!(validate_name("prod").unwrap(), "prod");
    }

    #[test]
    fn roundtrip_profile() {
        let dir = tempfile::tempdir().unwrap();
        let store = upsert(dir.path(), "cloud", "https://example.test", None, true).unwrap();
        assert_eq!(store.default_profile.as_deref(), Some("cloud"));
        assert_eq!(store.profiles[0].connection.origin, "https://example.test");
    }
}
