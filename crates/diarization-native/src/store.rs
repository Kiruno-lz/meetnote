use crate::{SpeakerLibraryDocument, SpeakerLibraryError};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub trait SpeakerProfileStore: Send + Sync {
    fn load(&self) -> Result<SpeakerLibraryDocument, SpeakerLibraryError>;
    fn replace(&self, document: &SpeakerLibraryDocument) -> Result<(), SpeakerLibraryError>;
}

pub struct JsonSpeakerProfileStore {
    path: PathBuf,
    io_lock: Mutex<()>,
}

impl JsonSpeakerProfileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            io_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SpeakerProfileStore for JsonSpeakerProfileStore {
    fn load(&self) -> Result<SpeakerLibraryDocument, SpeakerLibraryError> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(SpeakerLibraryDocument::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn replace(&self, document: &SpeakerLibraryDocument) -> Result<(), SpeakerLibraryError> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec_pretty(document)?;
        let temporary = parent.join(format!(
            ".{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("speaker-library"),
            uuid::Uuid::new_v4()
        ));
        let write_result = (|| -> Result<(), SpeakerLibraryError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            if let Ok(directory) = fs::File::open(parent) {
                let _ = directory.sync_all();
            }
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }
}

#[derive(Default)]
pub struct InMemorySpeakerProfileStore {
    document: Mutex<SpeakerLibraryDocument>,
}

impl SpeakerProfileStore for InMemorySpeakerProfileStore {
    fn load(&self) -> Result<SpeakerLibraryDocument, SpeakerLibraryError> {
        self.document
            .lock()
            .map(|document| document.clone())
            .map_err(|_| SpeakerLibraryError::LockPoisoned)
    }

    fn replace(&self, document: &SpeakerLibraryDocument) -> Result<(), SpeakerLibraryError> {
        *self
            .document
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)? = document.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_store_roundtrips_and_leaves_no_temporary_file() {
        let root = std::env::temp_dir().join(format!("meetnote-speakers-{}", uuid::Uuid::new_v4()));
        let path = root.join("library.json");
        let store = JsonSpeakerProfileStore::new(&path);
        let document = SpeakerLibraryDocument::default();

        store.replace(&document).unwrap();

        assert_eq!(store.load().unwrap(), document);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_json_store_starts_empty_without_writing() {
        let path = std::env::temp_dir().join(format!("missing-{}.json", uuid::Uuid::new_v4()));
        let store = JsonSpeakerProfileStore::new(&path);

        assert!(store.load().unwrap().profiles.is_empty());
        assert!(!path.exists());
    }
}
