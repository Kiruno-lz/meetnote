//! `devices.json`: paired devices, their preferences and the ledger of
//! recordings already imported. Device sync state, never a document fact.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::identity::DeviceIdentity;

pub const REGISTRY_SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    /// `sn:<serial>` for a verified device.
    pub key: String,
    pub identity: DeviceIdentity,
    /// Keys of the USB attachments ([`crate::AttachedDevice::key`]) this
    /// device was paired through: how a later wired connection is recognised
    /// without Bluetooth. A formatted card gets a new volume key, which
    /// pairing again adds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usb_links: Vec<String>,
    /// Where recordings are copied; `None` stores each inside its own
    /// document package in the workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_dir: Option<PathBuf>,
    #[serde(default = "enabled")]
    pub auto_process: bool,
    #[serde(default)]
    pub delete_after_import: bool,
    pub added_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_import: Option<ImportSummary>,
}

fn enabled() -> bool {
    true
}

impl PairedDevice {
    pub fn new(key: String, identity: DeviceIdentity, added_at_ms: u64) -> Self {
        Self {
            key,
            identity,
            usb_links: Vec::new(),
            import_dir: None,
            auto_process: true,
            delete_after_import: false,
            added_at_ms,
            last_import: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub finished_at_ms: u64,
    pub imported: usize,
    pub failed: usize,
}

/// One recording copied from a device: its place on the device's volume,
/// with the size and modification time that identify that version of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFile {
    pub device: String,
    pub path: String,
    pub size: u64,
    pub modified_s: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistry {
    pub schema: u32,
    #[serde(default)]
    pub devices: Vec<PairedDevice>,
    #[serde(default)]
    pub imported: Vec<ImportedFile>,
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self {
            schema: REGISTRY_SCHEMA,
            devices: Vec::new(),
            imported: Vec::new(),
        }
    }
}

impl DeviceRegistry {
    pub fn device(&self, key: &str) -> Option<&PairedDevice> {
        self.devices.iter().find(|device| device.key == key)
    }

    pub fn device_mut(&mut self, key: &str) -> Option<&mut PairedDevice> {
        self.devices.iter_mut().find(|device| device.key == key)
    }

    /// The paired device a USB attachment belongs to.
    pub fn linked(&self, attachment: &str) -> Option<&PairedDevice> {
        self.devices.iter().find(|device| {
            device.key == attachment || device.usb_links.iter().any(|link| link == attachment)
        })
    }

    /// Adds `device`, or refreshes the identity and USB links of the one with
    /// its key while keeping that one's preferences. A USB link belongs to
    /// one device only.
    pub fn pair(&mut self, device: PairedDevice) {
        for other in self
            .devices
            .iter_mut()
            .filter(|other| other.key != device.key)
        {
            other
                .usb_links
                .retain(|link| !device.usb_links.contains(link));
        }
        match self.device_mut(&device.key) {
            Some(existing) => {
                let mut identity = device.identity;
                identity.merge(&existing.identity);
                existing.identity = identity;
                for link in device.usb_links {
                    if !existing.usb_links.contains(&link) {
                        existing.usb_links.push(link);
                    }
                }
            }
            None => self.devices.push(device),
        }
    }

    /// Removes the device; its ledger goes with it, so pairing it again
    /// offers its recordings again. Imported documents are untouched.
    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|device| device.key != key);
        self.imported.retain(|file| file.device != key);
        self.devices.len() != before
    }

    pub fn ledger(&self, key: &str) -> impl Iterator<Item = &ImportedFile> {
        self.imported.iter().filter(move |file| file.device == key)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not a valid device registry: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{} has schema {found}; this MeetNote reads schema {REGISTRY_SCHEMA}", path.display())]
    Schema { path: PathBuf, found: u32 },
    #[error("cannot write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// The registry file. Every change runs under an exclusive lock and lands by
/// atomic replacement, so the UI and an import in progress never lose each
/// other's writes.
#[derive(Clone, Debug)]
pub struct RegistryStore {
    path: PathBuf,
}

impl RegistryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `devices.json` beside the settings file.
    pub fn beside_settings(settings: &Path) -> Self {
        Self::new(settings.with_file_name("devices.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<DeviceRegistry, RegistryError> {
        let _lock = self.lock()?;
        self.read()
    }

    /// Applies `change` to the current registry and saves the result.
    pub fn update<R>(
        &self,
        change: impl FnOnce(&mut DeviceRegistry) -> R,
    ) -> Result<(DeviceRegistry, R), RegistryError> {
        let _lock = self.lock()?;
        let mut registry = self.read()?;
        let result = change(&mut registry);
        self.write(&registry)?;
        Ok((registry, result))
    }

    fn lock(&self) -> Result<File, RegistryError> {
        let write = |source| RegistryError::Write {
            path: self.path.clone(),
            source,
        };
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(write)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("json.lock"))
            .map_err(write)?;
        lock.lock_exclusive().map_err(write)?;
        Ok(lock)
    }

    fn read(&self) -> Result<DeviceRegistry, RegistryError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DeviceRegistry::default());
            }
            Err(source) => {
                return Err(RegistryError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let registry: DeviceRegistry =
            serde_json::from_slice(&bytes).map_err(|source| RegistryError::Parse {
                path: self.path.clone(),
                source,
            })?;
        if registry.schema != REGISTRY_SCHEMA {
            return Err(RegistryError::Schema {
                path: self.path.clone(),
                found: registry.schema,
            });
        }
        Ok(registry)
    }

    fn write(&self, registry: &DeviceRegistry) -> Result<(), RegistryError> {
        let write = |source| RegistryError::Write {
            path: self.path.clone(),
            source,
        };
        let bytes = serde_json::to_vec_pretty(registry).map_err(|error| write(error.into()))?;
        let temporary = self.path.with_extension(format!(
            "json.tmp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let result = (|| {
            let mut file = File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(key: &str) -> PairedDevice {
        PairedDevice::new(
            key.into(),
            DeviceIdentity {
                name: "DJI Mic 3 TX".into(),
                serial: Some("9SFXN8N024XU81".into()),
                ..Default::default()
            },
            1,
        )
    }

    #[test]
    fn a_missing_file_is_an_empty_registry_and_changes_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = RegistryStore::beside_settings(&directory.path().join("settings.json"));
        assert_eq!(store.load().unwrap(), DeviceRegistry::default());
        store
            .update(|registry| {
                registry.pair(device("sn:A"));
                registry.imported.push(ImportedFile {
                    device: "sn:A".into(),
                    path: "DJI_001.WAV".into(),
                    size: 10,
                    modified_s: 5,
                    document: None,
                });
            })
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.devices, [device("sn:A")]);
        assert!(loaded.devices[0].auto_process);
        assert!(!loaded.devices[0].delete_after_import);
        assert_eq!(loaded.ledger("sn:A").count(), 1);
        assert_eq!(store.path(), directory.path().join("devices.json"));
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn pairing_again_keeps_preferences_and_removal_drops_the_ledger() {
        let mut registry = DeviceRegistry::default();
        registry.pair(device("sn:A"));
        registry.device_mut("sn:A").unwrap().import_dir = Some("/tmp/in".into());
        let mut again = device("sn:A");
        again.identity.firmware = Some("03.01.00.10".into());
        registry.pair(again);
        assert_eq!(registry.devices.len(), 1);
        let paired = registry.device("sn:A").unwrap();
        assert_eq!(paired.import_dir.as_deref(), Some(Path::new("/tmp/in")));
        assert_eq!(paired.identity.firmware.as_deref(), Some("03.01.00.10"));
        registry.imported.push(ImportedFile {
            device: "sn:A".into(),
            path: "a.wav".into(),
            size: 1,
            modified_s: 1,
            document: None,
        });
        assert!(registry.remove("sn:A"));
        assert!(registry.imported.is_empty());
        assert!(!registry.remove("sn:A"));
    }

    #[test]
    fn usb_links_find_the_device_and_move_on_pairing_again() {
        let mut registry = DeviceRegistry::default();
        let mut mic = device("sn:9SFXN8N024XU81");
        mic.usb_links = vec!["vol:OLD".into()];
        registry.pair(mic.clone());
        assert_eq!(
            registry.linked("vol:OLD").map(|d| d.key.as_str()),
            Some("sn:9SFXN8N024XU81")
        );
        assert_eq!(
            registry.linked("sn:9SFXN8N024XU81").map(|d| d.key.as_str()),
            Some("sn:9SFXN8N024XU81")
        );
        assert!(registry.linked("vol:OTHER").is_none());

        // The card was formatted: pairing again adds the new volume.
        mic.usb_links = vec!["vol:NEW".into()];
        registry.pair(mic);
        assert_eq!(registry.devices.len(), 1);
        assert_eq!(registry.devices[0].usb_links, ["vol:OLD", "vol:NEW"]);

        // Another unit paired through the same volume takes the link over.
        let mut other = device("sn:OTHER0000000001");
        other.usb_links = vec!["vol:NEW".into()];
        registry.pair(other);
        assert_eq!(registry.devices[0].usb_links, ["vol:OLD"]);
        assert_eq!(
            registry.linked("vol:NEW").map(|d| d.key.as_str()),
            Some("sn:OTHER0000000001")
        );
    }

    #[test]
    fn rejects_unknown_schemas_and_invalid_json_with_the_path() {
        let directory = tempfile::tempdir().unwrap();
        let store = RegistryStore::new(directory.path().join("devices.json"));
        fs::write(store.path(), r#"{"schema": 9}"#).unwrap();
        let error = store.load().unwrap_err().to_string();
        assert!(
            error.contains("schema 9") && error.contains("devices.json"),
            "{error}"
        );
        fs::write(store.path(), "{").unwrap();
        assert!(matches!(store.load(), Err(RegistryError::Parse { .. })));
        assert!(
            store.update(|_| ()).is_err(),
            "a broken file is never overwritten"
        );
        assert_eq!(fs::read_to_string(store.path()).unwrap(), "{");
    }
}
