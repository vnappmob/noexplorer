//! Account secret keys in the OS credential store: macOS Keychain, Windows Credential Manager,
//! Secret Service (GNOME Keyring / KWallet) on Linux. See ADR-020.
//!
//! config.json only keeps a `keychain: true` flag per profile. When the store is unavailable
//! (e.g. a minimal Linux box without Secret Service), the secret stays in the 0600 file as before.

use std::sync::OnceLock;

use tauri::AppHandle;

/// The app identifier, set once at startup (`init`), so providers can reach the store without an
/// `AppHandle` (Box rotates its refresh token on every refresh and must save the new one).
static SERVICE: OnceLock<String> = OnceLock::new();

pub fn init(app: &AppHandle) {
    let _ = SERVICE.set(app.config().identifier.clone());
}

/// Where secrets go. A trait so the config logic can be unit-tested with an in-memory store.
pub trait SecretStore {
    fn get(&self, id: &str) -> Option<String>;
    /// True when the secret was stored.
    fn set(&self, id: &str, secret: &str) -> bool;
    fn delete(&self, id: &str);
}

/// The OS store. Entries are `service = <app identifier>`, `account = <profile id>`, so dev builds
/// (run with the `….dev` identifier) never read or overwrite the installed app's entries.
pub struct OsStore {
    service: String,
}

impl OsStore {
    pub fn new(app: &AppHandle) -> Self {
        Self { service: app.config().identifier.clone() }
    }

    /// The store for this app, once `init` ran (None in unit tests).
    pub fn global() -> Option<Self> {
        SERVICE.get().map(|s| Self { service: s.clone() })
    }

    fn entry(&self, id: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(&self.service, id).ok()
    }
}

impl SecretStore for OsStore {
    fn get(&self, id: &str) -> Option<String> {
        self.entry(id)?.get_password().ok()
    }

    fn set(&self, id: &str, secret: &str) -> bool {
        self.entry(id).is_some_and(|e| e.set_password(secret).is_ok())
    }

    fn delete(&self, id: &str) {
        if let Some(e) = self.entry(id) {
            let _ = e.delete_credential();
        }
    }
}
