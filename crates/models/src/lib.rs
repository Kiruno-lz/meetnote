use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const CATALOG_SCHEMA_VERSION: &str = "meetnote.model-catalog.v1";

pub mod download {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    #[derive(Debug, thiserror::Error)]
    pub enum DownloadError {
        #[error("model source is not an HTTP URL: {0}")]
        UnsupportedSource(String),
        #[error("model download request failed: {0}")]
        Request(#[source] Box<ureq::Error>),
        #[error("model download IO failed: {0}")]
        Io(#[from] std::io::Error),
        #[error("model archive error: {0}")]
        Archive(String),
        #[error("model checksum mismatch for {path}: expected {expected}, got {actual}")]
        ChecksumMismatch {
            path: String,
            expected: String,
            actual: String,
        },
        #[error("model download cancelled")]
        Cancelled,
    }

    impl From<ureq::Error> for DownloadError {
        fn from(error: ureq::Error) -> Self {
            Self::Request(Box::new(error))
        }
    }

    #[derive(Clone, Default)]
    pub struct DownloadControl {
        paused: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
        wait_lock: Arc<Mutex<()>>,
        wait_condvar: Arc<Condvar>,
    }

    impl DownloadControl {
        pub fn pause(&self) {
            self.paused.store(true, Ordering::Release);
        }

        pub fn resume(&self) {
            self.paused.store(false, Ordering::Release);
            self.wait_condvar.notify_all();
        }

        pub fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
            self.wait_condvar.notify_all();
        }

        pub fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }

        pub(crate) fn checkpoint(&self) -> Result<(), DownloadError> {
            if self.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            if self.paused.load(Ordering::Acquire) {
                let guard = self
                    .wait_lock
                    .lock()
                    .map_err(|_| std::io::Error::other("download control lock poisoned"))?;
                let _guard = self
                    .wait_condvar
                    .wait_while(guard, |_| {
                        self.paused.load(Ordering::Acquire)
                            && !self.cancelled.load(Ordering::Acquire)
                    })
                    .map_err(|_| std::io::Error::other("download control lock poisoned"))?;
            }
            if self.is_cancelled() {
                Err(DownloadError::Cancelled)
            } else {
                Ok(())
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct BundleArtifact {
        pub filename: String,
        pub source: String,
        pub expected_sha256: Option<String>,
        pub archive_member: Option<String>,
    }

    pub fn download_file(
        source: &str,
        destination: impl AsRef<Path>,
        expected_sha256: Option<&str>,
        on_progress: impl FnMut(u64, Option<u64>),
    ) -> Result<(), DownloadError> {
        download_file_controlled(
            source,
            destination,
            expected_sha256,
            &DownloadControl::default(),
            on_progress,
        )
    }

    pub fn download_file_controlled(
        source: &str,
        destination: impl AsRef<Path>,
        expected_sha256: Option<&str>,
        control: &DownloadControl,
        mut on_progress: impl FnMut(u64, Option<u64>),
    ) -> Result<(), DownloadError> {
        if !source.starts_with("https://") && !source.starts_with("http://") {
            return Err(DownloadError::UnsupportedSource(source.into()));
        }
        control.checkpoint()?;
        let destination = destination.as_ref();
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let partial = destination.with_extension("part");
        let existing = partial
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let mut request = ureq::get(source);
        if existing > 0 {
            request = request.set("Range", &format!("bytes={existing}-"));
        }
        let response = request.call()?;
        let append = existing > 0 && response.status() == 206;
        let offset = if append { existing } else { 0 };
        let total = response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|length| length + offset);
        let mut output = if append {
            OpenOptions::new().append(true).open(&partial)?
        } else {
            File::create(&partial)?
        };
        let mut hash = Sha256::new();
        if append {
            let mut previous = File::open(&partial)?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = previous.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hash.update(&buffer[..read]);
            }
        }
        let mut reader = response.into_reader();
        let mut downloaded = offset;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            control.checkpoint()?;
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            hash.update(&buffer[..read]);
            downloaded += read as u64;
            on_progress(downloaded, total);
        }
        output.flush()?;
        let actual = format!("{:x}", hash.finalize());
        if let Some(expected) = expected_sha256 {
            if !actual.eq_ignore_ascii_case(expected) {
                let _ = fs::remove_file(&partial);
                return Err(DownloadError::ChecksumMismatch {
                    path: partial.display().to_string(),
                    expected: expected.into(),
                    actual,
                });
            }
        }
        control.checkpoint()?;
        fs::rename(partial, destination)?;
        Ok(())
    }

    pub fn download_bundle(
        artifacts: &[BundleArtifact],
        destination: impl AsRef<Path>,
        on_progress: impl FnMut(usize, usize),
    ) -> Result<(), DownloadError> {
        download_bundle_controlled(
            artifacts,
            destination,
            &DownloadControl::default(),
            on_progress,
        )
    }

    pub fn download_bundle_controlled(
        artifacts: &[BundleArtifact],
        destination: impl AsRef<Path>,
        control: &DownloadControl,
        mut on_progress: impl FnMut(usize, usize),
    ) -> Result<(), DownloadError> {
        let destination = destination.as_ref();
        fs::create_dir_all(destination)?;
        for (index, artifact) in artifacts.iter().enumerate() {
            control.checkpoint()?;
            let target = destination.join(&artifact.filename);
            if target.is_file() {
                on_progress(index + 1, artifacts.len());
                continue;
            }
            if let Some(member) = &artifact.archive_member {
                let archive_path = destination.join(format!("{}.archive.part", artifact.filename));
                download_file_controlled(
                    &artifact.source,
                    &archive_path,
                    artifact.expected_sha256.as_deref(),
                    control,
                    |_, _| {},
                )?;
                extract_archive_member(&archive_path, member, &target)?;
                fs::remove_file(archive_path)?;
            } else {
                download_file_controlled(
                    &artifact.source,
                    &target,
                    artifact.expected_sha256.as_deref(),
                    control,
                    |_, _| {},
                )?;
            }
            on_progress(index + 1, artifacts.len());
        }
        Ok(())
    }

    pub fn download_bundle_with_progress(
        artifacts: &[BundleArtifact],
        destination: impl AsRef<Path>,
        control: &DownloadControl,
        mut on_progress: impl FnMut(usize, usize, u64, Option<u64>),
    ) -> Result<(), DownloadError> {
        let destination = destination.as_ref();
        fs::create_dir_all(destination)?;
        for (index, artifact) in artifacts.iter().enumerate() {
            control.checkpoint()?;
            let target = destination.join(&artifact.filename);
            if target.is_file() {
                on_progress(index + 1, artifacts.len(), 0, None);
                continue;
            }
            let mut report = |completed: u64, total: Option<u64>| {
                on_progress(index, artifacts.len(), completed, total)
            };
            if let Some(member) = &artifact.archive_member {
                let archive_path = destination.join(format!("{}.archive.part", artifact.filename));
                download_file_controlled(
                    &artifact.source,
                    &archive_path,
                    artifact.expected_sha256.as_deref(),
                    control,
                    &mut report,
                )?;
                extract_archive_member(&archive_path, member, &target)?;
                fs::remove_file(archive_path)?;
            } else {
                download_file_controlled(
                    &artifact.source,
                    &target,
                    artifact.expected_sha256.as_deref(),
                    control,
                    &mut report,
                )?;
            }
            on_progress(index + 1, artifacts.len(), 0, None);
        }
        Ok(())
    }

    fn extract_archive_member(
        archive_path: &Path,
        member: &str,
        destination: &Path,
    ) -> Result<(), DownloadError> {
        let input = File::open(archive_path)?;
        let decoder = bzip2::read::BzDecoder::new(input);
        let mut archive = tar::Archive::new(decoder);
        let temporary = destination.with_extension("part");
        let mut found = false;
        for entry in archive
            .entries()
            .map_err(|error| DownloadError::Archive(error.to_string()))?
        {
            let mut entry = entry.map_err(|error| DownloadError::Archive(error.to_string()))?;
            let path = entry
                .path()
                .map_err(|error| DownloadError::Archive(error.to_string()))?;
            if path.to_string_lossy().ends_with(member) {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut output = File::create(&temporary)?;
                std::io::copy(&mut entry, &mut output)?;
                output.flush()?;
                fs::rename(&temporary, destination)?;
                found = true;
                break;
            }
        }
        if found {
            Ok(())
        } else {
            Err(DownloadError::Archive(format!(
                "archive member not found: {member}"
            )))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn extracts_a_named_bundle_member() {
            let root = std::env::temp_dir().join(format!("meetnote-bundle-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let archive_path = root.join("fixture.tar.bz2");
            let file = File::create(&archive_path).unwrap();
            let encoder = bzip2::write::BzEncoder::new(file, bzip2::Compression::fast());
            let mut archive = tar::Builder::new(encoder);
            let payload = b"onnx-fixture";
            let mut header = tar::Header::new_gnu();
            header.set_path("model-root/model.onnx").unwrap();
            header.set_size(payload.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append(&header, &payload[..]).unwrap();
            archive.into_inner().unwrap().finish().unwrap();

            let destination = root.join("segmentation.onnx");
            extract_archive_member(&archive_path, "model.onnx", &destination).unwrap();
            assert_eq!(fs::read(&destination).unwrap(), payload);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

pub mod manager;

pub use manager::ModelManager;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub schema_version: String,
    pub models: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogEntry {
    pub id: String,
    pub task: String,
    pub model_family: String,
    pub candidate_backends: Vec<String>,
    pub source_format: String,
    pub source: String,
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub estimated_vram_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub license: String,
    pub requires_auth: bool,
    pub status: String,
    #[serde(default)]
    pub artifact_files: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<CatalogArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogArtifact {
    pub filename: String,
    pub source: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub archive_member: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelInstallState {
    Missing,
    Partial,
    Installed,
}

#[derive(Debug, Clone)]
pub struct ModelAvailability {
    pub model: ModelCatalogEntry,
    pub install_state: ModelInstallState,
    pub artifact_paths: Vec<std::path::PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("failed to read model catalog {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse model catalog: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("invalid model catalog: {0}")]
    Invalid(String),
}

impl ModelCatalog {
    pub fn parse(input: &str) -> Result<Self, CatalogError> {
        let catalog: Self = serde_json::from_str(input)?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let path = path.as_ref();
        let input = std::fs::read_to_string(path).map_err(|source| CatalogError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&input)
    }

    pub fn find(&self, model_id: &str) -> Option<&ModelCatalogEntry> {
        self.models.iter().find(|model| model.id == model_id)
    }

    pub fn browse(
        &self,
        model_root: impl AsRef<Path>,
        task: Option<&str>,
        backend_id: Option<&str>,
    ) -> Vec<ModelAvailability> {
        self.models
            .iter()
            .filter(|model| task.is_none_or(|task| model.task == task))
            .filter(|model| backend_id.is_none_or(|backend| model.supports_backend(backend)))
            .map(|model| model.availability(model_root.as_ref()))
            .collect()
    }

    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.schema_version != CATALOG_SCHEMA_VERSION {
            return Err(CatalogError::Invalid(format!(
                "unsupported schema version: {}",
                self.schema_version
            )));
        }
        if self.models.is_empty() {
            return Err(CatalogError::Invalid("models must not be empty".into()));
        }

        let mut ids = HashSet::new();
        for model in &self.models {
            if model.id.trim().is_empty() {
                return Err(CatalogError::Invalid("model id must not be empty".into()));
            }
            if !ids.insert(&model.id) {
                return Err(CatalogError::Invalid(format!(
                    "duplicate model id: {}",
                    model.id
                )));
            }
            if model.source.trim().is_empty() {
                return Err(CatalogError::Invalid(format!(
                    "model source is empty: {}",
                    model.id
                )));
            }
            if model.license.trim().is_empty() {
                return Err(CatalogError::Invalid(format!(
                    "model license is empty: {}",
                    model.id
                )));
            }
            if model.status == "installed" && model.sha256.is_none() {
                return Err(CatalogError::Invalid(format!(
                    "installed model has no sha256: {}",
                    model.id
                )));
            }
        }
        Ok(())
    }
}

impl ModelCatalogEntry {
    pub fn supports_backend(&self, backend_id: &str) -> bool {
        self.candidate_backends
            .iter()
            .any(|candidate| candidate == backend_id)
    }

    pub fn install_directory(&self, model_root: impl AsRef<Path>) -> std::path::PathBuf {
        model_root.as_ref().join(&self.task).join(&self.id)
    }

    pub fn artifact_filenames(&self) -> Vec<String> {
        if !self.artifacts.is_empty() {
            return self
                .artifacts
                .iter()
                .map(|artifact| artifact.filename.clone())
                .collect();
        }
        let filename = self
            .source
            .split('?')
            .next()
            .and_then(|source| source.rsplit('/').next())
            .filter(|filename| !filename.is_empty())
            .unwrap_or("model.bin");
        vec![filename.to_string()]
    }

    pub fn artifact_paths(&self, model_root: impl AsRef<Path>) -> Vec<std::path::PathBuf> {
        let directory = self.install_directory(model_root);
        self.artifact_filenames()
            .into_iter()
            .map(|filename| directory.join(filename))
            .collect()
    }

    pub fn availability(&self, model_root: impl AsRef<Path>) -> ModelAvailability {
        let artifact_paths = self.artifact_paths(model_root);
        let present = artifact_paths.iter().filter(|path| path.is_file()).count();
        let install_state = if present == 0 {
            ModelInstallState::Missing
        } else if present == artifact_paths.len() {
            ModelInstallState::Installed
        } else {
            ModelInstallState::Partial
        };
        ModelAvailability {
            model: self.clone(),
            install_state,
            artifact_paths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use download::DownloadError;

    #[test]
    fn loads_the_repository_catalog() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/models/catalog.json");
        let catalog = ModelCatalog::from_path(path).unwrap();

        assert!(catalog
            .models
            .iter()
            .any(|model| model.id == "whisper-large-v3"));
        assert!(catalog
            .models
            .iter()
            .any(|model| model.task == "diarization"));
    }

    #[test]
    fn native_diarization_bundle_pins_every_required_artifact() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/models/catalog.json");
        let catalog = ModelCatalog::from_path(path).unwrap();
        let model = catalog.find("speakrs-community-1-cpu").unwrap();

        assert_eq!(model.candidate_backends, vec!["diarization-native"]);
        assert_eq!(model.artifact_files.len(), 10);
        assert_eq!(model.artifacts.len(), 10);
        assert!(model.artifacts.iter().all(|artifact| {
            artifact
                .sha256
                .as_ref()
                .is_some_and(|checksum| checksum.len() == 64)
        }));
    }

    #[test]
    fn browse_uses_catalog_backend_compatibility() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/models/catalog.json");
        let catalog = ModelCatalog::from_path(path).unwrap();

        let whisper_models =
            catalog.browse("/tmp/meetnote-models", Some("asr"), Some("whisper-rs"));
        assert!(!whisper_models.is_empty());
        assert!(whisper_models
            .iter()
            .all(|availability| availability.model.supports_backend("whisper-rs")));
        assert!(whisper_models
            .iter()
            .all(|availability| availability.model.source_format == "ggml"));

        let native_models = catalog.browse(
            "/tmp/meetnote-models",
            Some("diarization"),
            Some("diarization-native"),
        );
        assert_eq!(native_models.len(), 1);
        assert_eq!(native_models[0].model.id, "speakrs-community-1-cpu");
    }

    #[test]
    fn rejects_non_http_model_sources_before_writing() {
        let result = download::download_file(
            "local/model.bin",
            "/tmp/meetnote-model.bin",
            None,
            |_, _| {},
        );

        assert!(matches!(
            result,
            Err(DownloadError::UnsupportedSource(source)) if source == "local/model.bin"
        ));
    }
}
