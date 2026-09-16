//! Local daemon bearer token (`server-control/auth.json`, mode `0600`).

use std::path::{Path, PathBuf};

use getrandom::getrandom;
use serde_json::{Value, json};

/// `{data_dir}/server-control/auth.json`.
#[must_use]
pub fn auth_json_path(data_dir: &Path) -> PathBuf {
    data_dir.join("server-control").join("auth.json")
}

/// Read a previously minted token.
#[must_use]
pub fn read_token(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(auth_json_path(data_dir)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn write_token(data_dir: &Path, token: &str) -> std::io::Result<()> {
    let dir = data_dir.join("server-control");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("auth.json");
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({ "token": token }))?
    );
    std::fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

fn mint() -> String {
    let mut bytes = [0u8; 32];
    getrandom(&mut bytes).unwrap_or(());
    hex::encode(bytes)
}

/// Load the stable local token, minting one on first call.
///
/// # Errors
///
/// IO.
pub fn load_or_mint(data_dir: &Path) -> std::io::Result<String> {
    load_or_mint_with(data_dir, None)
}

/// Load, mint, or overwrite with an operator-supplied `--auth-token`.
///
/// # Errors
///
/// IO.
pub fn load_or_mint_with(data_dir: &Path, override_token: Option<&str>) -> std::io::Result<String> {
    if let Some(token) = override_token.filter(|s| !s.is_empty()) {
        write_token(data_dir, token)?;
        return Ok(token.to_owned());
    }
    if let Some(existing) = read_token(data_dir) {
        return Ok(existing);
    }
    let token = mint();
    write_token(data_dir, &token)?;
    Ok(token)
}

/// Overwrite the token.
///
/// # Errors
///
/// IO.
pub fn rotate(data_dir: &Path) -> std::io::Result<String> {
    let token = mint();
    write_token(data_dir, &token)?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::{auth_json_path, load_or_mint};

    #[test]
    fn mints_stable_token_and_is_0600() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_mint(dir.path()).unwrap();
        let b = load_or_mint(dir.path()).unwrap();
        assert_eq!(a, b);
        assert!(!a.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(auth_json_path(dir.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
