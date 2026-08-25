use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const CATALOG_SCHEMA_VERSION: &str = "meetnote.model-catalog.v1";

pub mod download {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    pub enum DownloadError {
        #[error("model source is not an HTTP URL: {0}")]
        UnsupportedSource(String),
        #[error("model download request failed: {0}")]
        Request(#[from] ureq::Error),
        #[error("model download IO failed: {0}")]
        Io(#[from] std::io::Error),
        #[error("model checksum mismatch for {path}: expected {expected}, got {actual}")]
        ChecksumMismatch {
            path: String,
            expected: String,
            actual: String,
        },
    }

    pub fn download_file(
        source: &str,
        destination: impl AsRef<Path>,
        expected_sha256: Option<&str>,
        mut on_progress: impl FnMut(u64, Option<u64>),
    ) -> Result<(), DownloadError> {
        if !source.starts_with("https://") && !source.starts_with("http://") {
            return Err(DownloadError::UnsupportedSource(source.into()));
        }
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
                return Err(DownloadError::ChecksumMismatch {
                    path: partial.display().to_string(),
                    expected: expected.into(),
                    actual,
                });
            }
        }
        fs::rename(partial, destination)?;
        Ok(())
    }
}

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
    pub sha256: Option<String>,
    pub license: String,
    pub requires_auth: bool,
    pub status: String,
    #[serde(default)]
    pub artifact_files: Vec<String>,
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
