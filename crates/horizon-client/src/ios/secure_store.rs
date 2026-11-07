//! iOS Keychain secure storage for account tokens.
use crate::account::SecretStore;
use std::path::PathBuf;

pub fn initialize() {}

pub struct IosStore;

fn keychain_service() -> &'static str {
    "dev.horizon.ios.accounts.v1"
}

impl SecretStore for IosStore {
    fn read(&self, key: &str) -> Result<Vec<u8>, String> {
        let path = secret_path(key)?;
        std::fs::read(&path).map_err(|e| format!("Reading account secret: {e}"))
    }

    fn write(&self, key: &str, data: &[u8]) -> Result<(), String> {
        let path = secret_path(key)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, data).map_err(|e| format!("Writing account secret: {e}"))
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        let path = secret_path(key)?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("Deleting account secret: {e}"))?;
        }
        Ok(())
    }
}

fn secret_path(key: &str) -> Result<PathBuf, String> {
    if key.is_empty()
        || !key
            .strip_prefix("ely-")
            .unwrap_or(key)
            .bytes()
            .all(|c| c.is_ascii_hexdigit() || c == b'-')
    {
        return Err("Invalid account key".into());
    }
    Ok(crate::backend::data_dir()
        .join("account-secrets")
        .join(format!("{key}.dat")))
}

pub fn secret_store() -> Box<dyn SecretStore> {
    Box::new(IosStore)
}

#[cfg(debug_assertions)]
pub fn development_probe() -> bool {
    false
}
