use contracts::{ComputeDevice, ModelRef};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

const SETTINGS_SCHEMA_VERSION: &str = "meetnote.settings.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: String,
    pub asr_backend_id: Option<String>,
    pub asr_model: Option<ModelRef>,
    pub diarization_backend_id: Option<String>,
    pub diarization_model: Option<ModelRef>,
    pub compute_device: ComputeDevice,
    pub language: Option<String>,
    pub expected_speakers: Option<u32>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION.into(),
            asr_backend_id: None,
            asr_model: None,
            diarization_backend_id: None,
            diarization_model: None,
            compute_device: ComputeDevice::Auto,
            language: None,
            expected_speakers: None,
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::Invalid(format!(
                "unsupported settings schema version: {}",
                self.schema_version
            )));
        }
        if self.asr_backend_id.as_deref() == Some("")
            || self.diarization_backend_id.as_deref() == Some("")
        {
            return Err(SettingsError::Invalid(
                "backend ids must be non-empty when configured".into(),
            ));
        }
        if let (Some(backend_id), Some(model)) = (&self.asr_backend_id, &self.asr_model) {
            if backend_id != &model.backend_id {
                return Err(SettingsError::Invalid(
                    "ASR backend id does not match ASR model backend id".into(),
                ));
            }
        }
        if let (Some(backend_id), Some(model)) =
            (&self.diarization_backend_id, &self.diarization_model)
        {
            if backend_id != &model.backend_id {
                return Err(SettingsError::Invalid(
                    "diarization backend id does not match diarization model backend id".into(),
                ));
            }
        }
        if self.expected_speakers == Some(0) {
            return Err(SettingsError::Invalid(
                "expected speakers must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("failed to read settings {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to write settings {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse settings: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("invalid settings: {0}")]
    Invalid(String),
    #[error("settings lock poisoned")]
    LockPoisoned,
}

pub trait SettingsRepository: Send + Sync {
    fn load(&self) -> Result<AppSettings, SettingsError>;
    fn save(&self, settings: &AppSettings) -> Result<(), SettingsError>;
}

#[derive(Debug, Clone)]
pub struct JsonSettingsRepository {
    path: PathBuf,
}

impl JsonSettingsRepository {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SettingsRepository for JsonSettingsRepository {
    fn load(&self) -> Result<AppSettings, SettingsError> {
        if !self.path.is_file() {
            return Ok(AppSettings::default());
        }
        let content = fs::read_to_string(&self.path).map_err(|source| SettingsError::Read {
            path: self.path.display().to_string(),
            source,
        })?;
        let settings: AppSettings = serde_json::from_str(&content)?;
        settings.validate()?;
        Ok(settings)
    }

    fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| SettingsError::Write {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let content = serde_json::to_vec_pretty(settings)?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, content).map_err(|source| SettingsError::Write {
            path: temporary.display().to_string(),
            source,
        })?;
        fs::rename(&temporary, &self.path).map_err(|source| SettingsError::Write {
            path: self.path.display().to_string(),
            source,
        })
    }
}

#[derive(Clone)]
pub struct SettingsService {
    repository: Arc<dyn SettingsRepository>,
    current: Arc<RwLock<AppSettings>>,
}

impl SettingsService {
    pub fn open(repository: Arc<dyn SettingsRepository>) -> Result<Self, SettingsError> {
        let settings = repository.load()?;
        Ok(Self {
            repository,
            current: Arc::new(RwLock::new(settings)),
        })
    }

    pub fn snapshot(&self) -> Result<AppSettings, SettingsError> {
        self.current
            .read()
            .map_err(|_| SettingsError::LockPoisoned)
            .map(|settings| settings.clone())
    }

    pub fn update(&self, settings: AppSettings) -> Result<AppSettings, SettingsError> {
        settings.validate()?;
        self.repository.save(&settings)?;
        *self
            .current
            .write()
            .map_err(|_| SettingsError::LockPoisoned)? = settings.clone();
        Ok(settings)
    }

    pub fn reset(&self) -> Result<AppSettings, SettingsError> {
        self.update(AppSettings::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::ModelRef;

    #[test]
    fn settings_round_trip_without_a_database() {
        let path = std::env::temp_dir().join(format!(
            "meetnote-settings-{}-{}.json",
            std::process::id(),
            unique_suffix()
        ));
        let repository = Arc::new(JsonSettingsRepository::open(&path));
        let service = SettingsService::open(repository).unwrap();
        let settings = AppSettings {
            asr_backend_id: Some("whisper-rs".into()),
            asr_model: Some(ModelRef {
                backend_id: "whisper-rs".into(),
                model_id: "whisper-base-en-ggml".into(),
                revision: None,
                artifact_path: "models/asr/base.bin".into(),
            }),
            ..AppSettings::default()
        };

        service.update(settings.clone()).unwrap();
        let reopened =
            SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();

        assert_eq!(reopened.snapshot().unwrap(), settings);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_zero_expected_speakers() {
        let mut settings = AppSettings::default();
        settings.expected_speakers = Some(0);
        assert!(matches!(
            settings.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("greater than zero")
        ));
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
