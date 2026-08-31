use crate::NativeDiarizationError;
use contracts::ModelRef;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reports_the_first_missing_artifact() {
        let root = std::env::temp_dir().join(format!("meetnote-model-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        let error = NativeModelBundle::from_root(&root).probe_cpu().unwrap_err();

        assert!(matches!(error, NativeDiarizationError::ModelMissing(_)));
        assert!(error.to_string().contains("segmentation-3.0.onnx"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
