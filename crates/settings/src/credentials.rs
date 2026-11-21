//! `credentials.json` beside `settings.json`: each provider's API key or
//! account sign-in. Owner-only, replaced atomically under its own lock, and
//! never quoted in an error or a `Debug` output.

use crate::{lock_beside, write_atomically, SettingsError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const CREDENTIALS_SCHEMA_VERSION: &str = "meetnote.credentials.v1";
const FILE_NAME: &str = "credentials.json";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProviderCredential {
    ApiKey { key: String },
    ChatGpt(ChatGptAccount),
}

/// A ChatGPT sign-in, as `llm::vendors::oauth::chatgpt::ChatGptTokens`
/// carries it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatGptAccount {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub account_id: String,
    pub email: String,
    pub plan: Option<String>,
    /// Unix seconds; 0 when unknown.
    pub expires_at: u64,
    pub subscription_until: Option<u64>,
}

impl ProviderCredential {
    /// Every secret the credential holds, for scrubbing messages.
    pub fn secrets(&self) -> Vec<&str> {
        match self {
            Self::ApiKey { key } => vec![key],
            Self::ChatGpt(account) => vec![
                &account.access_token,
                &account.refresh_token,
                &account.id_token,
            ],
        }
    }
}

impl fmt::Debug for ProviderCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey { .. } => formatter.write_str("ApiKey(<redacted>)"),
            Self::ChatGpt(account) => formatter
                .debug_struct("ChatGpt")
                .field("account_id", &account.account_id)
                .field("email", &account.email)
                .field("plan", &account.plan)
                .field("expires_at", &account.expires_at)
                .field("subscription_until", &account.subscription_until)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialsFile {
    schema_version: String,
    providers: BTreeMap<String, ProviderCredential>,
}

#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    /// The store in the directory that holds `settings_path`.
    pub fn beside(settings_path: impl AsRef<Path>) -> Self {
        Self {
            path: settings_path.as_ref().with_file_name(FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, provider: &str) -> Result<Option<ProviderCredential>, SettingsError> {
        let _lock = lock_beside(&self.path)?;
        Ok(self.load()?.providers.remove(provider))
    }

    /// Stores `credential` for `provider`, or removes it when `None`.
    pub fn set(
        &self,
        provider: &str,
        credential: Option<ProviderCredential>,
    ) -> Result<(), SettingsError> {
        self.update(provider, |_| Ok::<_, SettingsError>(credential))
            .map(drop)
    }

    /// Reads `provider`'s credential, replaces it with what `change` returns
    /// and writes the file only when it changed, all under the store's lock:
    /// a token refresh persists its rotation before anyone can read the old
    /// tokens again.
    pub fn update<E: From<SettingsError>>(
        &self,
        provider: &str,
        change: impl FnOnce(Option<ProviderCredential>) -> Result<Option<ProviderCredential>, E>,
    ) -> Result<Option<ProviderCredential>, E> {
        let _lock = lock_beside(&self.path)?;
        let mut file = self.load()?;
        let current = file.providers.get(provider).cloned();
        let next = change(current.clone())?;
        if next != current {
            match next.clone() {
                Some(credential) => file.providers.insert(provider.to_owned(), credential),
                None => file.providers.remove(provider),
            };
            file.schema_version = CREDENTIALS_SCHEMA_VERSION.into();
            let content = serde_json::to_vec_pretty(&file).map_err(SettingsError::from)?;
            write_atomically(&self.path, &content)?;
        }
        Ok(next)
    }

    fn load(&self) -> Result<CredentialsFile, SettingsError> {
        let read_error = |source| SettingsError::Read {
            path: self.path.display().to_string(),
            source,
        };
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CredentialsFile::default());
            }
            Err(error) => return Err(read_error(error)),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(SettingsError::Invalid(format!(
                    "{} is readable by other users; make it owner-only (chmod 600)",
                    self.path.display()
                )));
            }
        }
        let _ = metadata;
        let content = fs::read(&self.path).map_err(read_error)?;
        // serde's messages may quote the offending value, which can be a
        // secret: only the position is reported.
        let file: CredentialsFile = serde_json::from_slice(&content).map_err(|error| {
            SettingsError::Invalid(format!(
                "{} is not a valid credentials file (line {}, column {})",
                self.path.display(),
                error.line(),
                error.column()
            ))
        })?;
        if file.schema_version != CREDENTIALS_SCHEMA_VERSION {
            return Err(SettingsError::Invalid(format!(
                "{} has an unsupported schema version",
                self.path.display()
            )));
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempdir::Dir, CredentialStore) {
        let directory = tempdir::Dir::new();
        let store = CredentialStore::beside(directory.0.join("settings.json"));
        (directory, store)
    }

    /// A directory removed when dropped.
    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);

        impl Dir {
            pub fn new() -> Self {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "meetnote-credentials-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn account() -> ChatGptAccount {
        ChatGptAccount {
            access_token: "access-secret".into(),
            refresh_token: "refresh-secret".into(),
            id_token: "id-secret".into(),
            account_id: "acct-1".into(),
            email: "user@example.com".into(),
            plan: Some("plus".into()),
            expires_at: 1_790_000_000,
            subscription_until: None,
        }
    }

    #[test]
    fn credentials_round_trip_in_the_documented_format() {
        let (_directory, store) = store();
        assert_eq!(store.get("deepseek").unwrap(), None);
        store
            .set(
                "deepseek",
                Some(ProviderCredential::ApiKey {
                    key: "sk-deepseek".into(),
                }),
            )
            .unwrap();
        store
            .set("openai", Some(ProviderCredential::ChatGpt(account())))
            .unwrap();

        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "schemaVersion": "meetnote.credentials.v1",
                "providers": {
                    "deepseek": {"type": "apiKey", "key": "sk-deepseek"},
                    "openai": {
                        "type": "chatGpt", "accessToken": "access-secret",
                        "refreshToken": "refresh-secret", "idToken": "id-secret",
                        "accountId": "acct-1", "email": "user@example.com", "plan": "plus",
                        "expiresAt": 1_790_000_000_u64, "subscriptionUntil": null
                    }
                }
            })
        );
        assert_eq!(
            store.get("openai").unwrap(),
            Some(ProviderCredential::ChatGpt(account()))
        );
        store.set("deepseek", None).unwrap();
        assert_eq!(store.get("deepseek").unwrap(), None);
        assert!(store.get("openai").unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only_and_a_readable_one_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let (_directory, store) = store();
        store
            .set(
                "deepseek",
                Some(ProviderCredential::ApiKey {
                    key: "sk-private".into(),
                }),
            )
            .unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644)).unwrap();
        let error = store.get("deepseek").unwrap_err().to_string();
        assert!(error.contains("owner-only"), "{error}");
        assert!(!error.contains("sk-private"));
        assert!(store.set("deepseek", None).is_err());
    }

    #[test]
    fn a_malformed_file_is_reported_without_its_content() {
        let (_directory, store) = store();
        fs::write(
            store.path(),
            r#"{"schemaVersion":"meetnote.credentials.v1","providers":{"x":{"type":"chatGpt","expiresAt":"sk-leak"}}}"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let error = store.get("x").unwrap_err().to_string();
        assert!(error.contains("line 1"), "{error}");
        assert!(!error.contains("sk-leak"), "{error}");
    }

    #[test]
    fn debug_output_redacts_every_secret() {
        let debug = format!(
            "{:?} {:?}",
            ProviderCredential::ApiKey {
                key: "sk-visible".into()
            },
            ProviderCredential::ChatGpt(account())
        );
        for secret in ["sk-visible", "access-secret", "refresh-secret", "id-secret"] {
            assert!(!debug.contains(secret), "{debug}");
        }
        assert!(debug.contains("user@example.com"));
    }

    #[test]
    fn concurrent_updates_serialize_under_the_lock() {
        let (_directory, store) = store();
        let threads = (0..8)
            .map(|index| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .set(
                            &format!("p{index}"),
                            Some(ProviderCredential::ApiKey {
                                key: format!("k{index}"),
                            }),
                        )
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        for index in 0..8 {
            assert!(store.get(&format!("p{index}")).unwrap().is_some());
        }
    }

    #[test]
    fn an_unchanged_update_does_not_write() {
        let (_directory, store) = store();
        store.update("p", |_| Ok::<_, SettingsError>(None)).unwrap();
        assert!(!store.path().exists());
    }
}
