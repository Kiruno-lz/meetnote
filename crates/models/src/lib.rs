use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

const CATALOG_SCHEMA_VERSION: &str = "meetnote.model-catalog.v1";

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
}
