//! Account secrets in the iOS Keychain: Horizon's alone, kept on this device only, readable after
//! its first unlock (Android encrypts them with a Keystore key). Never in the Documents folder,
//! which the Files app shows.
use crate::account::SecretStore;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
const SERVICE: &str = "dev.horizon.game.accounts";

fn account(key: &str) -> Result<&str, String> {
    if key.is_empty()
        || !key
            .strip_prefix("ely-")
            .unwrap_or(key)
            .bytes()
            .all(|c| c.is_ascii_hexdigit() || c == b'-')
    {
        return Err("Invalid account key".into());
    }
    Ok(key)
}

pub struct IosStore;

impl SecretStore for IosStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let _guard = LOCK.lock().map_err(|_| "Account storage unavailable")?;
        match horizon_gamecontroller::keychain_get(SERVICE, account(key)?) {
            Ok(Some(bytes)) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "Invalid saved account".into()),
            Ok(None) => Ok(None),
            Err(status) => Err(format!("Cannot read saved account (Keychain {status})")),
        }
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let _guard = LOCK.lock().map_err(|_| "Account storage unavailable")?;
        horizon_gamecontroller::keychain_set(SERVICE, account(key)?, secret.as_bytes())
            .map_err(|status| format!("Cannot save account (Keychain {status})"))
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        let _guard = LOCK.lock().map_err(|_| "Account storage unavailable")?;
        horizon_gamecontroller::keychain_delete(SERVICE, account(key)?)
            .map_err(|status| format!("Cannot remove saved account (Keychain {status})"))
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// Android's development probe exercises its Keystore across two launches; the Keychain needs no
/// such check, so a development build starts normally.
#[cfg(debug_assertions)]
pub fn development_probe() -> bool {
    false
}
