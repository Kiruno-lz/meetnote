use crate::NativeDiarizationError;
use contracts::{DiarizationStageConfiguration, ModelRef};
use std::path::{Path, PathBuf};

pub const REQUIRED_CPU_ARTIFACTS: &[&str] = &[
    "segmentation-3.0.onnx",
    "wespeaker-voxceleb-resnet34.onnx",
    "wespeaker-voxceleb-resnet34.onnx.data",
    "plda_lda.npy",
    "plda_tr.npy",
    "plda_mu.npy",
    "plda_psi.npy",
    "plda_mean1.npy",
    "plda_mean2.npy",
    "wespeaker-voxceleb-resnet34.min_num_samples.txt",
];

#[derive(Debug, Clone)]
pub struct NativeModelBundle {
    root: PathBuf,
}

impl NativeModelBundle {
    pub fn from_model(model: &ModelRef) -> Result<Self, NativeDiarizationError> {
        if model.backend_id != "diarization-native" {
            return Err(NativeDiarizationError::ModelIncompatible(format!(
                "model backend `{}` does not match diarization-native",
                model.backend_id
            )));
        }
        let bundle = Self {
            root: PathBuf::from(&model.artifact_path),
        };
        if !bundle.root.is_dir() {
            return Err(NativeDiarizationError::ModelMissing(format!(
                "model bundle directory does not exist: {}",
                bundle.root.display()
            )));
        }
        Ok(bundle)
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn probe_cpu(&self) -> Result<(), NativeDiarizationError> {
        if !self.root.is_dir() {
            return Err(NativeDiarizationError::ModelMissing(format!(
                "model bundle directory does not exist: {}",
                self.root.display()
            )));
        }
        for filename in REQUIRED_CPU_ARTIFACTS {
            let path = self.root.join(filename);
            if !path.is_file() {
                return Err(NativeDiarizationError::ModelMissing(format!(
                    "required model artifact does not exist: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

pub(crate) struct PreparedNativeModelBundle {
    bundle: NativeModelBundle,
    temporary_root: Option<PathBuf>,
}

impl PreparedNativeModelBundle {
    pub(crate) fn root(&self) -> &Path {
        self.bundle.root()
    }

    pub(crate) fn probe_cpu(&self) -> Result<(), NativeDiarizationError> {
        self.bundle.probe_cpu()
    }
}

impl Drop for PreparedNativeModelBundle {
    fn drop(&mut self) {
        if let Some(root) = &self.temporary_root {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

pub(crate) fn prepare_model_bundle(
    model: &ModelRef,
    configuration: &DiarizationStageConfiguration,
) -> Result<PreparedNativeModelBundle, NativeDiarizationError> {
    let base = NativeModelBundle::from_model(model)?;
    if configuration.is_empty() {
        return Ok(PreparedNativeModelBundle {
            bundle: base,
            temporary_root: None,
        });
    }

    let root = std::env::temp_dir().join(format!("meetnote-native-stage-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).map_err(|error| {
        NativeDiarizationError::ModelMissing(format!(
            "failed to create temporary native stage bundle {}: {error}",
            root.display()
        ))
    })?;

    let result = (|| {
        for filename in REQUIRED_CPU_ARTIFACTS {
            link_file(&base.root().join(filename), &root.join(filename))?;
        }
        if let Some(segmentation) = &configuration.segmentation_model {
            let source = stage_file(segmentation, "segmentation-3.0.onnx");
            link_file(&source, &root.join("segmentation-3.0.onnx"))?;
        }
        if let Some(embedding) = &configuration.embedding_model {
            for filename in [
                "wespeaker-voxceleb-resnet34.onnx",
                "wespeaker-voxceleb-resnet34.onnx.data",
                "wespeaker-voxceleb-resnet34.min_num_samples.txt",
            ] {
                let source = stage_file(embedding, filename);
                link_file(&source, &root.join(filename))?;
            }
        }
        Ok::<(), NativeDiarizationError>(())
    })();

    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&root);
        return Err(error);
    }

    Ok(PreparedNativeModelBundle {
        bundle: NativeModelBundle::from_root(root.clone()),
        temporary_root: Some(root),
    })
}

fn stage_file(stage: &ModelRef, filename: &str) -> PathBuf {
    let path = PathBuf::from(&stage.artifact_path);
    if path.is_dir() {
        return path.join(filename);
    }
    if path.file_name().and_then(|name| name.to_str()) == Some(filename) {
        return path;
    }
    path.parent()
        .map(|parent| parent.join(filename))
        .unwrap_or(path)
}

fn link_file(source: &Path, destination: &Path) -> Result<(), NativeDiarizationError> {
    if !source.is_file() {
        return Err(NativeDiarizationError::ModelMissing(format!(
            "native diarization stage artifact does not exist: {}",
            source.display()
        )));
    }
    if destination.exists() {
        std::fs::remove_file(destination).map_err(|error| {
            NativeDiarizationError::ModelMissing(format!(
                "failed to replace native diarization stage artifact {}: {error}",
                destination.display()
            ))
        })?;
    }
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(source, destination).is_ok() {
            return Ok(());
        }
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_file(source, destination).is_ok() {
            return Ok(());
        }
    }
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| {
            NativeDiarizationError::ModelMissing(format!(
                "failed to prepare native diarization stage artifact {}: {error}",
                destination.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SPEAKRS_MODEL_ID;
    use contracts::{AlgorithmRef, DiarizationStageConfiguration};

    #[test]
    #[ignore = "requires explicit MEETNOTE_COREML_MODEL_ROOT; hashes actual installed files only, no inference"]
    fn benchmark_coreml_integrity_gate_with_explicit_installed_assets() {
        let root = std::env::var("MEETNOTE_COREML_MODEL_ROOT").expect("explicit model root");
        let catalog = models::ModelCatalog::from_path(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/models/catalog.json"),
        )
        .unwrap();
        let manager = models::ModelManager::new(catalog, root);
        let model = manager
            .model_ref(crate::SPEAKRS_COREML_MODEL_ID, "diarization-native")
            .unwrap();
        assert_eq!(model.artifacts.len(), 76);
        for index in 0..5 {
            let started = std::time::Instant::now();
            verify_coreml_execution_assets(&model, Path::new(&model.artifact_path)).unwrap();
            println!(
                "integrity-gate: sample={index} seconds={:.9}",
                started.elapsed().as_secs_f64()
            );
        }
    }

    #[test]
    fn probe_reports_the_first_missing_artifact() {
        let root = std::env::temp_dir().join(format!("meetnote-model-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        let error = NativeModelBundle::from_root(&root).probe_cpu().unwrap_err();

        assert!(matches!(error, NativeDiarizationError::ModelMissing(_)));
        assert!(error.to_string().contains("segmentation-3.0.onnx"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prepares_overridden_stage_files_without_copying_the_base_bundle() {
        let root = std::env::temp_dir().join(format!("meetnote-base-{}", uuid::Uuid::new_v4()));
        let alternate =
            std::env::temp_dir().join(format!("meetnote-stage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&alternate).unwrap();
        for filename in REQUIRED_CPU_ARTIFACTS {
            std::fs::write(root.join(filename), format!("base:{filename}")).unwrap();
        }
        std::fs::write(
            alternate.join("segmentation-3.0.onnx"),
            "alternate-segmentation",
        )
        .unwrap();
        std::fs::write(
            alternate.join("wespeaker-voxceleb-resnet34.onnx"),
            "alternate-embedding",
        )
        .unwrap();
        std::fs::write(
            alternate.join("wespeaker-voxceleb-resnet34.onnx.data"),
            "alternate-data",
        )
        .unwrap();
        std::fs::write(
            alternate.join("wespeaker-voxceleb-resnet34.min_num_samples.txt"),
            "1",
        )
        .unwrap();

        let model = ModelRef {
            backend_id: "diarization-native".into(),
            model_id: SPEAKRS_MODEL_ID.into(),
            revision: Some("test".into()),
            artifact_path: root.display().to_string(),
            artifacts: Vec::new(),
        };
        let configuration = DiarizationStageConfiguration {
            segmentation_model: Some(ModelRef {
                backend_id: "diarization-native".into(),
                model_id: "alternate-segmentation".into(),
                revision: None,
                artifact_path: alternate
                    .join("segmentation-3.0.onnx")
                    .display()
                    .to_string(),
                artifacts: Vec::new(),
            }),
            embedding_model: Some(ModelRef {
                backend_id: "diarization-native".into(),
                model_id: "alternate-embedding".into(),
                revision: None,
                artifact_path: alternate
                    .join("wespeaker-voxceleb-resnet34.onnx")
                    .display()
                    .to_string(),
                artifacts: Vec::new(),
            }),
            clustering_algorithm: Some(AlgorithmRef {
                id: "speakrs-ahc-plda-vbx".into(),
                revision: None,
                parameters: None,
            }),
        };
        let prepared = prepare_model_bundle(&model, &configuration).unwrap();
        assert_eq!(
            std::fs::read_to_string(prepared.root().join("segmentation-3.0.onnx")).unwrap(),
            "alternate-segmentation"
        );
        assert_eq!(
            std::fs::read_to_string(prepared.root().join("wespeaker-voxceleb-resnet34.onnx"))
                .unwrap(),
            "alternate-embedding"
        );
        prepared.probe_cpu().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(alternate).unwrap();
    }
}

/// Fixed compiled graphs must match their complete declared manifest before native loading.
/// File-presence probes remain separate; this executes once per new pipeline/session.
pub(crate) fn verify_coreml_execution_assets(
    model: &ModelRef,
    root: &Path,
) -> Result<(), NativeDiarizationError> {
    if model.model_id != crate::SPEAKRS_COREML_MODEL_ID {
        return Ok(());
    }
    let required = speakrs::ModelBundle::required_files(speakrs::ExecutionMode::CoreMl);
    let names = required
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let incompatible = |detail: &str| {
        NativeDiarizationError::ModelIncompatible(format!(
            "CoreML execution requires a complete direct-file SHA-256 manifest: {detail}"
        ))
    };
    if model.artifacts.len() != required.len() {
        return Err(incompatible("resolve the model through ModelManager"));
    }
    let mut expected = std::collections::BTreeMap::new();
    for artifact in &model.artifacts {
        let Some(checksum) = artifact.expected_sha256.as_deref() else {
            return Err(incompatible("an artifact is missing its SHA-256"));
        };
        if artifact.archive_member.is_some()
            || checksum.len() != 64
            || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !names.contains(artifact.filename.as_str())
        {
            return Err(incompatible(
                "unexpected artifact, archive checksum or invalid SHA-256",
            ));
        }
        if expected
            .insert(artifact.filename.as_str(), checksum)
            .is_some()
        {
            return Err(incompatible("duplicate artifact names"));
        }
    }
    for (filename, checksum) in expected {
        let path = root.join(filename);
        let observed = models::probe::file_sha256(&path).map_err(|error| {
            let message = format!("cannot verify model artifact {}: {error}", path.display());
            if error.kind() == std::io::ErrorKind::NotFound {
                NativeDiarizationError::ModelMissing(message)
            } else {
                NativeDiarizationError::ModelLoadFailed(message)
            }
        })?;
        if !observed.eq_ignore_ascii_case(checksum) {
            return Err(NativeDiarizationError::ModelLoadFailed(format!(
                "model artifact checksum mismatch: {} (expected {checksum}, observed {observed})",
                path.display()
            )));
        }
    }
    Ok(())
}
