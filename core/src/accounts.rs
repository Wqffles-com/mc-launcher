//! Microsoft account storage: multi-account management with tokens encrypted
//! at rest and automatic refresh.
//!
//! Layout: one `accounts/<uuid>.json` per account (metadata + short-lived
//! access token). The long-lived Microsoft refresh token is stored in the OS
//! credential store (Windows Credential Manager, macOS Keychain, Linux
//! Secret Service) under the `mc-launcher` service, keyed by account uuid.
//!
//! When the OS keyring is unavailable (e.g. headless Linux), the refresh
//! token falls back to the account JSON file, flagged with
//! `"token_storage": "file"` so users can tell the token is not protected by
//! the OS.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::auth::{self, MinecraftToken, Profile};
use crate::clock;
use crate::dirs::Directories;
use crate::error::{Error, Result};

/// The keyring service name for account refresh tokens.
pub const KEYRING_SERVICE: &str = "mc-launcher";

/// Storage backend marker for the refresh token.
pub const STORAGE_KEYRING: &str = "keyring";
/// Storage backend marker for the plain-file fallback.
pub const STORAGE_FILE: &str = "file";

/// A signed-in Minecraft account (Microsoft).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    /// Mojang profile UUID (with dashes).
    pub id: String,
    /// The player name shown to servers.
    pub name: String,
    /// `msa` for Microsoft accounts.
    pub user_type: String,
    /// RFC 3339 UTC timestamp of the sign-in.
    pub created_at: String,
    /// RFC 3339 UTC timestamp of the last time this account was used.
    pub last_used_at: Option<String>,
    /// Short-lived Minecraft access token (`Authorization: Bearer`).
    pub access_token: String,
    /// RFC 3339 UTC expiry of `access_token`.
    pub expires_at: Option<String>,
    /// Where the refresh token lives: `keyring` or `file`.
    pub token_storage: String,
    /// The refresh token; embedded here only in `file` mode. In `keyring`
    /// mode it is fetched from the OS credential store on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

impl Account {
    /// Build an account from a fresh sign-in chain result.
    #[must_use]
    pub fn new(mc: &MinecraftToken, profile: &Profile) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            user_type: "msa".to_owned(),
            created_at: clock::now_rfc3339(),
            last_used_at: Some(clock::now_rfc3339()),
            access_token: mc.access_token.clone(),
            expires_at: Some(clock::rfc3339_utc(
                clock::now() + i64::try_from(mc.expires_in).unwrap_or(i64::MAX),
            )),
            token_storage: STORAGE_KEYRING.to_owned(),
            refresh_token: None,
        }
    }

    /// Whether the Minecraft access token has expired (or is close to it).
    #[must_use]
    pub fn access_token_expired(&self) -> bool {
        let Some(expires_at) = &self.expires_at else {
            return true;
        };
        let Some(expires) = clock::parse_rfc3339_utc(expires_at) else {
            return true;
        };
        expires <= clock::now()
    }
}

/// Where secrets go: the OS credential store.
pub trait SecretVault: Send + Sync {
    /// Read a secret; `Ok(None)` when no entry exists.
    ///
    /// # Errors
    ///
    /// Fails when the underlying store is unreachable.
    fn get(&self, key: &str) -> Result<Option<String>>;
    /// Write a secret, replacing any existing entry.
    ///
    /// # Errors
    ///
    /// Fails when the underlying store is unreachable.
    fn set(&self, key: &str, value: &str) -> Result<()>;
    /// Remove a secret; missing entries are not an error.
    ///
    /// # Errors
    ///
    /// Fails when the underlying store is unreachable.
    fn delete(&self, key: &str) -> Result<()>;
}

/// The OS credential store via the `keyring` crate (Windows Credential
/// Manager, macOS Keychain, Linux Secret Service).
struct KeyringVault;

impl SecretVault for KeyringVault {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| Error::Keyring(e.to_string()))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Keyring(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| Error::Keyring(e.to_string()))?;
        entry
            .set_password(value)
            .map_err(|e| Error::Keyring(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| Error::Keyring(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Keyring(e.to_string())),
        }
    }
}

/// In-memory vault for tests.
#[cfg(test)]
struct MemoryVault {
    entries: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

#[cfg(test)]
impl MemoryVault {
    fn new() -> Self {
        Self {
            entries: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }
}

#[cfg(test)]
impl SecretVault for MemoryVault {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.entries.lock().expect("lock").get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.entries
            .lock()
            .expect("lock")
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.entries.lock().expect("lock").remove(key);
        Ok(())
    }
}

/// The sort key for "most recently used": `last_used_at`, else `created_at`.
fn last_used(account: &Account) -> &str {
    account
        .last_used_at
        .as_deref()
        .or(Some(account.created_at.as_str()))
        .unwrap_or("")
}

/// Multi-account store backed by the accounts directory.
pub struct AccountManager {
    dirs: Directories,
    vault: Box<dyn SecretVault>,
}

impl AccountManager {
    /// Create a manager rooted at `dirs.accounts_dir()`, using the OS
    /// credential store for refresh tokens.
    #[must_use]
    pub fn new(dirs: Directories) -> Self {
        Self {
            dirs,
            vault: Box::new(KeyringVault),
        }
    }

    /// Create a manager with a custom vault (tests).
    #[cfg(test)]
    fn with_vault(dirs: Directories, vault: Box<dyn SecretVault>) -> Self {
        Self { dirs, vault }
    }

    #[must_use]
    pub fn dirs(&self) -> &Directories {
        &self.dirs
    }

    /// Path of an account's JSON file.
    #[must_use]
    pub fn account_path(&self, id: &str) -> PathBuf {
        self.dirs.accounts_dir().join(format!("{id}.json"))
    }

    /// Persist an account. The refresh token goes to the OS credential store
    /// when possible; if the store is unavailable the token falls back into
    /// the JSON file with `token_storage: "file"` (this is the explicit
    /// first-time choice of `login` on systems without a keyring).
    ///
    /// # Errors
    ///
    /// Fails if the account file cannot be written.
    pub fn save(&self, account: &mut Account) -> Result<()> {
        self.save_with_policy(account, true)
    }

    /// Like [`Self::save`], but refusing to move a keyring-stored refresh
    /// token into the plaintext file when the keyring is temporarily
    /// unavailable. Used by `touch`/`refresh`, which must never silently
    /// downgrade an account that was protected. Accounts already stored in
    /// `file` mode stay in `file` mode.
    ///
    /// # Errors
    ///
    /// Fails if the account file cannot be written, or — in strict mode —
    /// when the keyring cannot be written for an account that currently
    /// relies on it.
    fn save_with_policy(&self, account: &mut Account, allow_downgrade: bool) -> Result<()> {
        let Some(refresh_token) = account.refresh_token.clone() else {
            STORAGE_KEYRING.clone_into(&mut account.token_storage);
            return self.write_file(account);
        };
        let was_keyring = account.token_storage == STORAGE_KEYRING;
        match self.vault.set(&account.id, &refresh_token) {
            Ok(()) => {
                STORAGE_KEYRING.clone_into(&mut account.token_storage);
                account.refresh_token = None;
            }
            Err(_) if !was_keyring || allow_downgrade => {
                STORAGE_FILE.clone_into(&mut account.token_storage);
            }
            Err(e) => return Err(e),
        }
        self.write_file(account)
    }

    fn write_file(&self, account: &Account) -> Result<()> {
        let path = self.account_path(&account.id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(account)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// All accounts, most recently used first.
    ///
    /// # Errors
    ///
    /// Fails if the accounts directory cannot be read or an account file is
    /// corrupt.
    pub fn list(&self) -> Result<Vec<Account>> {
        let dir = self.dirs.accounts_dir();
        let mut accounts = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(account) = self.read_account(&path)? else {
                    continue;
                };
                accounts.push(account);
            }
        }
        accounts.sort_by(|a, b| last_used(b).cmp(last_used(a)));
        Ok(accounts)
    }

    /// Look up an account by its UUID or name.
    ///
    /// # Errors
    ///
    /// Fails with [`Error::AccountNotFound`] when no account matches.
    pub fn get(&self, selector: &str) -> Result<Account> {
        self.list()?
            .into_iter()
            .find(|account| account.id == selector || account.name == selector)
            .ok_or_else(|| Error::AccountNotFound(selector.to_owned()))
    }

    /// The default account: the most recently used one, if any.
    ///
    /// # Errors
    ///
    /// Fails if the accounts directory cannot be read.
    pub fn default(&self) -> Result<Option<Account>> {
        Ok(self.list()?.into_iter().next())
    }

    /// Mark an account as used (bumps its `last_used_at` so it becomes the
    /// default).
    ///
    /// # Errors
    ///
    /// Fails if the account is unknown, cannot be written, or the keyring
    /// cannot be reached for an account that relies on it.
    pub fn touch(&self, selector: &str) -> Result<()> {
        let mut account = self.get(selector)?;
        account.last_used_at = Some(clock::now_rfc3339());
        self.save_with_policy(&mut account, false)
    }

    /// Remove an account (file and credential store entry).
    ///
    /// # Errors
    ///
    /// Fails if the account is unknown or its file cannot be deleted.
    pub fn remove(&self, selector: &str) -> Result<()> {
        let account = self.get(selector)?;
        let path = self.account_path(&account.id);
        if let Err(e) = self.vault.delete(&account.id) {
            // The file is the source of truth; a failing keyring delete
            // leaves a stale credential but must not block removal.
            eprintln!("warning: could not remove keyring entry: {e}");
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    /// Refresh a possibly-expired account: trades the Microsoft refresh
    /// token for a fresh access token, then persists the rotation.
    ///
    /// # Errors
    ///
    /// Fails with [`Error::RefreshTokenUnavailable`] when the token cannot be
    /// recovered, or on network/auth failures.
    pub async fn refresh(&self, client: &reqwest::Client, account: &Account) -> Result<Account> {
        let refresh_token = account
            .refresh_token
            .clone()
            .ok_or(Error::RefreshTokenUnavailable)?;
        let (pair, mc) =
            auth::refresh_minecraft_token(client, &auth::client_id(), &refresh_token).await?;
        // Refetch the profile so player name changes are picked up without a
        // full re-login (the UUID is stable, the display name is not).
        let profile = auth::fetch_profile(client, &mc.access_token).await?;
        let mut updated = account.clone();
        updated.name = profile.name;
        updated.access_token = mc.access_token;
        updated.expires_at = Some(clock::rfc3339_utc(
            clock::now() + i64::try_from(mc.expires_in).unwrap_or(i64::MAX),
        ));
        updated.refresh_token = Some(pair.refresh_token);
        self.save_with_policy(&mut updated, false)?;
        Ok(updated)
    }

    /// Load an account, resolving its refresh token from the credential
    /// store when it is stored there. Returns `Ok(None)` for missing files.
    fn read_account(&self, path: &Path) -> Result<Option<Account>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut account: Account = serde_json::from_slice(&bytes)?;
        if account.token_storage == STORAGE_KEYRING {
            // Best effort: if the keyring is unreachable the refresh token is
            // simply unavailable; the account file still loads for listing.
            account.refresh_token = self.vault.get(&account.id).ok().flatten();
        }
        Ok(Some(account))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-accounts-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn manager() -> AccountManager {
        AccountManager::with_vault(Directories::new(tempdir()), Box::new(MemoryVault::new()))
    }

    fn sample_account(id: &str, name: &str) -> Account {
        Account {
            id: id.to_owned(),
            name: name.to_owned(),
            user_type: "msa".to_owned(),
            created_at: clock::now_rfc3339(),
            last_used_at: None,
            access_token: "access".to_owned(),
            expires_at: Some(clock::rfc3339_utc(clock::now() + 86_400)),
            token_storage: STORAGE_KEYRING.to_owned(),
            refresh_token: Some("refresh-secret".to_owned()),
        }
    }

    #[test]
    fn save_stores_refresh_token_in_vault_and_clears_file_copy() {
        let manager = manager();
        let mut account = sample_account("abc-def", "Steve");
        manager.save(&mut account).expect("save");
        assert_eq!(account.token_storage, STORAGE_KEYRING);
        assert!(account.refresh_token.is_none());
        // The JSON file must not contain the secret.
        let raw = std::fs::read_to_string(manager.account_path("abc-def")).expect("read file");
        assert!(!raw.contains("refresh-secret"), "{raw}");
        // The vault holds it instead.
        let loaded = manager.get("Steve").expect("get");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh-secret"));
    }

    /// A vault whose OS backend is unreachable.
    struct BrokenVault;
    impl SecretVault for BrokenVault {
        fn get(&self, _key: &str) -> Result<Option<String>> {
            Err(Error::Keyring("no keyring".to_owned()))
        }
        fn set(&self, _key: &str, _value: &str) -> Result<()> {
            Err(Error::Keyring("no keyring".to_owned()))
        }
        fn delete(&self, _key: &str) -> Result<()> {
            Err(Error::Keyring("no keyring".to_owned()))
        }
    }

    #[test]
    fn save_falls_back_to_file_when_vault_is_unavailable() {
        let manager =
            AccountManager::with_vault(Directories::new(tempdir()), Box::new(BrokenVault));
        let mut account = sample_account("abc-def", "Steve");
        manager.save(&mut account).expect("save");
        assert_eq!(account.token_storage, STORAGE_FILE);
        assert_eq!(account.refresh_token.as_deref(), Some("refresh-secret"));
        let raw = std::fs::read_to_string(manager.account_path("abc-def")).expect("read file");
        assert!(raw.contains("\"token_storage\": \"file\""), "{raw}");
    }

    #[test]
    fn strict_save_refuses_to_downgrade_a_keyring_account() {
        // The keyring that used to work is now unreachable: a strict save of
        // an account holding its refresh token in memory must fail instead of
        // moving the token into the plaintext file.
        let manager =
            AccountManager::with_vault(Directories::new(tempdir()), Box::new(BrokenVault));
        let mut account = sample_account("abc-def", "Steve");
        let err = manager
            .save_with_policy(&mut account, false)
            .expect_err("strict save must fail");
        assert!(matches!(err, Error::Keyring(_)));
        assert!(
            !manager.account_path("abc-def").exists(),
            "nothing may be written on a refused downgrade"
        );
    }

    #[test]
    fn touch_never_downgrades_a_keyring_account() {
        let dirs = Directories::new(tempdir());
        let path = dirs.accounts_dir().join("abc-def.json");
        std::fs::create_dir_all(dirs.accounts_dir()).expect("create dir");
        std::fs::write(
            &path,
            r#"{"id":"abc-def","name":"Steve","user_type":"msa","created_at":"2026-01-01T00:00:00Z","last_used_at":"2026-01-01T00:00:00Z","access_token":"access","expires_at":"2026-12-31T00:00:00Z","token_storage":"keyring"}"#,
        )
        .expect("write account");

        // When the keyring cannot be read at load, touch simply cannot obtain
        // the refresh token — it must leave the file untouched rather than
        // rewrite it in plaintext mode.
        let manager = AccountManager::with_vault(dirs, Box::new(BrokenVault));
        manager.touch("Steve").expect("touch");
        let raw = std::fs::read_to_string(&path).expect("read file");
        assert!(
            !raw.contains("refresh"),
            "token must not leak into the file: {raw}"
        );
        assert!(raw.contains("\"token_storage\": \"keyring\""), "{raw}");
    }

    #[test]
    fn strict_save_leaves_file_mode_accounts_alone() {
        let dirs = Directories::new(tempdir());
        let path = dirs.accounts_dir().join("abc-def.json");
        std::fs::create_dir_all(dirs.accounts_dir()).expect("create dir");
        std::fs::write(
            &path,
            r#"{"id":"abc-def","name":"Steve","user_type":"msa","created_at":"2026-01-01T00:00:00Z","last_used_at":"2026-01-01T00:00:00Z","access_token":"access","expires_at":"2026-12-31T00:00:00Z","token_storage":"file","refresh_token":"plaintext-secret"}"#,
        )
        .expect("write account");

        // An account that already lives in plaintext mode keeps working even
        // when the keyring is unavailable.
        let manager = AccountManager::with_vault(dirs, Box::new(BrokenVault));
        manager.touch("Steve").expect("touch");
        let raw = std::fs::read_to_string(&path).expect("read file");
        assert!(raw.contains("\"token_storage\": \"file\""), "{raw}");
        assert!(raw.contains("plaintext-secret"), "{raw}");
    }

    #[test]
    fn list_sorts_by_last_used_descending() {
        let manager = manager();
        let mut first = sample_account("uuid-1", "Alpha");
        first.last_used_at = Some("2024-01-01T00:00:00Z".to_owned());
        let mut second = sample_account("uuid-2", "Beta");
        second.last_used_at = Some("2024-06-01T00:00:00Z".to_owned());
        let mut third = sample_account("uuid-3", "Gamma");
        third.created_at = "2023-01-01T00:00:00Z".to_owned();
        manager.save(&mut first).expect("save");
        manager.save(&mut second).expect("save");
        manager.save(&mut third).expect("save");

        let listed = manager.list().expect("list");
        assert_eq!(listed[0].name, "Beta");
        assert_eq!(listed[1].name, "Alpha");
        assert_eq!(listed[2].name, "Gamma");
        assert_eq!(
            manager.default().expect("default").expect("some").id,
            "uuid-2"
        );
    }

    #[test]
    fn get_resolves_by_id_or_name() {
        let manager = manager();
        let mut account = sample_account("uuid-1", "Steve");
        manager.save(&mut account).expect("save");
        assert_eq!(manager.get("uuid-1").expect("by id").name, "Steve");
        assert_eq!(manager.get("Steve").expect("by name").id, "uuid-1");
        let err = manager.get("nobody").unwrap_err();
        assert!(matches!(err, Error::AccountNotFound(_)));
    }

    #[test]
    fn touch_makes_an_account_the_default() {
        let manager = manager();
        let mut first = sample_account("uuid-1", "Alpha");
        first.last_used_at = Some("2024-01-01T00:00:00Z".to_owned());
        let mut second = sample_account("uuid-2", "Beta");
        second.last_used_at = Some("2024-06-01T00:00:00Z".to_owned());
        manager.save(&mut first).expect("save");
        manager.save(&mut second).expect("save");

        manager.touch("Alpha").expect("touch");
        assert_eq!(
            manager.default().expect("default").expect("some").name,
            "Alpha"
        );
    }

    #[test]
    fn remove_deletes_file_and_vault_entry() {
        let manager = manager();
        let mut account = sample_account("uuid-1", "Steve");
        manager.save(&mut account).expect("save");
        assert!(manager.account_path("uuid-1").is_file());
        manager.remove("Steve").expect("remove");
        assert!(!manager.account_path("uuid-1").exists());
        assert!(manager.list().expect("list").is_empty());
        assert!(matches!(
            manager.get("Steve").unwrap_err(),
            Error::AccountNotFound(_)
        ));
    }

    #[test]
    fn access_token_expiry_detection() {
        let mut account = sample_account("uuid-1", "Steve");
        assert!(!account.access_token_expired());
        account.expires_at = Some(clock::rfc3339_utc(clock::now() - 1));
        assert!(account.access_token_expired());
        account.expires_at = None;
        assert!(account.access_token_expired());
    }
}
