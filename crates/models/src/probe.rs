//! What the model manager learned the last time it checked a model: the
//! artifact probe of the installed files and the reason the last download or
//! re-probe failed.
//!
//! Both live in `<model_root>/<task>/<model-id>.probe.json`, beside the install
//! directory rather than in it: processing provenance hashes every file of a
//! bundle's install directory, and a backend probe failing after a download
//! deletes that directory while its failure must stay readable.

use crate::download::DownloadError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const PROBE_RECORD_SCHEMA_VERSION: &str = "meetnote.model-probe.v1";

/// The manager operation that probed a model or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelOperation {
    /// A download, including the artifact probe that follows it.
    Download,
    /// `ModelManager::reprobe` of an installed model.
    Reprobe,
}

/// Why a download or probe failed, in words a user can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ModelFailure {
    #[error("model source is not an HTTP URL: {url}")]
    UnsupportedSource { url: String },
    /// The request failed after the automatic retries.
    #[error("model download request failed: {message}")]
    Network { message: String },
    #[error("model file IO failed: {message}")]
    Io { message: String },
    #[error("model size mismatch for {path}: expected {expected}, got {actual}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("model checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("model archive error: {message}")]
    Archive { message: String },
    /// A candidate backend rejected the installed artifacts.
    #[error("backend probe failed for {backend_id}: {message}")]
    BackendProbe { backend_id: String, message: String },
}

impl ModelFailure {
    /// `None` for a cancellation, which is not a failure.
    pub(crate) fn from_download(error: &DownloadError) -> Option<Self> {
        Some(match error {
            DownloadError::Cancelled => return None,
            DownloadError::UnsupportedSource(url) => Self::UnsupportedSource { url: url.clone() },
            DownloadError::Request(error) => Self::Network {
                message: error.to_string(),
            },
            DownloadError::Io(error) => Self::Io {
                message: error.to_string(),
            },
            DownloadError::SizeMismatch {
                path,
                expected,
                actual,
            } => Self::SizeMismatch {
                path: path.clone(),
                expected: *expected,
                actual: *actual,
            },
            DownloadError::ChecksumMismatch {
                path,
                expected,
                actual,
            } => Self::ChecksumMismatch {
                path: path.clone(),
                expected: expected.clone(),
                actual: actual.clone(),
            },
            DownloadError::Archive(message) => Self::Archive {
                message: message.clone(),
            },
        })
    }
}

/// One artifact's checksum check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChecksum {
    pub filename: String,
    /// The catalog SHA-256; for an archive member it is the archive's.
    pub expected_sha256: Option<String>,
    #[serde(flatten)]
    pub result: ChecksumResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ChecksumResult {
    /// The file's SHA-256 equals the catalog value.
    Matched,
    /// The downloaded archive matched; the extracted member has no checksum
    /// of its own.
    ArchiveMatched,
    /// A re-probe cannot check an archive member again: its archive is not
    /// kept after extraction.
    ArchiveNotRechecked,
    /// The catalog declares no SHA-256 for the artifact.
    NotDeclared,
    Mismatched {
        actual_sha256: String,
    },
    Unreadable {
        message: String,
    },
}

impl ChecksumResult {
    /// The artifact was checked against the catalog and matched.
    pub fn verified(&self) -> bool {
        matches!(self, Self::Matched | Self::ArchiveMatched)
    }
}

/// One candidate backend's probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProbeResult {
    pub backend_id: String,
    #[serde(flatten)]
    pub result: BackendCheck,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum BackendCheck {
    Passed,
    Failed {
        message: String,
    },
    /// The manager has no `BackendProbe`, so no backend looked at the files.
    NotRegistered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum ProbeOutcome {
    Passed,
    /// The first checksum or backend failure.
    Failed {
        reason: ModelFailure,
    },
}

/// The artifact probe of a model's installed files: checksums first, then
/// every candidate backend. It does not replace the preflight before
/// processing or the backend's real load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRecord {
    pub kind: ModelOperation,
    /// When the probe started, in Unix milliseconds.
    pub probed_at_ms: u64,
    /// The catalog revision that was probed.
    pub revision: Option<String>,
    pub checksums: Vec<ArtifactChecksum>,
    pub backends: Vec<BackendProbeResult>,
    pub outcome: ProbeOutcome,
}

impl ProbeRecord {
    pub fn passed(&self) -> bool {
        self.outcome == ProbeOutcome::Passed
    }
}

/// The last failed download or re-probe of a model; a later successful one
/// removes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFailureRecord {
    pub operation: ModelOperation,
    /// Unix milliseconds.
    pub failed_at_ms: u64,
    pub revision: Option<String>,
    pub reason: ModelFailure,
}

/// A probe record file that exists but cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid model probe record {}: {reason}", path.display())]
pub struct ProbeRecordError {
    pub path: PathBuf,
    pub reason: String,
}

/// The contents of one model's probe record file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModelRecord {
    pub probe: Option<ProbeRecord>,
    pub failure: Option<ModelFailureRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRecord {
    schema_version: String,
    model_id: String,
    #[serde(default)]
    probe: Option<ProbeRecord>,
    #[serde(default)]
    failure: Option<ModelFailureRecord>,
}

/// An absent file is an empty record.
pub(crate) fn read_record(path: &Path, model_id: &str) -> Result<ModelRecord, ProbeRecordError> {
    let invalid = |reason: String| ProbeRecordError {
        path: path.to_path_buf(),
        reason,
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ModelRecord::default())
        }
        Err(error) => return Err(invalid(error.to_string())),
    };
    let stored: StoredRecord =
        serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
    if stored.schema_version != PROBE_RECORD_SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported schema version: {}",
            stored.schema_version
        )));
    }
    if stored.model_id != model_id {
        return Err(invalid(format!(
            "record belongs to model {}",
            stored.model_id
        )));
    }
    Ok(ModelRecord {
        probe: stored.probe,
        failure: stored.failure,
    })
}

/// Replaces the whole file atomically; an unreadable earlier file is
/// overwritten.
pub(crate) fn write_record(
    path: &Path,
    model_id: &str,
    record: &ModelRecord,
) -> std::io::Result<()> {
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
    let stored = StoredRecord {
        schema_version: PROBE_RECORD_SCHEMA_VERSION.into(),
        model_id: model_id.into(),
        probe: record.probe.clone(),
        failure: record.failure.clone(),
    };
    let content = serde_json::to_vec_pretty(&stored).map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!(
        "json.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temporary, content)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

pub(crate) fn file_sha256(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "meetnote-probe-record-{name}-{}-{}",
                std::process::id(),
                now_ms()
            ))
            .join("asr/fixture.probe.json")
    }

    fn failed_record() -> ModelRecord {
        ModelRecord {
            probe: Some(ProbeRecord {
                kind: ModelOperation::Reprobe,
                probed_at_ms: 1_700_000_000_000,
                revision: Some("fixture-v1".into()),
                checksums: vec![
                    ArtifactChecksum {
                        filename: "model.onnx".into(),
                        expected_sha256: Some("aa".into()),
                        result: ChecksumResult::Mismatched {
                            actual_sha256: "bb".into(),
                        },
                    },
                    ArtifactChecksum {
                        filename: "tokens.txt".into(),
                        expected_sha256: None,
                        result: ChecksumResult::NotDeclared,
                    },
                ],
                backends: vec![BackendProbeResult {
                    backend_id: "fake".into(),
                    result: BackendCheck::Failed {
                        message: "missing tokens".into(),
                    },
                }],
                outcome: ProbeOutcome::Failed {
                    reason: ModelFailure::ChecksumMismatch {
                        path: "/models/asr/fixture/model.onnx".into(),
                        expected: "aa".into(),
                        actual: "bb".into(),
                    },
                },
            }),
            failure: Some(ModelFailureRecord {
                operation: ModelOperation::Reprobe,
                failed_at_ms: 1_700_000_000_000,
                revision: Some("fixture-v1".into()),
                reason: ModelFailure::BackendProbe {
                    backend_id: "fake".into(),
                    message: "missing tokens".into(),
                },
            }),
        }
    }

    #[test]
    fn the_record_file_format_is_pinned() {
        let path = temp_path("format");
        write_record(&path, "fixture", &failed_record()).unwrap();

        let written: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

        assert_eq!(
            written,
            serde_json::json!({
                "schemaVersion": "meetnote.model-probe.v1",
                "modelId": "fixture",
                "probe": {
                    "kind": "reprobe",
                    "probedAtMs": 1_700_000_000_000_u64,
                    "revision": "fixture-v1",
                    "checksums": [
                        {"filename": "model.onnx", "expectedSha256": "aa",
                         "result": "mismatched", "actualSha256": "bb"},
                        {"filename": "tokens.txt", "expectedSha256": null,
                         "result": "not-declared"}
                    ],
                    "backends": [
                        {"backendId": "fake", "result": "failed", "message": "missing tokens"}
                    ],
                    "outcome": {
                        "result": "failed",
                        "reason": {"kind": "checksum-mismatch",
                                   "path": "/models/asr/fixture/model.onnx",
                                   "expected": "aa", "actual": "bb"}
                    }
                },
                "failure": {
                    "operation": "reprobe",
                    "failedAtMs": 1_700_000_000_000_u64,
                    "revision": "fixture-v1",
                    "reason": {"kind": "backend-probe", "backendId": "fake",
                               "message": "missing tokens"}
                }
            })
        );
        assert_eq!(read_record(&path, "fixture").unwrap(), failed_record());
        fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn a_missing_file_is_an_empty_record_and_an_invalid_one_reports_its_path() {
        let path = temp_path("invalid");
        assert_eq!(
            read_record(&path, "fixture").unwrap(),
            ModelRecord::default()
        );

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{not json").unwrap();
        let error = read_record(&path, "fixture").unwrap_err();
        assert_eq!(error.path, path);
        assert!(error.to_string().contains("fixture.probe.json"), "{error}");

        write_record(&path, "other", &failed_record()).unwrap();
        let error = read_record(&path, "fixture").unwrap_err();
        assert_eq!(error.reason, "record belongs to model other");

        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["schemaVersion"] = "meetnote.model-probe.v0".into();
        stored["modelId"] = "fixture".into();
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let error = read_record(&path, "fixture").unwrap_err();
        assert!(error.reason.contains("unsupported schema version"));
        fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn failures_read_as_actionable_sentences() {
        assert_eq!(
            ModelFailure::BackendProbe {
                backend_id: "sherpa-onnx".into(),
                message: "tokens.txt is missing".into(),
            }
            .to_string(),
            "backend probe failed for sherpa-onnx: tokens.txt is missing"
        );
        assert_eq!(
            ModelFailure::from_download(&DownloadError::ChecksumMismatch {
                path: "/m/model.bin.part".into(),
                expected: "aa".into(),
                actual: "bb".into(),
            })
            .unwrap()
            .to_string(),
            "model checksum mismatch for /m/model.bin.part: expected aa, got bb"
        );
        assert_eq!(ModelFailure::from_download(&DownloadError::Cancelled), None);
    }
}
